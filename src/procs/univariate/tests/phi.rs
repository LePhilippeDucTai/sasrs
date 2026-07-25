use super::super::*;
use super::*;
use crate::dataset::SasDataset;
use polars::df;

#[test]
fn phi_inv_known_quantiles() {
    // Standard probit reference values.
    assert!((phi_inv(0.5)).abs() < 1e-12, "phi_inv(0.5)");
    assert!((phi_inv(0.975) - 1.959963985).abs() < 1e-7, "phi_inv(0.975)");
    assert!((phi_inv(0.95) - 1.644853627).abs() < 1e-7, "phi_inv(0.95)");
    assert!((phi_inv(0.025) + 1.959963985).abs() < 1e-7, "phi_inv(0.025)");
    // Round-trip with probnorm.
    for &p in &[0.01, 0.1, 0.3, 0.6, 0.9, 0.99] {
        let z = phi_inv(p);
        assert!((probnorm(z) - p).abs() < 1e-9, "roundtrip p={p}");
    }
}

#[test]
fn shapiro_wilk_w_near_one_for_normalish() {
    // Symmetric, roughly normal sample → W close to 1, large p (not reject).
    let xs = sorted_of(&[-2.0, -1.0, 0.0, 1.0, 2.0]);
    let (w, p) = shapiro_wilk(&xs);
    let w = w.unwrap();
    assert!(w > 0.9 && w <= 1.0, "W={w}");
    let p = p.unwrap();
    assert!((0.0..=1.0).contains(&p), "p={p}");
    assert!(p > 0.3, "p should be large for ~normal data, got {p}");
}

#[test]
fn shapiro_wilk_low_w_for_outlier() {
    // A strong outlier makes the data non-normal → smaller W, smaller p.
    let normalish = sorted_of(&[-2.0, -1.0, 0.0, 1.0, 2.0]);
    let skewed = sorted_of(&[1.0, 2.0, 3.0, 4.0, 100.0]);
    let (wn, _) = shapiro_wilk(&normalish);
    let (ws, ps) = shapiro_wilk(&skewed);
    assert!(ws.unwrap() < wn.unwrap(), "outlier W should be smaller");
    assert!(ps.unwrap() < 0.2, "p for skewed sample should be small: {:?}", ps);
}

#[test]
fn shapiro_wilk_out_of_range() {
    assert_eq!(shapiro_wilk(&[1.0, 2.0]), (None, None)); // n<3
}

#[test]
fn anderson_darling_known_sample() {
    // Sample {1,2,3,4,5}: mean=3, s=sqrt(2.5)=1.5811388.
    // z = (x-3)/s = {-1.264911,-0.632456,0,0.632456,1.264911}.
    // Compute A² directly from the definition and compare.
    let xs = sorted_of(&[1.0, 2.0, 3.0, 4.0, 5.0]);
    let (mean, s) = moments(&xs);
    let z: Vec<f64> = xs.iter().map(|&x| (x - mean) / s).collect();
    let (a2, p) = anderson_darling(&z, 5);
    // Hand-computed value for this z-vector (verified in Python against the
    // exact A² definition) = 0.1435942.
    assert!((a2 - 0.1435942).abs() < 1e-4, "A²={a2}");
    let p = p.unwrap();
    assert!((0.0..=1.0).contains(&p), "p={p}");
    assert!(p > 0.5, "near-normal → large p, got {p}");
}

#[test]
fn anderson_darling_pvalue_monotone() {
    // Larger A² → smaller p (upper-tail).
    assert!(ad_pvalue(0.3) > ad_pvalue(1.0));
    assert!(ad_pvalue(1.0) > ad_pvalue(3.0));
    assert!((0.0..=1.0).contains(&ad_pvalue(0.1)));
    assert!((0.0..=1.0).contains(&ad_pvalue(5.0)));
}

#[test]
fn cramer_von_mises_known_sample() {
    let xs = sorted_of(&[1.0, 2.0, 3.0, 4.0, 5.0]);
    let (mean, s) = moments(&xs);
    let z: Vec<f64> = xs.iter().map(|&x| (x - mean) / s).collect();
    let (w2, p) = cramer_von_mises(&z, 5);
    // Hand-computed W² for this z-vector (verified in Python against the
    // exact W² definition) = 0.0193421.
    assert!((w2 - 0.0193421).abs() < 1e-5, "W²={w2}");
    let p = p.unwrap();
    assert!((0.0..=1.0).contains(&p), "p={p}");
}

#[test]
fn kolmogorov_smirnov_known_sample() {
    let xs = sorted_of(&[1.0, 2.0, 3.0, 4.0, 5.0]);
    let (mean, s) = moments(&xs);
    let z: Vec<f64> = xs.iter().map(|&x| (x - mean) / s).collect();
    let (d, p) = kolmogorov_smirnov(&z, 5);
    // D = max over i of |F_n − Φ|. For this symmetric z-vector,
    // Φ(z) = {0.10282,0.26354,0.5,0.73646,0.89718}; the largest gap is at
    // the first point: |0.2 − 0.10282| = 0.09718, vs |0.10282 − 0| etc.
    // Computed reference D ≈ 0.13646.
    assert!((d - 0.13646).abs() < 1e-3, "D={d}");
    assert!(p.unwrap() > 0.1, "near-normal → not significant");
}

#[test]
fn cvm_pvalue_monotone() {
    assert!(cvm_pvalue(0.05) > cvm_pvalue(0.2));
    assert!(cvm_pvalue(0.2) > cvm_pvalue(0.8));
}

#[test]
fn normality_block_emitted_only_with_normal() {
    let mut session = make_session();
    let df = df!["x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]].unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x")] };
    write_dataset(&mut session, "T", ds);
    let ast = UnivariateAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        var: vec!["x".into()],
        by: vec![],
        weight: None,
        output: None,
        normal: true,
        plots: vec![],
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("Tests for Normality"), "listing: {listing}");
    assert!(listing.contains("Shapiro-Wilk"), "listing: {listing}");
    assert!(listing.contains("Anderson-Darling"), "listing: {listing}");
    assert!(listing.contains("Cramer-von Mises"), "listing: {listing}");
    assert!(listing.contains("Kolmogorov-Smirnov"), "listing: {listing}");
}

#[test]
fn normality_degenerate_note_no_panic() {
    let mut session = make_session();
    // Constant column → zero variance → NOTE, no panic, no table.
    let df = df!["x" => [5.0_f64, 5.0, 5.0, 5.0]].unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x")] };
    write_dataset(&mut session, "T", ds);
    let ast = UnivariateAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        var: vec!["x".into()],
        by: vec![],
        weight: None,
        output: None,
        normal: true,
        plots: vec![],
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("Tests for Normality"), "listing: {listing}");
    assert!(listing.contains("at least 3 nonmissing"), "listing: {listing}");
}

#[test]
fn parse_normal_option_on_proc() {
    let ast = parse_univ("proc univariate data=a normal; var x; run;").unwrap();
    assert!(ast.normal);
}

#[test]
fn parse_normal_option_on_var() {
    let ast = parse_univ("proc univariate data=a; var x / normal; run;").unwrap();
    assert!(ast.normal);
    assert_eq!(ast.var, vec!["x"]);
}

#[test]
fn parse_graphics_statements_skipped() {
    let ast = parse_univ(
        "proc univariate data=a; var x; histogram x / normal; qqplot x; run;",
    )
    .unwrap();
    assert_eq!(ast.plots.len(), 2);
    assert_eq!(ast.plots[0].kind, UnivariatePlotKind::Histogram);
    assert_eq!(ast.plots[0].var.as_deref(), Some("x"));
    assert_eq!(ast.plots[1].kind, UnivariatePlotKind::QqPlot);
    assert_eq!(ast.plots[1].var.as_deref(), Some("x"));
    assert_eq!(ast.var, vec!["x"]);
}

#[test]
fn parse_data_and_var() {
    let ast = parse_univ("proc univariate data=work.t; var x; run;").unwrap();
    assert_eq!(ast.data.as_ref().unwrap().name, "t");
    assert_eq!(ast.var, vec!["x"]);
}

#[test]
fn parse_by_statement_captured() {
    let ast =
        parse_univ("proc univariate data=work.t; by g descending h; var x y; run;").unwrap();
    assert_eq!(ast.var, vec!["x", "y"]);
    assert_eq!(
        ast.by,
        vec![("g".to_string(), false), ("h".to_string(), true)]
    );
}

#[test]
fn parse_noprint_and_default_var() {
    let ast = parse_univ("proc univariate data=a noprint; run;").unwrap();
    assert!(ast.var.is_empty());
    assert_eq!(ast.data.as_ref().unwrap().name, "a");
}

#[test]
fn parse_output_statement() {
    let ast = parse_univ(
        "proc univariate data=a noprint; var x; output out=o mean=mx n=nx q1=q1x; run;",
    )
    .unwrap();
    let out = ast.output.as_ref().unwrap();
    assert_eq!(out.out.name, "o");
    assert_eq!(
        out.specs,
        vec![
            ("mean".to_string(), vec!["mx".to_string()]),
            ("n".to_string(), vec!["nx".to_string()]),
            ("q1".to_string(), vec!["q1x".to_string()]),
        ]
    );
}

// ───────────────────────────── WEIGHT tests ────────────────────────────

#[test]
fn parse_weight_statement() {
    let ast = parse_univ("proc univariate data=a; var x; weight w; run;").unwrap();
    assert_eq!(ast.weight.as_deref(), Some("w"));
    assert_eq!(ast.var, vec!["x"]);
}
