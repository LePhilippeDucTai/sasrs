use super::super::*;
use super::*;
use polars::df;

/// M36.7 — the BY heading is emitted INSIDE the per-model header block:
/// after "The REG Procedure" and before "Model: MODEL1" / "Dependent
/// Variable:", NOT before the title/page banner (Bug 1).
#[test]
fn test_by_heading_inside_header_block() {
    let mut session = make_session();
    let frame = df![
        "g" => [1.0_f64, 1.0, 1.0, 2.0, 2.0, 2.0],
        "y" => [2.0_f64, 4.0, 6.0, 1.0, 3.0, 5.0],
        "x" => [1.0_f64, 2.0, 3.0, 1.0, 2.0, 3.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("g"), num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let mut ast = single_model_ast(
        DatasetRef { libref: Some("WORK".into()), name: "T".into() },
        basic_model("y", &["x"]),
    );
    ast.by = vec!["g".into()];
    execute(&ast, &mut session).unwrap();
    let out = session.listing.into_string();
    let lines: Vec<&str> = out.lines().map(|l| l.trim()).collect();
    // For the first group locate "The REG Procedure", "g=1", and the model
    // label, and assert the ordering proc-line < heading < model-label.
    let proc_i = lines.iter().position(|l| *l == "The REG Procedure").unwrap();
    let head_i = lines.iter().position(|l| *l == "g=1").unwrap();
    let model_i = lines.iter().position(|l| *l == "Model: MODEL1").unwrap();
    assert!(
        proc_i < head_i && head_i < model_i,
        "expected proc < heading < model label; got {proc_i} {head_i} {model_i}\n{out}"
    );
}

/// M36.7 — Bug 2: with a WEIGHT the MODEL R residual summary is weighted.
/// (a) the printed "Sum of Squared Residuals" equals the weighted ANOVA
/// Error SS, and (b) all-ones weights reproduce the unweighted summary
/// byte-for-byte.
#[test]
fn test_weighted_residual_summary_matches_error_ss() {
    // Parse the "Error" ANOVA SS and the "Sum of Squared Residuals" line
    // out of a MODEL .../r listing.
    fn error_ss(listing: &str) -> String {
        listing
            .lines()
            .find(|l| l.trim_start().starts_with("Error "))
            .and_then(|l| l.split_whitespace().nth(2))
            .unwrap()
            .to_string()
    }
    fn sum_sq_resid(listing: &str) -> String {
        listing
            .lines()
            .find(|l| l.trim_start().starts_with("Sum of Squared Residuals"))
            .and_then(|l| l.split_whitespace().last())
            .unwrap()
            .to_string()
    }

    let run = |weight: Option<&str>| -> String {
        let mut session = make_session();
        let frame = df![
            "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0],
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
            "w" => [3.0_f64, 1.5, 2.0, 0.5, 4.0, 1.0],
            "ones" => [1.0_f64, 1.0, 1.0, 1.0, 1.0, 1.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![
                num_meta("y"),
                num_meta("x"),
                num_meta("w"),
                num_meta("ones"),
            ],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
        let mut model = basic_model("y", &["x"]);
        model.r = true;
        let mut ast = single_model_ast(
            DatasetRef { libref: Some("WORK".into()), name: "T".into() },
            model,
        );
        ast.weight = weight.map(|w| w.to_string());
        execute(&ast, &mut session).unwrap();
        session.listing.into_string()
    };

    // (a) Weighted run: printed Sum of Squared Residuals == ANOVA Error SS.
    let weighted = run(Some("w"));
    assert_eq!(
        sum_sq_resid(&weighted),
        error_ss(&weighted),
        "weighted Sum of Squared Residuals must equal ANOVA Error SS\n{weighted}"
    );

    // (b) All-ones WEIGHT reproduces the unweighted residual summary exactly.
    let none = run(None);
    let ones = run(Some("ones"));
    let block = |l: &str| -> String {
        l.lines()
            .filter(|x| {
                let t = x.trim_start();
                t.starts_with("Sum of Residuals")
                    || t.starts_with("Sum of Squared Residuals")
                    || t.starts_with("Predicted Residual SS")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(block(&none), block(&ones), "all-ones weight changed summary");
}

/// ID prepends an `Id` leading column to the MODEL R Output Statistics table.
#[test]
fn test_id_column_in_r_table() {
    let mut session = make_session();
    let frame = df![
        "name" => [10.0_f64, 20.0, 30.0, 40.0, 50.0],
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("name"), num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let mut model = basic_model("y", &["x"]);
    model.r = true;
    let mut ast = single_model_ast(
        DatasetRef { libref: Some("WORK".into()), name: "T".into() },
        model,
    );
    ast.id = vec!["name".into()];
    execute(&ast, &mut session).unwrap();
    let out = session.listing.into_string();
    assert!(out.contains("Output Statistics"), "{out}");
    // The ID values 10..50 appear as a leading column.
    assert!(out.contains("Id"), "{out}");
    assert!(out.contains("10") && out.contains("50"), "{out}");
}

#[test]
fn test_outest_covout_outseb_edf_rows() {
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 5.0, 7.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = parse_reg(
        "proc reg data=work.t outest=est covout outseb edf; model y = x; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let (out, _) = session.libs.get("WORK").unwrap().read("EST").unwrap();
    let names: Vec<String> = out.vars.iter().map(|v| v.name.clone()).collect();
    assert!(names.contains(&"_NAME_".to_string()));
    assert!(names.contains(&"_IN_".to_string()));
    assert!(names.contains(&"_P_".to_string()));
    assert!(names.contains(&"_EDF_".to_string()));
    // _TYPE_ rows: PARMS, COV x2 (Intercept, x), SEB.
    let tidx = out.vars.iter().position(|v| v.name == "_TYPE_").unwrap();
    let tcol = decode_column(&out, tidx).unwrap();
    let types: Vec<String> = tcol
        .iter()
        .map(|v| match v {
            crate::value::Value::Char(s) => s.trim_end().to_string(),
            _ => String::new(),
        })
        .collect();
    assert_eq!(types, vec!["PARMS", "COV", "COV", "SEB"]);
}

#[test]
fn test_m369_ridge0_equals_ols() {
    // RIDGE=0 back-transformed estimates reproduce the OLS β (intercept +
    // slopes) within 1e-7.
    let (cols, y) = m369_setup();
    let n = y.len();
    let std = standardize_for_ridge(&cols, &y);
    let b_star = ridge_beta_star(&std.r, &std.r_xy, 0.0).unwrap();
    let (b0, slopes) = back_transform(&b_star, &std);
    let xm = design(true, &[&cols[0], &cols[1]], n);
    let fit = ols_fit(&xm, &y).unwrap();
    assert!((b0 - fit.beta[0]).abs() < 1e-7, "intercept {b0} vs {}", fit.beta[0]);
    assert!((slopes[0] - fit.beta[1]).abs() < 1e-7);
    assert!((slopes[1] - fit.beta[2]).abs() < 1e-7);
}

#[test]
fn test_m369_ridge_shrinks_monotone() {
    // ‖b*(k)‖ ≤ ‖b*(0)‖ and strictly decreasing in k under collinearity.
    let (cols, y) = m369_setup();
    let std = standardize_for_ridge(&cols, &y);
    let ks = [0.0, 0.01, 0.05, 0.1, 0.5];
    let mut prev = f64::INFINITY;
    let n0 = norm2(&ridge_beta_star(&std.r, &std.r_xy, 0.0).unwrap());
    for &k in &ks {
        let nk = norm2(&ridge_beta_star(&std.r, &std.r_xy, k).unwrap());
        assert!(nk <= n0 + 1e-12, "‖b*({k})‖ {nk} > ‖b*(0)‖ {n0}");
        if k > 0.0 {
            assert!(nk < prev, "norm not strictly decreasing at k={k}");
        }
        prev = nk;
    }
}

#[test]
fn test_m369_outvif_k0_equals_ordinary_vif() {
    // Ridge VIF at k=0 == ordinary VIF (within 1e-7); decreases with k.
    let (cols, y) = m369_setup();
    let std = standardize_for_ridge(&cols, &y);
    let (_tol, ord_vif) = vif_tol(&cols);
    let rvif0 = ridge_vif(&std.r, 0.0).unwrap();
    for j in 0..cols.len() {
        assert!(
            (rvif0[j] - ord_vif[j]).abs() < 1e-7,
            "ridge VIF(0) {} != ordinary VIF {} at {j}",
            rvif0[j],
            ord_vif[j]
        );
    }
    let rvif1 = ridge_vif(&std.r, 0.1).unwrap();
    for j in 0..cols.len() {
        assert!(rvif1[j] < rvif0[j], "ridge VIF did not decrease at {j}");
    }
}

#[test]
fn test_m369_pcomit0_equals_ols_and_full_drop_zero() {
    let (cols, y) = m369_setup();
    let n = y.len();
    let p = cols.len();
    let std = standardize_for_ridge(&cols, &y);
    // m=0 → OLS β.
    let b_star0 = ipc_beta_star(&std.r, &std.r_xy, 0).unwrap();
    let (b0, slopes) = back_transform(&b_star0, &std);
    let xm = design(true, &[&cols[0], &cols[1]], n);
    let fit = ols_fit(&xm, &y).unwrap();
    assert!((b0 - fit.beta[0]).abs() < 1e-7);
    assert!((slopes[0] - fit.beta[1]).abs() < 1e-7);
    assert!((slopes[1] - fit.beta[2]).abs() < 1e-7);
    // m=p → standardized estimates all 0; intercept = ȳ.
    let b_star_all = ipc_beta_star(&std.r, &std.r_xy, p).unwrap();
    for b in &b_star_all {
        assert!(b.abs() < 1e-12, "IPC drop-all coef not zero: {b}");
    }
    let (b0_all, slopes_all) = back_transform(&b_star_all, &std);
    for s in &slopes_all {
        assert!(s.abs() < 1e-12);
    }
    assert!((b0_all - std.y_mean).abs() < 1e-9, "intercept != ȳ");
}

#[test]
fn test_m369_parse_ridge_range() {
    // ridge=0 to 0.1 by 0.05 → {0, 0.05, 0.1}.
    let ast = parse_reg("proc reg ridge=0 to 0.1 by 0.05; model y = x; run;").unwrap();
    let r = &ast.data_options.ridge;
    assert_eq!(r.len(), 3, "ridge list {:?}", r);
    assert!((r[0] - 0.0).abs() < 1e-12);
    assert!((r[1] - 0.05).abs() < 1e-12);
    assert!((r[2] - 0.1).abs() < 1e-12);
}

#[test]
fn test_m369_parse_ridge_outvif_outest() {
    let ast =
        parse_reg("proc reg ridge=0 0.1 outvif outest=e; model y = x; run;").unwrap();
    assert_eq!(ast.data_options.ridge, vec![0.0, 0.1]);
    assert!(ast.data_options.outvif);
    assert!(ast.data_options.outest.is_some());
    assert!(ast.data_options.pcomit.is_empty());
}

#[test]
fn test_m369_parse_pcomit_outest() {
    let ast = parse_reg("proc reg pcomit=1 outest=e; model y = x; run;").unwrap();
    assert_eq!(ast.data_options.pcomit, vec![1.0]);
    assert!(ast.data_options.outest.is_some());
    assert!(ast.data_options.ridge.is_empty());
    assert!(!ast.data_options.outvif);
}

#[test]
fn test_m369_outest_ridge_rows_and_columns() {
    // End-to-end: RIDGE= + OUTVIF + OUTEST= produces _RIDGE_ column and
    // RIDGE / RIDGEVIF rows; the k=0 RIDGE row reproduces the OLS PARMS.
    let mut session = make_session();
    let (cols, y) = m369_setup();
    let frame = df![
        "y" => y.clone(),
        "x1" => cols[0].clone(),
        "x2" => cols[1].clone()
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = parse_reg(
        "proc reg data=work.t ridge=0 0.1 outvif outest=est; model y = x1 x2; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let (out, _) = session.libs.get("WORK").unwrap().read("EST").unwrap();
    let names: Vec<String> = out.vars.iter().map(|v| v.name.clone()).collect();
    assert!(names.contains(&"_RIDGE_".to_string()), "no _RIDGE_ col: {names:?}");
    let tidx = out.vars.iter().position(|v| v.name == "_TYPE_").unwrap();
    let tcol = decode_column(&out, tidx).unwrap();
    let types: Vec<String> = tcol
        .iter()
        .map(|v| match v {
            crate::value::Value::Char(s) => s.trim_end().to_string(),
            _ => String::new(),
        })
        .collect();
    // PARMS then RIDGE/RIDGEVIF pairs for k=0 and k=0.1.
    assert_eq!(types[0], "PARMS");
    assert!(types.iter().filter(|t| *t == "RIDGE").count() == 2);
    assert!(types.iter().filter(|t| *t == "RIDGEVIF").count() == 2);
    // The k=0 RIDGE row's Intercept equals the OLS PARMS Intercept.
    let ridge_idx = out.vars.iter().position(|v| v.name == "_RIDGE_").unwrap();
    let ridge_col = decode_column(&out, ridge_idx).unwrap();
    let intidx = out.vars.iter().position(|v| v.name == "Intercept").unwrap();
    let intcol = decode_column(&out, intidx).unwrap();
    let parms_int = value_to_num(&intcol[0]).unwrap();
    // Find the RIDGE row with _RIDGE_==0.
    let k0row = (0..types.len())
        .find(|&i| types[i] == "RIDGE" && value_to_num(&ridge_col[i]) == Some(0.0))
        .unwrap();
    assert!((value_to_num(&intcol[k0row]).unwrap() - parms_int).abs() < 1e-6);
}

#[test]
fn test_m369_value_list_plain() {
    let ast =
        parse_reg("proc reg ridge=0 0.01 0.05 0.1; model y = x; run;").unwrap();
    assert_eq!(ast.data_options.ridge, vec![0.0, 0.01, 0.05, 0.1]);
}
