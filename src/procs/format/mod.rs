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
    // Consume the trailing `;` of the `proc format` statement header,
    // recognising `LIBRARY=`/`LIB=`/`CNTLOUT=`/`CNTLIN=`/`FMTLIB` along the
    // way; any other header option is skipped token-by-token (none else
    // supported yet).
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
        } else {
            // Skip any other unrecognised proc-header option token.
            ts.next();
        }
    }

    let mut values: Vec<(String, UserFormat)> = Vec::new();
    let mut invalues: Vec<(String, UserInformat)> = Vec::new();
    let mut pictures: Vec<(String, UserPicture)> = Vec::new();

    loop {
        // Skip stray semicolons.
        while ts.peek().kind == TokenKind::Semi {
            ts.next();
        }

        if ts.peek().kind == TokenKind::Eof {
            break;
        }

        if ts.peek().is_kw("run") || ts.peek().is_kw("quit") {
            ts.next();
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            break;
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
            // Unknown sub-statement: skip it.
            ts.skip_to_semi();
        }
    }

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
