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
    assert_eq!(
        ast.model.unwrap().event,
        Some(common::ResponseEvent::Value("1".to_string()))
    );
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
            param_explicit: true,
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
            event: Some(common::ResponseEvent::Value("1".into())),
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
    // Ordered response with 3 levels. J02-P1 — the former data were perfectly
    // separated by x (y=1 for x<2, y=2 for 2≤x<3, y=3 for x≥3): the fit
    // diverged and the listing printed NaN standard errors in silence, which
    // is now an explicit ERROR. The overlapping data of the m34
    // logistic_ordinal fixture keep the intent (monotone intercepts).
    let frame = df![
        "y" => [1.0_f64, 2.0, 3.0, 1.0, 2.0, 3.0, 1.0, 2.0, 3.0, 2.0, 3.0, 1.0, 3.0, 2.0],
        "x" => [1.0_f64, 1.0, 1.0, 2.0, 2.0, 2.0, 3.0, 3.0, 3.0, 4.0, 4.0, 4.0, 5.0, 5.0]
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
// « Convergence criterion (XCONV=1E-8) satisfied. ».
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
        !listing.contains("Convergence criterion (XCONV=1E-8) satisfied."),
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
        listing.contains("Convergence criterion (XCONV=1E-8) satisfied."),
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
    // J02-P1 — the saturated information matrix used to yield NaN standard
    // errors printed without diagnostic; the fit is now rejected explicitly
    // after the doc-quoted non-convergence WARNING.
    let err = execute(&ast, &mut session).unwrap_err().to_string();
    assert!(
        err.contains("information matrix of the ordinal logistic model is singular"),
        "err: {err}"
    );
    let log = session.log.into_string();
    assert!(
        log.contains("WARNING"),
        "ordinal separation must produce a WARNING (not a NOTE):\n{log}"
    );
    let listing = session.listing.take_string();
    assert!(
        !listing.contains("Convergence criterion (XCONV=1E-8) satisfied."),
        "ordinal listing must not claim convergence:\n{listing}"
    );
}

// ── J02-P1 : contrat LOGISTIC (replis silencieux supprimés) ──────────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic ou la correction (texte, code de sortie). Oracle de sévérité :
// CONTRIBUTING §5 ; sémantique SAS citée par test.

/// Exécute un programme complet (mode déterministe) : log, listing, code.
fn run_sas(src: &str) -> crate::RunOutcome {
    crate::run(
        src,
        crate::RunOptions {
            deterministic: true,
            ..Default::default()
        },
    )
}

/// 2×2 counts of the m26 fixture (y=1/0, x=1/0, FREQ count).
const COUNTS: &str = "data counts; input y x count; datalines;
1 1 20
1 0 10
0 1 5
0 0 25
;
run;
";

/// Base : `y(desc)` était ignoré (seul `descending` était lu) et
/// `EVENT=FIRST|LAST` était avalé — le modèle portait sur y=0 en silence.
/// SAS/STAT 9.4, The LOGISTIC Procedure, MODEL statement, « Response
/// Variable Options » : DESCENDING (alias DESC) inverse l'ordre des niveaux ;
/// EVENT=FIRST|LAST désigne le premier/dernier niveau ordonné.
#[test]
fn ra_j02_p1_logistic_response_desc_and_event_first_last() {
    for (opts, modeled) in [
        ("(desc)", "y=1."),
        ("(event=last)", "y=1."),
        ("(event=first)", "y=0."),
        ("(desc event=last)", "y=0."),
        ("(event='1')", "y=1."),
    ] {
        let out = run_sas(&format!(
            "{COUNTS}proc logistic data=counts; model y{opts} = x; freq count; run;"
        ));
        assert_eq!(out.exit_code, 0, "{opts}: {}", out.log);
        assert!(
            out.listing.contains(&format!(
                "PROC LOGISTIC is modeling the probability that {modeled}"
            )),
            "{opts}: {}",
            out.listing
        );
    }
}

/// Base : `CLASS x / PARAM=REF REF=FIRST;` lisait `param`, `ref`, `first`
/// comme des variables CLASS ; x restait codée REF=LAST (référence 1).
/// SAS/STAT 9.4, The LOGISTIC Procedure, CLASS statement : les options
/// après `/` s'appliquent à toutes les variables (même sémantique que la
/// forme parenthésée), une option parenthésée prime.
#[test]
fn ra_j02_p1_logistic_class_slash_options() {
    let out = run_sas(&format!(
        "{COUNTS}proc logistic data=counts; class x / param=ref ref=first;
         model y(descending) = x; freq count; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    // REF=FIRST: the design column is level 1 (reference 0), log OR = ln 10.
    assert!(out.listing.contains("x 1"), "{}", out.listing);
    assert!(!out.listing.contains("x 0"), "{}", out.listing);
    assert!(out.listing.contains("2.3026"), "{}", out.listing);
    assert!(!out.log.contains("WARNING"), "{}", out.log);

    // A parenthesized option overrides the global one.
    let out = run_sas(&format!(
        "{COUNTS}proc logistic data=counts; class x(ref=last) / param=reference ref=first;
         model y(descending) = x; freq count; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(out.listing.contains("x 0"), "{}", out.listing);

    // Unsupported global options are the same ERROR as in parentheses.
    for (stmt, msg) in [
        (
            "class x / param=effect;",
            "CLASS PARAM=EFFECT is not supported in PROC LOGISTIC",
        ),
        (
            "class x / missing;",
            "Unknown or unsupported CLASS option 'MISSING' in PROC LOGISTIC.",
        ),
        (
            "class x / ref='1';",
            "CLASS REF='1' is not supported in PROC LOGISTIC",
        ),
    ] {
        let out = run_sas(&format!(
            "{COUNTS}proc logistic data=counts; {stmt} model y = x; freq count; run;"
        ));
        assert_eq!(out.exit_code, 2, "{stmt}: {}", out.log);
        assert!(out.log.contains(msg), "{stmt}: {}", out.log);
    }
}

/// Base : sans PARAM=, le codage REF était appliqué sans diagnostic alors que
/// le défaut SAS est PARAM=EFFECT (SAS/STAT 9.4, The LOGISTIC Procedure,
/// CLASS statement). WARNING provisoire jusqu'à roadmap-avancee J05-P2.
#[test]
fn ra_j02_p1_logistic_default_param_warning() {
    let out = run_sas(&format!(
        "{COUNTS}proc logistic data=counts; class x; model y(descending) = x; freq count; run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert!(
        out.log.contains(
            "WARNING: CLASS variable X is coded with PARAM=REF; the SAS default \
             PARAM=EFFECT is not supported in PROC LOGISTIC. The CLASS parameter \
             estimates differ from SAS, the odds ratios are identical (planned: \
             roadmap-avancee J05-P2)."
        ),
        "{}",
        out.log
    );
    // The fit itself still runs (odds ratio 0 vs 1 = 0.1).
    assert!(out.listing.contains("0.100"), "{}", out.listing);
}

/// Base : LOWER=, UPPER=, STDXBETA=, RESCHI=, RESDEV=, H=, PREDPROBS= étaient
/// sautés jeton par jeton (OUT= sans les colonnes demandées) et un OUTPUT
/// sans OUT= était abandonné sans dataset. ERROR (J05-P4 lèvera la première).
#[test]
fn ra_j02_p1_logistic_output_keywords_error() {
    for kw in [
        "lower=lo",
        "upper=up",
        "stdxbeta=se",
        "reschi=rc",
        "resdev=rd",
        "h=lev",
        "predprobs=i",
    ] {
        let out = run_sas(&format!(
            "{COUNTS}proc logistic data=counts; model y = x; freq count;
             output out=o p=phat {kw}; run;"
        ));
        let bad = kw.split('=').next().unwrap().to_uppercase();
        assert_eq!(out.exit_code, 2, "{kw}: {}", out.log);
        assert!(
            out.log.contains(&format!(
                "The OUTPUT option '{bad}' is not supported in PROC LOGISTIC; it can \
                 affect results and cannot be ignored (planned: roadmap-avancee J05-P4)."
            )),
            "{kw}: {}",
            out.log
        );
        assert!(!out.log.contains("WORK.O has"), "{kw}: {}", out.log);
    }
    let out = run_sas(&format!(
        "{COUNTS}proc logistic data=counts; model y = x; freq count; output p=phat; run;"
    ));
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert!(
        out.log
            .contains("OUTPUT without OUT= is not supported in PROC LOGISTIC"),
        "{}",
        out.log
    );
}

/// Base : réponse ordinale parfaitement séparée → matrice d'information
/// saturée, inversion échouée, SE/Wald/p imprimés « NaN » sans diagnostic.
/// ERROR explicite (le Hessien exact de roadmap-avancee J05-P7 reprendra ce
/// chemin).
#[test]
fn ra_j02_p1_logistic_ordinal_singular_se_error() {
    let out = run_sas(
        "data sep; input y x @@; datalines;
1 1 1 2 2 9 2 10 3 17 3 18
;
run;
proc logistic data=sep; model y = x; run;",
    );
    assert_eq!(out.exit_code, 2, "{}", out.log);
    assert!(
        out.log.contains(
            "The information matrix of the ordinal logistic model is singular in PROC \
             LOGISTIC; standard errors, Wald chi-squares and p-values cannot be computed."
        ),
        "{}",
        out.log
    );
    assert!(!out.listing.contains("NaN"), "{}", out.listing);
}

/// Base : les niveaux CLASS étaient calculés sur toutes les lignes lues ; un
/// niveau présent seulement dans des observations écartées (réponse ou
/// prédicteur manquant) créait une colonne nulle et une ERROR de singularité
/// trompeuse. SAS/STAT 9.4, The LOGISTIC Procedure, « Missing Values » : ces
/// observations ne sont pas utilisées — les niveaux viennent des
/// observations utilisées.
#[test]
fn ra_j02_p1_logistic_class_levels_from_used_obs() {
    let out = run_sas(
        "data cl; input y g $ z count; datalines;
1 a 1 20
1 a 3 5
0 a 1 5
0 a 3 20
1 b 2 10
0 b 2 25
1 b 4 3
0 b 4 8
1 c . 7
. c 2 3
;
run;
proc logistic data=cl; class g(param=ref); model y(descending) = g z; freq count; run;",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    let cli = out
        .listing
        .lines()
        .find(|l| l.trim_start().starts_with("g "))
        .expect("class level row");
    assert_eq!(cli.split_whitespace().collect::<Vec<_>>(), ["g", "a", "b"]);
}

/// Base : « Convergence criterion (GCONV=1E-8) satisfied. » alors que le
/// critère testé est le changement relatif des paramètres
/// (max|Δβ|/(1+max|β|) < 1E-8) : libellé véridique XCONV, binaire et ordinal.
#[test]
fn ra_j02_p1_logistic_convergence_label_xconv() {
    let out = run_sas(&format!(
        "{COUNTS}proc logistic data=counts; model y(descending) = x; freq count; run;"
    ));
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(
        out.listing
            .contains("     Convergence criterion (XCONV=1E-8) satisfied."),
        "{}",
        out.listing
    );
    assert!(!out.listing.contains("GCONV"), "{}", out.listing);
}

/// Base : les instructions LOGISTIC valides non implémentées (ODDSRATIO,
/// UNITS, TEST, STRATA, ROC, SCORE, EXACT…) recevaient « 180-322 … not
/// valid » ; elles portent désormais le message du catalogue du contrat.
/// EFFECTPLOT (graphique seul) → WARNING d'affichage.
#[test]
fn ra_j02_p1_logistic_unsupported_statements() {
    for stmt in [
        "oddsratio x",
        "units x=2",
        "test x=0",
        "strata x",
        "roc 'r' x",
        "score data=counts out=s",
        "exact x",
        "nloptions maxiter=5",
    ] {
        let out = run_sas(&format!(
            "{COUNTS}proc logistic data=counts; model y = x; freq count; {stmt}; run;"
        ));
        let kw = stmt.split_whitespace().next().unwrap().to_uppercase();
        assert_eq!(out.exit_code, 2, "{stmt}: {}", out.log);
        assert!(
            out.log.contains(&format!(
                "The {kw} statement is not supported in PROC LOGISTIC; it can affect \
                 results and cannot be ignored."
            )),
            "{stmt}: {}",
            out.log
        );
        assert!(!out.log.contains("180-322"), "{stmt}: {}", out.log);
    }
    let out = run_sas(&format!(
        "{COUNTS}proc logistic data=counts; model y(descending) = x; freq count; effectplot; run;"
    ));
    assert_eq!(out.exit_code, 1, "{}", out.log);
    assert!(
        out.log.contains(
            "The EFFECTPLOT statement is ignored in PROC LOGISTIC; display customization \
             is not supported."
        ),
        "{}",
        out.log
    );
}
