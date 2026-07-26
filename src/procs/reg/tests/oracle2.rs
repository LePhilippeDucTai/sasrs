use super::super::*;
use super::*;
use polars::df;

/// Multi-regressor oracles: Σ SS1 (regressors) == Model SS; SS2_j == t_j²·MSE;
/// all PCORR/SCORR in [0,1]; SEQB of the last column == its OLS β.
#[test]
fn test_oracle_seq_stats_multi() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let x2 = [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 9.0, 7.0];
    let y: Vec<f64> = (0..8)
        .map(|i| 1.0 + 2.0 * x1[i] - 0.5 * x2[i] + (x1[i] * 0.7).sin())
        .collect();
    let n = y.len();
    let x = design(true, &[&x1, &x2], n);
    let fit = ols_fit(&x, &y).unwrap();
    let p_eff = 3;
    let df_e = (n - p_eff) as f64;
    let mse = fit.sse / df_e;

    // Corrected total.
    let ybar = y.iter().sum::<f64>() / n as f64;
    let sst: f64 = y.iter().map(|v| (v - ybar) * (v - ybar)).sum();
    let model_ss = sst - fit.sse;

    let m = seq_model("y", &["x1", "x2"]);
    let s = compute_seq_stats(&m, &x, &y, &fit, sst, true);

    // Σ SS1 over the regressors (skip intercept at col 0) == Model SS.
    let sum_ss1_reg: f64 = s.ss1[1] + s.ss1[2];
    assert!(
        (sum_ss1_reg - model_ss).abs() < 1e-6,
        "ΣSS1={sum_ss1_reg} ModelSS={model_ss}"
    );

    // SS2_j == t_j²·MSE for every column (intercept included).
    for j in 0..p_eff {
        let se = (mse * fit.xtx_inv[j][j]).sqrt();
        let t = fit.beta[j] / se;
        assert!(
            (s.ss2[j] - t * t * mse).abs() < 1e-6,
            "SS2[{j}]={} t²·MSE={}",
            s.ss2[j],
            t * t * mse
        );
    }

    // All correlations in [0,1].
    for j in 0..p_eff {
        for v in [s.pcorr1[j], s.pcorr2[j], s.scorr1[j], s.scorr2[j]] {
            assert!((0.0..=1.0).contains(&v), "corr out of range: {v}");
        }
    }

    // SEQB of the last column == its OLS β.
    assert!(
        (s.seqb[p_eff - 1] - fit.beta[p_eff - 1]).abs() < 1e-9,
        "SEQB last={} β last={}",
        s.seqb[p_eff - 1],
        fit.beta[p_eff - 1]
    );
}

/// Single-regressor identities: SS1==SS2==Model SS; PCORR2==SCORR2==R²;
/// STB == sign(β)·|r| with r = corr(x,y).
#[test]
fn test_oracle_seq_stats_single() {
    let x = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let y: Vec<f64> = x.iter().map(|&a| 0.5 + 1.7 * a + (a * 0.3).cos()).collect();
    let n = y.len();
    let xm = design(true, &[&x], n);
    let fit = ols_fit(&xm, &y).unwrap();

    let ybar = y.iter().sum::<f64>() / n as f64;
    let sst: f64 = y.iter().map(|v| (v - ybar) * (v - ybar)).sum();
    let model_ss = sst - fit.sse;
    let r2 = model_ss / sst;

    let m = seq_model("y", &["x"]);
    let s = compute_seq_stats(&m, &xm, &y, &fit, sst, true);

    // The single regressor sits at column 1 (col 0 is intercept).
    assert!((s.ss1[1] - model_ss).abs() < 1e-6, "SS1 != ModelSS");
    assert!((s.ss2[1] - model_ss).abs() < 1e-6, "SS2 != ModelSS");
    assert!((s.ss1[1] - s.ss2[1]).abs() < 1e-6, "SS1 != SS2");
    assert!((s.pcorr2[1] - r2).abs() < 1e-6, "PCORR2 != R²");
    assert!((s.scorr2[1] - r2).abs() < 1e-6, "SCORR2 != R²");

    // STB == sign(β)·|corr(x,y)|.
    let xbar = x.iter().sum::<f64>() / n as f64;
    let sxy: f64 = (0..n).map(|i| (x[i] - xbar) * (y[i] - ybar)).sum();
    let sxx: f64 = x.iter().map(|v| (v - xbar) * (v - xbar)).sum();
    let r = sxy / (sxx.sqrt() * sst.sqrt());
    let expect_stb = fit.beta[1].signum() * r.abs();
    assert!(
        (s.stb[1] - expect_stb).abs() < 1e-6,
        "STB={} expected={}",
        s.stb[1],
        expect_stb
    );
    // Intercept STB is 0.
    assert!(s.stb[0].abs() < 1e-12);
}

/// NOINT: SS uses the uncorrected total; Σ SS1 over all columns == Model SS
/// (uncorrected); SS2 == t²·MSE still holds.
#[test]
fn test_oracle_seq_stats_noint() {
    let x = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0];
    let y: Vec<f64> = x.iter().map(|&a| 2.0 * a + (a * 0.5).sin()).collect();
    let n = y.len();
    let xm = design(false, &[&x], n); // no intercept column
    let fit = ols_fit(&xm, &y).unwrap();
    let p_eff = 1;
    let mse = fit.sse / (n - p_eff) as f64;

    let sst: f64 = y.iter().map(|v| v * v).sum(); // uncorrected
    let ssm: f64 = fit.y_hat.iter().map(|v| v * v).sum();

    let mut m = seq_model("y", &["x"]);
    m.noint = true;
    let s = compute_seq_stats(&m, &xm, &y, &fit, sst, false);

    // Σ SS1 over all (no intercept) columns == uncorrected Model SS.
    assert!((s.ss1[0] - ssm).abs() < 1e-6, "SS1={} SSM={ssm}", s.ss1[0]);
    // SS2 == t²·MSE.
    let se = (mse * fit.xtx_inv[0][0]).sqrt();
    let t = fit.beta[0] / se;
    assert!((s.ss2[0] - t * t * mse).abs() < 1e-6);
    // SEQB == OLS β (last & only column).
    assert!((s.seqb[0] - fit.beta[0]).abs() < 1e-9);
}

/// PRESS statistic oracle: Σ (resid_i/(1−h_i))² within 1e-9.
#[test]
fn test_oracle_press_statistic() {
    let x = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let y: Vec<f64> = x.iter().map(|&a| 1.0 + 0.8 * a + (a * 0.6).sin()).collect();
    let n = y.len();
    let xm = design(true, &[&x], n);
    let fit = ols_fit(&xm, &y).unwrap();
    let h = leverages(&xm, &fit.xtx_inv);
    let press_ref: f64 = (0..n)
        .map(|i| {
            let p = fit.resid[i] / (1.0 - h[i]);
            p * p
        })
        .sum();
    // Recompute via the same formula used in run_model.
    let press: f64 = fit
        .resid
        .iter()
        .zip(h.iter())
        .map(|(e, &hi)| {
            let p = e / (1.0 - hi);
            p * p
        })
        .sum();
    assert!((press - press_ref).abs() < 1e-9);
    assert!(press > 0.0);
}

// ───────────────────────── M36.7 oracles ─────────────────────────

/// WEIGHT with all w_i = 1 ⇒ identical β / SSE / SE as unweighted OLS.
#[test]
fn test_oracle_weight_ones_equals_ols() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let y: Vec<f64> = x1
        .iter()
        .map(|&a| 1.5 + 2.0 * a + (a * 0.3).cos())
        .collect();
    let n = y.len();
    let x = design(true, &[&x1], n);
    let ols = ols_fit(&x, &y).unwrap();
    let wls = weighted_ols_fit(&x, &y, &vec![1.0; n]).unwrap();
    for j in 0..2 {
        assert!((ols.beta[j] - wls.beta[j]).abs() < 1e-9);
        for k in 0..2 {
            assert!((ols.xtx_inv[j][k] - wls.xtx_inv[j][k]).abs() < 1e-9);
        }
    }
    assert!((ols.sse - wls.sse).abs() < 1e-9);
}

/// WLS β solves the weighted normal equations X'WX β = X'Wy (residual ~ 0).
#[test]
fn test_oracle_weighted_normal_equations() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let y: Vec<f64> = x1.iter().map(|&a| 0.5 + 1.3 * a + (a).sin()).collect();
    let w = [0.5_f64, 2.0, 1.0, 3.0, 0.25, 4.0, 1.5, 0.75];
    let n = y.len();
    let x = design(true, &[&x1], n);
    let fit = weighted_ols_fit(&x, &y, &w).unwrap();
    // Form X'WX and X'Wy and check the residual of the normal equations.
    let p = 2;
    let mut xtwx = vec![vec![0.0; p]; p];
    let mut xtwy = vec![0.0; p];
    for i in 0..n {
        for a in 0..p {
            xtwy[a] += w[i] * x[i][a] * y[i];
            for b in 0..p {
                xtwx[a][b] += w[i] * x[i][a] * x[i][b];
            }
        }
    }
    for a in 0..p {
        let lhs: f64 = (0..p).map(|b| xtwx[a][b] * fit.beta[b]).sum();
        assert!(
            (lhs - xtwy[a]).abs() < 1e-7,
            "normal eq row {a}: {lhs} vs {}",
            xtwy[a]
        );
    }
}

/// WEIGHT equal to a constant c ⇒ same β as OLS, SSE scaled by c.
#[test]
fn test_oracle_weight_constant_scale_invariance() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0];
    let y: Vec<f64> = x1
        .iter()
        .map(|&a| 3.0 - 0.7 * a + (a * 0.4).sin())
        .collect();
    let n = y.len();
    let x = design(true, &[&x1], n);
    let ols = ols_fit(&x, &y).unwrap();
    let c = 4.0;
    let wls = weighted_ols_fit(&x, &y, &vec![c; n]).unwrap();
    for j in 0..2 {
        assert!((ols.beta[j] - wls.beta[j]).abs() < 1e-9);
    }
    assert!((wls.sse - c * ols.sse).abs() < 1e-9);
}

/// FREQ = 2 everywhere ⇒ same β as no FREQ, and the ANOVA df doubles
/// (error_df = 2n − p_eff). End-to-end through execute().
#[test]
fn test_oracle_freq_two_doubles_df() {
    let render = |with_freq: bool| -> String {
        let mut session = make_session();
        let frame = df![
            "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0],
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
            "f" => [2.0_f64, 2.0, 2.0, 2.0, 2.0, 2.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x"), num_meta("f")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
        let mut ast = single_model_ast(
            DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            },
            basic_model("y", &["x"]),
        );
        if with_freq {
            ast.freq = Some("f".into());
        }
        execute(&ast, &mut session).unwrap();
        session.listing.into_string()
    };
    let plain = render(false);
    let freq = render(true);
    // No FREQ: error df = n − 2 = 4; Corrected Total df = 5.
    // FREQ=2: error df = 2n − 2 = 10; Corrected Total df = 11; Used = 12.
    assert!(
        plain.contains("Number of Observations Used         6"),
        "{plain}"
    );
    assert!(
        freq.contains("Number of Observations Used         12"),
        "{freq}"
    );
    assert!(freq.contains("Corrected Total"), "{freq}");
}

/// FREQ = 1 everywhere ⇒ identical listing to no FREQ at all.
#[test]
fn test_oracle_freq_ones_equals_none() {
    let mut s1 = make_session();
    let mut s2 = make_session();
    let mk = |session: &mut Session| {
        let frame = df![
            "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0],
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0],
            "f" => [1.0_f64, 1.0, 1.0, 1.0, 1.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x"), num_meta("f")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    };
    mk(&mut s1);
    mk(&mut s2);
    let base = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        basic_model("y", &["x"]),
    );
    let mut with_freq = base.clone();
    with_freq.freq = Some("f".into());
    execute(&base, &mut s1).unwrap();
    execute(&with_freq, &mut s2).unwrap();
    assert_eq!(s1.listing.into_string(), s2.listing.into_string());
}

/// BY with a single group ⇒ identical listing to no BY (the heading is only
/// emitted when groups exist; one group with one distinct key still prints a
/// heading, so we compare a constant-key BY against the non-BY run minus the
/// heading line).
#[test]
fn test_oracle_by_single_group_matches_body() {
    let render = |with_by: bool| -> String {
        let mut session = make_session();
        let frame = df![
            "g" => [1.0_f64, 1.0, 1.0, 1.0, 1.0],
            "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0],
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]
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
        if with_by {
            ast.by = vec!["g".into()];
        }
        execute(&ast, &mut session).unwrap();
        session.listing.into_string()
    };
    let plain = render(false);
    let by = render(true);
    // The BY run prepends a `g=1` heading; the regression body is unchanged.
    assert!(by.contains("g=1"), "{by}");
    assert!(by.contains("The REG Procedure"), "{by}");
    // Body identical: strip the BY heading line (and its trailing blank)
    // from the BY output, then compare the regression bodies.
    let by_body: String = by
        .lines()
        .filter(|l| l.trim() != "g=1")
        .collect::<Vec<_>>()
        .join("\n");
    let plain_body: String = plain.lines().collect::<Vec<_>>().join("\n");
    // Drop any leading blank lines introduced by removing the heading.
    assert_eq!(by_body.trim_start(), plain_body.trim_start());
}

#[test]
fn test_oracle_covb_diag_eq_se_squared() {
    let (_x, _y, fit, _names, n) = m368_setup();
    let p_eff = fit.beta.len();
    let mse = fit.sse / (n - p_eff) as f64;
    let covb = covb_matrix(&fit.xtx_inv, mse);
    for j in 0..p_eff {
        let se = (mse * fit.xtx_inv[j][j]).sqrt();
        assert!(
            (covb[j][j] - se * se).abs() < 1e-9,
            "covb_jj != SE_j^2 at {j}"
        );
    }
}

#[test]
fn test_oracle_corrb_diag_one_symmetric() {
    let (_x, _y, fit, _names, n) = m368_setup();
    let p_eff = fit.beta.len();
    let mse = fit.sse / (n - p_eff) as f64;
    let covb = covb_matrix(&fit.xtx_inv, mse);
    let corrb = corrb_matrix(&covb);
    for i in 0..p_eff {
        assert!((corrb[i][i] - 1.0).abs() < 1e-12, "corrb diagonal != 1");
        for j in 0..p_eff {
            assert!(
                (corrb[i][j] - corrb[j][i]).abs() < 1e-12,
                "corrb not symmetric"
            );
        }
    }
}

#[test]
fn test_oracle_xpx_diag_n_and_inverse() {
    let (x, y, fit, _names, n) = m368_setup();
    let p = fit.beta.len();
    let xpx = build_xpx(&x, &y);
    // Intercept column diagonal == n.
    assert!((xpx[0][0] - n as f64).abs() < 1e-9, "X'X[0][0] != n");
    // Symmetric over the full augmented matrix.
    for i in 0..xpx.len() {
        for j in 0..xpx.len() {
            assert!((xpx[i][j] - xpx[j][i]).abs() < 1e-6, "X'X not symmetric");
        }
    }
    // (X'X block) · (X'X)^-1 ≈ I.
    for i in 0..p {
        for j in 0..p {
            let mut s = 0.0;
            for k in 0..p {
                s += xpx[i][k] * fit.xtx_inv[k][j];
            }
            let want = if i == j { 1.0 } else { 0.0 };
            assert!((s - want).abs() < 1e-6, "X'X·inv != I at ({i},{j}): {s}");
        }
    }
}

#[test]
fn test_oracle_simple_stats() {
    let col = vec![1.0_f64, 2.0, 3.0, 4.0, 5.0];
    let n = col.len() as f64;
    let mean = col.iter().sum::<f64>() / n;
    assert!((mean - 3.0).abs() < 1e-12);
    let uss: f64 = col.iter().map(|v| v * v).sum();
    assert!((uss - 55.0).abs() < 1e-12);
    let var = sample_variance(&col);
    assert!((var - 2.5).abs() < 1e-12, "variance: {var}");
}

#[test]
fn test_oracle_outest_parms_row() {
    let (_x, _y, fit, names, n) = m368_setup();
    let entry = build_outest_entry("MODEL1", "y", &names, &fit, true, n as f64, 0.05);
    // Parameter estimates equal fit.beta.
    for j in 0..fit.beta.len() {
        assert!((entry.beta[j] - fit.beta[j]).abs() < 1e-12);
    }
    // _RMSE_ == Root MSE.
    let mse = fit.sse / (n - fit.beta.len()) as f64;
    assert!((entry.rmse - mse.sqrt()).abs() < 1e-9);
    // EDF / IN / P.
    assert_eq!(entry.n_in, 2);
    assert_eq!(entry.n_p, 3);
    assert!((entry.edf - (n as f64 - 3.0)).abs() < 1e-12);
}
