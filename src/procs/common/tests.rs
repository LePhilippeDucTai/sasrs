//! Tests des combinateurs de parsing partagés par les PROCs.
use super::*;
use crate::source::SourceFile;
use crate::testkit::*;

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
    parse_proc_body(&mut ts, "FOO", |ts, kw| match kw {
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
    parse_proc_body(&mut ts, "FOO", |_ts, _kw| Ok(false)).unwrap();
    assert!(ts.peek().is_kw("data"));
}

#[test]
fn contract_unknown_statement_errors_at_original_span() {
    let src = SourceFile::new("proc foo; bogus x y; var a; run;");
    let mut ts = proc_stream(&src);
    ts.expect_semi().unwrap();
    let expected_span = ts.peek().span;
    let err = parse_proc_body(&mut ts, "FOO", |_ts, _kw| Ok(false)).unwrap_err();
    match err {
        SasError::Parse { msg, span } => {
            assert!(msg.contains("180-322"));
            assert!(msg.contains("BOGUS") && msg.contains("PROC FOO"));
            assert_eq!(span, expected_span);
        }
        other => panic!("expected parse error, got {other:?}"),
    }
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

// Oracles: CONTRIBUTING §5 (severity) and the SAS references in
// docs/support-contract.md (180-322, global statements inside PROC).
#[test]
fn contract_unimplemented_semantic_statements_reject_proc() {
    for (proc_name, statement) in [
        ("glm", "by g"),
        ("logistic", "weight w"),
        ("glm", "freq n"),
        ("glm", "output out=bad"),
        ("glm", "id x"),
        ("glm", "where x=1"),
        ("reg", "class g"),
        ("fastclus maxclusters=2", "id x"),
        ("mixed", "estimate 'diff' x 1"),
        ("glimmix", "lsmeans g"),
        ("reg", "reweight x > 1"),
        ("reg", "refit"),
        ("catalog", "delete f"),
        ("catalog", "copy out=other"),
        ("import", "guessingrows=1000"),
    ] {
        let source = format!("proc {proc_name}; {statement}; run;");
        let src = SourceFile::new(source);
        let mut ts = StatementStream::new(&src).unwrap();
        let err = ts
            .next_block()
            .unwrap()
            .0
            .err()
            .expect(&src.text)
            .to_string();
        assert!(
            err.contains("not supported") && err.contains("cannot be ignored"),
            "{}: {err}",
            src.text
        );
    }
}

#[test]
fn contract_unknown_statement_all_proc_parsers() {
    for header in [
        "anova",
        "append base=a data=b",
        "catalog",
        "cluster",
        "compare",
        "contents",
        "corr",
        "datasets",
        "discrim",
        "distance",
        "export",
        "factor",
        "fastclus maxclusters=2",
        "format",
        "freq",
        "gchart",
        "genmod",
        "glimmix",
        "glm",
        "gplot",
        "import",
        "logistic",
        "means",
        "mixed",
        "npar1way",
        "options",
        "plot",
        "princomp",
        "print",
        "printto",
        "rank",
        "reg",
        "report",
        "sgplot",
        "sort",
        "summary",
        "tabulate",
        "transpose",
        "ttest",
        "univariate",
    ] {
        let src = SourceFile::new(format!("proc {header}; invented xyz; run;"));
        let mut ts = StatementStream::new(&src).unwrap();
        let err = ts.next_block().unwrap().0.err().expect(header).to_string();
        assert!(
            err.contains("180-322") && err.contains("INVENTED"),
            "{header}: {err}"
        );
    }
}

#[test]
fn contract_display_warnings_are_drained_once_even_on_error() {
    let src = SourceFile::new(
        "proc foo; format x 8.2; label x='X'; attrib length label='X'; bogus; run;",
    );
    let mut ts = proc_stream(&src);
    ts.expect_semi().unwrap();
    assert!(parse_proc_body(&mut ts, "FOO", |_ts, _kw| Ok(false)).is_err());
    let effects = ts.take_proc_effects();
    assert_eq!(effects.len(), 3);
    for effect in effects {
        match effect {
            crate::parser::ProcParseEffect::Warning(message) => {
                assert!(message.contains("ignored in PROC FOO"));
            }
            _ => panic!("expected warning"),
        }
    }
    assert!(ts.take_proc_effects().is_empty());
}

#[test]
fn contract_attrib_storage_effect_is_error() {
    for statement in ["attrib x length=3", "attrib x informat=8."] {
        let src = SourceFile::new(format!("proc foo; {statement}; run;"));
        let mut ts = proc_stream(&src);
        ts.expect_semi().unwrap();
        let err = parse_proc_body(&mut ts, "FOO", |_ts, _kw| Ok(false)).unwrap_err();
        assert!(
            err.to_string()
                .contains("ATTRIB statement is not supported")
        );
    }
}

#[test]
fn contract_implemented_statements_keep_their_semantics() {
    let src = SourceFile::new("proc sort data=a out=b; by descending x; run;");
    let mut ts = proc_stream(&src);
    let ast = crate::procs::sort::parse(&mut ts).unwrap();
    assert_eq!(ast.by, vec![("x".to_string(), true)]);
    assert!(ts.take_proc_effects().is_empty());
}

#[test]
fn contract_comments_and_implicit_boundary() {
    let src = SourceFile::new("proc foo; * comment; ;; proc print; run;");
    let mut ts = proc_stream(&src);
    ts.expect_semi().unwrap();
    parse_proc_body(&mut ts, "FOO", |_ts, _kw| Ok(false)).unwrap();
    assert!(ts.peek().is_kw("proc"));
}

#[test]
fn contract_executor_warning_error_and_recovery() {
    let out = crate::run(
        "data t; x=1; output; run; proc print data=t; format x 8.2; var x; run;",
        crate::RunOptions {
            deterministic: true,
            ..Default::default()
        },
    );
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert_eq!(out.log.matches("WARNING:").count(), 1);
    assert!(out.listing.contains("x"));

    let out = crate::run(
        "data t; x=1; output; run; proc sort data=t out=bad; by x; invented; run; proc print data=t; run;",
        crate::RunOptions {
            deterministic: true,
            ..Default::default()
        },
    );
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert!(out.log.contains("180-322"));
    assert!(!out.log.contains("data set WORK.BAD has"));
    assert!(out.listing.contains("x"));
}

#[test]
fn contract_globals_execute_inside_proc() {
    let tmp = tempfile::tempdir().unwrap();
    let src = SourceFile::new(
        "data t; x=1; output; run; proc print data=t; title 'Inner title'; footnote 'Inner foot'; options ls=100; libname here '.'; filename f 'input.sas'; ods select all; var x; run;",
    );
    let mut session = make_session();
    session.base_dir = tmp.path().to_path_buf();
    crate::executor::run_program(&src, &mut session);
    assert_eq!(session.log.errors, 0);
    assert_eq!(session.log.warnings, 0);
    assert!(session.libs.get("HERE").is_ok());
    // Verify actual output, not just that parsing accepted the statements.
    let listing = session.listing.take_string();
    assert!(listing.contains("Inner title"), "{listing}");
    assert!(listing.contains("Inner foot"), "{listing}");
}

#[test]
fn contract_global_error_prevents_proc_output() {
    let outcome = crate::run(
        "data t; x=1; output; run; proc sort data=t out=bad; libname x XLSX 'bad.xlsx'; by x; run;",
        crate::RunOptions {
            deterministic: true,
            ..Default::default()
        },
    );
    assert_eq!(outcome.exit_code, 2, "{}", outcome.log);
    assert!(outcome.log.contains("LIBNAME engine XLSX"));
    assert!(!outcome.log.contains("data set WORK.BAD has"));
}

#[test]
fn contract_globals_survive_later_proc_error_without_leaking_warnings() {
    let outcome = crate::run(
        "data t; x=1; output; run; proc print data=t; title 'Retained'; format x 8.2; invented; run; proc print data=t; run;",
        crate::RunOptions {
            deterministic: true,
            ..Default::default()
        },
    );
    assert_eq!(outcome.exit_code, 2, "{}", outcome.log);
    assert_eq!(outcome.log.matches("WARNING:").count(), 1);
    assert_eq!(outcome.log.matches("180-322").count(), 1);
    assert!(outcome.listing.contains("Retained"));
}

// ───────────── J03-P2 : variance pondérée et partitions WEIGHT ─────────

#[test]
fn weighted_variance_vardef_df_and_weight_oracle() {
    // Oracle weighted_stats « poids-egaux-* » : x=1..8, w=2 partout.
    // CSS = 84, W = 16 : DF → 84/15 = 5.6 ; WEIGHT → 84/(16−32/16) = 6.
    let pairs: Vec<(f64, f64)> = (1..=8).map(|x| (x as f64, 2.0)).collect();
    let df = weighted_variance(&pairs, VarDef::Df).unwrap();
    assert!((df - 5.6).abs() < 1e-12, "DF = {df}");
    let w = weighted_variance(&pairs, VarDef::Weight).unwrap();
    assert!((w - 6.0).abs() < 1e-12, "WEIGHT = {w}");
    // n = 1 → missing, même si le diviseur serait positif (W−1 = 6).
    let one = vec![(42.0, 7.0)];
    assert!(weighted_variance(&one, VarDef::Df).is_none());
}

#[test]
fn weighted_quantile_def5_shared_exact_boundary() {
    // Oracle « borne-cumulee-exacte » : x=[1,2,3], w=[1,1,2], W=4 :
    // médiane t=2 == cum → (2+3)/2 ; Q1 t=1 == cum → (1+2)/2 ; Q3 → 3.
    let pairs = vec![(1.0, 1.0), (2.0, 1.0), (3.0, 2.0)];
    let q = |p: f64| weighted_quantile_def5(&pairs, p).unwrap();
    assert_eq!(q(0.5), 2.5);
    assert_eq!(q(0.25), 1.5);
    assert_eq!(q(0.75), 3.0);
    assert_eq!(q(0.01), 1.0);
    assert_eq!(q(0.99), 3.0);
}

#[test]
fn partition_weighted_strict_nmiss_rule() {
    // x manquant à poids valide → NMiss ; poids invalide (manquant ou ≤ 0)
    // → exclu de N ET de NMiss, quelle que soit la valeur de x.
    let values = vec![
        Value::Num(1.0),
        Value::Num(7.0),
        Value::Num(9.0),
        Value::missing(),
    ];
    let weights = vec![
        Value::Num(2.0),
        Value::missing(),
        Value::Num(0.0),
        Value::Num(3.0),
    ];
    let rows = vec![0, 1, 2, 3];
    let (pairs, nmiss) = partition_weighted_strict(&values, &weights, &rows);
    assert_eq!(pairs, vec![(1.0, 2.0)]);
    assert_eq!(nmiss, 1);
}

#[test]
fn partition_weighted_lax_keeps_zero_weight_in_n() {
    // Défaut UNIVARIATE : poids nul/négatif → poids effectif 0 mais
    // l'observation reste ; poids manquant → aussi 0 (l'obs reste tant que
    // x est non manquant) ; x manquant → NMiss.
    let values = vec![
        Value::Num(1.0),
        Value::Num(5.0),
        Value::Num(9.0),
        Value::Num(4.0),
        Value::missing(),
    ];
    let weights = vec![
        Value::Num(1.0),
        Value::Num(0.0),
        Value::Num(1.0),
        Value::missing(),
        Value::Num(2.0),
    ];
    let rows = vec![0, 1, 2, 3, 4];
    let (pairs, nmiss) = partition_weighted_lax(&values, &weights, &rows);
    assert_eq!(pairs, vec![(1.0, 1.0), (5.0, 0.0), (9.0, 1.0), (4.0, 0.0)]);
    assert_eq!(nmiss, 1);
}
