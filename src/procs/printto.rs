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

    // Parse sub-statements until `run;` or `quit;`
    loop {
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
        ts.skip_to_semi();
    }

    Ok(PrinttoAst { log, print, new, reset })
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
        _ => {
            Err(SasError::parse(
                "expected a fileref or quoted path after '='",
                tok.span,
            ))
        }
    }
}

/// Execute PROC PRINTTO.
pub fn execute(ast: &PrinttoAst, session: &mut Session) -> Result<()> {
    if ast.reset {
        // Reset both destinations
        session.printto_log = None;
        session.printto_print = None;
        session.log.note("PROCEDURE PRINTTO: log and print destinations reset to default.");
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
mod tests {
    use super::*;
    use crate::session::Session;
    use crate::source::SourceFile;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn parse_printto_src(src: &str) -> Result<PrinttoAst> {
        let source = SourceFile::new(src);
        let mut ts = crate::parser::StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "printto"
        parse(&mut ts)
    }

    // ── Parse tests ───────────────────────────────────────────────────────────

    #[test]
    fn parse_reset_bare() {
        let ast = parse_printto_src("proc printto; run;").unwrap();
        assert!(ast.reset);
        assert!(ast.log.is_none());
        assert!(ast.print.is_none());
        assert!(!ast.new);
    }

    #[test]
    fn parse_log_path() {
        let ast = parse_printto_src("proc printto log='/tmp/mylog.txt'; run;").unwrap();
        assert!(!ast.reset);
        assert_eq!(ast.log.as_deref(), Some("/tmp/mylog.txt"));
        assert!(ast.print.is_none());
    }

    #[test]
    fn parse_print_path() {
        let ast = parse_printto_src("proc printto print='/tmp/out.lst'; run;").unwrap();
        assert!(!ast.reset);
        assert_eq!(ast.print.as_deref(), Some("/tmp/out.lst"));
        assert!(ast.log.is_none());
    }

    #[test]
    fn parse_new_option() {
        let ast = parse_printto_src("proc printto log='/tmp/log.txt' new; run;").unwrap();
        assert!(ast.new);
        assert!(ast.log.is_some());
    }

    #[test]
    fn parse_log_fileref() {
        let ast = parse_printto_src("proc printto log=mylog; run;").unwrap();
        assert_eq!(ast.log.as_deref(), Some("mylog"));
    }

    // ── Execute tests ─────────────────────────────────────────────────────────

    #[test]
    fn execute_log_stores_path() {
        let mut session = make_session();
        let ast = PrinttoAst {
            log: Some("/tmp/mylog.txt".to_string()),
            print: None,
            new: false,
            reset: false,
        };
        execute(&ast, &mut session).unwrap();

        assert!(session.printto_log.is_some());
        let log = session.log.into_string();
        assert!(log.contains("log redirected"), "log: {log}");
    }

    #[test]
    fn execute_print_stores_path() {
        let mut session = make_session();
        let ast = PrinttoAst {
            log: None,
            print: Some("/tmp/out.lst".to_string()),
            new: false,
            reset: false,
        };
        execute(&ast, &mut session).unwrap();

        assert!(session.printto_print.is_some());
    }

    #[test]
    fn execute_reset_clears_paths() {
        let mut session = make_session();
        session.printto_log = Some(PathBuf::from("/tmp/old.log"));
        session.printto_print = Some(PathBuf::from("/tmp/old.lst"));

        let ast = PrinttoAst {
            log: None,
            print: None,
            new: false,
            reset: true,
        };
        execute(&ast, &mut session).unwrap();

        assert!(session.printto_log.is_none());
        assert!(session.printto_print.is_none());
        let log = session.log.into_string();
        assert!(log.contains("reset"), "log: {log}");
    }

    #[test]
    fn execute_new_noted_in_log() {
        let mut session = make_session();
        let ast = PrinttoAst {
            log: Some("/tmp/newlog.txt".to_string()),
            print: None,
            new: true,
            reset: false,
        };
        execute(&ast, &mut session).unwrap();

        let log = session.log.into_string();
        assert!(log.contains("NEW"), "log: {log}");
    }

    #[test]
    fn execute_does_not_affect_listing_output() {
        // Default listing output should be unaffected by PRINTTO
        let mut session = make_session();
        let ast = PrinttoAst {
            log: Some("/tmp/ignored.txt".to_string()),
            print: None,
            new: false,
            reset: false,
        };
        execute(&ast, &mut session).unwrap();

        // listing should still be empty (PRINTTO alone writes nothing to listing)
        let listing = session.listing.into_string();
        assert!(listing.is_empty(), "listing should be empty after PRINTTO: {listing}");
    }
}
