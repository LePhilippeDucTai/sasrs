use super::*;
use crate::source::SourceFile;

fn parse_mixed(src: &str) -> Result<MixedAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // proc
    ts.next(); // mixed
    parse(&mut ts)
}

// Oracle dataset: balanced one-way, y = (1,3,5,7), subjects A,A,B,B.
fn oracle() -> (Vec<f64>, Vec<Vec<f64>>, Vec<usize>) {
    let y = vec![1.0, 3.0, 5.0, 7.0];
    let x = vec![vec![1.0]; 4];
    let subj_of = vec![0, 0, 1, 1];
    (y, x, subj_of)
}

// ── parse tests ──

#[test]
fn test_parse_basic() {
    let ast = parse_mixed(
        "proc mixed; class subj; model y = / solution; random intercept / subject=subj type=vc; run;",
    )
    .unwrap();
    assert_eq!(ast.method, Method::Reml);
    assert_eq!(ast.class_vars, vec!["subj"]);
    let m = ast.model.unwrap();
    assert_eq!(m.response, "y");
    assert!(m.fixed.is_empty());
    assert!(m.solution);
    let r = ast.random.unwrap();
    assert_eq!(r.effects, vec!["intercept"]);
    assert_eq!(r.subject.as_deref(), Some("subj"));
    assert_eq!(r.cov_type, CovType::Vc);
}

#[test]
fn test_parse_method_ml() {
    let ast = parse_mixed(
        "proc mixed method=ml; class subj; model y = ; random intercept / subject=subj; run;",
    )
    .unwrap();
    assert_eq!(ast.method, Method::Ml);
}

#[test]
fn test_parse_type_cs_and_ar() {
    let ast = parse_mixed(
        "proc mixed; class s; model y = ; random intercept / subject=s type=cs; run;",
    )
    .unwrap();
    assert_eq!(ast.random.unwrap().cov_type, CovType::Cs);

    let ast2 = parse_mixed(
        "proc mixed; class s; model y = ; random intercept / subject=s type=ar(1); run;",
    )
    .unwrap();
    assert_eq!(ast2.random.unwrap().cov_type, CovType::Ar1);
}

#[test]
fn test_parse_lsmeans_estimate_contrast() {
    let ast = parse_mixed(
        "proc mixed covtest; class g; model y = g / solution; \
         lsmeans g / diff pdiff cl alpha=0.1; \
         estimate 'a vs b' g 1 -1; contrast 'c' g 1 -1; run;",
    )
    .unwrap();
    assert!(ast.covtest);
    assert_eq!(ast.lsmeans.len(), 1);
    assert_eq!(ast.lsmeans[0].effect, "g");
    assert!(ast.lsmeans[0].diff);
    assert!(ast.lsmeans[0].pdiff);
    assert!(ast.lsmeans[0].cl);
    assert!((ast.lsmeans[0].alpha - 0.1).abs() < 1e-12);
    assert_eq!(ast.estimate_labels, vec!["a vs b"]);
    assert_eq!(ast.contrast_labels, vec!["c"]);
}

// ── invariant tests (the verified oracle) ──

#[test]
fn test_reml_variance_components() {
    let (y, x, subj_of) = oracle();
    let fit = fit_mixed(&y, &x, &subj_of, 2, Method::Reml, false).unwrap();
    assert!((fit.sigma2_u - 7.0).abs() < 1e-6, "sigma2_u={}", fit.sigma2_u);
    assert!((fit.sigma2_e - 2.0).abs() < 1e-6, "sigma2_e={}", fit.sigma2_e);
}

#[test]
fn test_ml_variance_components() {
    let (y, x, subj_of) = oracle();
    let fit = fit_mixed(&y, &x, &subj_of, 2, Method::Ml, false).unwrap();
    assert!((fit.sigma2_u - 3.0).abs() < 1e-6, "sigma2_u={}", fit.sigma2_u);
    assert!((fit.sigma2_e - 2.0).abs() < 1e-6, "sigma2_e={}", fit.sigma2_e);
}

#[test]
fn test_reml_ne_ml() {
    let (y, x, subj_of) = oracle();
    let reml = fit_mixed(&y, &x, &subj_of, 2, Method::Reml, false).unwrap();
    let ml = fit_mixed(&y, &x, &subj_of, 2, Method::Ml, false).unwrap();
    assert!((reml.sigma2_u - ml.sigma2_u).abs() > 1.0);
}

#[test]
fn test_intercept_estimate_and_se() {
    let (y, x, subj_of) = oracle();
    let fit = fit_mixed(&y, &x, &subj_of, 2, Method::Reml, false).unwrap();
    // μ̂ = 4.0
    assert!((fit.beta[0] - 4.0).abs() < 1e-6, "beta={}", fit.beta[0]);
    // SE(μ̂) = sqrt(Var) = 2.0
    let se = fit.cov_beta[0][0].sqrt();
    assert!((se - 2.0).abs() < 1e-4, "se={}", se);
}

#[test]
fn test_pvalue_oracle() {
    // t = 2, df = 1 → two-sided p = 0.2952.
    let p = 2.0 * (1.0 - student_t_cdf(2.0, 1.0));
    assert!((p - 0.2952).abs() < 1e-4, "p={p}");
}

#[test]
fn test_ar1_random_intercept_defers_to_repeated() {
    // TYPE=AR(1) directly on a RANDOM intercept is not implemented; it must
    // produce a clear error directing the user to REPEATED.
    let session_ds = small_ds();
    let (mut session, _) = session_ds;
    let ast = parse_mixed(
        "proc mixed; class subj; model y = ; random intercept / subject=subj type=ar(1); run;",
    )
    .unwrap();
    let err = execute(&ast, &mut session).unwrap_err();
    assert!(
        err.to_string().contains("REPEATED"),
        "got: {err}"
    );
}

/// Build a small Session with a WORK.B dataset and return it.
fn small_ds() -> (crate::session::Session, ()) {
    use crate::dataset::{SasDataset, VarMeta};
    use crate::session::Session;
    use crate::value::VarType;
    use polars::df;
    use std::path::PathBuf;

    let mut session = Session::new(None, PathBuf::from("."), true).unwrap();
    let frame = df![
        "subj" => ["A", "A", "B", "B"],
        "t" => [1.0_f64, 2.0, 1.0, 2.0],
        "y" => [1.0_f64, 3.0, 5.0, 7.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![
            VarMeta { name: "subj".into(), ty: VarType::Char, length: 1, format: None, label: None },
            VarMeta { name: "t".into(), ty: VarType::Num, length: 8, format: None, label: None },
            VarMeta { name: "y".into(), ty: VarType::Num, length: 8, format: None, label: None },
        ],
    };
    session.libs.get("WORK").unwrap().write("B", &ds).unwrap();
    session.last_dataset = Some("WORK.B".to_string());
    (session, ())
}

// ── General-path tests ──

#[test]
fn test_general_x_equals_ols_when_v_identity() {
    // With V = σ²I (no random / repeated correlation), the GLS estimate of a
    // CLASS factor must equal OLS from least_squares.
    use crate::stat::least_squares;
    // Two-level CLASS factor g (A,B) plus intercept; reference-cell coding
    // drops the last level (B), so columns = [intercept, g A].
    let g = vec![
        Value::Char("A".into()),
        Value::Char("A".into()),
        Value::Char("B".into()),
        Value::Char("B".into()),
        Value::Char("B".into()),
    ];
    let y = vec![2.0, 4.0, 5.0, 7.0, 9.0];
    let cols = vec![("g".to_string(), g)];
    let design = build_design(&cols, &["g".to_string()], &["g".to_string()], false, 5).unwrap();
    assert_eq!(design.len(), 2);
    assert_eq!(design[0].label, "Intercept");
    assert_eq!(design[1].label, "g A");
    let x: Vec<Vec<f64>> = (0..5)
        .map(|i| design.iter().map(|c| c.values[i]).collect())
        .collect();
    let beta_ols = least_squares(&x, &y).unwrap();

    // GLS with V = I: build_v_gen RandomVc with σ²_u=0, σ²_e=1.
    let subj_of = vec![0usize, 0, 1, 1, 1];
    let within = vec![0usize, 1, 0, 1, 2];
    let v = build_v_gen(GenCov::RandomVc, &[0.0, 1.0], 5, &subj_of, &within);
    let (_n2, beta_gls, _cb) = neg2_loglik_gen(&y, &x, &v, Method::Ml).unwrap();
    for (a, b) in beta_ols.iter().zip(&beta_gls) {
        assert!((a - b).abs() < 1e-8, "ols={a} gls={b}");
    }
}

#[test]
fn test_un_saturated_equals_sample_cov() {
    // Balanced t=2, 4 subjects. Within-subject vectors:
    //   A=(1,3) B=(3,1) C=(5,7) D=(7,5)
    // Both time means = 4 → intercept-only β̂ = grand mean = 4.
    // MLE UN block (divide by N=4): UN(1,1)=5, UN(2,2)=5, UN(2,1)=3.
    let y = vec![1.0, 3.0, 3.0, 1.0, 5.0, 7.0, 7.0, 5.0];
    let subj_of = vec![0usize, 0, 1, 1, 2, 2, 3, 3];
    let within = vec![0usize, 1, 0, 1, 0, 1, 0, 1];
    let x: Vec<Vec<f64>> = vec![vec![1.0]; 8];

    // Initial L = diag(sqrt(var)); var≈ sample.
    let u0 = vec![0.5 * 5.0_f64.ln(), 0.0, 0.5 * 5.0_f64.ln()];
    let fit = fit_gen(
        &y,
        &x,
        GenCov::RepeatedUn { t: 2 },
        &subj_of,
        &within,
        Method::Ml,
        &u0,
    )
    .unwrap();
    // theta order: UN(1,1), UN(2,1), UN(2,2).
    assert!((fit.theta[0] - 5.0).abs() < 1e-4, "UN(1,1)={}", fit.theta[0]);
    assert!((fit.theta[1] - 3.0).abs() < 1e-4, "UN(2,1)={}", fit.theta[1]);
    assert!((fit.theta[2] - 5.0).abs() < 1e-4, "UN(2,2)={}", fit.theta[2]);
    assert!((fit.beta[0] - 4.0).abs() < 1e-4, "beta={}", fit.beta[0]);

    // The listing reports covariance parameters to 4 decimals; confirm the
    // estimates round to exactly the SAS-faithful values at 4 dp.
    assert_eq!(fmt4(fit.theta[0]), "5.0000", "UN(1,1) 4dp");
    assert_eq!(fmt4(fit.theta[1]), "3.0000", "UN(2,1) 4dp");
    assert_eq!(fmt4(fit.theta[2]), "5.0000", "UN(2,2) 4dp");
    assert_eq!(fmt4(fit.beta[0]), "4.0000", "intercept 4dp");
}

#[test]
fn test_ar1_sanity() {
    // Small AR(1) dataset: ρ̂ ∈ (−1,1), σ²>0, optimizer reduces −2logL.
    let y = vec![1.0, 2.0, 3.0, 2.0, 4.0, 6.0, 5.0, 7.0];
    let subj_of = vec![0usize, 0, 0, 0, 1, 1, 1, 1];
    let within = vec![0usize, 1, 2, 3, 0, 1, 2, 3];
    let x: Vec<Vec<f64>> = vec![vec![1.0]; 8];
    let u0 = vec![0.1, 1.0_f64.ln()];
    let fit = fit_gen(
        &y,
        &x,
        GenCov::RepeatedAr1,
        &subj_of,
        &within,
        Method::Reml,
        &u0,
    )
    .unwrap();
    let rho = fit.theta[0];
    let s2 = fit.theta[1];
    assert!(rho > -1.0 && rho < 1.0, "rho={rho}");
    assert!(s2 > 0.0, "s2={s2}");
    assert!(
        fit.neg2ll <= fit.neg2_start + 1e-9,
        "neg2ll={} start={}",
        fit.neg2ll,
        fit.neg2_start
    );
}

#[test]
fn test_un_execute_runs() {
    // End-to-end: REPEATED UN executes without error and produces listing.
    let (mut session, _) = small_ds();
    let ast = parse_mixed(
        "proc mixed method=ml; class subj; model y = / solution; \
         repeated / subject=subj type=un; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = take_listing(&mut session);
    assert!(listing.contains("UN(1,1)"), "listing missing UN rows:\n{listing}");
    assert!(listing.contains("Unstructured"));
}

#[test]
fn test_ar1_execute_runs() {
    // End-to-end: REPEATED AR(1) executes and reports AR(1) + Residual rows.
    let (mut session, _) = small_ds();
    let ast = parse_mixed(
        "proc mixed; class subj; model y = / solution; \
         repeated / subject=subj type=ar(1); run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = take_listing(&mut session);
    assert!(listing.contains("AR(1)"), "missing AR(1):\n{listing}");
    assert!(listing.contains("Autoregressive"));
}

fn take_listing(session: &mut crate::session::Session) -> String {
    session.listing.into_string()
}

#[test]
fn test_profile_search_matches_closed_form() {
    // The general (golden-section) path should reproduce the closed form
    // on the balanced oracle.
    let (y, x, subj_of) = oracle();
    let (s2u, s2e) = profile_search(&y, &x, &subj_of, Method::Reml).unwrap();
    assert!((s2u - 7.0).abs() < 1e-2, "s2u={s2u}");
    assert!((s2e - 2.0).abs() < 1e-2, "s2e={s2e}");
}
