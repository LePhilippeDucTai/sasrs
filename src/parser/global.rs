//! Statements globaux : LIBNAME, OPTIONS, TITLEn.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! Appelé par `parser::next_block()` ; le mot-clé de tête est encore dans
//! le stream (peek) ou déjà identifié par l'appelant — convention :
//! l'appelant N'A PAS consommé le mot-clé, `parse_global` le consomme.
//!
//! ## LIBNAME
//! - `libname ref 'chemin' ;`  → `GlobalStmt::Libname` (chemin = littéral
//!   chaîne ; relatif → résolu contre `Session::base_dir` à l'exécution).
//! - `libname ref clear ;`     → `GlobalStmt::LibnameClear`.
//!
//! ## TITLE
//! - `title 'texte' ;` / `titleN 'texte' ;` (N=1..9, suffixe dans
//!   l'ident) ; sans texte → efface. M1 : seul TITLE1 est rendu par le
//!   listing.
//!
//! ## OPTIONS
//! - `options name[=valeur]... ;` → liste brute. L'exécution (executor)
//!   applique `ls=` (40..=256) et ignore le reste avec WARNING
//!   "Option XXX is not yet supported".

use super::{title_level, StatementStream};
use crate::ast::GlobalStmt;
use crate::error::{Result, SasError};
use crate::token::{Span, StrSuffix, TokenKind};

/// Parse a global statement (LIBNAME, OPTIONS, or TITLEn).
///
/// The leading keyword token must still be in the stream (not yet consumed);
/// this function consumes it and the closing `;`.
pub fn parse_global(ts: &mut StatementStream) -> Result<GlobalStmt> {
    let head = ts.peek().clone();
    let kw = match head.ident() {
        Some(s) => s.to_ascii_lowercase(),
        None => {
            return Err(SasError::parse(
                "expected LIBNAME, OPTIONS, or TITLE keyword",
                head.span,
            ));
        }
    };

    if kw == "libname" {
        ts.next(); // consume `libname`
        parse_libname(ts)
    } else if kw == "options" {
        ts.next(); // consume `options`
        parse_options(ts)
    } else if let Some(n) = title_level(&kw) {
        ts.next(); // consume `titleN`
        parse_title(ts, n)
    } else {
        Err(SasError::parse(
            format!(
                "Expected LIBNAME, OPTIONS, or TITLEn; got '{}'",
                kw.to_uppercase()
            ),
            head.span,
        ))
    }
}

// ── LIBNAME ─────────────────────────────────────────────────────────────────

fn parse_libname(ts: &mut StatementStream) -> Result<GlobalStmt> {
    // Expect the libref identifier.
    let ref_tok = ts.peek().clone();
    let libref = match ref_tok.ident() {
        Some(s) => s.to_string(),
        None => {
            return Err(SasError::parse(
                "LIBNAME requires a libref (1–8 character identifier)",
                ref_tok.span,
            ));
        }
    };

    // Validate: libref must be ≤ 8 characters.
    if libref.len() > 8 {
        return Err(SasError::parse(
            format!(
                "The libref {} exceeds the maximum of 8 characters.",
                libref.to_uppercase()
            ),
            ref_tok.span,
        ));
    }
    ts.next(); // consume libref

    // Peek at what follows: `clear` keyword, a string literal (path), or
    // an engine identifier followed by a string literal.
    let next_tok = ts.peek().clone();

    // `libname ref clear ;`
    if next_tok.is_kw("clear") {
        ts.next(); // consume `clear`
        ts.expect_semi()?;
        return Ok(GlobalStmt::LibnameClear { libref });
    }

    // `libname ref 'path' ;`  — no engine
    if let TokenKind::Str { value, suffix } = &next_tok.kind {
        if *suffix != StrSuffix::None {
            return Err(SasError::parse(
                "LIBNAME path must be a plain string literal (no date/time suffix)",
                next_tok.span,
            ));
        }
        let path = value.clone();
        ts.next(); // consume the string literal
        ts.expect_semi()?;
        return Ok(GlobalStmt::Libname { libref, engine: None, path });
    }

    // `libname ref <engine> 'path' ;`  — engine identifier before the path.
    // Known engines: CSV, XLSX, EXCEL, PARQUET, BASE, V9 (plus any identifier
    // is accepted and uppercased; the executor emits an error for unknowns).
    if let TokenKind::Ident(eng) = &next_tok.kind {
        let engine = eng.to_ascii_uppercase();
        ts.next(); // consume the engine identifier

        // Now expect the path string literal.
        let path_tok = ts.peek().clone();
        match &path_tok.kind {
            TokenKind::Str { value, suffix } => {
                if *suffix != StrSuffix::None {
                    return Err(SasError::parse(
                        "LIBNAME path must be a plain string literal (no date/time suffix)",
                        path_tok.span,
                    ));
                }
                let path = value.clone();
                ts.next(); // consume the string literal
                ts.expect_semi()?;
                return Ok(GlobalStmt::Libname { libref, engine: Some(engine), path });
            }
            _ => {
                return Err(SasError::parse(
                    format!(
                        "Expected a quoted path after engine {} for libref {}; \
                         got an unexpected token.",
                        engine,
                        libref.to_uppercase()
                    ),
                    path_tok.span,
                ));
            }
        }
    }

    Err(SasError::parse(
        format!(
            "Expected a quoted path or CLEAR after libref {}; \
             got an unexpected token.",
            libref.to_uppercase()
        ),
        next_tok.span,
    ))
}

// ── TITLE ────────────────────────────────────────────────────────────────────

fn parse_title(ts: &mut StatementStream, n: u8) -> Result<GlobalStmt> {
    // `title ;` or `titleN ;` — no text, clears the title.
    if ts.peek().kind == TokenKind::Semi {
        ts.expect_semi()?;
        return Ok(GlobalStmt::Title { n, text: None });
    }

    // Only a quoted string literal is accepted in M1.
    //
    // Note: SAS itself accepts unquoted text after TITLE (e.g. `title My Report;`),
    // but our M1 parser intentionally restricts this to quoted string literals only.
    // Unquoted text after TITLE is an error here; this keeps the AST unambiguous and
    // avoids complex multi-token text concatenation. Callers should quote their titles.
    let text_tok = ts.peek().clone();
    match &text_tok.kind {
        TokenKind::Str { value, suffix } => {
            if *suffix != StrSuffix::None {
                return Err(SasError::parse(
                    "TITLE text must be a plain string literal (no date/time suffix)",
                    text_tok.span,
                ));
            }
            let text = value.clone();
            ts.next(); // consume the string literal
            ts.expect_semi()?;
            Ok(GlobalStmt::Title { n, text: Some(text) })
        }
        _ => {
            // Unquoted text or any non-string token after TITLE.
            Err(SasError::parse(
                "TITLE text must be a quoted string literal, e.g. title 'My Report';",
                text_tok.span,
            ))
        }
    }
}

// ── OPTIONS ──────────────────────────────────────────────────────────────────

fn parse_options(ts: &mut StatementStream) -> Result<GlobalStmt> {
    let mut opts: Vec<(String, Option<String>)> = Vec::new();

    // Collect `name` or `name=value` pairs until `;`.
    loop {
        if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
            break;
        }

        // The option name must be an identifier.
        let name_tok = ts.peek().clone();
        let name = match name_tok.ident() {
            Some(s) => s.to_string(),
            None => {
                return Err(SasError::parse(
                    "Expected an option name (identifier) in OPTIONS statement",
                    name_tok.span,
                ));
            }
        };
        ts.next(); // consume the name

        // Check for an `=` (value follows) or just a flag.
        if ts.peek().kind == TokenKind::Eq {
            ts.next(); // consume `=`
            let val_tok = ts.peek().clone();
            let value = parse_option_value(ts, &val_tok.span)?;
            opts.push((name, Some(value)));
        } else {
            // Boolean flag: `nocenter`, `center`, etc.
            opts.push((name, None));
        }
    }

    ts.expect_semi()?;
    Ok(GlobalStmt::Options(opts))
}

/// Parse the value token after `=` in an OPTIONS pair.
/// Accepts: identifier, integer or float number, plain string literal.
fn parse_option_value(ts: &mut StatementStream, _span: &Span) -> Result<String> {
    let val_tok = ts.peek().clone();
    match &val_tok.kind {
        TokenKind::Ident(s) => {
            let s = s.clone();
            ts.next();
            Ok(s)
        }
        TokenKind::Num(f) => {
            let f = *f;
            ts.next();
            // Format integers without a trailing ".0" for readability.
            if f.fract() == 0.0 && f.abs() < 1e15 {
                Ok(format!("{}", f as i64))
            } else {
                Ok(format!("{}", f))
            }
        }
        TokenKind::Str { value, suffix } => {
            if *suffix != StrSuffix::None {
                return Err(SasError::parse(
                    "OPTIONS value must be a plain string literal (no date/time suffix)",
                    val_tok.span,
                ));
            }
            let v = value.clone();
            ts.next();
            Ok(v)
        }
        _ => Err(SasError::parse(
            "Expected an identifier, number, or quoted string as OPTIONS value",
            val_tok.span,
        )),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::GlobalStmt;
    use crate::source::SourceFile;

    fn parse(src: &str) -> Result<GlobalStmt> {
        let sf = SourceFile::new(src);
        let mut ts = StatementStream::new(&sf).unwrap();
        parse_global(&mut ts)
    }

    // ── LIBNAME ──────────────────────────────────────────────────────────────

    #[test]
    fn libname_with_path() {
        let stmt = parse("libname mylib '/data/sas';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Libname {
                libref: "mylib".into(),
                engine: None,
                path: "/data/sas".into(),
            }
        );
    }

    #[test]
    fn libname_relative_path() {
        let stmt = parse("libname outlib 'output/results';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Libname {
                libref: "outlib".into(),
                engine: None,
                path: "output/results".into(),
            }
        );
    }

    #[test]
    fn libname_with_csv_engine() {
        let stmt = parse("libname csvlib csv '/data/csv';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Libname {
                libref: "csvlib".into(),
                engine: Some("CSV".into()),
                path: "/data/csv".into(),
            }
        );
    }

    #[test]
    fn libname_with_xlsx_engine() {
        let stmt = parse("libname xl xlsx '/data/xl';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Libname {
                libref: "xl".into(),
                engine: Some("XLSX".into()),
                path: "/data/xl".into(),
            }
        );
    }

    #[test]
    fn libname_with_parquet_engine() {
        let stmt = parse("libname pq parquet '/data/pq';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Libname {
                libref: "pq".into(),
                engine: Some("PARQUET".into()),
                path: "/data/pq".into(),
            }
        );
    }

    #[test]
    fn libname_engine_is_uppercased() {
        let stmt = parse("libname x Csv '/tmp';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Libname {
                libref: "x".into(),
                engine: Some("CSV".into()),
                path: "/tmp".into(),
            }
        );
    }

    #[test]
    fn libname_clear() {
        let stmt = parse("libname mylib clear;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::LibnameClear {
                libref: "mylib".into(),
            }
        );
    }

    #[test]
    fn libname_clear_case_insensitive() {
        let stmt = parse("LIBNAME MYLIB CLEAR;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::LibnameClear {
                libref: "MYLIB".into(),
            }
        );
    }

    #[test]
    fn libname_libref_too_long_is_error() {
        // "toolonglib" = 10 characters — must error.
        let err = parse("libname toolonglib '/path';").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.to_lowercase().contains("libref") || msg.contains("8"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    fn libname_missing_libref_is_error() {
        // `libname '/path';` — no libref identifier.
        let err = parse("libname '/path';").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.to_lowercase().contains("libref"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    fn libname_missing_path_is_error() {
        // `libname mylib 123;` — path is not a string literal.
        let err = parse("libname mylib 123;").unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    // ── TITLE ────────────────────────────────────────────────────────────────

    #[test]
    fn title_simple() {
        let stmt = parse("title 'My Report';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Title {
                n: 1,
                text: Some("My Report".into()),
            }
        );
    }

    #[test]
    fn title_uppercase() {
        let stmt = parse("TITLE 'My Report';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Title {
                n: 1,
                text: Some("My Report".into()),
            }
        );
    }

    #[test]
    fn title3() {
        let stmt = parse("title3 'Section Header';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Title {
                n: 3,
                text: Some("Section Header".into()),
            }
        );
    }

    #[test]
    fn title9() {
        let stmt = parse("title9 'Footer';").unwrap();
        assert_eq!(stmt, GlobalStmt::Title { n: 9, text: Some("Footer".into()) });
    }

    #[test]
    fn title_without_text_clears() {
        // `title;` — no text, clears title 1.
        let stmt = parse("title;").unwrap();
        assert_eq!(stmt, GlobalStmt::Title { n: 1, text: None });
    }

    #[test]
    fn title5_without_text_clears() {
        let stmt = parse("title5;").unwrap();
        assert_eq!(stmt, GlobalStmt::Title { n: 5, text: None });
    }

    #[test]
    fn title_unquoted_text_is_error() {
        // SAS accepts unquoted text but our M1 parser requires a quoted literal.
        // `title foo;` must return a parse error.
        let err = parse("title foo;").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.to_lowercase().contains("quoted") || msg.to_lowercase().contains("string"),
            "expected an error about quoted string, got: {msg}"
        );
    }

    // ── OPTIONS ──────────────────────────────────────────────────────────────

    #[test]
    fn options_ls_and_nocenter() {
        let stmt = parse("options ls=80 nocenter;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Options(vec![
                ("ls".into(), Some("80".into())),
                ("nocenter".into(), None),
            ])
        );
    }

    #[test]
    fn options_string_value() {
        let stmt = parse("options fmtsearch=(mylib) notes='yes';").unwrap_err();
        // `(` is not a valid option value — we get a parse error.
        // This sub-test verifies we don't panic.
        let _ = stmt;

        // A proper string value:
        let stmt = parse("options label='My value';").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Options(vec![("label".into(), Some("My value".into()))])
        );
    }

    #[test]
    fn options_empty_is_ok() {
        // `options;` — empty list is accepted (no-op per spec).
        let stmt = parse("options;").unwrap();
        assert_eq!(stmt, GlobalStmt::Options(vec![]));
    }

    #[test]
    fn options_multiple_flags_and_values() {
        let stmt = parse("options center ps=60 linesize=132 nodate;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Options(vec![
                ("center".into(), None),
                ("ps".into(), Some("60".into())),
                ("linesize".into(), Some("132".into())),
                ("nodate".into(), None),
            ])
        );
    }

    #[test]
    fn options_float_value() {
        // A float value should be formatted without trailing `.0` for integers.
        let stmt = parse("options decimals=2.5;").unwrap();
        assert_eq!(
            stmt,
            GlobalStmt::Options(vec![("decimals".into(), Some("2.5".into()))])
        );
    }
}
