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
    /// (nom, définition brute à parser en UserFormat)
    pub values: Vec<(String, UserFormat)>,
    /// (nom, définition brute à parser en UserInformat) — M18.2
    pub invalues: Vec<(String, UserInformat)>,
    /// (nom, définition brute à parser en UserPicture) — M18.3
    pub pictures: Vec<(String, UserPicture)>,
}

/// Parse `proc format; value ... ; [value ... ;] run;`
/// Called AFTER "proc format" has been consumed. Consumes through `run;`/`quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<FormatAst> {
    // Consume the trailing `;` of the `proc format` statement header.
    // There may be options between `proc format` and `;` (none supported yet).
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        // Skip any unrecognised proc-header options (none for FORMAT).
        ts.next();
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
        values,
        invalues,
        pictures,
    })
}

pub fn execute(ast: &FormatAst, session: &mut Session) -> Result<()> {
    for (name, uf) in &ast.values {
        let uname = name.to_uppercase();
        session
            .log
            .note(&format!("Format {} has been output.", uname));
        session.format_catalog.define(&uname, uf.clone());
    }
    for (name, ui) in &ast.invalues {
        let uname = name.to_uppercase();
        session
            .log
            .note(&format!("Informat {} has been output.", uname));
        session.format_catalog.define_informat(&uname, ui.clone());
    }
    for (name, up) in &ast.pictures {
        let uname = name.to_uppercase();
        session
            .log
            .note(&format!("Format {} has been output.", uname));
        session.format_catalog.define_picture(&uname, up.clone());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
