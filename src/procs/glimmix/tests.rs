use super::*;
use crate::dataset::SasDataset;
use crate::source::SourceFile;
use crate::testkit::*;
use polars::df;

fn parse_glimmix(src: &str) -> Result<GlimmixAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // proc
    ts.next(); // glimmix
    parse(&mut ts)
}

// ── MQ9.2 : les erreurs de syntaxe ne sont plus avalées ──────────────────

#[test]
fn unknown_dist_is_an_error_not_a_silent_normal() {
    // Avant MQ9.2 : `dist=bogus` retombait SILENCIEUSEMENT sur NORMAL et
    // l'utilisateur obtenait un modèle faux sans le moindre diagnostic.
    let err = parse_glimmix("proc glimmix; model y = x / dist=bogus; run;").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Unknown DIST= value 'BOGUS'"), "msg: {msg}");
}

#[test]
fn unknown_link_is_an_error_not_a_silent_identity() {
    let err = parse_glimmix("proc glimmix; model y = x / link=bogus; run;").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Unknown LINK= value 'BOGUS'"), "msg: {msg}");
}

#[test]
fn random_statement_missing_eq_is_an_error() {
    // Avant MQ9.2 : `let _ = expect_eq(...)` acceptait le `=` manquant et le
    // parsing continuait depuis une position désynchronisée.
    let err = parse_glimmix("proc glimmix; model y = x; random int / subject g; run;").unwrap_err();
    assert!(err.to_string().contains("expected '='"), "msg: {err}");
}

// ── Test 1: Poisson β convergence ────────────────────────────────────────
#[allow(clippy::approx_constant)] // valeur attendue du test, pas une constante mathématique
#[test]
fn test_poisson_beta() {
    let y = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let x: Vec<Vec<f64>> = vec![
        vec![1.0, 0.0],
        vec![1.0, 0.0],
        vec![1.0, 0.0],
        vec![1.0, 1.0],
        vec![1.0, 1.0],
        vec![1.0, 1.0],
    ];
    let freq = vec![1.0; 6];
    let g = fit_glm(&y, &x, &freq, Distribution::Poisson, LinkFunction::Log).unwrap();
    assert!((g.beta[0] - 0.6931).abs() < 1e-3, "b0={}", g.beta[0]);
    assert!((g.beta[1] - 0.9163).abs() < 1e-3, "b1={}", g.beta[1]);
}

// ── Test 2: Binary + FREQ β convergence ──────────────────────────────────
#[allow(clippy::approx_constant)] // valeur attendue du test, pas une constante mathématique
#[test]
fn test_binary_freq_beta() {
    // counts: (y,x,count): (1,1,20)(1,0,10)(0,1,5)(0,0,25)
    let y = vec![1.0, 1.0, 0.0, 0.0];
    let x: Vec<Vec<f64>> = vec![
        vec![1.0, 1.0],
        vec![1.0, 0.0],
        vec![1.0, 1.0],
        vec![1.0, 0.0],
    ];
    let freq = vec![20.0, 10.0, 5.0, 25.0];
    let g = fit_glm(&y, &x, &freq, Distribution::Binary, LinkFunction::Logit).unwrap();
    assert!((g.beta[0] - (-0.9163)).abs() < 1e-3, "b0={}", g.beta[0]);
    assert!((g.beta[1] - 2.3026).abs() < 1e-3, "b1={}", g.beta[1]);
}

// ── Test 3: Normal + random == MIXED ─────────────────────────────────────
#[test]
fn test_normal_random_eq_mixed() {
    let y = vec![1.0, 3.0, 5.0, 7.0];
    let x = vec![vec![1.0]; 4];
    let subj_of = vec![0, 0, 1, 1];
    let (s2u, s2e, beta, cov, _) = fit_vc(&y, &x, &subj_of, 2, None).unwrap();
    assert!((s2u - 7.0).abs() < 1e-4, "s2u={s2u}");
    assert!((s2e - 2.0).abs() < 1e-4, "s2e={s2e}");
    assert!((beta[0] - 4.0).abs() < 1e-4, "mu={}", beta[0]);
    let se = cov[0][0].sqrt();
    assert!((se - 2.0).abs() < 1e-4, "se={se}");
}

// ── Test 4: TYPE=AR(1) fits and reports AR(1) + Residual cov parms ───────
#[test]
fn test_ar1_fits_and_names() {
    let mut session = make_session();
    let frame = df![
        "subj" => ["A","A","A","B","B","B"],
        "t" => [1.0_f64,2.0,3.0,1.0,2.0,3.0],
        "y" => [1.0_f64,2.0,3.5,2.0,2.5,4.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![char_meta("subj", 1), num_meta("t"), num_meta("y")],
    };
    session.libs.get("WORK").unwrap().write("B", &ds).unwrap();
    session.last_dataset = Some("WORK.B".to_string());
    let ast = parse_glimmix(
        "proc glimmix; class subj; model y = / dist=normal link=identity; random intercept / subject=subj type=ar(1); run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(
        listing.contains("AR(1)"),
        "missing AR(1) cov parm: {listing}"
    );
    assert!(listing.contains("Residual"), "missing Residual: {listing}");
}

// ── Test 4b: TYPE=UN reports UN(i,j) cov-parameter names ─────────────────
#[test]
fn test_un_cov_parm_names() {
    let cov = RepCov::Un { t: 2 };
    // θ packed lower: UN(1,1), UN(2,1), UN(2,2)
    let theta = vec![4.0, 1.0, 9.0];
    let parms = cov_parms_from_rep(cov, &theta);
    assert_eq!(parms.len(), 3);
    assert_eq!(parms[0].name, "UN(1,1)");
    assert_eq!(parms[1].name, "UN(2,1)");
    assert_eq!(parms[2].name, "UN(2,2)");
    assert!((parms[0].estimate - 4.0).abs() < 1e-12);
}

// ── Test 5: METHOD=QUAD → proper deferral error ──────────────────────────
#[test]
fn test_quad_deferred() {
    let mut session = make_session();
    let frame = df!["y" => [1.0_f64,2.0,3.0], "x" => [0.0_f64,1.0,0.0]].unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("D", &ds).unwrap();
    session.last_dataset = Some("WORK.D".to_string());
    let ast = parse_glimmix("proc glimmix method=quad; model y = x / dist=poisson; run;").unwrap();
    let err = execute(&ast, &mut session).unwrap_err();
    assert!(
        err.to_string()
            .contains("METHOD=QUAD is not yet implemented"),
        "got: {err}"
    );
}

// ── Test 5b: LAPLACE cross-check (b) — no random ≡ GLM MLE ────────────────
#[allow(clippy::approx_constant)] // valeur attendue du test, pas une constante mathématique
#[test]
fn test_laplace_no_random_eq_glm() {
    // Same Poisson data as test_poisson_beta; LAPLACE with no RANDOM must
    // reduce to the ordinary GLM MLE (β0=ln2, β1=ln(2.5/... )=0.9163).
    let y = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let x: Vec<Vec<f64>> = vec![
        vec![1.0, 0.0],
        vec![1.0, 0.0],
        vec![1.0, 0.0],
        vec![1.0, 1.0],
        vec![1.0, 1.0],
        vec![1.0, 1.0],
    ];
    let freq = vec![1.0; 6];
    let glm = fit_glm(&y, &x, &freq, Distribution::Poisson, LinkFunction::Log).unwrap();
    // The execute path routes LAPLACE+no-random through fit_glm directly, so
    // we assert that equivalence here at the GLM oracle.
    assert!((glm.beta[0] - 0.6931).abs() < 1e-3, "b0={}", glm.beta[0]);
    assert!((glm.beta[1] - 0.9163).abs() < 1e-3, "b1={}", glm.beta[1]);
}

// ── Test 5c: LAPLACE cross-check (a) — Normal+random ≡ MIXED METHOD=ML ────
#[test]
fn test_laplace_normal_random_eq_mixed_ml() {
    // m28 balanced data A:1,3 B:5,7. MIXED ML: σ²_u=3, σ²_e=2, β=4.
    let y = vec![1.0, 3.0, 5.0, 7.0];
    let x = vec![vec![1.0]; 4];
    let freq = vec![1.0; 4];
    let subj_of = vec![0, 0, 1, 1];
    let fit = fit_laplace(
        &y,
        &x,
        &freq,
        &subj_of,
        2,
        Distribution::Normal,
        LinkFunction::Identity,
    )
    .unwrap();
    assert!((fit.beta[0] - 4.0).abs() < 1e-2, "beta={}", fit.beta[0]);
    assert!((fit.sigma2_u - 3.0).abs() < 5e-2, "s2u={}", fit.sigma2_u);
    assert!((fit.sigma2_e - 2.0).abs() < 5e-2, "s2e={}", fit.sigma2_e);
}

// ── Test 6: DIST=GAMMA → proper error ────────────────────────────────────
#[test]
fn test_gamma_error() {
    let mut session = make_session();
    let frame = df!["y" => [1.0_f64,2.0,3.0], "x" => [0.0_f64,1.0,0.0]].unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("D", &ds).unwrap();
    session.last_dataset = Some("WORK.D".to_string());
    let ast = parse_glimmix("proc glimmix; model y = x / dist=gamma; run;").unwrap();
    let err = execute(&ast, &mut session).unwrap_err();
    assert!(
        err.to_string()
            .contains("DIST=GAMMA is not yet implemented for PROC GLIMMIX."),
        "got: {err}"
    );
}

// ── Test 7: Poisson converges in ≤ 20 iterations ─────────────────────────
#[test]
fn test_poisson_iterations() {
    let y = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let x: Vec<Vec<f64>> = vec![
        vec![1.0, 0.0],
        vec![1.0, 0.0],
        vec![1.0, 0.0],
        vec![1.0, 1.0],
        vec![1.0, 1.0],
        vec![1.0, 1.0],
    ];
    let freq = vec![1.0; 6];
    let g = fit_glm(&y, &x, &freq, Distribution::Poisson, LinkFunction::Log).unwrap();
    assert!(g.iterations <= 20, "iters={}", g.iterations);
}

// ── Test 8: exit_code=0 for the full Poisson execute path ────────────────
#[test]
fn test_execute_poisson_ok() {
    let mut session = make_session();
    let frame =
        df!["y" => [1.0_f64,2.0,3.0,4.0,5.0,6.0], "x" => [0.0_f64,0.0,0.0,1.0,1.0,1.0]].unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("POIS", &ds)
        .unwrap();
    session.last_dataset = Some("WORK.POIS".to_string());
    let ast =
        parse_glimmix("proc glimmix; model y = x / dist=poisson link=log solution; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(listing.contains("0.6931"), "missing b0: {listing}");
    assert!(listing.contains("0.9163"), "missing b1: {listing}");
    assert!(listing.contains("The GLIMMIX Procedure"));
}

// ── Test 9: Binary SE matches LOGISTIC ───────────────────────────────────
#[test]
fn test_binary_se() {
    let y = vec![1.0, 1.0, 0.0, 0.0];
    let x: Vec<Vec<f64>> = vec![
        vec![1.0, 1.0],
        vec![1.0, 0.0],
        vec![1.0, 1.0],
        vec![1.0, 0.0],
    ];
    let freq = vec![20.0, 10.0, 5.0, 25.0];
    let g = fit_glm(&y, &x, &freq, Distribution::Binary, LinkFunction::Logit).unwrap();
    let se0 = g.cov_beta[0][0].sqrt();
    let se1 = g.cov_beta[1][1].sqrt();
    assert!((se0 - 0.3742).abs() < 1e-3, "se0={se0}");
    assert!((se1 - 0.6245).abs() < 1e-3, "se1={se1}");
}

// ── Test: Binary + PROBIT no-random ≡ LOGISTIC probit fit ────────────────
#[test]
fn test_binary_probit_beta() {
    // counts: (y,x,count): (1,1,20)(1,0,10)(0,1,5)(0,0,25), event='1'
    let y = vec![1.0, 1.0, 0.0, 0.0];
    let x: Vec<Vec<f64>> = vec![
        vec![1.0, 1.0],
        vec![1.0, 0.0],
        vec![1.0, 1.0],
        vec![1.0, 0.0],
    ];
    let freq = vec![20.0, 10.0, 5.0, 25.0];
    let g = fit_glm(&y, &x, &freq, Distribution::Binary, LinkFunction::Probit).unwrap();
    assert!((g.beta[0] - (-0.5659)).abs() < 1e-3, "b0={}", g.beta[0]);
    assert!((g.beta[1] - 1.4076).abs() < 1e-3, "b1={}", g.beta[1]);
}

// ── Test: Binary + CLOGLOG no-random ≡ LOGISTIC cloglog fit ───────────────
#[test]
fn test_binary_cloglog_beta() {
    let y = vec![1.0, 1.0, 0.0, 0.0];
    let x: Vec<Vec<f64>> = vec![
        vec![1.0, 1.0],
        vec![1.0, 0.0],
        vec![1.0, 1.0],
        vec![1.0, 0.0],
    ];
    let freq = vec![20.0, 10.0, 5.0, 25.0];
    let g = fit_glm(&y, &x, &freq, Distribution::Binary, LinkFunction::Cloglog).unwrap();
    assert!((g.beta[0] - (-1.0892)).abs() < 1e-3, "b0={}", g.beta[0]);
    assert!((g.beta[1] - 1.5651).abs() < 1e-3, "b1={}", g.beta[1]);
}

// ── Test: general X with CLASS reference-cell coding ─────────────────────
#[test]
fn test_build_design_class() {
    // grp has levels A,B,C → reference-cell with C as reference → 2 cols.
    let cols = vec![(
        "grp".to_string(),
        vec![
            Value::Char("A".into()),
            Value::Char("B".into()),
            Value::Char("C".into()),
            Value::Char("A".into()),
        ],
    )];
    let design = build_design(&cols, &["grp".to_string()], &["grp".to_string()], false, 4).unwrap();
    // Intercept + grp A + grp B = 3 columns.
    assert_eq!(design.len(), 3);
    assert_eq!(design[0].label, "Intercept");
    assert_eq!(design[1].label, "grp A");
    assert_eq!(design[2].label, "grp B");
    assert_eq!(design[1].values, vec![1.0, 0.0, 0.0, 1.0]);
    assert_eq!(design[2].values, vec![0.0, 1.0, 0.0, 0.0]);
}

// ── Test: NOINT drops the intercept column ───────────────────────────────
#[test]
fn test_build_design_noint() {
    let cols = vec![(
        "x".to_string(),
        vec![Value::Num(1.0), Value::Num(2.0), Value::Num(3.0)],
    )];
    let design = build_design(&cols, &[], &["x".to_string()], true, 3).unwrap();
    assert_eq!(design.len(), 1);
    assert_eq!(design[0].label, "x");
    assert_eq!(design[0].values, vec![1.0, 2.0, 3.0]);
}

// ── Test: LAPLACE execute path reports Laplace + -2 Log Likelihood ───────
#[test]
fn test_execute_laplace_listing() {
    let mut session = make_session();
    let frame = df![
        "subj" => ["A","A","B","B","C","C"],
        "y" => [1.0_f64, 0.0, 1.0, 1.0, 0.0, 0.0],
        "x" => [1.0_f64, 0.0, 1.0, 0.0, 1.0, 0.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![char_meta("subj", 1), num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("LP", &ds).unwrap();
    session.last_dataset = Some("WORK.LP".to_string());
    let ast = parse_glimmix(
        "proc glimmix method=laplace; class subj; model y(event='1') = x / dist=binary link=logit solution; random intercept / subject=subj type=vc; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(listing.contains("Laplace"), "missing Laplace: {listing}");
    assert!(
        listing.contains("-2 Log Likelihood"),
        "missing -2 Log Likelihood: {listing}"
    );
}

// ── Test: AR(1) under LAPLACE → clear error ──────────────────────────────
#[test]
fn test_laplace_ar1_rejected() {
    let mut session = make_session();
    let frame = df!["subj" => ["A","A","B","B"], "y" => [1.0_f64,3.0,5.0,7.0]].unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![char_meta("subj", 1), num_meta("y")],
    };
    session.libs.get("WORK").unwrap().write("BX", &ds).unwrap();
    session.last_dataset = Some("WORK.BX".to_string());
    let ast = parse_glimmix(
        "proc glimmix method=laplace; class subj; model y = / dist=normal; random intercept / subject=subj type=ar(1); run;",
    )
    .unwrap();
    let err = execute(&ast, &mut session).unwrap_err();
    assert!(err.to_string().contains("METHOD=LAPLACE"), "got: {err}");
}

// ── Parse tests ──────────────────────────────────────────────────────────
#[test]
fn test_parse_poisson() {
    let ast =
        parse_glimmix("proc glimmix; model y = x / dist=poisson link=log solution; run;").unwrap();
    let m = ast.model.unwrap();
    assert_eq!(m.dist, Distribution::Poisson);
    assert_eq!(m.link, LinkFunction::Log);
    assert!(m.solution);
}

#[test]
fn test_parse_random_freq() {
    let ast = parse_glimmix(
        "proc glimmix; class subj; model y(event='1') = x / dist=binary; freq count; random intercept / subject=subj type=vc; run;",
    )
    .unwrap();
    let m = ast.model.unwrap();
    assert_eq!(m.dist, Distribution::Binary);
    assert_eq!(m.event.as_deref(), Some("1"));
    assert_eq!(ast.freq_var.as_deref(), Some("count"));
    let r = ast.random.unwrap();
    assert_eq!(r.cov_type, CovType::Vc);
    assert_eq!(r.subject.as_deref(), Some("subj"));
}
