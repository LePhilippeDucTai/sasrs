use super::*;

#[test]
fn test_one_sample_basic() {
    // [1,2,3,4,5] vs H0=0: n=5, mean=3, s≈1.5811, se≈0.7071, t≈4.2426, df=4.
    let values = [1.0, 2.0, 3.0, 4.0, 5.0];
    let r = one_sample(&values, 0.0, 0.05, TTestSides::TwoTailed);
    assert_eq!(r.n, 5);
    assert!((r.mean - 3.0).abs() < 1e-12);
    assert!(
        (r.std.unwrap() - 2.5_f64.sqrt()).abs() < 1e-9,
        "std={:?}",
        r.std
    );
    assert!(
        (r.se.unwrap() - 0.5_f64.sqrt()).abs() < 1e-9,
        "se={:?}",
        r.se
    );
    assert!((r.df - 4.0).abs() < 1e-12);
    let t = r.t.unwrap();
    assert!((t - 4.2426).abs() < 1e-4, "t={t}");
    let p = r.p.unwrap();
    assert!(p < 0.015, "p={p}");
}

#[test]
fn test_one_sided_p() {
    // t = 4.2426 with df=4. Two-sided p = 0.013263 (R: 2*pt(-4.2426,4)).
    // Upper one-sided p = Pr(T>t) = p2/2 ≈ 0.0066314.
    // Lower one-sided p = Pr(T<t) = 1 - upper ≈ 0.9933686.
    let values = [1.0, 2.0, 3.0, 4.0, 5.0];
    let two = one_sample(&values, 0.0, 0.05, TTestSides::TwoTailed)
        .p
        .unwrap();
    let up = one_sample(&values, 0.0, 0.05, TTestSides::Upper).p.unwrap();
    let lo = one_sample(&values, 0.0, 0.05, TTestSides::Lower).p.unwrap();
    assert!((two - 0.013263).abs() < 1e-4, "two={two}");
    assert!((up - 0.0066314).abs() < 1e-4, "up={up}");
    assert!((lo - 0.9933686).abs() < 1e-4, "lo={lo}");
    // Consistency: upper + lower = 1, two = 2*upper.
    assert!((up + lo - 1.0).abs() < 1e-9);
    assert!((two - 2.0 * up).abs() < 1e-9);
}

#[test]
fn test_mean_ci() {
    // [1,2,3,4,5]: mean=3, se=sqrt(0.5)=0.707107, df=4.
    // t_{0.975,4}=2.776445; half=2.776445*0.707107=1.963243.
    // 95% CL Mean = [1.036757, 4.963243].
    let values = [1.0, 2.0, 3.0, 4.0, 5.0];
    let r = one_sample(&values, 0.0, 0.05, TTestSides::TwoTailed);
    assert!(
        (r.mean_lcl.unwrap() - 1.036757).abs() < 1e-4,
        "lcl={:?}",
        r.mean_lcl
    );
    assert!(
        (r.mean_ucl.unwrap() - 4.963243).abs() < 1e-4,
        "ucl={:?}",
        r.mean_ucl
    );
    // Std CL (chi-square) for s=1.581139, df=4:
    // chi2_{0.975,4}=11.143287, chi2_{0.025,4}=0.484419.
    // std L = s*sqrt(4/11.143287)=0.947247; std U = s*sqrt(4/0.484419)=4.543297.
    assert!(
        (r.std_lcl.unwrap() - 0.947247).abs() < 1e-3,
        "stdL={:?}",
        r.std_lcl
    );
    assert!(
        (r.std_ucl.unwrap() - 4.543297).abs() < 1e-3,
        "stdU={:?}",
        r.std_ucl
    );
}

#[test]
fn test_two_sample_pooled() {
    // A=[1,2,3] (mean 2, s 1), B=[5,6,7] (mean 6, s 1).
    let a = [1.0, 2.0, 3.0];
    let b = [5.0, 6.0, 7.0];
    let r = two_sample(&a, &b, 0.05, TTestSides::TwoTailed);
    assert_eq!(r.n_a, 3);
    assert_eq!(r.n_b, 3);
    let (tp, dfp, _pp) = r.pooled.unwrap();
    assert!((tp - (-4.8990)).abs() < 1e-4, "t_pool={tp}");
    assert!((dfp - 4.0).abs() < 1e-12, "df_pool={dfp}");
    let (ts, dfs, _ps) = r.satterthwaite.unwrap();
    assert!((ts.abs() - 4.8990).abs() < 1e-4, "t_satt={ts}");
    assert!((dfs - 4.0).abs() < 1e-6, "df_satt={dfs}");
    let (f, _df1, _df2, _pf) = r.f_test.unwrap();
    assert!((f - 1.0).abs() < 1e-12, "F={f}");
}

#[test]
fn test_paired_simple() {
    // x=[2,4,6], y=[1,2,3]: diffs=[1,2,3], mean=2, s=1, se≈0.5774, t≈3.4641, df=2.
    let diffs = [1.0, 2.0, 3.0];
    let r = one_sample(&diffs, 0.0, 0.05, TTestSides::TwoTailed);
    assert_eq!(r.n, 3);
    assert!((r.mean - 2.0).abs() < 1e-12);
    assert!((r.std.unwrap() - 1.0).abs() < 1e-12);
    assert!((r.se.unwrap() - 1.0 / 3.0_f64.sqrt()).abs() < 1e-9);
    assert!((r.df - 2.0).abs() < 1e-12);
    let t = r.t.unwrap();
    assert!((t - 3.4641).abs() < 1e-4, "t={t}");
}

// --- parser + executor smoke tests ---

use crate::dataset::{SasDataset, VarMeta};
use crate::source::SourceFile;
use polars::df;
use std::path::PathBuf;

fn make_session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

fn num_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.into(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}
fn char_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.into(),
        ty: VarType::Char,
        length: 1,
        format: None,
        label: None,
    }
}

fn parse_ttest(src: &str) -> Result<TTestAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // proc
    ts.next(); // ttest
    parse(&mut ts)
}

#[test]
fn parse_options_and_statements() {
    let ast =
        parse_ttest("proc ttest data=a h0=5 alpha=0.10 sides=u equal=no; var x y; class g; run;")
            .unwrap();
    assert_eq!(ast.data_options.input.as_ref().unwrap().name, "a");
    assert!((ast.proc_options.h0 - 5.0).abs() < 1e-12);
    assert!((ast.proc_options.alpha - 0.10).abs() < 1e-12);
    assert!(!ast.proc_options.equal);
    assert!(matches!(ast.proc_options.sides, TTestSides::Upper));
    assert_eq!(ast.var_vars, vec!["x", "y"]);
    assert_eq!(ast.class_var.as_deref(), Some("g"));
}

#[test]
fn parse_paired_pairs() {
    let ast = parse_ttest("proc ttest data=a; paired x*y z*w; run;").unwrap();
    assert_eq!(
        ast.paired_vars,
        vec![
            ("x".to_string(), "y".to_string()),
            ("z".to_string(), "w".to_string())
        ]
    );
}

#[test]
fn parse_unknown_option_errors() {
    let r = parse_ttest("proc ttest data=a bogus; run;");
    assert!(r.is_err());
    assert!(r.err().unwrap().to_string().contains("BOGUS"));
}

#[test]
fn execute_two_sample_listing() {
    let mut session = make_session();
    let df = df![
        "g" => ["A", "A", "A", "B", "B", "B"],
        "x" => [1.0_f64, 2.0, 3.0, 5.0, 6.0, 7.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("g"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let ast = TTestAst {
        data_options: TTestDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            output: None,
        },
        proc_options: TTestProcOptions::default(),
        var_vars: vec!["x".into()],
        class_var: Some("g".into()),
        paired_vars: vec![],
        by: vec![],
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(listing.contains("The TTEST Procedure"), "{listing}");
    assert!(listing.contains("Two-Sample t Tests"), "{listing}");
    assert!(listing.contains("Pooled"), "{listing}");
    assert!(listing.contains("Satterthwaite"), "{listing}");
    assert!(listing.contains("Equality of Variances"), "{listing}");
}

#[test]
fn execute_one_sample_and_paired_listing() {
    let mut session = make_session();
    let df = df![
        "x" => [2.0_f64, 4.0, 6.0],
        "y" => [1.0_f64, 2.0, 3.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    // One-sample on all numeric vars.
    let ast1 = TTestAst {
        data_options: TTestDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            output: None,
        },
        proc_options: TTestProcOptions::default(),
        var_vars: vec![],
        class_var: None,
        paired_vars: vec![],
        by: vec![],
    };
    execute(&ast1, &mut session).unwrap();
    let l1 = session.listing.take_string();
    assert!(l1.contains("One-Sample t Tests"), "{l1}");

    // Paired x*y.
    let mut session2 = make_session();
    let df2 = df![
        "x" => [2.0_f64, 4.0, 6.0],
        "y" => [1.0_f64, 2.0, 3.0]
    ]
    .unwrap();
    let ds2 = SasDataset {
        df: df2,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    session2.libs.get("WORK").unwrap().write("T", &ds2).unwrap();
    let ast2 = TTestAst {
        data_options: TTestDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            output: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "OUT".into(),
            }),
        },
        proc_options: TTestProcOptions::default(),
        var_vars: vec![],
        class_var: None,
        paired_vars: vec![("x".into(), "y".into())],
        by: vec![],
    };
    execute(&ast2, &mut session2).unwrap();
    let l2 = session2.listing.take_string();
    assert!(l2.contains("Paired t Tests"), "{l2}");
    // OUT= dataset written.
    let (out, _) = session2.libs.get("WORK").unwrap().read("OUT").unwrap();
    assert_eq!(out.n_obs(), 1);
    assert_eq!(session2.last_dataset.as_deref(), Some("WORK.OUT"));
}

#[test]
fn parse_by_statement() {
    let ast = parse_ttest("proc ttest data=a ci=90; var x; by g; run;").unwrap();
    assert_eq!(ast.by, vec![("g".to_string(), false)]);
    assert!(ast.proc_options.ci_explicit);
    assert!((ast.proc_options.ci - 90.0).abs() < 1e-12);
}

#[test]
fn execute_one_sample_by_groups() {
    // Two BY groups; each is an independent one-sample t test. Sorted by g.
    // g=1: x=[1,2,3,4,5] → mean 3, t=4.2426; g=2: x=[10,20,30] → mean 20.
    let mut session = make_session();
    let df = df![
        "g" => [1.0_f64, 1.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 10.0, 20.0, 30.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("g"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let ast = TTestAst {
        data_options: TTestDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            output: None,
        },
        proc_options: TTestProcOptions::default(),
        var_vars: vec!["x".into()],
        class_var: None,
        paired_vars: vec![],
        by: vec![("g".into(), false)],
    };
    execute(&ast, &mut session).unwrap();
    let l = session.listing.take_string();
    // BY headings for both groups, two distinct One-Sample sections.
    assert!(l.contains("g=1"), "{l}");
    assert!(l.contains("g=2"), "{l}");
    assert_eq!(l.matches("One-Sample t Tests").count(), 2, "{l}");
    // Group 1 mean and t printed; group 2 mean 20.
    assert!(l.contains(" 3.0000"), "{l}");
    assert!(l.contains(" 4.2426"), "{l}");
    assert!(l.contains("20.0000"), "{l}");
}

#[test]
fn execute_by_unsorted_errors() {
    let mut session = make_session();
    let df = df![
        "g" => [2.0_f64, 1.0],
        "x" => [10.0_f64, 1.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("g"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = TTestAst {
        data_options: TTestDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            output: None,
        },
        proc_options: TTestProcOptions::default(),
        var_vars: vec!["x".into()],
        class_var: None,
        paired_vars: vec![],
        by: vec![("g".into(), false)],
    };
    let err = execute(&ast, &mut session).unwrap_err();
    assert!(
        err.to_string().contains("not sorted in ascending sequence"),
        "{err}"
    );
}

#[test]
fn execute_ci_and_sides_columns() {
    // CI= triggers CL columns; SIDES=U triggers the one-sided p header.
    let mut session = make_session();
    let df = df!["x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let po = TTestProcOptions {
        ci_explicit: true,
        sides: TTestSides::Upper,
        ..Default::default()
    };
    let ast = TTestAst {
        data_options: TTestDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            output: None,
        },
        proc_options: po,
        var_vars: vec!["x".into()],
        class_var: None,
        paired_vars: vec![],
        by: vec![],
    };
    execute(&ast, &mut session).unwrap();
    let l = session.listing.take_string();
    assert!(l.contains("95% CL Mean L"), "{l}");
    assert!(l.contains("95% CL Std L"), "{l}");
    assert!(l.contains("Pr > t"), "{l}");
    // 95% CL Mean bounds [1.0368, 4.9632].
    assert!(l.contains("1.0368"), "{l}");
    assert!(l.contains("4.9632"), "{l}");
}
