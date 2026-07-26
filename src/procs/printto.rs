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
//! # Sémantique v1
//!
//! v1 = implémentation minimale documentée :
//! - Les chemins de redirection sont stockés dans la `Session` (champs
//!   `printto_log` et `printto_print`, ajoutés à `Session` pour M21.1).
//! - `proc printto;` nu (aucune option) réinitialise les deux destinations.
//! - NOTE émise dans le log actuel (non redirigé) : "PROCEDURE PRINTTO used".
//! - Le **routage réel** (écriture physique vers le fichier) est différé à M22
//!   (couche ODS). La raison : le routage demande un trait `OutputDestination`
//!   qui n'existe pas encore ; insérer ici un File I/O ad hoc casserait les
//!   tests de snapshot existants (byte-identiques). Ce comportement est
//!   documenté comme déviation connue.
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
    common::parse_proc_body(ts, |_ts, _kw| Ok(false))?;

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

/// Execute PROC PRINTTO.
pub fn execute(ast: &PrinttoAst, session: &mut Session) -> Result<()> {
    if ast.reset {
        // Reset both destinations
        session.printto_log = None;
        session.printto_print = None;
        session
            .log
            .note("PROCEDURE PRINTTO: log and print destinations reset to default.");
    } else {
        if let Some(ref path) = ast.log {
            let resolved = session.resolve_path(path);
            session.log.note(&format!(
                "PROCEDURE PRINTTO: log redirected to '{}'{}.",
                resolved.display(),
                if ast.new { " (NEW)" } else { "" }
            ));
            session.printto_log = Some(resolved);
        }
        if let Some(ref path) = ast.print {
            let resolved = session.resolve_path(path);
            session.log.note(&format!(
                "PROCEDURE PRINTTO: print redirected to '{}'{}.",
                resolved.display(),
                if ast.new { " (NEW)" } else { "" }
            ));
            session.printto_print = Some(resolved);
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
