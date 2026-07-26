//! Tests des combinateurs de parsing partagés par les PROCs.
use super::*;
use crate::session::Session;
use crate::source::SourceFile;
use std::path::PathBuf;

/// Construit un `StatementStream` positionné sur le premier token utile,
/// après avoir consommé `proc <name>` (même construction que les modules
/// `print`/`sort`/`means`). Le `Vec` de tokens appartient au `SourceFile`
/// que l'appelant doit garder vivant.
fn proc_stream<'a>(src: &'a SourceFile) -> StatementStream<'a> {
    let mut ts = StatementStream::new(src).unwrap();
    ts.next(); // "proc"
    ts.next(); // <proc name>
    ts
}

fn make_session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

// ── parse_proc_options ────────────────────────────────────────────────

#[test]
fn options_recognizes_and_stops_on_semi() {
    // proc foo data=lib.x noobs ;
    let src = SourceFile::new("proc foo data=lib.x noobs; run;");
    let mut ts = proc_stream(&src);
    let mut data: Option<DatasetRef> = None;
    let mut noobs = false;
    parse_proc_options(&mut ts, "FOO", |ts, kw| match kw {
        "data" => {
            data = Some(parse_dataset_opt(ts, "DATA")?);
            Ok(true)
        }
        "noobs" => {
            ts.next();
            noobs = true;
            Ok(true)
        }
        _ => Ok(false),
    })
    .unwrap();
    assert_eq!(
        data,
        Some(DatasetRef {
            libref: Some("lib".into()),
            name: "x".into()
        })
    );
    assert!(noobs);
    // The `;` was consumed; the body starts at `run`.
    assert!(ts.peek().is_kw("run"));
}

#[test]
fn options_unknown_returns_unknown_option_error() {
    let src = SourceFile::new("proc foo bogus; run;");
    let mut ts = proc_stream(&src);
    // Capture the bad token's span before driving the loop.
    let bad_span = ts.peek().span;
    let err = parse_proc_options(&mut ts, "FOO", |_ts, _kw| Ok(false)).unwrap_err();
    match err {
        SasError::Parse { msg, span } => {
            assert_eq!(msg, "Unexpected option 'BOGUS' on PROC FOO statement.");
            assert_eq!(span, bad_span);
        }
        other => panic!("expected a parse error, got {other:?}"),
    }
}

#[test]
fn options_non_ident_token_is_unknown_option() {
    // A non-identifier leading token (here `=`) → unknown option error.
    let src = SourceFile::new("proc foo = bar; run;");
    let mut ts = proc_stream(&src);
    let err = parse_proc_options(&mut ts, "FOO", |_ts, _kw| Ok(true)).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("Unexpected option '?' on PROC FOO statement."),
        "msg: {msg}"
    );
}

// ── parse_proc_body ───────────────────────────────────────────────────

#[test]
fn body_skips_stray_semis_and_stops_on_run() {
    // Leading stray `;;`, one known sub-statement, then `run;`.
    let src = SourceFile::new("proc foo;;; var a b; run; data after;");
    let mut ts = proc_stream(&src);
    // Consume the header `;` first so we are at the body.
    ts.expect_semi().unwrap();
    let mut vars: Option<Vec<String>> = None;
    parse_proc_body(&mut ts, |ts, kw| match kw {
        "var" => {
            ts.next();
            vars = Some(ts.parse_name_list()?);
            ts.expect_semi()?;
            Ok(true)
        }
        _ => Ok(false),
    })
    .unwrap();
    assert_eq!(vars, Some(vec!["a".into(), "b".into()]));
    // `run;` was consumed; next block head is `data`.
    assert!(ts.peek().is_kw("data"));
}

#[test]
fn body_stops_on_quit() {
    let src = SourceFile::new("proc foo; quit; data after;");
    let mut ts = proc_stream(&src);
    ts.expect_semi().unwrap();
    parse_proc_body(&mut ts, |_ts, _kw| Ok(false)).unwrap();
    assert!(ts.peek().is_kw("data"));
}

#[test]
fn body_recovers_unknown_substatement_via_skip_to_semi() {
    // Unknown sub-statement `bogus x y;` must be skipped, then `var a;`
    // is dispatched, then `run;` stops.
    let src = SourceFile::new("proc foo; bogus x y; var a; run;");
    let mut ts = proc_stream(&src);
    ts.expect_semi().unwrap();
    let mut seen: Vec<String> = Vec::new();
    let mut vars: Option<Vec<String>> = None;
    parse_proc_body(&mut ts, |ts, kw| {
        seen.push(kw.to_string());
        match kw {
            "var" => {
                ts.next();
                vars = Some(ts.parse_name_list()?);
                ts.expect_semi()?;
                Ok(true)
            }
            _ => Ok(false),
        }
    })
    .unwrap();
    // `bogus` was dispatched (returned false → skip_to_semi), then `var`.
    assert_eq!(seen, vec!["bogus".to_string(), "var".to_string()]);
    assert_eq!(vars, Some(vec!["a".into()]));
    assert!(ts.at_eof());
}

// ── consume_option_eq / parse_dataset_opt ─────────────────────────────────────

#[test]
fn consume_option_eq_consumes_name_and_eq() {
    let src = SourceFile::new("proc foo data= lib.x; run;");
    let mut ts = proc_stream(&src);
    // Positioned on `data`.
    assert!(ts.peek().is_kw("data"));
    consume_option_eq(&mut ts, "DATA").unwrap();
    // Both `data` and `=` consumed; now on the dataset ref.
    assert!(ts.peek().is_kw("lib"));
}

#[test]
fn consume_option_eq_missing_eq_errors() {
    let src = SourceFile::new("proc foo data lib.x; run;");
    let mut ts = proc_stream(&src);
    let err = consume_option_eq(&mut ts, "DATA").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("expected '=' after DATA"), "msg: {msg}");
}

#[test]
fn parse_dataset_opt_happy_path() {
    let src = SourceFile::new("proc foo data=lib.x; run;");
    let mut ts = proc_stream(&src);
    let r = parse_dataset_opt(&mut ts, "DATA").unwrap();
    assert_eq!(
        r,
        DatasetRef {
            libref: Some("lib".into()),
            name: "x".into()
        }
    );
}

#[test]
fn parse_out_opt_happy_path() {
    let src = SourceFile::new("proc foo out=work.b; run;");
    let mut ts = proc_stream(&src);
    let r = parse_out_opt(&mut ts).unwrap();
    assert_eq!(
        r,
        DatasetRef {
            libref: Some("work".into()),
            name: "b".into()
        }
    );
}

// ── unknown_option_error ──────────────────────────────────────────────

#[test]
fn unknown_option_error_exact_string_and_span() {
    let src = SourceFile::new("proc foo bogus; run;");
    let ts = proc_stream(&src);
    let span = ts.peek().span;
    let err = unknown_option_error(&ts, "PRINT");
    match err {
        SasError::Parse { msg, span: s } => {
            assert_eq!(msg, "Unexpected option 'BOGUS' on PROC PRINT statement.");
            assert_eq!(s, span);
        }
        other => panic!("expected a parse error, got {other:?}"),
    }
}

// ── resolve_last_dataset ──────────────────────────────────────────────

#[test]
fn resolve_last_dataset_uses_explicit_data() {
    let session = make_session();
    let explicit = Some(DatasetRef {
        libref: Some("WORK".into()),
        name: "T".into(),
    });
    let r = resolve_last_dataset(&explicit, &session).unwrap();
    assert_eq!(Some(r), explicit);
}

#[test]
fn resolve_last_dataset_decodes_libref_dot_name() {
    let mut session = make_session();
    session.last_dataset = Some("WORK.MYDATA".to_string());
    let r = resolve_last_dataset(&None, &session).unwrap();
    assert_eq!(
        r,
        DatasetRef {
            libref: Some("WORK".into()),
            name: "MYDATA".into()
        }
    );
}

#[test]
fn resolve_last_dataset_none_errors() {
    let session = make_session();
    let err = resolve_last_dataset(&None, &session).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("_LAST_") || msg.contains("undefined"),
        "msg: {msg}"
    );
}

// ── squelette MODEL (MQ4.6) ───────────────────────────────────────────

#[test]
fn model_response_reads_ident_and_advances() {
    let src = SourceFile::new("proc foo y = x; run;");
    let mut ts = proc_stream(&src);
    let r = parse_model_response(&mut ts, "expected response variable in MODEL").unwrap();
    assert_eq!(r, "y");
    assert_eq!(ts.peek().kind, TokenKind::Eq);
}

#[test]
fn model_response_non_ident_errors_with_given_message() {
    let src = SourceFile::new("proc foo = x; run;");
    let mut ts = proc_stream(&src);
    let err = parse_model_response(&mut ts, "expected response variable").unwrap_err();
    assert!(err.to_string().contains("expected response variable"));
}

#[test]
fn model_eq_consumes_or_errors() {
    let src = SourceFile::new("proc foo = x; run;");
    let mut ts = proc_stream(&src);
    expect_model_eq(&mut ts, "expected '=' in MODEL statement").unwrap();
    assert!(ts.peek().is_kw("x"));

    let src2 = SourceFile::new("proc foo x; run;");
    let mut ts2 = proc_stream(&src2);
    let err = expect_model_eq(&mut ts2, "expected '=' in MODEL statement").unwrap_err();
    assert!(err.to_string().contains("expected '=' in MODEL statement"));
}

#[test]
fn response_options_event_and_descending() {
    let src = SourceFile::new("proc foo (event='1' descending) = x; run;");
    let mut ts = proc_stream(&src);
    let (event, descending) = parse_response_options(&mut ts);
    assert_eq!(event.as_deref(), Some("1"));
    assert!(descending);
    // Positioned on `=` after the closing paren.
    assert_eq!(ts.peek().kind, TokenKind::Eq);
}

#[test]
fn response_options_absent_consumes_nothing() {
    let src = SourceFile::new("proc foo = x; run;");
    let mut ts = proc_stream(&src);
    let (event, descending) = parse_response_options(&mut ts);
    assert_eq!(event, None);
    assert!(!descending);
    assert_eq!(ts.peek().kind, TokenKind::Eq);
}

#[test]
fn effect_list_stops_at_slash_without_consuming() {
    let src = SourceFile::new("proc foo a b c / noprint; run;");
    let mut ts = proc_stream(&src);
    let effects = parse_effect_list(&mut ts);
    assert_eq!(effects, vec!["a".to_string(), "b".into(), "c".into()]);
    assert_eq!(ts.peek().kind, TokenKind::Slash);
}

#[test]
fn model_lhs_reads_dependents_and_consumes_eq() {
    let src = SourceFile::new("proc foo y1 y2 = a; run;");
    let mut ts = proc_stream(&src);
    let deps = parse_model_lhs(&mut ts);
    assert_eq!(deps, vec!["y1".to_string(), "y2".into()]);
    // `=` consumed; positioned on the first effect.
    assert!(ts.peek().is_kw("a"));
}

#[test]
fn effect_terms_builds_star_chains() {
    let src = SourceFile::new("proc foo a b*c / solution; run;");
    let mut ts = proc_stream(&src);
    let (effects, terms) = parse_effect_terms(&mut ts);
    assert_eq!(effects, vec!["a".to_string(), "b*c".into()]);
    assert_eq!(
        terms,
        vec![vec!["a".to_string()], vec!["b".into(), "c".into()]]
    );
    assert_eq!(ts.peek().kind, TokenKind::Slash);
}
