use super::super::*;
use super::*;
use crate::dataset::SasDataset;
use crate::value::VarType;
use polars::df;

#[test]
fn execute_by_unsorted_errors() {
    let mut session = make_session();
    // NOT sorted by sex: F,M,F.
    let df = df![
        "sex" => ["F", "M", "F"],
        "x" => [1.0_f64, 2.0, 3.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![char_meta("sex"), num_meta("x")] };
    write_dataset(&mut session, "T", ds);

    let ast = MeansAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        summary: false,
        noprint: false,
        stats: vec![],
        class: vec![],
        var: vec!["x".into()],
        by: vec![("sex".into(), false)],
        weight: None,
        alpha: 0.05,
        printalltypes: false,
        ways: vec![],
        types: vec![],
        output: None,
    };
    let r = execute(&ast, &mut session);
    assert!(r.is_err());
    let msg = r.err().unwrap().to_string();
    assert!(
        msg.contains("not sorted in ascending sequence")
            && msg.contains("sex=M")
            && msg.contains("sex=F"),
        "msg: {msg}"
    );
}

#[test]
fn execute_by_output_dataset_rows() {
    let mut session = make_session();
    // Sorted by sex: F,F,M.
    let df = df![
        "sex" => ["F", "F", "M"],
        "x" => [2.0_f64, 4.0, 10.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![char_meta("sex"), num_meta("x")] };
    write_dataset(&mut session, "T", ds);

    let ast = MeansAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        summary: false,
        noprint: true,
        stats: vec![],
        class: vec![],
        var: vec!["x".into()],
        by: vec![("sex".into(), false)],
        weight: None,
        alpha: 0.05,
        printalltypes: false,
        ways: vec![],
        types: vec![],
        output: Some(MeansOutput {
            out: DatasetRef { libref: Some("WORK".into()), name: "O".into() },
            specs: vec![("mean".into(), "x".into(), "mx".into())],
        }),
    };
    execute(&ast, &mut session).unwrap();

    // No CLASS → one row per BY group (k=0, _TYPE_=0).
    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    assert_eq!(out.n_obs(), 2);
    let sex = read_num_col(&session, "O", "sex"); // char decoded
    let mx = read_num_col(&session, "O", "mx");
    assert_eq!(sex[0], Value::Char("F".into()));
    assert_eq!(sex[1], Value::Char("M".into()));
    assert_eq!(mx[0], Value::Num(3.0));
    assert_eq!(mx[1], Value::Num(10.0));
    // BY column comes before _TYPE_.
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names[0], "sex");
    assert!(names.contains(&"_TYPE_"));
}

#[test]
fn execute_weight_report_and_exclusions() {
    let mut session = make_session();
    // x: 1,2,3, bad(w<=0), bad(missing w), bad(missing x)
    // weights: 1,2,3, 5, ., 4  -> only first three usable.
    let df = df![
        "x" => [Some(1.0_f64), Some(2.0), Some(3.0), Some(9.0), Some(7.0), None],
        "w" => [Some(1.0_f64), Some(2.0), Some(3.0), Some(0.0), None, Some(4.0)]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("w")] };
    write_dataset(&mut session, "T", ds);

    let ast = MeansAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        summary: false,
        noprint: true,
        stats: vec!["n".into(), "nmiss".into(), "mean".into(), "sum".into()],
        class: vec![],
        var: vec!["x".into()],
        by: vec![],
        weight: Some("w".into()),
        alpha: 0.05,
        printalltypes: false,
        ways: vec![],
        types: vec![],
        output: Some(MeansOutput {
            out: DatasetRef { libref: Some("WORK".into()), name: "O".into() },
            specs: vec![
                ("n".into(), "x".into(), "nx".into()),
                ("nmiss".into(), "x".into(), "nmx".into()),
                ("mean".into(), "x".into(), "mx".into()),
                ("sum".into(), "x".into(), "sx".into()),
            ],
        }),
    };
    execute(&ast, &mut session).unwrap();

    let nx = read_num_col(&session, "O", "nx");
    let nmx = read_num_col(&session, "O", "nmx");
    let mx = read_num_col(&session, "O", "mx");
    let sx = read_num_col(&session, "O", "sx");
    assert_eq!(nx, vec![Value::Num(3.0)]);
    assert_eq!(nmx, vec![Value::Num(3.0)]); // w<=0, missing w, missing x
    assert_eq!(sx, vec![Value::Num(14.0)]); // weighted sum Σw_i x_i
    if let Value::Num(m) = mx[0] {
        assert!((m - 14.0 / 6.0).abs() < 1e-12, "mean = {m}");
    } else {
        panic!("mean numeric");
    }
}

#[test]
fn execute_weight_with_by() {
    let mut session = make_session();
    // Sorted by g: a(values 1,2,3 weights 1,2,3) b(values 10,20 weights 1,1)
    let df = df![
        "g" => ["a", "a", "a", "b", "b"],
        "x" => [1.0_f64, 2.0, 3.0, 10.0, 20.0],
        "w" => [1.0_f64, 2.0, 3.0, 1.0, 1.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x"), num_meta("w")] };
    write_dataset(&mut session, "T", ds);

    let ast = MeansAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        summary: false,
        noprint: true,
        stats: vec![],
        class: vec![],
        var: vec!["x".into()],
        by: vec![("g".into(), false)],
        weight: Some("w".into()),
        alpha: 0.05,
        printalltypes: false,
        ways: vec![],
        types: vec![],
        output: Some(MeansOutput {
            out: DatasetRef { libref: Some("WORK".into()), name: "O".into() },
            specs: vec![("mean".into(), "x".into(), "mx".into())],
        }),
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    assert_eq!(out.n_obs(), 2);
    let mx = read_num_col(&session, "O", "mx");
    // a: 14/6 = 2.33333 ; b: (10+20)/2 = 15.
    if let Value::Num(m) = mx[0] {
        assert!((m - 14.0 / 6.0).abs() < 1e-12, "a mean = {m}");
    } else {
        panic!("numeric");
    }
    assert_eq!(mx[1], Value::Num(15.0));
}

#[test]
fn execute_weight_with_class() {
    let mut session = make_session();
    // class g: a(1,2,3 w 1,2,3) b(10 w 5)
    let df = df![
        "g" => ["a", "a", "a", "b"],
        "x" => [1.0_f64, 2.0, 3.0, 10.0],
        "w" => [1.0_f64, 2.0, 3.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x"), num_meta("w")] };
    write_dataset(&mut session, "T", ds);

    let ast = MeansAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        summary: false,
        noprint: true,
        stats: vec![],
        class: vec!["g".into()],
        var: vec!["x".into()],
        by: vec![],
        weight: Some("w".into()),
        alpha: 0.05,
        printalltypes: false,
        ways: vec![],
        types: vec![],
        output: Some(MeansOutput {
            out: DatasetRef { libref: Some("WORK".into()), name: "O".into() },
            specs: vec![("mean".into(), "x".into(), "mx".into())],
        }),
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    // _TYPE_ 0 (overall) + 2 levels = 3 rows.
    assert_eq!(out.n_obs(), 3);
    let ty = read_num_col(&session, "O", "_TYPE_");
    let mx = read_num_col(&session, "O", "mx");
    // overall: Σwx/Σw = (1+4+9+50)/(1+2+3+5) = 64/11 = 5.81818...
    assert_eq!(ty[0], Value::Num(0.0));
    if let Value::Num(m) = mx[0] {
        assert!((m - 64.0 / 11.0).abs() < 1e-12, "overall mean = {m}");
    } else {
        panic!("numeric");
    }
    // level a (_TYPE_=1): 14/6 ; level b: 10.
    if let Value::Num(m) = mx[1] {
        assert!((m - 14.0 / 6.0).abs() < 1e-12, "a mean = {m}");
    } else {
        panic!("numeric");
    }
    assert_eq!(mx[2], Value::Num(10.0));
}

// ──────────────────────── confidence-interval tests ────────────────────

#[test]
fn t_quantile_known_values() {
    // t_{0.975, 1} ≈ 12.7062
    assert!((t_quantile(0.975, 1.0) - 12.7062).abs() < 1e-3);
    // t_{0.975, 10} ≈ 2.2281
    assert!((t_quantile(0.975, 10.0) - 2.2281).abs() < 1e-3);
    // t_{0.975, large} → z_{0.975} ≈ 1.95996
    assert!((t_quantile(0.975, 100000.0) - 1.95996).abs() < 1e-3);
    // Symmetry and median.
    assert_eq!(t_quantile(0.5, 7.0), 0.0);
    assert!((t_quantile(0.025, 10.0) + t_quantile(0.975, 10.0)).abs() < 1e-6);
}

#[test]
fn cl_percent_label_values() {
    assert_eq!(cl_percent_label(0.05), "95");
    assert_eq!(cl_percent_label(0.10), "90");
    assert_eq!(cl_percent_label(0.01), "99");
}

#[test]
fn ods_output_summary_captures_dataset() {
    let mut session = make_session();
    let df = df!["x" => [Some(2.0_f64), Some(4.0), Some(6.0)]].unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x")] };
    write_dataset(&mut session, "T", ds);

    // Activate ODS OUTPUT Summary=means_out.
    session.set_ods_output(&[(
        "summary".into(),
        DatasetRef { libref: None, name: "means_out".into() },
    )]);

    let ast = means_ast_var_x();
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("MEANS_OUT").unwrap();
    assert_eq!(out.n_obs(), 1, "one row per VAR variable");
    // Columns: Variable, N, Mean, StdDev, Min, Max.
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["Variable", "N", "Mean", "StdDev", "Min", "Max"]);

    // Variable name column is char "x".
    let var_idx = out.vars.iter().position(|v| v.name == "Variable").unwrap();
    assert_eq!(out.vars[var_idx].ty, VarType::Char);

    assert_eq!(read_num_col(&session, "MEANS_OUT", "N"), vec![Value::Num(3.0)]);
    assert_eq!(read_num_col(&session, "MEANS_OUT", "Mean"), vec![Value::Num(4.0)]);
    // std of [2,4,6] = 2.
    assert_eq!(read_num_col(&session, "MEANS_OUT", "StdDev"), vec![Value::Num(2.0)]);
    assert_eq!(read_num_col(&session, "MEANS_OUT", "Min"), vec![Value::Num(2.0)]);
    assert_eq!(read_num_col(&session, "MEANS_OUT", "Max"), vec![Value::Num(6.0)]);

    // last_dataset points at the captured dataset.
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.MEANS_OUT"));
}

#[test]
fn ods_output_summary_case_insensitive_table_name() {
    let mut session = make_session();
    let df = df!["x" => [Some(1.0_f64), Some(3.0)]].unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x")] };
    write_dataset(&mut session, "T", ds);

    // Registered under a different casing; matching must be case-insensitive.
    session.set_ods_output(&[(
        "SuMMaRy".into(),
        DatasetRef { libref: None, name: "o".into() },
    )]);

    execute(&means_ast_var_x(), &mut session).unwrap();
    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    assert_eq!(out.n_obs(), 1);
    assert_eq!(read_num_col(&session, "O", "Mean"), vec![Value::Num(2.0)]);
}

#[test]
fn ods_output_inactive_writes_no_dataset() {
    // Invariant: with an empty ods_output_map, no capture dataset is written.
    let mut session = make_session();
    let df = df!["x" => [Some(2.0_f64), Some(4.0)]].unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x")] };
    write_dataset(&mut session, "T", ds);

    execute(&means_ast_var_x(), &mut session).unwrap();

    // No "SUMMARY" dataset, and last_dataset unchanged (still the input T).
    assert!(session.libs.get("WORK").unwrap().read("SUMMARY").is_err());
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.T"));
}

#[test]
fn ods_output_summary_multiple_vars_one_row_each() {
    let mut session = make_session();
    let df = df![
        "x" => [Some(1.0_f64), Some(3.0)],
        "y" => [Some(10.0_f64), Some(20.0)],
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("y")] };
    write_dataset(&mut session, "T", ds);

    session.set_ods_output(&[(
        "summary".into(),
        DatasetRef { libref: None, name: "o".into() },
    )]);

    let mut ast = means_ast_var_x();
    ast.var = vec!["x".into(), "y".into()];
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    assert_eq!(out.n_obs(), 2, "one row per VAR variable");
    assert_eq!(
        read_num_col(&session, "O", "Mean"),
        vec![Value::Num(2.0), Value::Num(15.0)]
    );
}
