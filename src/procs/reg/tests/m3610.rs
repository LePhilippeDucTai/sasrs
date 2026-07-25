use super::super::*;
use super::*;
use polars::df;

// ───────────────────────── M36.10 MTEST / run-group ─────────────────────────

#[test]
fn test_m3610_parse_multi_response() {
    // Two responses on the LHS → two dependents; regressors as before.
    let ast = parse_reg("proc reg data=a; model y1 y2 = x1 x2; run;").unwrap();
    let m = &ast.models[0].model;
    assert_eq!(m.dependents, vec!["y1", "y2"]);
    assert_eq!(m.regressors, vec!["x1", "x2"]);
    assert_eq!(m.dependent(), "y1");
    // Single response stays a one-element vector.
    let ast2 = parse_reg("proc reg data=a; model y = x1 x2; run;").unwrap();
    assert_eq!(ast2.models[0].model.dependents, vec!["y"]);
}

#[test]
fn test_m3610_multi_response_prints_block_per_dependent() {
    // A two-response MODEL (`model y1 y2 = x;`) must print a SEPARATE
    // univariate regression analysis for EACH dependent — one full
    // "Dependent Variable: y1" block then a full "Dependent Variable: y2"
    // block, each with its own ANOVA and Parameter Estimates — in MODEL
    // order, followed by the MTEST table(s).
    let mut session = make_session();
    let frame = df![
        "y1" => [2.0_f64, 4.0, 5.0, 4.0, 5.0, 7.0, 8.0, 9.0],
        "y2" => [1.0_f64, 3.0, 2.0, 5.0, 4.0, 6.0, 7.0, 8.0],
        "x"  => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y1"), num_meta("y2"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let ast = parse_reg(
        "proc reg data=work.t; model y1 y2 = x; mtest x; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();

    // BOTH per-response blocks are present.
    assert!(
        listing.contains("Dependent Variable: y1"),
        "missing y1 block: {listing}"
    );
    assert!(
        listing.contains("Dependent Variable: y2"),
        "missing y2 block: {listing}"
    );
    // MODEL order: the y1 block precedes the y2 block.
    let p1 = listing.find("Dependent Variable: y1").unwrap();
    let p2 = listing.find("Dependent Variable: y2").unwrap();
    assert!(p1 < p2, "y1 block must precede y2 block: {listing}");
    // Each response carries its own full analysis (two ANOVA tables).
    assert_eq!(
        listing.matches("Analysis of Variance").count(),
        2,
        "expected one ANOVA per response: {listing}"
    );
    // The MTEST table prints ONCE, after both per-response blocks.
    let mtest_pos = listing
        .find("Multivariate Test")
        .expect("MTEST table missing");
    assert!(mtest_pos > p2, "MTEST must follow the per-response blocks");
}

#[test]
fn test_m3610_parse_mtest_and_rungroup() {
    let ast = parse_reg(
        "proc reg data=a; model y1 y2 = x1 x2; \
         mtest; \
         overall: mtest x1, x2; \
         var x3 x4; add x3; delete x1; \
         reweight x1 > 5; refit; paint obs / red; run;",
    )
    .unwrap();
    let entry = &ast.models[0];
    // MTEST: one unlabeled (default), one labeled with two equations.
    assert_eq!(entry.mtests.len(), 2);
    assert!(entry.mtests[0].label.is_none());
    assert!(entry.mtests[0].equations.is_empty());
    assert_eq!(entry.mtests[1].label.as_deref(), Some("overall"));
    assert_eq!(entry.mtests[1].equations.len(), 2);
    // ADD / DELETE / VAR recorded.
    assert_eq!(entry.add, vec!["x3"]);
    assert_eq!(entry.delete, vec!["x1"]);
    assert_eq!(ast.var_list, vec!["x3", "x4"]);
    // Deferred statements flagged.
    assert!(ast.reweight_seen);
    assert!(ast.refit_seen);
    assert!(ast.paint_seen);
}

#[test]
fn test_m3610_statistic_identities() {
    // From three positive generalized eigenvalues, verify the internal
    // identities between the four statistics.
    let lambda = vec![2.5_f64, 1.0, 0.3];
    let stats = mtest_statistics(&lambda, 3.0, 3.0, 20.0);
    let by_name = |n: &str| stats.iter().find(|s| s.name == n).unwrap().value;
    let wilks: f64 = lambda.iter().map(|&l| 1.0 / (1.0 + l)).product();
    let pillai: f64 = lambda.iter().map(|&l| l / (1.0 + l)).sum();
    let hlt: f64 = lambda.iter().sum();
    let roy = 2.5_f64;
    assert!((by_name("Wilks' Lambda") - wilks).abs() < 1e-12);
    assert!((by_name("Pillai's Trace") - pillai).abs() < 1e-12);
    assert!((by_name("Hotelling-Lawley Trace") - hlt).abs() < 1e-12);
    assert!((by_name("Roy's Greatest Root") - roy).abs() < 1e-12);
}

#[test]
fn test_m3610_generalized_eig_symmetric_spd() {
    // E SPD, H symmetric. Generalized eigenvalues of E⁻¹H must match the
    // ordinary eigenvalues of E⁻¹H computed densely.
    let e = vec![vec![4.0, 1.0], vec![1.0, 3.0]];
    let h = vec![vec![2.0, 0.5], vec![0.5, 1.0]];
    let lambda = generalized_eigenvalues(&e, &h).unwrap();
    // Reference: eigenvalues of E⁻¹H.
    let einv = linalg::invert_matrix(&e).unwrap();
    let eih = linalg::matrix_mult(&einv, &h);
    // 2×2 eigenvalues via trace/det.
    let tr = eih[0][0] + eih[1][1];
    let det = eih[0][0] * eih[1][1] - eih[0][1] * eih[1][0];
    let disc = (tr * tr - 4.0 * det).max(0.0).sqrt();
    let mut refv = vec![(tr + disc) / 2.0, (tr - disc) / 2.0];
    refv.sort_by(|a, b| b.partial_cmp(a).unwrap());
    assert!((lambda[0] - refv[0]).abs() < 1e-9, "{lambda:?} vs {refv:?}");
    assert!((lambda[1] - refv[1]).abs() < 1e-9, "{lambda:?} vs {refv:?}");
}

#[test]
fn test_m3610_single_response_reduces_to_anova_f() {
    // Oracle: with one response the four MTEST statistics give the same F as
    // the ANOVA overall F. Build the regression by hand and compare.
    // Data: y = response, x1, x2 regressors, n=8.
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let x2 = [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0, 7.0];
    let y = [3.0_f64, 5.0, 8.0, 9.0, 13.0, 14.0, 18.0, 20.0];
    let n = y.len();
    // Design with intercept.
    let x_mat: Vec<Vec<f64>> = (0..n)
        .map(|i| vec![1.0, x1[i], x2[i]])
        .collect();
    let fit = ols_fit(&x_mat, &y).unwrap();
    let p_eff = 3usize;
    let intercept = true;
    // ANOVA overall F = (SSR/p_reg) / (SSE/(n-p_eff)).
    let ybar = y.iter().sum::<f64>() / n as f64;
    let sst: f64 = y.iter().map(|v| (v - ybar) * (v - ybar)).sum();
    let ssr = sst - fit.sse;
    let p_reg = 2.0; // x1, x2
    let v = (n - p_eff) as f64;
    let anova_f = (ssr / p_reg) / (fit.sse / v);

    // MTEST: q_resp = 1, default hypothesis = both regressors zero.
    let xt = linalg::transpose(&x_mat);
    let xtx = linalg::matrix_mult(&xt, &x_mat);
    let xtx_inv = linalg::invert_matrix(&xtx).unwrap();
    // Y is n×1.
    let y_mat: Vec<Vec<f64>> = y.iter().map(|&v| vec![v]).collect();
    let xty = linalg::matrix_mult(&xt, &y_mat);
    let b = linalg::matrix_mult(&xtx_inv, &xty);
    let yt = linalg::transpose(&y_mat);
    let yty = linalg::matrix_mult(&yt, &y_mat);
    let ytx = linalg::matrix_mult(&yt, &x_mat);
    let ytx_b = linalg::matrix_mult(&ytx, &b);
    let e = vec![vec![yty[0][0] - ytx_b[0][0]]];
    // L = non-intercept rows.
    let base = intercept as usize;
    let l: Vec<Vec<f64>> = (0..2)
        .map(|k| {
            let mut r = vec![0.0; p_eff];
            r[base + k] = 1.0;
            r
        })
        .collect();
    let lb = linalg::matrix_mult(&l, &b);
    let lt = linalg::transpose(&l);
    let lxi = linalg::matrix_mult(&l, &xtx_inv);
    let lxil = linalg::matrix_mult(&lxi, &lt);
    let lxil_inv = linalg::invert_matrix(&lxil).unwrap();
    let mid = linalg::matrix_mult(&linalg::transpose(&lb), &lxil_inv);
    let h = linalg::matrix_mult(&mid, &lb);
    let lambda = generalized_eigenvalues(&e, &h).unwrap();
    let stats = mtest_statistics(&lambda, 1.0, 2.0, v);
    // Every statistic's F approximation must equal the ANOVA F.
    for s in &stats {
        assert!(
            (s.f - anova_f).abs() < 1e-4,
            "{} F={} vs ANOVA F={}",
            s.name,
            s.f,
            anova_f
        );
    }
    // H == model SS (SSR) and E == SSE for q_resp=1; both symmetric/SPD.
    assert!((h[0][0] - ssr).abs() < 1e-6);
    assert!((e[0][0] - fit.sse).abs() < 1e-6);
    assert!(e[0][0] > 0.0);
}

#[test]
fn test_m3610_mtest_execute_prints_table() {
    let mut session = make_session();
    let frame = df![
        "y1" => [3.0_f64, 5.0, 8.0, 9.0, 13.0, 14.0, 18.0, 20.0],
        "y2" => [1.0_f64, 2.0, 2.5, 4.0, 4.5, 6.0, 6.5, 8.0],
        "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
        "x2" => [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0, 7.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y1"), num_meta("y2"), num_meta("x1"), num_meta("x2")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = parse_reg(
        "proc reg data=work.t; model y1 y2 = x1 x2; mymt: mtest; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("Multivariate Test: mymt"), "{listing}");
    assert!(listing.contains("Wilks' Lambda"), "{listing}");
    assert!(listing.contains("Pillai's Trace"), "{listing}");
    assert!(listing.contains("Hotelling-Lawley Trace"), "{listing}");
    assert!(listing.contains("Roy's Greatest Root"), "{listing}");
}

#[test]
fn test_m3610_add_delete_applied_to_fit() {
    // ADD/DELETE edit the regressor set for the final fit. Verify a NOTE is
    // emitted and the fit uses the edited set (x2 only, after deleting x1 and
    // adding x2 which was not in the original MODEL).
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 5.0, 7.0],
        "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
        "x2" => [6.0_f64, 5.0, 4.0, 3.0, 2.0, 1.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = parse_reg(
        "proc reg data=work.t; model y = x1; add x2; delete x1; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    let listing = session.listing.into_string();
    // The fitted parameter table must list x2 (the edited regressor).
    assert!(listing.contains("x2"), "{listing}");
    assert!(
        log.contains("ADD/DELETE statements were applied"),
        "log: {log}"
    );
}

#[test]
fn test_m3610_deferred_notes() {
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 5.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = parse_reg(
        "proc reg data=work.t; model y = x; reweight x > 3; refit; paint obs; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("REWEIGHT statement"), "log: {log}");
    assert!(log.contains("REFIT statement"), "log: {log}");
    assert!(log.contains("PAINT statement"), "log: {log}");
}

#[cfg(not(feature = "graphics"))]
#[test]
fn plots_request_defers_under_default_build() {
    let mut req = PlotRequests::default();
    req.diagnostics = true;
    req.explicit = true;
    let log = run_plots(false, false, req, vec![]);
    assert!(log.contains("REG PLOTS= request"), "log: {log}");
    assert!(log.contains("deferred"), "log: {log}");
}

/// PLOTS=NONE suppresses the automatic diagnostic image/NOTE (even with ODS on).
#[test]
fn plots_none_suppresses_automatic_diagnostic() {
    let mut req = PlotRequests::default();
    req.none = true;
    req.explicit = true;
    let log = run_plots(true, true, req, vec![]);
    assert!(!log.contains("REG diagnostics"), "log: {log}");
    assert!(!log.contains("image deferred"), "log: {log}");
}

#[cfg(feature = "graphics")]
#[test]
fn plots_request_renders_images_under_graphics() {
    let dir = std::env::temp_dir().join("sasrs_reg_plots_test");
    let _ = std::fs::create_dir_all(&dir);
    let mut session = make_session();
    session.ods_graphics.enabled = true;
    session.ods_graphics.output_dir = dir.clone();
    session.ods_graphics.file_stem = Some("regplt".into());
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
    let mut ast = single_model_ast(
        DatasetRef { libref: Some("WORK".into()), name: "T".into() },
        basic_model("y", &["x"]),
    );
    ast.plot_requests = PlotRequests { all: true, explicit: true, ..Default::default() };
    ast.plot_statements = vec![PlotPair { y: PlotVar::Residual, x: PlotVar::Predicted }];
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("written"), "log: {log}");
    // The first image is the automatic diagnostic (regplt_1); subsequent
    // images come from PLOTS=ALL + the PLOT statement. Confirm at least the
    // automatic and one request image exist.
    let p1 = dir.join("regplt_1.png");
    assert!(p1.exists(), "automatic diagnostic image missing: {p1:?}");
    let p2 = dir.join("regplt_2.png");
    assert!(p2.exists(), "requested plot image missing: {p2:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(not(feature = "graphics"))]
#[test]
fn plot_statement_defers_under_default_build() {
    let pairs = vec![PlotPair { y: PlotVar::Residual, x: PlotVar::Predicted }];
    let log = run_plots(false, false, PlotRequests::default(), pairs);
    assert!(log.contains("REG PLOT statement"), "log: {log}");
    assert!(log.contains("deferred"), "log: {log}");
}

/// Byte-identity: a model WITHOUT any PLOTS=/PLOT requests produces exactly
/// the same log as today (no new NOTE lines from M36.11).
#[test]
fn no_plots_request_is_byte_identical() {
    let bare = run_plots(false, false, PlotRequests::default(), vec![]);
    assert!(!bare.contains("REG PLOTS="), "log: {bare}");
    assert!(!bare.contains("REG PLOT statement"), "log: {bare}");
    // And matches the pre-existing run_diag(false,…) output exactly.
    let diag = run_diag(false, None, None);
    assert_eq!(bare, diag);
}
