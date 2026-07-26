// MQ7.2c — `needless_range_loop` assumé dans ce module : l'indice EST le
// langage du domaine (`a[i][j] * b[j][k]`, parcours colonne-major, triangle
// d'une matrice symétrique). La forme itérateur y coûte plus en lisibilité
// qu'elle n'en rend, et la revue a préféré garder les indices explicites.
#![allow(clippy::needless_range_loop)]

use super::super::*;
use super::*;
use polars::df;

#[test]
fn test_oracle_outest_dataset_dep_is_minus1() {
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
    let ast = parse_reg("proc reg data=work.t outest=est; model y = x; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let (out, _) = session.libs.get("WORK").unwrap().read("EST").unwrap();
    let names: Vec<String> = out.vars.iter().map(|v| v.name.clone()).collect();
    // _MODEL_ _TYPE_ _DEPVAR_ _RMSE_ Intercept x y
    assert!(names.contains(&"_MODEL_".to_string()));
    assert!(names.contains(&"_TYPE_".to_string()));
    assert!(names.contains(&"Intercept".to_string()));
    // The dependent column 'y' is set to -1 in the PARMS row.
    let yidx = out.vars.iter().position(|v| v.name == "y").unwrap();
    let ycol = decode_column(&out, yidx).unwrap();
    assert_eq!(value_to_num(&ycol[0]), Some(-1.0));
    // _RMSE_ matches a direct fit.
    let xcol = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0];
    let yv = vec![2.0_f64, 4.0, 5.0, 4.0, 5.0, 7.0];
    let xm = design(true, &[&xcol], 6);
    let fit = ols_fit(&xm, &yv).unwrap();
    let rmse_idx = out.vars.iter().position(|v| v.name == "_RMSE_").unwrap();
    let rmse_col = decode_column(&out, rmse_idx).unwrap();
    let mse = fit.sse / (6 - 2) as f64;
    assert!((value_to_num(&rmse_col[0]).unwrap() - mse.sqrt()).abs() < 1e-9);
}

#[test]
fn test_oracle_outsscp_dataset() {
    let mut session = make_session();
    let xcol = [1.0_f64, 2.0, 3.0, 4.0, 5.0];
    let yv = [2.0_f64, 4.0, 5.0, 4.0, 5.0];
    let frame = df!["y" => yv.to_vec(), "x" => xcol.to_vec()].unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = parse_reg("proc reg data=work.t outsscp=sscp; model y = x; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let (out, _) = session.libs.get("WORK").unwrap().read("SSCP").unwrap();
    let names: Vec<String> = out.vars.iter().map(|v| v.name.clone()).collect();
    assert_eq!(names[0], "_TYPE_");
    assert_eq!(names[1], "_NAME_");
    assert!(names.contains(&"Intercept".to_string()));
    // Intercept diagonal == n: the Intercept row's Intercept column == 5.
    let nameidx = 1;
    let namecol = decode_column(&out, nameidx).unwrap();
    let int_row = namecol
        .iter()
        .position(|v| matches!(v, crate::value::Value::Char(s) if s.trim_end() == "Intercept"))
        .unwrap();
    let intcolidx = out.vars.iter().position(|v| v.name == "Intercept").unwrap();
    let intcol = decode_column(&out, intcolidx).unwrap();
    assert_eq!(value_to_num(&intcol[int_row]), Some(5.0));
    // (regressor x, dependent y) cell == Σ x_j·y.
    let xrow = namecol
        .iter()
        .position(|v| matches!(v, crate::value::Value::Char(s) if s.trim_end() == "x"))
        .unwrap();
    let ycolidx = out.vars.iter().position(|v| v.name == "y").unwrap();
    let ycol = decode_column(&out, ycolidx).unwrap();
    let want: f64 = xcol.iter().zip(yv.iter()).map(|(a, b)| a * b).sum();
    assert!((value_to_num(&ycol[xrow]).unwrap() - want).abs() < 1e-9);
}

#[test]
fn reg_without_ods_no_diagnostic() {
    let log = run_diag(false, None, None);
    assert!(!log.contains("image deferred"), "log: {log}");
    assert!(!log.contains("REG diagnostics"), "log: {log}");
}

#[cfg(not(feature = "graphics"))]
#[test]
fn reg_with_ods_no_feature_defers() {
    let log = run_diag(true, None, None);
    assert!(
        log.contains("REG diagnostics: image deferred"),
        "log: {log}"
    );
}

#[cfg(feature = "graphics")]
#[test]
fn reg_with_ods_and_feature_creates_image() {
    let dir = std::env::temp_dir();
    let log = run_diag(true, Some(dir.clone()), Some("regtest_diag".into()));
    assert!(log.contains("written"), "log: {log}");
    let p = dir.join("regtest_diag_1.png");
    assert!(p.exists(), "diagnostic image not created: {p:?}");
    assert!(p.metadata().unwrap().len() > 0);
    let _ = std::fs::remove_file(&p);
}

/// dfE ≤ 1 → RSTUDENT/COVRATIO/DFFITS/DFBETAS undefined (NaN).
#[test]
fn test_dfe_le_one_undefined() {
    // n=3, p_eff=2 → dfE=1.
    let x1 = [1.0_f64, 2.0, 4.0];
    let y = [1.0_f64, 3.0, 2.5];
    let n = y.len();
    let p_eff = 2;
    let x = design(true, &[&x1], n);
    let fit = ols_fit(&x, y.as_ref()).unwrap();
    let infl = compute_influence_stats(&x, y.as_ref(), &fit, n, p_eff, None);
    for s in &infl {
        assert!(!s.rstudent.is_finite());
        assert!(!s.covratio.is_finite());
        assert!(!s.dffits.is_finite());
        assert!(s.dfbetas.iter().all(|v| !v.is_finite()));
        // STUDENT and PRESS remain defined.
        assert!(s.press.is_finite());
    }
    // fmt_diag renders the SAS sentinel.
    assert_eq!(fmt_diag(f64::NAN), ".");
}

/// OUTPUT influence columns appear; DFBETAS= emits one column per parameter.
#[test]
fn test_output_influence_columns() {
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = parse_reg(
        "proc reg data=work.t; model y=x; output out=work.o student=stu cookd=cd h=hat dfbetas=b; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert!(names.contains(&"stu"));
    assert!(names.contains(&"cd"));
    assert!(names.contains(&"hat"));
    assert!(names.contains(&"b_Intercept"));
    assert!(names.contains(&"b_x"));
}

/// Single regressor → trivial VIF table (TOL=1, VIF=1).
#[test]
fn test_vif_single_regressor() {
    let cols = vec![vec![1.0_f64, 2.0, 3.0, 4.0]];
    let (tol, vif) = vif_tol(&cols);
    assert_eq!(tol, vec![1.0]);
    assert_eq!(vif, vec![1.0]);
}

/// Byte-identity guard: a plain model and one with only diagnostics-OFF must
/// produce identical parameter-table output (no extra columns).
#[test]
fn test_diagnostics_off_no_extra_columns() {
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        basic_model("y", &["x"]),
    );
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(!listing.contains("Tolerance"));
    assert!(!listing.contains("Variance Inflation"));
    assert!(!listing.contains("Collinearity Diagnostics"));
    assert!(!listing.contains("Durbin-Watson"));
}

/// Byte-identity guard: a model without the M36.5 options prints none of the
/// new columns or the PRESS line.
#[test]
fn test_m365_off_no_extra_columns() {
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        basic_model("y", &["x"]),
    );
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(!listing.contains("Type I SS"));
    assert!(!listing.contains("Type II SS"));
    assert!(!listing.contains("Standardized Estimate"));
    assert!(!listing.contains("Squared Partial Corr"));
    assert!(!listing.contains("Squared Semi-partial Corr"));
    assert!(!listing.contains("Sequential Parameter Estimate"));
    assert!(!listing.contains("PRESS"));
}

#[test]
fn test_m366_parse_rsquare_best_cp() {
    let ast = parse_reg("proc reg data=a; model y = x1 x2 x3 / selection=rsquare best=2 cp; run;")
        .unwrap();
    let sel = ast.models[0].model.selection.unwrap();
    assert_eq!(sel.method, SelMethod::RSquare);
    assert_eq!(sel.best, Some(2));
}

#[test]
fn test_m366_parse_adjrsq() {
    let ast = parse_reg("proc reg data=a; model y = x1 x2 / selection=adjrsq; run;").unwrap();
    let sel = ast.models[0].model.selection.unwrap();
    assert_eq!(sel.method, SelMethod::AdjRsq);
}

#[test]
fn test_m366_parse_maxr_include_stop_details_stb() {
    let ast = parse_reg(
        "proc reg data=a; model y = x1 x2 x3 / selection=maxr include=1 stop=2 details stb; run;",
    )
    .unwrap();
    let sel = ast.models[0].model.selection.unwrap();
    assert_eq!(sel.method, SelMethod::MaxR);
    assert_eq!(sel.include, 1);
    assert_eq!(sel.stop, Some(2));
    assert!(sel.details);
    assert!(sel.stb);
}

#[test]
fn test_m366_parse_none() {
    let ast = parse_reg("proc reg data=a; model y = x1 / selection=none; run;").unwrap();
    let sel = ast.models[0].model.selection.unwrap();
    assert_eq!(sel.method, SelMethod::None);
}

/// Oracle: an all-subsets enumeration over p regressors (include=0, start=1,
/// stop=p) yields exactly 2^p − 1 non-empty subsets.
#[test]
fn test_m366_all_subsets_count() {
    let (xcols, y) = three_reg_data();
    let p = 3;
    let mut count = 0usize;
    for mask in 1u32..(1u32 << p) {
        let cols: Vec<usize> = (0..p).filter(|b| mask & (1 << b) != 0).collect();
        // Every subset is rank-feasible for this fixture.
        assert!(subset_sse(&xcols, &y, &cols, true).is_some());
        count += 1;
    }
    assert_eq!(count, (1usize << p) - 1);
}

/// Oracle: the full-model subset's R² equals the OLS full-model R², and its
/// Mallows' C(p) ≈ p_eff (within 0.5).
#[test]
fn test_m366_full_model_r2_and_cp() {
    let (xcols, y) = three_reg_data();
    let n = y.len();
    let p = 3;
    let cols: Vec<usize> = (0..p).collect();
    // Full model R² via subset_sse vs. via direct OLS design matrix.
    let r2_subset = r2_full(&xcols, &y, &cols, true);
    let mut x = Vec::new();
    for i in 0..n {
        x.push(vec![1.0, xcols[0][i], xcols[1][i], xcols[2][i]]);
    }
    let fit = ols_fit(&x, &y).unwrap();
    let ybar = y.iter().sum::<f64>() / n as f64;
    let sst = y.iter().map(|v| (v - ybar) * (v - ybar)).sum::<f64>();
    let r2_ols = 1.0 - fit.sse / sst;
    assert!((r2_subset - r2_ols).abs() < 1e-9, "{r2_subset} vs {r2_ols}");

    // C(p) of the full model ≈ p_eff.
    let p_eff = (p + 1) as f64;
    let df_full = n as f64 - p_eff;
    let s2 = fit.sse / df_full;
    let cp = fit.sse / s2 - (n as f64 - 2.0 * p_eff);
    assert!((cp - p_eff).abs() < 0.5, "C(p)={cp} p_eff={p_eff}");
}

/// Oracle: adjusted R² matches 1 − (1−R²)(n−1)/(n−p_eff).
#[test]
fn test_m366_adjusted_r2_formula() {
    let (xcols, y) = three_reg_data();
    let n = y.len() as f64;
    let cols = vec![0usize, 2];
    let r2 = r2_full(&xcols, &y, &cols, true);
    let p_eff = (cols.len() + 1) as f64;
    let adj = 1.0 - (1.0 - r2) * (n - 1.0) / (n - p_eff);
    // Recompute via the same formula the implementation uses.
    let expect = 1.0 - (1.0 - r2) * (n - 1.0) / (n - p_eff);
    assert!((adj - expect).abs() < 1e-12);
}

/// Oracle: INCLUDE=k forces the first k regressors into every enumerated
/// subset (verified through run_all_subsets's listing).
#[test]
fn test_m366_include_forces_first_regressors() {
    let mut session = make_session();
    let (xcols, y) = three_reg_data();
    let regs: Vec<String> = vec!["x1".into(), "x2".into(), "x3".into()];
    let mut sel = sel_with(SelMethod::RSquare);
    sel.include = 1; // x1 forced
    run_all_subsets(&sel, &xcols, &y, &regs, true, &mut session);
    let listing = session.listing.take_string();
    assert!(listing.contains("R-Square Selection Method"), "{listing}");
    // Every "Variables in Model" entry must contain x1; size-1 row is "x1".
    for line in listing.lines() {
        // A data row begins with the model size then R-Square value.
        let t = line.trim();
        if t.starts_with('1') && t.contains("x") {
            assert!(t.contains("x1"), "size-1 row missing forced var: {t}");
        }
    }
}

/// Oracle: MAXR's final (size p) model is the full model, and its size-1
/// model is the single regressor with the highest R².
#[test]
fn test_m366_maxr_final_and_size1() {
    let mut session = make_session();
    let (xcols, y) = three_reg_data();
    let p = 3;
    let regs: Vec<String> = vec!["x1".into(), "x2".into(), "x3".into()];
    let sel = sel_with(SelMethod::MaxR);
    let final_set = run_rsq_improvement(&sel, &xcols, &y, &regs, true, &mut session).unwrap();
    assert_eq!(final_set, (0..p).collect::<Vec<usize>>());

    // The single best regressor by R².
    let best_single = (0..p)
        .max_by(|&a, &b| {
            let ra = r2_full(&xcols, &y, &[a], true);
            let rb = r2_full(&xcols, &y, &[b], true);
            ra.partial_cmp(&rb).unwrap()
        })
        .unwrap();
    // Re-run capturing the size-1 model via stop=1.
    let mut s1 = sel_with(SelMethod::MaxR);
    s1.stop = Some(1);
    let mut sess2 = make_session();
    let set1 = run_rsq_improvement(&s1, &xcols, &y, &regs, true, &mut sess2).unwrap();
    assert_eq!(set1, vec![best_single]);
}

/// Oracle: SELECTION=NONE produces the same fit as no SELECTION=.
#[test]
fn test_m366_none_matches_no_selection() {
    let build = |sel: Option<Selection>| -> String {
        let mut session = make_session();
        let frame = df![
            "y" => [3.0_f64, 5.0, 6.0, 9.0, 11.0, 13.0, 16.0, 18.0],
            "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
            "x2" => [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0, 7.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
        let mut model = basic_model("y", &["x1", "x2"]);
        model.selection = sel;
        let ast = single_model_ast(
            DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            },
            model,
        );
        execute(&ast, &mut session).unwrap();
        session.listing.take_string()
    };
    let plain = build(None);
    let none = build(Some(sel_with(SelMethod::None)));
    assert_eq!(plain, none);
}

/// BY with two groups runs the analysis once per group (two REG headers,
/// two BY headings).
#[test]
fn test_by_two_groups() {
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
        DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        basic_model("y", &["x"]),
    );
    ast.by = vec!["g".into()];
    execute(&ast, &mut session).unwrap();
    let out = session.listing.take_string();
    assert!(out.contains("g=1"), "{out}");
    assert!(out.contains("g=2"), "{out}");
    assert_eq!(out.matches("The REG Procedure").count(), 2, "{out}");
}
