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

use crate::error::{Result, SasError};
use crate::formats::userdef::{
    Bound, InformatRange, InformatValue, PictureDirectives, PictureRange, Range, UserFormat,
    UserInformat, UserPicture,
};
use crate::parser::StatementStream;
use crate::session::Session;
use crate::token::TokenKind;

mod invalue;
mod picture;
mod value;

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
    // Consume the trailing `;` of the `proc format` statement header,
    // recognising `LIBRARY=`/`LIB=` along the way; any other header option is
    // skipped token-by-token (none else supported yet).
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("lib") || ts.peek().is_kw("library") {
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
        } else {
            // Skip any other unrecognised proc-header option token (e.g. the
            // future CNTLOUT=/CNTLIN=/FMTLIB of M39.2/M39.3).
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
    })
}

pub fn execute(ast: &FormatAst, session: &mut Session) -> Result<()> {
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
    let catalog = std::rc::Rc::make_mut(&mut session.format_catalog);
    for (name, uf) in &ast.values {
        let uname = name.to_uppercase();
        session
            .log
            .note(&format!("Format {} has been output.", uname));
        catalog.define(&uname, uf.clone());
    }
    for (name, ui) in &ast.invalues {
        let uname = name.to_uppercase();
        session
            .log
            .note(&format!("Informat {} has been output.", uname));
        catalog.define_informat(&uname, ui.clone());
    }
    for (name, up) in &ast.pictures {
        let uname = name.to_uppercase();
        session
            .log
            .note(&format!("Format {} has been output.", uname));
        catalog.define_picture(&uname, up.clone());
    }

    if ast.lib != "WORK" {
        persist_library_catalog(session, ast)?;
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
fn persist_library_catalog(session: &mut Session, ast: &FormatAst) -> Result<()> {
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
    for (name, uf) in &ast.values {
        entry.define(&name.to_uppercase(), uf.clone());
    }
    for (name, ui) in &ast.invalues {
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
