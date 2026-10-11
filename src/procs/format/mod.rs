//! PROC FORMAT (jalon M4).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc format ; value sexfmt 1='Male' 2='Female' other='?' ;
//! value $cityfmt 'PAR'='Paris' ; run ;`
//!
//! - Parser chaque statement VALUE en `formats::userdef::UserFormat`
//!   (plages : valeur, `a-b`, `low-<b`, `a<-high`, listes virgule).
//! - Enregistrer dans `session.format_catalog` (nom upcase, `$` inclus
//!   pour les formats char). NOTE par format : "Format SEXFMT has been
//!   output." — en session seulement, pas de catalogue persistant
//!   (limitation documentée dans README).
//! - INVALUE (informats utilisateur) : M4+, ERROR propre d'ici là.
//!
//! ## Naming convention
//! Format names are registered WITHOUT a leading `$` transformation beyond
//! what the user writes. The `$` prefix is kept as part of the name, e.g.
//! `$CITYFMT`. `FormatCatalog::define` upcases the whole string, so the
//! stored key is `$CITYFMT`. When `FormatSpec::parse` sees `$CITYFMT.` it
//! produces `name="$CITYFMT"`, which matches the catalog key exactly.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::formats::userdef::{
    Bound, InformatRange, InformatValue, PictureDirectives, PictureRange, Range, UserFormat,
    UserInformat, UserPicture,
};
use crate::parser::StatementStream;
use crate::session::Session;
use crate::token::TokenKind;

mod cntl;
mod fmtlib;
mod invalue;
mod picture;
mod value;

use fmtlib::render_fmtlib;
use invalue::*;
use picture::*;
use value::*;

pub struct FormatAst {
    /// M39.1 — `LIBRARY=`/`LIB=<libref>` (UPPERCASE, défaut `"WORK"`) : cible
    /// du catalogue. `"WORK"` = comportement historique, purement en mémoire.
    /// Tout autre libref persiste un sidecar JSON après RUN (voir `execute`).
    pub lib: String,
    /// (nom, définition brute à parser en UserFormat)
    pub values: Vec<(String, UserFormat)>,
    /// (nom, définition brute à parser en UserInformat) — M18.2
    pub invalues: Vec<(String, UserInformat)>,
    /// (nom, définition brute à parser en UserPicture) — M18.3
    pub pictures: Vec<(String, UserPicture)>,
    /// M39.2 — `CNTLOUT=<ds>` : dépose le catalogue résultant (après ce step)
    /// dans un dataset. `None` = pas demandé.
    pub cntlout: Option<DatasetRef>,
    /// M39.2 — `CNTLIN=<ds>` : (re)définit des formats/informats depuis un
    /// dataset de contrôle, appliqué AVANT les sous-statements VALUE/INVALUE
    /// de ce même step (qui peuvent donc l'écraser). `None` = pas demandé.
    pub cntlin: Option<DatasetRef>,
    /// M39.3 — `FMTLIB` : option d'en-tête sans valeur. Après RUN, liste dans
    /// le listing le contenu (VALUE/PICTURE/INVALUE) du catalogue CIBLÉ par
    /// `lib` (WORK par défaut), tel qu'il est APRÈS ce step (donc après
    /// CNTLIN= et les sous-statements). Voir `render_fmtlib`.
    pub fmtlib: bool,
}

/// Parse `proc format [library=<libref>] ; value ... ; [value ... ;] run;`
/// Called AFTER "proc format" has been consumed. Consumes through `run;`/`quit;`.
///
/// ## M39.1 — `LIBRARY=`/`LIB=`
/// `proc format library=<libref>;` targets a NON-default catalog: the libref
/// must be a simple one-level name (`LIBRARY=work`, `LIBRARY=perm`, default
/// `WORK` when omitted). Real SAS also accepts a two-level catalog name
/// (`LIBRARY=libref.catalog`, e.g. `work.formats2`) to pick a NAMED catalog
/// inside the libref — this build stores exactly one catalog per libref
/// (`formats.sascat.json` at the libref's root, see `formats::mod`), so a
/// two-level name is a clean deferral (ERROR), not silently ignored or
/// misrouted.
pub fn parse(ts: &mut StatementStream) -> Result<FormatAst> {
    let mut lib = "WORK".to_string();
    let mut cntlout: Option<DatasetRef> = None;
    let mut cntlin: Option<DatasetRef> = None;
    let mut fmtlib = false;
    // J02-P8 — MAXLABLEN=/MAXSELEN= seen in the header (option name, span).
    let mut max_len_option: Option<(&'static str, crate::token::Span)> = None;
    // Consume the trailing `;` of the `proc format` statement header,
    // recognising `LIBRARY=`/`LIB=`/`CNTLOUT=`/`CNTLIN=`/`FMTLIB` along the
    // way. J02-P8 — the other options used to be skipped token by token: the
    // other SAS 9.4 options are diagnosed (`header_option_contract`), an
    // unknown option is an ERROR « Unexpected option ».
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("cntlout") {
            ts.next(); // consume "cntlout"
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse(
                    "expected '=' after CNTLOUT in PROC FORMAT",
                    ts.peek().span,
                ));
            }
            ts.next(); // consume '='
            cntlout = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("cntlin") {
            ts.next(); // consume "cntlin"
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse(
                    "expected '=' after CNTLIN in PROC FORMAT",
                    ts.peek().span,
                ));
            }
            ts.next(); // consume '='
            cntlin = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("lib") || ts.peek().is_kw("library") {
            ts.next(); // consume "lib"/"library"
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse(
                    "expected '=' after LIBRARY in PROC FORMAT",
                    ts.peek().span,
                ));
            }
            ts.next(); // consume '='
            let tok = ts.peek().clone();
            let Some(name) = tok.ident().map(str::to_string) else {
                return Err(SasError::parse(
                    "expected a libref after LIBRARY= in PROC FORMAT",
                    tok.span,
                ));
            };
            ts.next(); // consume the libref
            if ts.peek().kind == TokenKind::Dot {
                return Err(SasError::parse(
                    format!(
                        "PROC FORMAT LIBRARY={}.<catalog> (two-level catalog name) \
                         is not supported in this build; use a one-level libref \
                         (LIBRARY={}) — one catalog per libref.",
                        name.to_uppercase(),
                        name.to_uppercase()
                    ),
                    ts.peek().span,
                ));
            }
            lib = name.to_uppercase();
        } else if ts.peek().is_kw("fmtlib") {
            ts.next(); // consume "fmtlib" — bare option, no value.
            fmtlib = true;
        } else if !header_option_contract(ts, &mut max_len_option)? {
            return Err(crate::procs::common::unknown_option_error(ts, "FORMAT"));
        }
    }
    // MAXLABLEN=/MAXSELEN= set how many characters of the labels / of the
    // start and end values appear « in the CNTLOUT= data set or in the
    // output of the FMTLIB option » : a data set with CNTLOUT= (ERROR),
    // display only otherwise (WARNING).
    if let Some((opt, span)) = max_len_option {
        if cntlout.is_some() {
            return Err(SasError::parse(
                format!(
                    "The {opt} option is not supported in PROC FORMAT with CNTLOUT=; it can \
                     affect results and cannot be ignored."
                ),
                span,
            ));
        }
        ts.warn_ignored_display(format!(
            "The {opt} option is ignored in PROC FORMAT; display customization is not \
             supported."
        ));
    }

    let mut values: Vec<(String, UserFormat)> = Vec::new();
    let mut invalues: Vec<(String, UserInformat)> = Vec::new();
    let mut pictures: Vec<(String, UserPicture)> = Vec::new();

    crate::procs::common::parse_proc_body(ts, "FORMAT", |ts, kw| {
        // J02-P8 — SELECT and EXCLUDE (SAS 9.4 PROC FORMAT statements) choose
        // the entries written by CNTLOUT= and listed by FMTLIB; they used to
        // be reported « 180-322 … not valid ».
        if kw == "select" || kw == "exclude" {
            return Err(SasError::parse(
                format!(
                    "The {} statement is not supported in PROC FORMAT; it can affect results \
                     and cannot be ignored.",
                    kw.to_ascii_uppercase()
                ),
                ts.peek().span,
            ));
        }
        if ts.peek().is_kw("value") {
            ts.next(); // consume "value"
            let (name, uf) = parse_value_stmt(ts)?;
            values.push((name, uf));
        } else if ts.peek().is_kw("invalue") {
            ts.next(); // consume "invalue"
            let (name, ui) = parse_invalue_stmt(ts)?;
            invalues.push((name, ui));
        } else if ts.peek().is_kw("picture") {
            ts.next(); // consume "picture"
            let (name, up) = parse_picture_stmt(ts)?;
            pictures.push((name, up));
        } else {
            return Ok(false);
        }
        Ok(true)
    })?;

    Ok(FormatAst {
        lib,
        values,
        invalues,
        pictures,
        cntlout,
        cntlin,
        fmtlib,
    })
}

/// J02-P8 — the SAS 9.4 PROC FORMAT options that sasrs does not implement
/// (Base SAS 9.4 Procedures Guide, PROC FORMAT statement) ; they used to be
/// skipped token by token. Returns `Ok(false)` without consuming anything
/// for an option that is not one of them. NOREPLACE (« prevents a new
/// informat or format from replacing an existing one ») and LOCALE (catalog
/// named after the locale) change which definitions are stored → ERROR ;
/// PAGE (FMTLIB layout) is display only → WARNING ; MAXLABLEN=/MAXSELEN= are
/// recorded in `max_len` and diagnosed once the header is read (their
/// diagnostic depends on CNTLOUT=).
fn header_option_contract(
    ts: &mut StatementStream,
    max_len: &mut Option<(&'static str, crate::token::Span)>,
) -> Result<bool> {
    let span = ts.peek().span;
    let kw = ts.peek().ident().unwrap_or("").to_ascii_lowercase();
    match kw.as_str() {
        "noreplace" | "locale" => Err(SasError::parse(
            format!(
                "The {} option is not supported in PROC FORMAT; it can affect results and \
                 cannot be ignored.",
                kw.to_ascii_uppercase()
            ),
            span,
        )),
        "page" => {
            ts.next();
            ts.warn_ignored_display(
                "The PAGE option is ignored in PROC FORMAT; display customization is not \
                 supported."
                    .to_string(),
            );
            Ok(true)
        }
        "maxlablen" | "maxselen" => {
            let opt = if kw == "maxlablen" {
                "MAXLABLEN="
            } else {
                "MAXSELEN="
            };
            crate::procs::common::consume_option_eq(ts, &opt[..opt.len() - 1])?;
            if !matches!(ts.peek().kind, TokenKind::Num(_)) {
                return Err(SasError::parse(
                    format!("expected a number of characters after {opt} in PROC FORMAT"),
                    ts.peek().span,
                ));
            }
            ts.next();
            *max_len = Some((opt, span));
            Ok(true)
        }
        _ => Ok(false),
    }
}

pub fn execute(ast: &FormatAst, session: &mut Session) -> Result<()> {
    // M39.2 — `CNTLIN=<ds>` : reconstruit des entrées VALUE/INVALUE depuis un
    // dataset de contrôle AVANT les sous-statements VALUE/INVALUE de ce même
    // step, pour qu'un `value`/`invalue` explicite du step garde le dernier
    // mot (CNTLIN= se comporte comme une option d'en-tête — un préchargement
    // — pas comme un sous-statement séquentiel). Lu ici (avant le
    // `Rc::make_mut` ci-dessous) car `read_cntlin` a besoin d'un accès large à
    // `session` (lecture de dataset via `session.libs`).
    let (cntlin_values, cntlin_invalues) = match &ast.cntlin {
        Some(cntlin_ref) => cntl::read_cntlin(cntlin_ref, session)?,
        None => (Vec::new(), Vec::new()),
    };

    // MQ9.8 — le catalogue est partagé par `Rc` (les étapes DATA le lisent
    // sans le copier). PROC FORMAT est le SEUL à le muter : `make_mut` clone
    // ici, et seulement ici, s'il reste des lecteurs.
    //
    // M39.1 : que LIBRARY= vise WORK ou un libref permanent, les nouvelles
    // définitions vont TOUJOURS dans `session.format_catalog` — c'est ce qui
    // les rend résolubles immédiatement dans CETTE session, WORK y compris.
    // Un libref non-WORK reçoit EN PLUS une copie dans
    // `session.libref_format_catalogs` puis un sidecar disque (ci-dessous) :
    // c'est cette seconde écriture, absente pour WORK, qui rend le catalogue
    // permanent.
    // M39.3 : en parallèle, `session.format_catalog_own_work` accumule les
    // MÊMES définitions mais UNIQUEMENT quand ce step vise WORK — c'est le
    // "slot" WORK que `formats::search::rebuild_format_catalog` consulte
    // (FMTSEARCH= posée) et que `FMTLIB` sans `LIB=` liste. N'affecte en rien
    // `session.format_catalog` ci-dessus (chemin legacy inchangé).
    let is_work = ast.lib == "WORK";
    let catalog = std::rc::Rc::make_mut(&mut session.format_catalog);
    for (name, uf) in cntlin_values.iter().chain(ast.values.iter()) {
        let uname = name.to_uppercase();
        session
            .log
            .note(&format!("Format {} has been output.", uname));
        catalog.define(&uname, uf.clone());
        if is_work {
            session.format_catalog_own_work.define(&uname, uf.clone());
        }
    }
    for (name, ui) in cntlin_invalues.iter().chain(ast.invalues.iter()) {
        let uname = name.to_uppercase();
        session
            .log
            .note(&format!("Informat {} has been output.", uname));
        catalog.define_informat(&uname, ui.clone());
        if is_work {
            session
                .format_catalog_own_work
                .define_informat(&uname, ui.clone());
        }
    }
    for (name, up) in &ast.pictures {
        let uname = name.to_uppercase();
        session
            .log
            .note(&format!("Format {} has been output.", uname));
        catalog.define_picture(&uname, up.clone());
        if is_work {
            session
                .format_catalog_own_work
                .define_picture(&uname, up.clone());
        }
    }

    if ast.lib != "WORK" {
        persist_library_catalog(session, ast, &cntlin_values, &cntlin_invalues)?;
    }

    // M39.2 — `CNTLOUT=<ds>` : dépose le catalogue RÉSULTANT (après CNTLIN=
    // et les sous-statements ci-dessus) dans un dataset. Le catalogue à
    // dumper est reconstruit indépendamment de `persist_library_catalog`
    // (qui peut avoir sauté l'accumulation pour un libref cloud sans
    // `catalog_dir()`) pour que CNTLOUT= reste correct dans tous les cas.
    if let Some(cntlout_ref) = &ast.cntlout {
        let dump = if ast.lib == "WORK" {
            (*session.format_catalog).clone()
        } else {
            let mut base = session
                .libref_format_catalogs
                .get(&ast.lib)
                .cloned()
                .unwrap_or_default();
            for (name, uf) in cntlin_values.iter().chain(ast.values.iter()) {
                base.define(&name.to_uppercase(), uf.clone());
            }
            for (name, ui) in cntlin_invalues.iter().chain(ast.invalues.iter()) {
                base.define_informat(&name.to_uppercase(), ui.clone());
            }
            for (name, up) in &ast.pictures {
                base.define_picture(&name.to_uppercase(), up.clone());
            }
            base
        };
        cntl::write_cntlout(cntlout_ref, &dump, session)?;
    }

    // M39.3 — si `OPTIONS FMTSEARCH=` a déjà été posée, ce step vient de
    // modifier une des deux sources que `rebuild_format_catalog` fusionne
    // (`format_catalog_own_work` pour WORK, `libref_format_catalogs[lib]`
    // sinon) : recalcule `session.format_catalog` pour que la résolution
    // reflète immédiatement la nouvelle définition, dans l'ordre déjà choisi.
    // No-op tant que FMTSEARCH= n'a jamais été posée (voir `formats::mod`).
    if !session.options.fmtsearch.is_empty() {
        crate::formats::search::rebuild_format_catalog(session);
    }

    // M39.3 — `FMTLIB` : liste le catalogue CIBLÉ (`ast.lib`) tel qu'il est
    // maintenant (après CNTLIN=/VALUE/INVALUE/PICTURE ci-dessus). Volontai-
    // rement APRÈS le rebuild ci-dessus : ni l'un ni l'autre n'a d'influence
    // sur l'autre (FMTLIB lit `format_catalog_own_work`/`libref_format_catalogs`
    // directement, jamais `session.format_catalog`), mais l'ordre garde le
    // step "état stable puis rendu" lisible.
    if ast.fmtlib {
        render_fmtlib(session, &ast.lib);
    }

    Ok(())
}

/// M39.1 — `PROC FORMAT LIBRARY=<libref>;` (libref ≠ WORK) : accumule les
/// définitions de CE step dans `session.libref_format_catalogs[libref]` (qui
/// tient déjà tout ce que le sidecar avait chargé au LIBNAME, cf.
/// `executor::global::libname::load_format_catalog_sidecar`) puis réécrit le
/// sidecar en entier. `libref` doit déjà être assigné (ERROR SAS sinon,
/// fidèle à `proc datasets lib=<inconnu>`). Un libref sans répertoire local
/// (backend cloud) reçoit une NOTE au lieu d'une écriture disque — les
/// formats restent utilisables pour la session en cours (déjà définis
/// ci-dessus dans `session.format_catalog`), simplement pas persistés.
///
/// M39.2 — `cntlin_values`/`cntlin_invalues` (les entrées reconstruites par
/// `CNTLIN=`, si présent) sont accumulées AVANT `ast.values`/`ast.invalues`,
/// dans le même ordre que la boucle principale de `execute` : un `LIBRARY=`
/// non-WORK persiste donc aussi ce qu'un `CNTLIN=` a reconstruit — pas
/// seulement ce qu'un `VALUE`/`INVALUE` explicite a défini.
fn persist_library_catalog(
    session: &mut Session,
    ast: &FormatAst,
    cntlin_values: &[(String, UserFormat)],
    cntlin_invalues: &[(String, UserInformat)],
) -> Result<()> {
    let provider = session.libs.get(&ast.lib)?;
    let dir = match provider.catalog_dir() {
        Some(d) => d.to_path_buf(),
        None => {
            session.log.note(&format!(
                "Library {} does not support a persistent format catalog in this \
                 build; the formats defined above are usable for this session only.",
                ast.lib
            ));
            return Ok(());
        }
    };
    drop(provider);

    let entry = session
        .libref_format_catalogs
        .entry(ast.lib.clone())
        .or_default();
    for (name, uf) in cntlin_values.iter().chain(ast.values.iter()) {
        entry.define(&name.to_uppercase(), uf.clone());
    }
    for (name, ui) in cntlin_invalues.iter().chain(ast.invalues.iter()) {
        entry.define_informat(&name.to_uppercase(), ui.clone());
    }
    for (name, up) in &ast.pictures {
        entry.define_picture(&name.to_uppercase(), up.clone());
    }
    // Empty catalog (e.g. all sub-statements failed to parse into anything —
    // should not happen given the checks above, kept as a defensive no-write
    // guard mirroring `dataset.rs`) → no file at all.
    entry.save_sidecar(&dir)
}

#[cfg(test)]
mod tests;
