//! PROC PRINTTO (jalon M21.1) — rediriger le log et/ou le listing.
//!
//! # Syntaxe
//! ```sas
//! proc printto [log=<fileref|'path'>] [print=<fileref|'path'>] [new];
//! run;
//!
//! proc printto; run;   /* reset — rétablit les destinations par défaut */
//! ```
//!
//! # Sémantique (J07-P5 — routage réel)
//!
//! - `LOG='path'` : le log produit APRÈS le `PROC PRINTTO` part
//!   réellement vers le fichier (mode ajout ; `NEW` remplace le contenu
//!   existant). Le segment routé ne rejoint jamais le log par défaut.
//! - `PRINT='path'` : même routage réel pour le listing.
//! - `PROC PRINTTO;` nu rétablit les destinations par défaut (les segments
//!   routés en cours sont écrits dans leurs fichiers).
//! - Précédence (doc SAS 9.4) : la dernière destination PRINTTO ouverte
//!   gagne pendant le run ; `--log`/`--print` de la CLI restent les
//!   destinations par défaut INITIALES (ils reçoivent tout ce qui n'est pas
//!   routé par PRINTTO). Les diagnostics structurés (J06-P2) couvrent tout
//!   le run, segments routés compris ; les compteurs WARNING/ERROR (donc le
//!   code retour) aussi.
//! - Erreur d'ouverture de fichier → ERROR comptée (exit 2), la destination
//!   en cours reste inchangée.
//!
//! # Invariant IMPORTANT
//!
//! Sans `PROC PRINTTO`, la sortie listing et log restent byte-identiques aux
//! snapshots m1–m20. C'est l'invariant le plus critique de ce fichier.

use crate::error::{Result, SasError};
use crate::parser::StatementStream;
use crate::procs::common;
use crate::session::Session;
use crate::token::TokenKind;

pub struct PrinttoAst {
    /// Path/fileref for the LOG destination; None = not specified.
    pub log: Option<String>,
    /// Path/fileref for the PRINT (listing) destination; None = not specified.
    pub print: Option<String>,
    /// NEW option: truncate the file (vs append).
    pub new: bool,
    /// True when `proc printto;` is used bare (no options) → reset mode.
    pub reset: bool,
}

/// Parse `proc printto [log=...] [print=...] [new]; run;`
/// Also handles `proc printto; run;` (reset mode).
/// Called AFTER "proc printto" has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<PrinttoAst> {
    let mut log: Option<String> = None;
    let mut print: Option<String> = None;
    let mut new = false;
    let mut reset = false;

    // Peek before consuming `;` to detect bare `proc printto;`
    if ts.peek().kind == TokenKind::Semi {
        // bare proc printto; — reset mode
        ts.next(); // consume `;`
        reset = true;
    } else {
        // Parse options until `;`
        loop {
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
                break;
            }
            if ts.peek().kind == TokenKind::Eof {
                break;
            }

            if ts.peek().is_kw("log") {
                ts.next();
                if ts.peek().kind != TokenKind::Eq {
                    return Err(SasError::parse("expected '=' after LOG", ts.peek().span));
                }
                ts.next(); // consume `=`
                log = Some(parse_path_or_ident(ts)?);
            } else if ts.peek().is_kw("print") {
                ts.next();
                if ts.peek().kind != TokenKind::Eq {
                    return Err(SasError::parse("expected '=' after PRINT", ts.peek().span));
                }
                ts.next(); // consume `=`
                print = Some(parse_path_or_ident(ts)?);
            } else if ts.peek().is_kw("new") {
                ts.next();
                new = true;
            } else {
                // Unknown option: skip token
                ts.next();
            }
        }
    }

    // Parse sub-statements until `run;` or `quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, "PRINTTO", |_ts, _kw| Ok(false))?;

    Ok(PrinttoAst {
        log,
        print,
        new,
        reset,
    })
}

/// Parse a string literal ('path') or an identifier (fileref).
fn parse_path_or_ident(ts: &mut StatementStream) -> Result<String> {
    let tok = ts.peek().clone();
    match &tok.kind {
        TokenKind::Str { value, .. } => {
            let s = value.clone();
            ts.next();
            Ok(s)
        }
        TokenKind::Ident(_) => {
            let ident = tok.ident().unwrap_or("").to_string();
            ts.next();
            Ok(ident)
        }
        _ => Err(SasError::parse(
            "expected a fileref or quoted path after '='",
            tok.span,
        )),
    }
}

/// Execute PROC PRINTTO (J07-P5 — routage réel).
pub fn execute(ast: &PrinttoAst, session: &mut Session) -> Result<()> {
    if ast.reset {
        // Reset both destinations: flush any open route to its file, then
        // restore the defaults.
        session.close_print_route();
        session.log.end_route();
        session.printto_log = None;
        session
            .log
            .note("PROCEDURE PRINTTO: log and print destinations reset to default.");
    } else {
        if let Some(ref path) = ast.log {
            let resolved = session.resolve_path(path);
            match session.log.begin_route(&resolved, ast.new) {
                Ok(()) => {
                    session.printto_log = Some(resolved);
                }
                Err(e) => session.log.error(&format!(
                    "PROCEDURE PRINTTO: cannot open log destination '{}': {}",
                    resolved.display(),
                    e
                )),
            }
        }
        if let Some(ref path) = ast.print {
            let resolved = session.resolve_path(path);
            match session.open_print_route(resolved.clone(), ast.new) {
                Ok(()) => {}
                Err(e) => session.log.error(&format!(
                    "PROCEDURE PRINTTO: cannot open print destination '{}': {}",
                    resolved.display(),
                    e
                )),
            }
        }
        if ast.log.is_none() && ast.print.is_none() {
            // Options were present but none we recognize — treat as no-op
            session.log.note("PROCEDURE PRINTTO used.");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
