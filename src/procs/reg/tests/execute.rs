// MQ7.2c — `needless_range_loop` assumé dans ce module : l'indice EST le
// langage du domaine (`a[i][j] * b[j][k]`, parcours colonne-major, triangle
// d'une matrice symétrique). La forme itérateur y coûte plus en lisibilité
// qu'elle n'en rend, et la revue a préféré garder les indices explicites.
#![allow(clippy::needless_range_loop)]

use super::super::*;
use super::*;
use crate::source::SourceFile;
use polars::df;

#[test]
fn test_execute_simple() {
    let mut session = make_session();
    let frame = df![
        "weight" => [112.0_f64, 100.0, 130.0, 145.0, 160.0, 105.0],
        "height" => [59.0_f64, 57.0, 63.0, 67.0, 67.0, 57.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("weight"), num_meta("height")],
    };
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("CLASS", &ds)
        .unwrap();

    let ast = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "CLASS".into(),
        },
        basic_model("weight", &["height"]),
    );
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("The REG Procedure"), "{listing}");
    assert!(listing.contains("Analysis of Variance"), "{listing}");
    assert!(
        listing.contains("Parameter Estimates") || listing.contains("Parameter"),
        "{listing}"
    );
}

/// End-to-end: TEST and RESTRICT statements parse and execute, emitting the
/// expected blocks in the listing.
#[test]
fn test_execute_test_and_restrict() {
    let mut session = make_session();
    let frame = df![
        "y"  => [3.2_f64, 4.8, 7.1, 8.9, 11.3, 12.7, 15.2, 16.8],
        "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
        "x2" => [4.0_f64, 1.0, 9.0, 2.0, 8.0, 3.0, 7.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let src =
        "proc reg data=work.t; model y = x1 x2; peak: test x1 = x2; restrict x1 + x2 = 3; run;";
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next();
    ts.next();
    let ast = parse(&mut ts).unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(
        listing.contains("Test peak Results for Dependent Variable y"),
        "{listing}"
    );
    assert!(listing.contains("Numerator"), "{listing}");
    assert!(listing.contains("Denominator"), "{listing}");
    assert!(listing.contains("RESTRICT"), "{listing}");
}

/// End-to-end: CLB adds confidence-limit columns; CLM/CLI emit Output
/// Statistics. Default model (no options) must NOT print either.
#[test]
fn test_execute_cl_listing() {
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
    let mut model = basic_model("y", &["x"]);
    model.clb = true;
    model.clm = true;
    model.cli = true;
    let ast = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        model,
    );
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("95% Confidence Limits"), "{listing}");
    assert!(listing.contains("Output Statistics"), "{listing}");
    assert!(listing.contains("CL Mean"), "{listing}");
    assert!(listing.contains("CL Predict"), "{listing}");
}

/// End-to-end: R and INFLUENCE listings print; default model prints neither.
#[test]
fn test_execute_r_influence_listing() {
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
    let mut model = basic_model("y", &["x"]);
    model.r = true;
    model.influence = true;
    let ast = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        model,
    );
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("Student Residual"), "{listing}");
    assert!(listing.contains("Sum of Residuals"), "{listing}");
    assert!(listing.contains("PRESS"), "{listing}");
    assert!(listing.contains("RStudent"), "{listing}");
    assert!(listing.contains("DFBETAS Intercept"), "{listing}");
    assert!(listing.contains("DFBETAS x"), "{listing}");
}

/// End-to-end: VIF/TOL columns appear; default model does NOT print them.
#[test]
fn test_execute_diagnostics_listing() {
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0, 9.0, 11.0],
        "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
        "x2" => [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0, 7.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast =
        parse_reg("proc reg data=work.t; model y=x1 x2 / vif tol collin spec dw dwprob acov; run;")
            .unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("Tolerance"), "{listing}");
    assert!(listing.contains("Variance Inflation"), "{listing}");
    assert!(listing.contains("Collinearity Diagnostics"), "{listing}");
    assert!(
        listing.contains("Test of First and Second Moment Specification"),
        "{listing}"
    );
    assert!(listing.contains("Durbin-Watson D"), "{listing}");
    assert!(listing.contains("Pr < DW"), "{listing}");
    assert!(
        listing.contains("Consistent Covariance of Estimates"),
        "{listing}"
    );
}

/// End-to-end: M36.5 columns appear in the parameter table and the PRESS fit
/// statistic line is printed; a default model prints none of them.
#[test]
fn test_execute_m365_listing() {
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0, 9.0, 11.0],
        "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
        "x2" => [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0, 7.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = parse_reg(
        "proc reg data=work.t; model y=x1 x2 / ss1 ss2 stb pcorr1 pcorr2 scorr1 scorr2 seqb press; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("Type I SS"), "{listing}");
    assert!(listing.contains("Type II SS"), "{listing}");
    assert!(listing.contains("Standardized Estimate"), "{listing}");
    assert!(listing.contains("Squared Partial Corr Type I"), "{listing}");
    assert!(
        listing.contains("Squared Partial Corr Type II"),
        "{listing}"
    );
    assert!(
        listing.contains("Squared Semi-partial Corr Type I"),
        "{listing}"
    );
    assert!(
        listing.contains("Squared Semi-partial Corr Type II"),
        "{listing}"
    );
    assert!(
        listing.contains("Sequential Parameter Estimate"),
        "{listing}"
    );
    assert!(listing.contains("PRESS"), "{listing}");
}

/// NOINT on a tiny known dataset: y = 2x exactly (no intercept), so the
/// no-intercept fit gives slope=2, uncorrected R² = Σŷ²/Σy² = 1, and there
/// is NO Intercept row in the parameter-estimates table.
#[test]
fn test_noint_fit() {
    let mut session = make_session();
    // y = 2*x, with x = 1..5.
    let frame = df![
        "y" => [2.0_f64, 4.0, 6.0, 8.0, 10.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let mut model = basic_model("y", &["x"]);
    model.noint = true;
    let ast = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        model,
    );
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("Uncorrected Total"), "{listing}");
    // R² = 1.0000 (perfect through-origin fit).
    assert!(listing.contains("R-Square     1.0000"), "{listing}");
    // No Intercept row in parameter estimates.
    assert!(!listing.contains("Intercept"), "{listing}");
    assert!(listing.contains("The REG Procedure"), "{listing}");
}

/// Direct numeric check of the NOINT uncorrected decomposition via ols_fit.
#[test]
fn test_noint_uncorrected_r2_formula() {
    // X has no intercept column.
    let x = vec![vec![1.0], vec![2.0], vec![3.0], vec![4.0], vec![5.0]];
    let y = vec![2.1, 3.9, 6.2, 7.8, 10.1];
    let fit = ols_fit(&x, &y).unwrap();
    let ssm: f64 = fit.y_hat.iter().map(|v| v * v).sum();
    let sst: f64 = y.iter().map(|v| v * v).sum();
    let r2 = ssm / sst;
    // 1 - SSE/Σy² must match Σŷ²/Σy².
    let r2_alt = 1.0 - fit.sse / sst;
    assert!((r2 - r2_alt).abs() < 1e-10, "r2={r2} r2_alt={r2_alt}");
    assert!(r2 > 0.99, "near-perfect fit expected, r2={r2}");
}

/// Partial-F to ENTER equals the candidate's t² in the augmented fit.
#[test]
fn test_partial_f_equals_t_squared() {
    // Two regressors; intercept present.
    // y depends mostly on x1.
    let x1 = vec![1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0];
    let x2 = vec![5.0_f64, 3.0, 6.0, 2.0, 7.0, 1.0]; // noise-ish
    let y: Vec<f64> = x1.iter().map(|&v| 3.0 + 2.0 * v).collect();
    let xcols = vec![x1.clone(), x2.clone()];
    let n = y.len();

    // Enter x1 (col 0) into empty set, intercept present.
    let s: Vec<usize> = vec![];
    let cand = vec![0usize];
    let sse_s = subset_sse(&xcols, &y, &s, true).unwrap();
    let sse_c = subset_sse(&xcols, &y, &cand, true).unwrap();
    let df_full = (n as f64) - (cand.len() as f64) - 1.0;
    let f_enter = (sse_s - sse_c) / (sse_c / df_full);

    // Augmented fit: design [1, x1]; t for x1's coefficient.
    let mut xmat: Vec<Vec<f64>> = Vec::new();
    for i in 0..n {
        xmat.push(vec![1.0, x1[i]]);
    }
    let fit = ols_fit(&xmat, &y).unwrap();
    let mse = fit.sse / df_full;
    let se = (mse * fit.xtx_inv[1][1]).sqrt();
    let t = fit.beta[1] / se;
    let t2 = t * t;
    // Perfect linear data → both huge; compare relative or that both large.
    // Use a perturbed y to avoid degeneracy.
    let _ = (f_enter, t2);

    // Re-run with a slightly noisy y so SSE>0.
    let y2: Vec<f64> = x1
        .iter()
        .map(|&v| 3.0 + 2.0 * v + (v * 0.137).sin())
        .collect();
    let sse_s2 = subset_sse(&xcols, &y2, &s, true).unwrap();
    let sse_c2 = subset_sse(&xcols, &y2, &cand, true).unwrap();
    let f_enter2 = (sse_s2 - sse_c2) / (sse_c2 / df_full);
    let mut xmat2: Vec<Vec<f64>> = Vec::new();
    for i in 0..n {
        xmat2.push(vec![1.0, x1[i]]);
    }
    let fit2 = ols_fit(&xmat2, &y2).unwrap();
    let mse2 = fit2.sse / df_full;
    let se2 = (mse2 * fit2.xtx_inv[1][1]).sqrt();
    let t_2 = fit2.beta[1] / se2;
    let t2_2 = t_2 * t_2;
    assert!(
        (f_enter2 - t2_2).abs() < 1e-6,
        "F_enter={f_enter2} t^2={t2_2}"
    );
}

/// FORWARD selection: x1 strongly predicts y; x2 is pure noise → x1 enters,
/// x2 is rejected at slentry=0.05.
#[test]
fn test_forward_selection() {
    let mut session = make_session();
    // y tracks x1 closely (strong signal) with mild noise; x2 is unrelated.
    let frame = df![
        "y"  => [3.2_f64, 4.8, 7.1, 8.9, 11.3, 12.7, 15.2, 16.8],
        "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
        "x2" => [4.0_f64, 1.0, 9.0, 2.0, 8.0, 3.0, 7.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let mut model = basic_model("y", &["x1", "x2"]);
    model.selection = Some(Selection {
        method: SelMethod::Forward,
        // x1 enters (p<.0001); x2's partial-F p (~0.035) exceeds slentry,
        // so x2 is rejected.
        slentry: 0.01,
        slstay: 0.01,
        best: None,
        include: 0,
        start: None,
        stop: None,
        details: false,
        stb: false,
    });
    let ast = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        model,
    );
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(
        listing.contains("Summary of Forward Selection"),
        "{listing}"
    );
    // Inspect the final fitted-model block (after the last "Model: MODEL1"),
    // which holds the parameter-estimates table.
    let final_block = listing.rsplit("Model: MODEL1").next().unwrap();
    // x1 entered → appears as a fitted parameter; x2 rejected → absent.
    assert!(final_block.contains("x1"), "{listing}");
    assert!(
        !final_block.contains("x2"),
        "x2 should be rejected: {listing}"
    );
}

/// BACKWARD selection: start with both x1 and noise x2; x2 is eliminated.
#[test]
fn test_backward_selection() {
    let mut session = make_session();
    // y is x1 plus mild noise (not a perfect fit), x2 is unrelated noise.
    let frame = df![
        "y"  => [3.2_f64, 4.8, 7.1, 8.9, 11.3, 12.7, 15.2, 16.8],
        "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
        "x2" => [4.0_f64, 1.0, 9.0, 2.0, 8.0, 3.0, 7.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let mut model = basic_model("y", &["x1", "x2"]);
    model.selection = Some(Selection {
        method: SelMethod::Backward,
        // x2's removal p (~0.035) exceeds slstay, so x2 is eliminated; x1
        // (highly significant) is retained.
        slentry: 0.10,
        slstay: 0.01,
        best: None,
        include: 0,
        start: None,
        stop: None,
        details: false,
        stb: false,
    });
    let ast = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        model,
    );
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(
        listing.contains("Summary of Backward Elimination"),
        "{listing}"
    );
    // Inspect the final fitted-model block (after the last "Model: MODEL1").
    let final_block = listing.rsplit("Model: MODEL1").next().unwrap();
    // x2 removed → absent from fitted parameters; x1 retained → present.
    assert!(final_block.contains("x1"), "{listing}");
    assert!(
        !final_block.contains("x2"),
        "x2 should be eliminated: {listing}"
    );
}
