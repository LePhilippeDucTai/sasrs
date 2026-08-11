use super::super::*;
use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::value::VarType;
use polars::df;

#[test]
fn execute_graphics_emits_deferred_note() {
    let mut session = make_session();
    let df = df!["x" => [1.0_f64, 2.0, 3.0]].unwrap();
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

    let listing = session.listing.take_string();
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

    let listing = session.listing.take_string();
    // Both numeric variables analyzed; char skipped.
    assert!(listing.contains("Variable: a"), "listing: {listing}");
    assert!(listing.contains("Variable: b"), "listing: {listing}");
    assert!(!listing.contains("Variable: g"), "listing: {listing}");
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
    let ds = SasDataset {
        df,
        vars: vec![char_meta("g", 4), num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = UnivariateAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        var: vec!["x".into()],
        by: vec![("g".into(), false)],
        weight: None,
        output: None,
        normal: false,
        plots: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(
        listing.contains("The UNIVARIATE Procedure"),
        "listing: {listing}"
    );
    assert!(listing.contains("g=a"), "listing: {listing}");
    assert!(listing.contains("g=b"), "listing: {listing}");
    // One Variable: x section per group (2 total).
    assert_eq!(
        listing.matches("Variable: x").count(),
        2,
        "listing: {listing}"
    );
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
    let ds = SasDataset {
        df,
        vars: vec![char_meta("g", 4), num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = UnivariateAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
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
        output: Some(UnivariateOutput {
            out: DatasetRef {
                libref: Some("WORK".into()),
                name: "O".into(),
            },
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
    let ds = SasDataset {
        df,
        vars: vec![char_meta("g", 4), num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = UnivariateAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        var: vec!["x".into()],
        by: vec![("g".into(), false)],
        weight: None,
        output: Some(UnivariateOutput {
            out: DatasetRef {
                libref: Some("WORK".into()),
                name: "O".into(),
            },
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
fn execute_weighted_moments() {
    let mut session = make_session();
    // values [1,2,3] weights [1,2,3] + an excluded row (w<=0).
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 99.0],
        "w" => [1.0_f64, 2.0, 3.0, 0.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("w")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = UnivariateAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        var: vec!["x".into()],
        by: vec![],
        weight: Some("w".into()),
        output: None,
        normal: false,
        plots: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(
        listing.contains("The UNIVARIATE Procedure"),
        "listing: {listing}"
    );
    assert!(listing.contains("Variable: x"), "listing: {listing}");
    assert!(listing.contains("Moments"), "listing: {listing}");
    // Weighted: Sum Weights = 6, Sum Observations = 14.
    assert!(listing.contains("Sum Weights"), "listing: {listing}");
    // M33.2: weighted Quantiles + Extreme Observations are now emitted.
    assert!(
        listing.contains("Quantiles (Definition 5)"),
        "listing: {listing}"
    );
    assert!(
        listing.contains("Extreme Observations"),
        "listing: {listing}"
    );
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
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("w")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = UnivariateAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        var: vec!["x".into()],
        by: vec![],
        weight: Some("w".into()),
        output: None,
        normal: false,
        plots: vec![],
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    // With equal weights the weighted mean equals the plain mean (20).
    assert!(listing.contains("Mean"), "listing: {listing}");
    // M33.2: the weighted Quantiles + Extremes tables are now emitted.
    // With unit weights the weighted quantiles reduce to Definition 5:
    // median of [10,20,30] = 20.
    assert!(
        listing.contains("Quantiles (Definition 5)"),
        "listing: {listing}"
    );
    assert!(listing.contains("Lowest Value"), "listing: {listing}");
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

#[cfg(feature = "graphics")]
#[test]
fn histogram_with_ods_and_feature_creates_image() {
    let dir = std::env::temp_dir();
    let log = run_plots(
        true,
        hist(),
        Some(dir.clone()),
        Some("univtest_hist".into()),
    );
    assert!(log.contains("written"), "log: {log}");
    let p = dir.join("univtest_hist_1.png");
    assert!(p.exists(), "image not created: {p:?}");
    assert!(p.metadata().unwrap().len() > 0);
    let _ = std::fs::remove_file(&p);
}

#[cfg(not(feature = "graphics"))]
#[test]
fn qqplot_with_ods_no_feature_defers_image() {
    let log = run_plots(true, qq(), None, None);
    assert!(log.contains("image deferred"), "log: {log}");
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

#[cfg(feature = "graphics")]
#[test]
fn probplot_cdfplot_ppplot_with_feature_create_images() {
    for (kind, stem) in [
        (UnivariatePlotKind::ProbPlot, "univtest_prob"),
        (UnivariatePlotKind::CdfPlot, "univtest_cdf"),
        (UnivariatePlotKind::PpPlot, "univtest_pp"),
    ] {
        let dir = std::env::temp_dir();
        let plots = vec![UnivariatePlot {
            kind,
            var: Some("x".into()),
        }];
        let log = run_plots(true, plots, Some(dir.clone()), Some(stem.into()));
        assert!(log.contains("written"), "{:?} log: {log}", kind);
        let p = dir.join(format!("{stem}_1.png"));
        assert!(p.exists(), "image not created: {p:?}");
        assert!(p.metadata().unwrap().len() > 0);
        let _ = std::fs::remove_file(&p);
    }
}

#[cfg(not(feature = "graphics"))]
#[test]
fn cdfplot_ppplot_with_ods_no_feature_defer_image() {
    for kind in [UnivariatePlotKind::CdfPlot, UnivariatePlotKind::PpPlot] {
        let plots = vec![UnivariatePlot {
            kind,
            var: Some("x".into()),
        }];
        let log = run_plots(true, plots, None, None);
        assert!(log.contains("image deferred"), "{:?} log: {log}", kind);
    }
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

// ── M38.3 : ODS OUTPUT Moments / BasicMeasures ─────────────────────────────

fn plain_ast(var: &[&str]) -> UnivariateAst {
    UnivariateAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        var: var.iter().map(|s| s.to_string()).collect(),
        by: vec![],
        weight: None,
        output: None,
        normal: false,
        plots: vec![],
    }
}

fn x12345_session() -> Session {
    let mut session = make_session();
    let df = df!["x" => [Some(1.0_f64), Some(2.0), Some(3.0), Some(4.0), Some(5.0)]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);
    session
}

/// `ods output Moments=m;` → dataset avec la structure SAS réelle
/// (VarName, Label1, cValue1, nValue1, Label2, cValue2, nValue2), une paire
/// gauche/droite par ligne du listing, nValue en pleine précision.
#[test]
fn ods_output_moments_captured() {
    let mut session = x12345_session();
    session.set_ods_output(&[(
        "Moments".into(),
        DatasetRef {
            libref: None,
            name: "m".into(),
        },
    )]);

    execute(&plain_ast(&["x"]), &mut session).unwrap();
    session.flush_ods_output().unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("M").unwrap();
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "VarName", "Label1", "cValue1", "nValue1", "Label2", "cValue2", "nValue2"
        ]
    );
    assert_eq!(out.n_obs(), 6, "6 lignes de la table Moments");

    // Ligne 1 : N=5 / Sum Weights=5 ; ligne 2 : Mean=3 / Sum Observations=15.
    let n1 = out.df.column("nValue1").unwrap().f64().unwrap();
    let n2 = out.df.column("nValue2").unwrap().f64().unwrap();
    assert_eq!(n1.get(0), Some(5.0));
    assert_eq!(n2.get(0), Some(5.0));
    assert_eq!(n1.get(1), Some(3.0));
    assert_eq!(n2.get(1), Some(15.0));
    let l1 = out.df.column("Label1").unwrap().str().unwrap();
    assert_eq!(l1.get(0), Some("N"));
    assert_eq!(l1.get(5), Some("Coeff Variation"));
}

/// `ods output BasicMeasures=b;` → VarName, LocMeasure, LocValue, VarMeasure,
/// VarValue ; 4 lignes, la 4e (Interquartile Range) sans mesure de position.
#[test]
fn ods_output_basic_measures_captured() {
    let mut session = x12345_session();
    session.set_ods_output(&[(
        "BasicMeasures".into(),
        DatasetRef {
            libref: None,
            name: "b".into(),
        },
    )]);

    execute(&plain_ast(&["x"]), &mut session).unwrap();
    session.flush_ods_output().unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("B").unwrap();
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "VarName",
            "LocMeasure",
            "LocValue",
            "VarMeasure",
            "VarValue"
        ]
    );
    assert_eq!(out.n_obs(), 4);
    let loc = out.df.column("LocMeasure").unwrap().str().unwrap();
    assert_eq!(loc.get(0), Some("Mean"));
    assert_eq!(loc.get(3), None, "4e ligne sans mesure de position");
    let locv = out.df.column("LocValue").unwrap().f64().unwrap();
    assert_eq!(locv.get(0), Some(3.0), "Mean");
    assert_eq!(locv.get(1), Some(3.0), "Median");
    let varm = out.df.column("VarMeasure").unwrap().str().unwrap();
    assert_eq!(varm.get(3), Some("Interquartile Range"));
}

/// Deux variables analysées → les tranches Moments s'empilent (12 lignes).
#[test]
fn ods_output_moments_two_vars_stack() {
    let mut session = make_session();
    let df = df![
        "x" => [Some(1.0_f64), Some(2.0)],
        "y" => [Some(10.0_f64), Some(30.0)],
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);
    session.set_ods_output(&[(
        "Moments".into(),
        DatasetRef {
            libref: None,
            name: "m".into(),
        },
    )]);

    execute(&plain_ast(&["x", "y"]), &mut session).unwrap();
    session.flush_ods_output().unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("M").unwrap();
    assert_eq!(out.n_obs(), 12, "6 lignes par variable");
    let vn = out.df.column("VarName").unwrap().str().unwrap();
    assert_eq!(vn.get(0), Some("x"));
    assert_eq!(vn.get(6), Some("y"));
}
