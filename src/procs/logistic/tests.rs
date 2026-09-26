use super::*;
use crate::dataset::SasDataset;
use crate::session::Session;
use crate::source::SourceFile;
use crate::testkit::*;
use polars::df;

fn parse_logistic(src: &str) -> Result<LogisticAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // proc
    ts.next(); // logistic
    parse(&mut ts)
}

// Helper: create the 2x2 oracle dataset
fn make_oracle_session() -> (Session, LogisticAst) {
    let session = make_session();
    // 4 rows (we use freq_var instead of repeated rows)
    let frame = df![
        "y" => [1.0_f64, 1.0, 0.0, 0.0],
        "x" => [1.0_f64, 0.0, 1.0, 0.0],
        "count" => [20.0_f64, 10.0, 5.0, 25.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x"), num_meta("count")],
    };
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("COUNTS", &ds)
        .unwrap();

    let ast = LogisticAst {
        data_options: LogisticDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "COUNTS".into(),
            }),
            descending: false,
        },
        class_vars: vec![],
        model: Some(LogisticModel {
            response: "y".into(),
            event: None,
            descending: true,
            predictors: vec!["x".into()],
            noprint: false,
            link: Link::Logit,
        }),
        freq_var: Some("count".into()),
        outputs: vec![],
    };
    (session, ast)
}

#[test]
fn test_parse_basic() {
    let ast = parse_logistic("proc logistic; model y = x; run;").unwrap();
    let m = ast.model.unwrap();
    assert_eq!(m.response, "y");
    assert_eq!(m.predictors, vec!["x"]);
    assert!(!m.descending);
    assert!(m.event.is_none());
}

#[test]
fn test_parse_descending() {
    let ast = parse_logistic("proc logistic; model y(descending) = x; run;").unwrap();
    assert!(ast.model.unwrap().descending);
}

#[test]
fn test_parse_event() {
    let ast = parse_logistic("proc logistic; model y(event='1') = x; run;").unwrap();
    assert_eq!(ast.model.unwrap().event, Some("1".to_string()));
}

#[test]
fn test_parse_freq() {
    let ast = parse_logistic("proc logistic; model y = x; freq cnt; run;").unwrap();
    assert_eq!(ast.freq_var, Some("cnt".to_string()));
}

#[test]
fn test_parse_class_allowed() {
    // CLASS declaration is valid at parse time (error only at execution)
    let ast = parse_logistic("proc logistic; class z; model y = x; run;").unwrap();
    assert_eq!(ast.class_vars.len(), 1);
    assert_eq!(ast.class_vars[0].name, "z");
    assert!(!ast.class_vars[0].ref_first);
}

#[test]
fn test_execute_beta_oracle() {
    let (mut session, ast) = make_oracle_session();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    // β₁ ≈ 2.3026
    assert!(
        listing.contains("2.3026") || listing.contains("2.302"),
        "β₁ not found in listing: {listing}"
    );
}

#[test]
fn test_execute_or_oracle() {
    let (mut session, ast) = make_oracle_session();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    // OR = exp(β₁) ≈ 10.000
    assert!(
        listing.contains("10.000") || listing.contains("10.0000"),
        "OR not found in listing: {listing}"
    );
}

#[test]
fn test_execute_se_oracle() {
    let (mut session, ast) = make_oracle_session();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    // SE(β₁) ≈ 0.6245
    assert!(
        listing.contains("0.6245") || listing.contains("0.624"),
        "SE not found in listing: {listing}"
    );
}

#[test]
fn test_execute_neg2logl() {
    let (mut session, ast) = make_oracle_session();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    // -2LogL ≈ 66.8990
    assert!(
        listing.contains("66.899") || listing.contains("66.8990"),
        "-2LogL not found in listing: {listing}"
    );
}

#[test]
fn test_execute_lr_test() {
    let (mut session, ast) = make_oracle_session();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    // LR χ² ≈ 16.279
    assert!(
        listing.contains("16.27") || listing.contains("16.279"),
        "LR test not found in listing: {listing}"
    );
}

// CLASS x (numeric 0/1) must reproduce the continuous-x fit: OR = 10,
// log-odds-ratio ln(10) ≈ 2.3026 (ref = last level = 1, so the modeled
// dummy is for level 0). With ref=last, the non-reference dummy is level 0,
// giving β for "0 vs 1" = −ln(10); SAS prints "x 0 vs 1" OR = 0.1. To get
// the same OR=10 as continuous x, we instead use a CLASS with the level
// ordering matching x. Verify the magnitude of the slope is ln(10).
#[test]
fn test_execute_class_reproduces_binary_or() {
    let session = make_session();
    let frame = df![
        "y" => [1.0_f64, 1.0, 0.0, 0.0],
        "x" => ["1", "0", "1", "0"],
        "count" => [20.0_f64, 10.0, 5.0, 25.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), char_meta("x", 8), num_meta("count")],
    };
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("CCLASS", &ds)
        .unwrap();

    let ast = LogisticAst {
        data_options: LogisticDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "CCLASS".into(),
            }),
            descending: false,
        },
        class_vars: vec![ClassVar {
            name: "x".into(),
            ref_first: false,
        }],
        model: Some(LogisticModel {
            response: "y".into(),
            event: None,
            descending: true,
            predictors: vec!["x".into()],
            noprint: false,
            link: Link::Logit,
        }),
        freq_var: Some("count".into()),
        outputs: vec![],
    };
    let mut session = session;
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    // ref = last level "1"; non-ref dummy is "0". β = ln(odds(0)/odds(1)) =
    // ln( (10/25)/(20/5) ) = ln(0.1) = −2.3026 → OR 0.1000. Magnitude ln(10).
    assert!(
        listing.contains("Class Level Information"),
        "missing Class Level Information: {listing}"
    );
    assert!(
        listing.contains("-2.3026") || listing.contains("2.3026"),
        "CLASS slope magnitude ln(10) not found: {listing}"
    );
    assert!(
        listing.contains("0.1000") || listing.contains("x 0 vs 1"),
        "CLASS odds ratio row not found: {listing}"
    );
}

fn tiny_link_session(link: Link) -> (Session, LogisticAst) {
    let session = make_session();
    let frame = df![
        "y" => [0.0_f64, 0.0, 1.0, 1.0, 1.0, 0.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 2.5]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("TINY", &ds)
        .unwrap();
    let ast = LogisticAst {
        data_options: LogisticDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "TINY".into(),
            }),
            descending: false,
        },
        class_vars: vec![],
        model: Some(LogisticModel {
            response: "y".into(),
            event: Some("1".into()),
            descending: false,
            predictors: vec!["x".into()],
            noprint: false,
            link,
        }),
        freq_var: None,
        outputs: vec![],
    };
    (session, ast)
}

#[test]
fn test_execute_probit_converges() {
    let (mut session, ast) = tiny_link_session(Link::Probit);
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(listing.contains("binary probit"), "model line: {listing}");
    // Estimates must be finite (no NaN/inf printed).
    assert!(
        !listing.contains("NaN") && !listing.contains("inf"),
        "{listing}"
    );
    // No odds-ratio table for non-logit links.
    assert!(
        !listing.contains("Odds Ratio Estimates"),
        "probit must omit odds ratios: {listing}"
    );
}

#[test]
fn test_execute_cloglog_converges() {
    let (mut session, ast) = tiny_link_session(Link::Cloglog);
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(listing.contains("binary cloglog"), "model line: {listing}");
    assert!(
        !listing.contains("NaN") && !listing.contains("inf"),
        "{listing}"
    );
}

#[test]
fn test_execute_ordinal_monotone_intercepts() {
    let session = make_session();
    // Ordered response with 3 levels, x increasing with category.
    let frame = df![
        "y" => [1.0_f64, 1.0, 2.0, 2.0, 3.0, 3.0, 1.0, 2.0, 3.0, 2.0],
        "x" => [1.0_f64, 1.5, 2.0, 2.5, 3.0, 3.5, 1.2, 2.2, 3.2, 2.4]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("ORD", &ds).unwrap();
    let ast = LogisticAst {
        data_options: LogisticDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "ORD".into(),
            }),
            descending: false,
        },
        class_vars: vec![],
        model: Some(LogisticModel {
            response: "y".into(),
            event: None,
            descending: false,
            predictors: vec!["x".into()],
            noprint: false,
            link: Link::Logit,
        }),
        freq_var: None,
        outputs: vec![],
    };
    let mut session = session;
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(
        listing.contains("cumulative logit"),
        "ordinal model not used: {listing}"
    );
    assert!(
        listing.contains("Intercept 1") && listing.contains("Intercept 2"),
        "multiple intercepts not printed: {listing}"
    );
    // Monotone intercepts α_1 < α_2: parse the Estimate column from the
    // "Intercept j" rows and assert ordering.
    let parse_intercept = |label: &str| -> f64 {
        let line = listing
            .lines()
            .find(|l| l.trim_start().starts_with(label))
            .unwrap_or_else(|| panic!("no line for {label}: {listing}"));
        // Columns: Parameter DF Estimate ... → 3rd whitespace field.
        let fields: Vec<&str> = line.split_whitespace().collect();
        // "Intercept" "1" "1" "<est>" ...  → index 3.
        fields[3].parse::<f64>().expect("estimate parse")
    };
    let a1 = parse_intercept("Intercept 1");
    let a2 = parse_intercept("Intercept 2");
    assert!(a1 < a2, "intercepts not monotone: a1={a1} a2={a2}");
}

#[test]
fn test_output_predicted_in_unit_interval() {
    let (mut session, mut ast) = make_oracle_session();
    ast.outputs = vec![LogisticOutput {
        out: DatasetRef {
            libref: Some("WORK".into()),
            name: "PRED".into(),
        },
        predicted: Some("phat".into()),
        xbeta: Some("eta".into()),
    }];
    execute(&ast, &mut session).unwrap();
    let (out, _notes) = session.libs.get("WORK").unwrap().read("PRED").unwrap();
    let pcol = out
        .vars
        .iter()
        .position(|v| v.name.eq_ignore_ascii_case("phat"))
        .expect("phat column");
    let vals = decode_column(&out, pcol).unwrap();
    for v in &vals {
        if let Some(p) = value_to_num(v) {
            assert!((0.0..=1.0).contains(&p), "predicted prob out of range: {p}");
        }
    }
    let log = session.log.into_string();
    assert!(
        log.contains("WORK.PRED has"),
        "creation NOTE missing: {log}"
    );
}

// ── J02-P5 : model_fallback_* — plus de replis silencieux ──────────────
//
// Syntaxe de référence : SAS/STAT 9.4 User's Guide, The LOGISTIC Procedure
// (PROC statement DESCENDING/ORDER=, MODEL statement LINK=, CLASS statement
// PARAM=/REF=).
// https://support.sas.com/documentation/cdl/en/statug/68162/HTML/default/statug_logistic_syntax_toc.htm

#[test]
fn model_fallback_logistic_unknown_link_is_error() {
    // Base : LINK=BOGUS retombait silencieusement sur LINK=LOGIT.
    let err = parse_logistic("proc logistic; model y = x / link=bogus; run;").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Unknown LINK= value 'BOGUS'"), "msg: {msg}");
}

#[test]
fn model_fallback_logistic_link_logit_nonregression() {
    // Non-régression : les trois liens rendus passent toujours.
    let cases = [
        ("logit", Link::Logit),
        ("probit", Link::Probit),
        ("cloglog", Link::Cloglog),
    ];
    for (lk, want) in cases {
        let src = format!("proc logistic; model y = x / link={lk}; run;");
        let ast = parse_logistic(&src).unwrap();
        assert_eq!(ast.model.unwrap().link, want, "link={lk}");
    }
}

#[test]
fn model_fallback_logistic_unknown_model_option_is_error() {
    // Base : toute option MODEL inconnue était ignorée en silence.
    let err = parse_logistic("proc logistic; model y = x / rsquare; run;").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("MODEL option 'RSQUARE'"), "msg: {msg}");
}

#[test]
fn model_fallback_logistic_order_is_error() {
    // Base : ORDER= était ignoré en silence (ordre des niveaux non honoré).
    let err = parse_logistic("proc logistic data=d order=internal; model y = x; run;").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("ORDER="), "msg: {msg}");
}

#[test]
fn model_fallback_logistic_proc_descending_is_honored() {
    // Base : l'option PROC DESCENDING était ignorée en silence.
    let ast = parse_logistic("proc logistic descending; model y = x; run;").unwrap();
    assert!(ast.data_options.descending);
    assert!(ast.model.unwrap().descending);
}

#[test]
fn model_fallback_logistic_class_param_ref_parsed() {
    // Base : `class a(param=ref ref=first) b;` enregistrait AUSSI param/ref/
    // first comme variables CLASS (jetons avalés pour des variables).
    let ast =
        parse_logistic("proc logistic; class a(param=ref ref=first) b; model y = x; run;").unwrap();
    assert_eq!(ast.class_vars.len(), 2, "vars: {:?}", ast.class_vars);
    assert_eq!(ast.class_vars[0].name, "a");
    assert!(ast.class_vars[0].ref_first);
    assert_eq!(ast.class_vars[1].name, "b");
    assert!(!ast.class_vars[1].ref_first);
}

#[test]
fn model_fallback_logistic_class_param_ref_last_default() {
    let ast = parse_logistic("proc logistic; class a(param=ref); model y = x; run;").unwrap();
    assert_eq!(ast.class_vars.len(), 1);
    assert!(!ast.class_vars[0].ref_first, "REF=LAST par défaut");
}

#[test]
fn model_fallback_logistic_class_param_non_ref_is_error() {
    // Base : PARAM=GLM était avalé sans effet (codage différent en silence).
    let err = parse_logistic("proc logistic; class a(param=glm); model y = x; run;").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("PARAM=GLM is not supported"), "msg: {msg}");
}

#[test]
fn model_fallback_logistic_class_ref_invalid_is_error() {
    let err = parse_logistic("proc logistic; class a(param=ref ref=middle); model y = x; run;")
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("REF=MIDDLE is invalid"), "msg: {msg}");
}

#[test]
fn model_fallback_logistic_interaction_is_error() {
    // Base : `a*b` était aplati en prédicteurs a et b.
    let err = parse_logistic("proc logistic; model y = a*b; run;").unwrap_err();
    assert!(err.to_string().contains("Interaction"), "err: {err}");
}

// ── J02-P6 : convergence_* — convergence véridique ─────────────────────
//
// Référence : SAS/STAT 9.4 User's Guide, The LOGISTIC Procedure, Details:
// Computational Details — « Failure to Converge » : en cas de séparation
// (quasi-)complète SAS émet « WARNING: The maximum likelihood estimate may
// not exist. », jamais une NOTE ni un silence ; le listing affiche
// « Iteration limit reached without convergence. » au lieu de
// « Convergence criterion (GCONV=1E-8) satisfied. ».
// https://support.sas.com/documentation/cdl/en/statug/68162/HTML/default/statug_logistic_details_toc.htm

#[test]
fn convergence_logistic_binary_separation_is_warning() {
    // Complete separation: y=0 for small x, y=1 for large x (MLE does not
    // exist). PROC LOGISTIC proceeds with the last iterate but must WARN.
    let session = make_session();
    let frame = df![
        "y" => [0.0_f64, 0.0, 0.0, 1.0, 1.0, 1.0],
        "x" => [1.0_f64, 2.0, 3.0, 9.0, 10.0, 11.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("SEPB", &ds)
        .unwrap();
    let ast = LogisticAst {
        data_options: LogisticDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "SEPB".into(),
            }),
            descending: false,
        },
        class_vars: vec![],
        model: Some(LogisticModel {
            response: "y".into(),
            event: None,
            descending: false,
            predictors: vec!["x".into()],
            noprint: false,
            link: Link::Logit,
        }),
        freq_var: None,
        outputs: vec![],
    };
    let mut session = session;
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(
        log.contains("WARNING") && log.contains("maximum likelihood estimate may not exist"),
        "SAS separation WARNING missing:\n{log}"
    );
    let listing = session.listing.take_string();
    assert!(
        listing.contains("Iteration limit reached without convergence."),
        "listing must report the failure:\n{listing}"
    );
    assert!(
        !listing.contains("Convergence criterion (GCONV=1E-8) satisfied."),
        "listing must not claim convergence on separation:\n{listing}"
    );
}

#[test]
fn convergence_logistic_binary_converged_fit_still_claims_satisfied() {
    // Non-regression: the oracle fit converges and keeps the satisfied line.
    let (mut session, ast) = make_oracle_session();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(
        listing.contains("Convergence criterion (GCONV=1E-8) satisfied."),
        "converged fit must keep the status line:\n{listing}"
    );
}

#[test]
fn convergence_logistic_ordinal_separation_is_warning() {
    // Ordinal response perfectly separated by x: the cumulative-logit ML
    // estimate diverges; the Newton loop hits its 25-iteration limit (the
    // singular-step break also reports the same doc-quoted warning).
    let session = make_session();
    let frame = df![
        "y" => [1.0_f64, 1.0, 2.0, 2.0, 3.0, 3.0],
        "x" => [1.0_f64, 2.0, 9.0, 10.0, 17.0, 18.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("SEPO", &ds)
        .unwrap();
    let ast = LogisticAst {
        data_options: LogisticDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "SEPO".into(),
            }),
            descending: false,
        },
        class_vars: vec![],
        model: Some(LogisticModel {
            response: "y".into(),
            event: None,
            descending: false,
            predictors: vec!["x".into()],
            noprint: false,
            link: Link::Logit,
        }),
        freq_var: None,
        outputs: vec![],
    };
    let mut session = session;
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(
        log.contains("WARNING"),
        "ordinal separation must produce a WARNING (not a NOTE):\n{log}"
    );
    let listing = session.listing.take_string();
    assert!(
        !listing.contains("Convergence criterion (GCONV=1E-8) satisfied."),
        "ordinal listing must not claim convergence:\n{listing}"
    );
}
