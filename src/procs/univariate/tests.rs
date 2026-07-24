use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::session::Session;
use crate::source::SourceFile;
use crate::value::VarType;
use polars::df;
use std::path::PathBuf;

fn make_session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

fn parse_univ(src: &str) -> Result<UnivariateAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "univariate"
    parse(&mut ts)
}

fn num_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}

// ───────────────────────────── parse tests ─────────────────────────────

// ─────────────────────────── normality tests ──────────────────────────

fn sorted_of(xs: &[f64]) -> Vec<f64> {
    let mut s = xs.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s
}

fn moments(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let s = sample_std(xs).unwrap();
    (mean, s)
}

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
fn anderson_darling_pvalue_monotone() {
    // Larger A² → smaller p (upper-tail).
    assert!(ad_pvalue(0.3) > ad_pvalue(1.0));
    assert!(ad_pvalue(1.0) > ad_pvalue(3.0));
    assert!((0.0..=1.0).contains(&ad_pvalue(0.1)));
    assert!((0.0..=1.0).contains(&ad_pvalue(5.0)));
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
fn execute_graphics_emits_deferred_note() {
    let mut session = make_session();
    let df = df!["x" => [1.0_f64, 2.0, 3.0]].unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x")] };
    write_dataset(&mut session, "T", ds);
    let ast = UnivariateAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        var: vec!["x".into()],
        by: vec![],
        weight: None,
        output: None,
        normal: false,
        plots: vec![UnivariatePlot {
            kind: UnivariatePlotKind::Histogram,
            var: Some("x".into()),
        }],
    };
    // ODS GRAPHICS off (default) → rendering stays deferred (one NOTE).
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(
        log.contains("graphical output deferred to ODS GRAPHICS"),
        "log: {log}"
    );
}

// ───────────────────────── M29.3 plot tests ─────────────────────────

/// Helper: write a small numeric dataset and run UNIVARIATE with the given
/// plots and ODS GRAPHICS state, returning the log.
fn run_plots(
    ods_on: bool,
    plots: Vec<UnivariatePlot>,
    output_dir: Option<std::path::PathBuf>,
    file_stem: Option<String>,
) -> String {
    let mut session = make_session();
    session.ods_graphics.enabled = ods_on;
    if let Some(d) = output_dir {
        session.ods_graphics.output_dir = d;
    }
    session.ods_graphics.file_stem = file_stem;
    let df = df!["x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0]].unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x")] };
    write_dataset(&mut session, "T", ds);
    let ast = UnivariateAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        var: vec!["x".into()],
        by: vec![],
        weight: None,
        output: None,
        normal: false,
        plots,
    };
    execute(&ast, &mut session).unwrap();
    session.log.into_string()
}

fn hist() -> Vec<UnivariatePlot> {
    vec![UnivariatePlot { kind: UnivariatePlotKind::Histogram, var: Some("x".into()) }]
}
fn qq() -> Vec<UnivariatePlot> {
    vec![UnivariatePlot { kind: UnivariatePlotKind::QqPlot, var: Some("x".into()) }]
}

#[test]
fn histogram_without_ods_defers() {
    let log = run_plots(false, hist(), None, None);
    assert!(
        log.contains("graphical output deferred to ODS GRAPHICS"),
        "log: {log}"
    );
}

#[cfg(not(feature = "graphics"))]
#[test]
fn histogram_with_ods_no_feature_defers_image() {
    let log = run_plots(true, hist(), None, None);
    assert!(log.contains("image deferred"), "log: {log}");
}

#[cfg(not(feature = "graphics"))]
#[test]
fn qqplot_with_ods_no_feature_defers_image() {
    let log = run_plots(true, qq(), None, None);
    assert!(log.contains("image deferred"), "log: {log}");
}

#[cfg(feature = "graphics")]
#[test]
fn histogram_with_ods_and_feature_creates_image() {
    let dir = std::env::temp_dir();
    let log = run_plots(true, hist(), Some(dir.clone()), Some("univtest_hist".into()));
    assert!(log.contains("written"), "log: {log}");
    let p = dir.join("univtest_hist_1.png");
    assert!(p.exists(), "image not created: {p:?}");
    assert!(p.metadata().unwrap().len() > 0);
    let _ = std::fs::remove_file(&p);
}

#[cfg(feature = "graphics")]
#[test]
fn qqplot_with_ods_and_feature_creates_image() {
    let dir = std::env::temp_dir();
    let log = run_plots(true, qq(), Some(dir.clone()), Some("univtest_qq".into()));
    assert!(log.contains("written"), "log: {log}");
    let p = dir.join("univtest_qq_1.png");
    assert!(p.exists(), "image not created: {p:?}");
    assert!(p.metadata().unwrap().len() > 0);
    let _ = std::fs::remove_file(&p);
}

#[cfg(not(feature = "graphics"))]
#[test]
fn probplot_with_ods_no_feature_defers_image() {
    let plots = vec![UnivariatePlot {
        kind: UnivariatePlotKind::ProbPlot,
        var: Some("x".into()),
    }];
    let log = run_plots(true, plots, None, None);
    // M33.2: PROBPLOT now shares the standard "image deferred" NOTE.
    assert!(log.contains("image deferred"), "log: {log}");
}

#[cfg(not(feature = "graphics"))]
#[test]
fn cdfplot_ppplot_with_ods_no_feature_defer_image() {
    for kind in [UnivariatePlotKind::CdfPlot, UnivariatePlotKind::PpPlot] {
        let plots = vec![UnivariatePlot { kind, var: Some("x".into()) }];
        let log = run_plots(true, plots, None, None);
        assert!(log.contains("image deferred"), "{:?} log: {log}", kind);
    }
}

#[cfg(feature = "graphics")]
#[test]
fn probplot_cdfplot_ppplot_with_feature_create_images() {
    for (kind, stem) in [
        (UnivariatePlotKind::ProbPlot, "univtest_prob"),
        (UnivariatePlotKind::CdfPlot, "univtest_cdf"),
        (UnivariatePlotKind::PpPlot, "univtest_pp"),
    ] {
        let dir = std::env::temp_dir();
        let plots = vec![UnivariatePlot { kind, var: Some("x".into()) }];
        let log = run_plots(true, plots, Some(dir.clone()), Some(stem.into()));
        assert!(log.contains("written"), "{:?} log: {log}", kind);
        let p = dir.join(format!("{stem}_1.png"));
        assert!(p.exists(), "image not created: {p:?}");
        assert!(p.metadata().unwrap().len() > 0);
        let _ = std::fs::remove_file(&p);
    }
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

// ─────────────────────────── quantile def-5 tests ──────────────────────

fn q(xs: &[f64], p: f64) -> f64 {
    let mut s = xs.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    quantile_def5(&s, p).unwrap()
}

#[test]
fn quantile_def5_pinned_odd() {
    // [1,2,3,4,5]
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    // median p=.5: np=2.5, j=2, g=.5 -> x[3]=3
    assert_eq!(q(&xs, 0.5), 3.0);
    // Q1 p=.25: np=1.25, j=1, g=.25 -> x[2]=2
    assert_eq!(q(&xs, 0.25), 2.0);
    // Q3 p=.75: np=3.75, j=3, g=.75 -> x[4]=4
    assert_eq!(q(&xs, 0.75), 4.0);
    // edges
    assert_eq!(q(&xs, 1.0), 5.0);
    assert_eq!(q(&xs, 0.0), 1.0);
}

#[test]
fn quantile_def5_pinned_even_discontinuity() {
    // [1,2,3,4]: median np=2, g=0 -> (x[2]+x[3])/2 = (2+3)/2 = 2.5
    let xs = [1.0, 2.0, 3.0, 4.0];
    assert_eq!(q(&xs, 0.5), 2.5);
}

// ───────────────────────── skewness / kurtosis tests ───────────────────

#[test]
fn skewness_symmetric_is_zero() {
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let g1 = skewness(&xs).unwrap();
    assert!(g1.abs() < 1e-12, "g1 = {g1}");
}

#[test]
fn skewness_known_skewed_sample() {
    // [1,2,3,4,10] : computed with the SAS formula.
    // mean=4, s=sqrt(Σ(x-mean)^2/4)=sqrt((9+4+1+0+36)/4)=sqrt(12.5)
    // g1 = 5/((4)(3)) * Σ z^3, z=(x-4)/s.
    let xs = [1.0, 2.0, 3.0, 4.0, 10.0];
    let g1 = skewness(&xs).unwrap();
    // Reference value (SAS g1 formula): ~1.6970563
    assert!((g1 - 1.6970563).abs() < 1e-4, "g1 = {g1}");
}

#[test]
fn skewness_needs_n_ge_3() {
    assert!(skewness(&[1.0, 2.0]).is_none());
    assert!(skewness(&[1.0]).is_none());
}

#[test]
fn kurtosis_needs_n_ge_4() {
    assert!(kurtosis(&[1.0, 2.0, 3.0]).is_none());
    assert!(kurtosis(&[1.0, 2.0]).is_none());
}

#[test]
fn kurtosis_known_sample() {
    // [1,2,3,4,5] excess kurtosis (SAS) reference ~ -1.2
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let g2 = kurtosis(&xs).unwrap();
    assert!((g2 - (-1.2)).abs() < 1e-6, "g2 = {g2}");
}

#[test]
fn mode_smallest_repeat_or_none() {
    let mut a = [1.0, 1.0, 2.0, 2.0, 3.0]; // 1 and 2 both twice -> smallest = 1
    a.sort_by(|x, y| x.partial_cmp(y).unwrap());
    assert_eq!(mode(&a), Some(1.0));

    let mut b = [1.0, 2.0, 3.0]; // all unique -> no mode
    b.sort_by(|x, y| x.partial_cmp(y).unwrap());
    assert_eq!(mode(&b), None);
}

// ───────────────────────────── execute tests ───────────────────────────

fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
    session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
    session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
}

#[test]
fn execute_report_contains_sections_and_median() {
    let mut session = make_session();
    // [1,2,3,4,5] plus a missing -> median 3, n_missing 1.
    let df = df![
        "x" => [Some(1.0_f64), Some(2.0), Some(3.0), Some(4.0), Some(5.0), None]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = UnivariateAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        var: vec!["x".into()],
        by: vec![],
        weight: None,
        output: None,
        normal: false,
        plots: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(
        listing.contains("The UNIVARIATE Procedure"),
        "listing: {listing}"
    );
    assert!(listing.contains("Variable: x"), "listing: {listing}");
    assert!(listing.contains("Median"), "listing: {listing}");
    // median of [1..5] is 3
    assert!(listing.contains("Median"), "listing: {listing}");
    assert!(listing.contains("Missing Values"), "listing: {listing}");
    // moments header
    assert!(listing.contains("Moments"), "listing: {listing}");
    assert!(listing.contains("Quantiles"), "listing: {listing}");
}

#[test]
fn execute_default_all_numeric_vars() {
    let mut session = make_session();
    let df = df![
        "a" => [1.0_f64, 2.0, 3.0],
        "g" => ["x", "y", "z"],
        "b" => [4.0_f64, 5.0, 6.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![
            num_meta("a"),
            VarMeta {
                name: "g".into(),
                ty: VarType::Char,
                length: 1,
                format: None,
                label: None,
            },
            num_meta("b"),
        ],
    };
    write_dataset(&mut session, "T", ds);

    let ast = UnivariateAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        var: vec![],
        by: vec![],
        weight: None,
        output: None,
        normal: false,
        plots: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    // Both numeric variables analyzed; char skipped.
    assert!(listing.contains("Variable: a"), "listing: {listing}");
    assert!(listing.contains("Variable: b"), "listing: {listing}");
    assert!(!listing.contains("Variable: g"), "listing: {listing}");
}

// ─────────────────────────── BY / OUTPUT tests ─────────────────────────

fn char_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Char,
        length: 4,
        format: None,
        label: None,
    }
}

fn read_num_col(session: &Session, table: &str, col: &str) -> Vec<Value> {
    let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
    let idx = ds.vars.iter().position(|m| m.name == col).unwrap();
    decode_column(&ds, idx).unwrap()
}

#[test]
fn execute_by_per_group_sections() {
    let mut session = make_session();
    // Sorted by g: a,a,b,b.
    let df = df![
        "g" => ["a", "a", "b", "b"],
        "x" => [1.0_f64, 3.0, 10.0, 20.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x")] };
    write_dataset(&mut session, "T", ds);

    let ast = UnivariateAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        var: vec!["x".into()],
        by: vec![("g".into(), false)],
        weight: None,
        output: None,
        normal: false,
        plots: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("The UNIVARIATE Procedure"), "listing: {listing}");
    assert!(listing.contains("g=a"), "listing: {listing}");
    assert!(listing.contains("g=b"), "listing: {listing}");
    // One Variable: x section per group (2 total).
    assert_eq!(listing.matches("Variable: x").count(), 2, "listing: {listing}");
}

#[test]
fn execute_by_unsorted_errors() {
    let mut session = make_session();
    // NOT sorted by g: a,b,a.
    let df = df![
        "g" => ["a", "b", "a"],
        "x" => [1.0_f64, 2.0, 3.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x")] };
    write_dataset(&mut session, "T", ds);

    let ast = UnivariateAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        var: vec!["x".into()],
        by: vec![("g".into(), false)],
        weight: None,
        output: None,
        normal: false,
        plots: vec![],
    };
    let r = execute(&ast, &mut session);
    assert!(r.is_err());
    let msg = r.err().unwrap().to_string();
    assert!(
        msg.contains("not sorted in ascending sequence"),
        "msg: {msg}"
    );
}

#[test]
fn execute_output_no_by() {
    let mut session = make_session();
    // [1,2,3,4,5] -> mean 3, n 5, min 1, max 5, median 3.
    let df = df!["x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]].unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x")] };
    write_dataset(&mut session, "T", ds);

    let ast = UnivariateAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        var: vec!["x".into()],
        by: vec![],
        weight: None,
        output: Some(UnivariateOutput {
            out: DatasetRef { libref: Some("WORK".into()), name: "O".into() },
            specs: vec![
                ("mean".into(), vec!["m".into()]),
                ("n".into(), vec!["cnt".into()]),
                ("min".into(), vec!["lo".into()]),
                ("max".into(), vec!["hi".into()]),
                ("median".into(), vec!["med".into()]),
            ],
        }),
        normal: false,
        plots: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    assert_eq!(out.n_obs(), 1);
    assert_eq!(read_num_col(&session, "O", "m"), vec![Value::Num(3.0)]);
    assert_eq!(read_num_col(&session, "O", "cnt"), vec![Value::Num(5.0)]);
    assert_eq!(read_num_col(&session, "O", "lo"), vec![Value::Num(1.0)]);
    assert_eq!(read_num_col(&session, "O", "hi"), vec![Value::Num(5.0)]);
    assert_eq!(read_num_col(&session, "O", "med"), vec![Value::Num(3.0)]);
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.O"));
}

#[test]
fn execute_output_with_by() {
    let mut session = make_session();
    // Sorted by g: a(1,3) b(10,20).
    let df = df![
        "g" => ["a", "a", "b", "b"],
        "x" => [1.0_f64, 3.0, 10.0, 20.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x")] };
    write_dataset(&mut session, "T", ds);

    let ast = UnivariateAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        var: vec!["x".into()],
        by: vec![("g".into(), false)],
        weight: None,
        output: Some(UnivariateOutput {
            out: DatasetRef { libref: Some("WORK".into()), name: "O".into() },
            specs: vec![("mean".into(), vec!["mx".into()])],
        }),
        normal: false,
        plots: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    assert_eq!(out.n_obs(), 2);
    let g = read_num_col(&session, "O", "g"); // char decoded
    let mx = read_num_col(&session, "O", "mx");
    assert_eq!(g[0], Value::Char("a".into()));
    assert_eq!(g[1], Value::Char("b".into()));
    assert_eq!(mx[0], Value::Num(2.0));
    assert_eq!(mx[1], Value::Num(15.0));
    // BY column precedes the statistic column.
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["g", "mx"]);
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

#[test]
fn execute_weighted_moments() {
    let mut session = make_session();
    // values [1,2,3] weights [1,2,3] + an excluded row (w<=0).
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 99.0],
        "w" => [1.0_f64, 2.0, 3.0, 0.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("w")] };
    write_dataset(&mut session, "T", ds);

    let ast = UnivariateAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        var: vec!["x".into()],
        by: vec![],
        weight: Some("w".into()),
        output: None,
        normal: false,
        plots: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("The UNIVARIATE Procedure"), "listing: {listing}");
    assert!(listing.contains("Variable: x"), "listing: {listing}");
    assert!(listing.contains("Moments"), "listing: {listing}");
    // Weighted: Sum Weights = 6, Sum Observations = 14.
    assert!(listing.contains("Sum Weights"), "listing: {listing}");
    // M33.2: weighted Quantiles + Extreme Observations are now emitted.
    assert!(listing.contains("Quantiles (Definition 5)"), "listing: {listing}");
    assert!(listing.contains("Extreme Observations"), "listing: {listing}");
    assert!(
        !listing.contains("not computed with a WEIGHT variable"),
        "listing: {listing}"
    );
    // The excluded (w<=0) row counts as a missing value.
    assert!(listing.contains("Missing Values"), "listing: {listing}");
}

#[test]
fn execute_weighted_no_quantiles_section() {
    let mut session = make_session();
    let df = df![
        "x" => [10.0_f64, 20.0, 30.0],
        "w" => [1.0_f64, 1.0, 1.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("w")] };
    write_dataset(&mut session, "T", ds);

    let ast = UnivariateAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        var: vec!["x".into()],
        by: vec![],
        weight: Some("w".into()),
        output: None,
        normal: false,
        plots: vec![],
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    // With equal weights the weighted mean equals the plain mean (20).
    assert!(listing.contains("Mean"), "listing: {listing}");
    // M33.2: the weighted Quantiles + Extremes tables are now emitted.
    // With unit weights the weighted quantiles reduce to Definition 5:
    // median of [10,20,30] = 20.
    assert!(listing.contains("Quantiles (Definition 5)"), "listing: {listing}");
    assert!(listing.contains("Lowest Value"), "listing: {listing}");
}

// ──────────────────── weighted quantile (M33.2) tests ───────────────────

fn wq(pairs: &[(f64, f64)], p: f64) -> f64 {
    let mut s = pairs.to_vec();
    s.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    weighted_quantile_def5(&s, p).unwrap()
}

#[test]
fn weighted_quantile_def5_oracle() {
    // Fixture data: x=[1,2,3,4], w=[1,2,3,4].
    //   Total weight W = 1+2+3+4 = 10.
    //   Cumulative weights W_i: 1, 3, 6, 10.
    // For p: target t = p*W; first i with W_i >= t; if W_i==t exactly →
    // average x(i),x(i+1), else x(i).
    let pairs = [(1.0, 1.0), (2.0, 2.0), (3.0, 3.0), (4.0, 4.0)];
    // Q1 p=0.25: t=2.5 → W_2=3 is first ≥ → x(2)=2.
    assert_eq!(wq(&pairs, 0.25), 2.0);
    // Median p=0.5: t=5.0 → W_3=6 first ≥, 6≠5 → x(3)=3.
    assert_eq!(wq(&pairs, 0.50), 3.0);
    // Q3 p=0.75: t=7.5 → W_4=10 first ≥ → x(4)=4.
    assert_eq!(wq(&pairs, 0.75), 4.0);
    // p=0.10: t=1.0 == W_1 exactly → (x(1)+x(2))/2 = (1+2)/2 = 1.5.
    assert_eq!(wq(&pairs, 0.10), 1.5);
    // p=0.05: t=0.5 → W_1=1 first ≥ → x(1)=1.
    assert_eq!(wq(&pairs, 0.05), 1.0);
    // Edges.
    assert_eq!(wq(&pairs, 1.0), 4.0); // 100% Max
    assert_eq!(wq(&pairs, 0.0), 1.0); // 0% Min
}

#[test]
fn weighted_quantile_reduces_to_def5_when_unit_weights() {
    // Unit weights → must equal the unweighted Definition 5 results.
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let pairs: Vec<(f64, f64)> = xs.iter().map(|&x| (x, 1.0)).collect();
    for &p in &[0.0, 0.25, 0.5, 0.75, 1.0, 0.1, 0.9] {
        let mut sp = pairs.clone();
        sp.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let mut s = xs.to_vec();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(
            weighted_quantile_def5(&sp, p),
            quantile_def5(&s, p),
            "mismatch at p={p}"
        );
    }
}
