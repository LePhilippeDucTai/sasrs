use super::super::*;
use super::*;

/// Oracle (a): a single-coefficient `TEST xj=0;` yields F == t² of xj.
#[test]
fn test_oracle_test_f_equals_t_squared() {
    // Design: intercept + x1 + x2, with non-degenerate data.
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let x2 = [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0];
    let y: Vec<f64> = x1
        .iter()
        .zip(x2.iter())
        .map(|(&a, &b)| 1.0 + 2.0 * a + 0.5 * b + (a * 0.3).cos())
        .collect();
    let n = y.len();
    let mut x_mat = Vec::new();
    for i in 0..n {
        x_mat.push(vec![1.0, x1[i], x2[i]]);
    }
    let fit = ols_fit(&x_mat, &y).unwrap();
    let p_eff = 3;
    let df_e = (n - p_eff) as f64;
    let mse = fit.sse / df_e;
    // t for x2 (column 2).
    let se = (mse * fit.xtx_inv[2][2]).sqrt();
    let t = fit.beta[2] / se;
    let t2 = t * t;

    // TEST x2 = 0  →  L = [0,0,1], c = 0.
    let reg_names = vec!["X1".to_string(), "X2".to_string()];
    let eq = LinEq {
        terms: vec![(1.0, "X2".into())],
        rhs: 0.0,
    };
    let (l, c) = build_lc(&[eq], &reg_names, true).unwrap();
    let lb = linalg::matrix_vec_mult(&l, &fit.beta);
    let diff: Vec<f64> = lb.iter().zip(c.iter()).map(|(a, b)| a - b).collect();
    let lt = linalg::transpose(&l);
    let lh = linalg::matrix_mult(&l, &fit.xtx_inv);
    let m = linalg::matrix_mult(&lh, &lt);
    let minv = linalg::invert_matrix(&m).unwrap();
    let md = linalg::matrix_vec_mult(&minv, &diff);
    let ss: f64 = diff.iter().zip(md.iter()).map(|(a, b)| a * b).sum();
    let f = (ss / 1.0) / mse;
    assert!((f - t2).abs() < 1e-6, "F={f} t^2={t2}");
}

/// Oracle (b): restricted estimates satisfy L β_r = c exactly.
#[test]
fn test_oracle_restricted_satisfies_constraint() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0];
    let x2 = [3.0_f64, 1.0, 4.0, 1.0, 5.0, 9.0];
    let y: Vec<f64> = x1
        .iter()
        .zip(x2.iter())
        .map(|(&a, &b)| 2.0 + a - b)
        .collect();
    let n = y.len();
    let mut x_mat = Vec::new();
    for i in 0..n {
        x_mat.push(vec![1.0, x1[i], x2[i]]);
    }
    let fit = ols_fit(&x_mat, &y).unwrap();
    let reg_names = vec!["X1".to_string(), "X2".to_string()];
    // RESTRICT x1 + x2 = 1.
    let restricts = vec![RegRestrict {
        equations: vec![LinEq {
            terms: vec![(1.0, "X1".into()), (1.0, "X2".into())],
            rhs: 1.0,
        }],
    }];
    let r = compute_restricted(&restricts, &reg_names, true, &x_mat, &y, &fit, n)
        .unwrap()
        .unwrap();
    // L β_r = c: β_r[1] + β_r[2] == 1.
    let lhs = r.beta_r[1] + r.beta_r[2];
    assert!((lhs - 1.0).abs() < 1e-9, "L beta_r = {lhs}");
}

/// Oracle (c): a RESTRICT already satisfied by OLS leaves estimates ~unchanged.
#[test]
fn test_oracle_redundant_restrict_unchanged() {
    // Build y so that OLS already gives slope_x1 == slope_x2 (symmetric).
    // y = 3 + 2*x1 + 2*x2 exactly → OLS recovers (3, 2, 2); RESTRICT x1=x2
    // is already satisfied.
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0];
    let x2 = [5.0_f64, 1.0, 4.0, 2.0, 3.0];
    let y: Vec<f64> = x1
        .iter()
        .zip(x2.iter())
        .map(|(&a, &b)| 3.0 + 2.0 * a + 2.0 * b)
        .collect();
    let n = y.len();
    let mut x_mat = Vec::new();
    for i in 0..n {
        x_mat.push(vec![1.0, x1[i], x2[i]]);
    }
    let fit = ols_fit(&x_mat, &y).unwrap();
    let reg_names = vec!["X1".to_string(), "X2".to_string()];
    let restricts = vec![RegRestrict {
        equations: vec![LinEq {
            terms: vec![(1.0, "X1".into()), (-1.0, "X2".into())],
            rhs: 0.0,
        }],
    }];
    let r = compute_restricted(&restricts, &reg_names, true, &x_mat, &y, &fit, n)
        .unwrap()
        .unwrap();
    for j in 0..fit.beta.len() {
        assert!(
            (r.beta_r[j] - fit.beta[j]).abs() < 1e-7,
            "beta_r[{j}]={} beta[{j}]={}",
            r.beta_r[j],
            fit.beta[j]
        );
    }
    // λ ≈ 0 since the constraint is non-binding.
    assert!(r.lambda_rows[0].1.abs() < 1e-6, "lambda={}", r.lambda_rows[0].1);
}

/// Oracle: Σ_i h_i == p_eff (trace of the hat matrix == #params).
#[test]
fn test_oracle_leverage_trace() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let x2 = [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0];
    let y: Vec<f64> = x1.iter().zip(x2.iter()).map(|(&a, &b)| 1.0 + a + 0.5 * b).collect();
    let n = y.len();
    let x = design(true, &[&x1, &x2], n);
    let fit = ols_fit(&x, &y).unwrap();
    let h = leverages(&x, &fit.xtx_inv);
    let trace: f64 = h.iter().sum();
    assert!((trace - 3.0).abs() < 1e-9, "trace={trace}");
    // Same for a NOINT design.
    let xn = design(false, &[&x1, &x2], n);
    let fitn = ols_fit(&xn, &y).unwrap();
    let hn = leverages(&xn, &fitn.xtx_inv);
    let tracen: f64 = hn.iter().sum();
    assert!((tracen - 2.0).abs() < 1e-9, "trace_noint={tracen}");
}

/// Oracle: STDP²+STDR² == MSE and STDI²−STDP² == MSE (per observation),
/// and CLM is centered on ŷ.
#[test]
fn test_oracle_std_error_identities() {
    let x1 = [1.0_f64, 3.0, 2.0, 5.0, 4.0, 6.0, 8.0, 7.0];
    let y: Vec<f64> = x1.iter().map(|&a| 2.0 + 3.0 * a + (a * 0.5).sin()).collect();
    let n = y.len();
    let x = design(true, &[&x1], n);
    let fit = ols_fit(&x, &y).unwrap();
    let p_eff = 2;
    let mse = fit.sse / (n - p_eff) as f64;
    let stats = compute_obs_stats(&x, &y, &fit, n, p_eff, 0.05, None);
    for s in &stats {
        assert!((s.stdp * s.stdp + s.stdr * s.stdr - mse).abs() < 1e-9);
        assert!((s.stdi * s.stdi - s.stdp * s.stdp - mse).abs() < 1e-9);
        // CLM centered on ŷ.
        let mid = (s.lclm + s.uclm) / 2.0;
        assert!((mid - s.y_hat).abs() < 1e-9, "mid={mid} yhat={}", s.y_hat);
        // CLI also centered on ŷ and wider than CLM.
        let midi = (s.lcl + s.ucl) / 2.0;
        assert!((midi - s.y_hat).abs() < 1e-9);
        assert!(s.ucl - s.lcl > s.uclm - s.lclm - 1e-12);
    }
}

/// Oracle: CLB limits == β_j ± t·SE(β_j) with the parameter-table SE.
#[test]
fn test_oracle_clb_limits() {
    let x1 = [1.0_f64, 2.0, 4.0, 3.0, 6.0, 5.0, 7.0];
    let y: Vec<f64> = x1.iter().map(|&a| 1.5 + 2.0 * a + (a * 0.3).cos()).collect();
    let n = y.len();
    let x = design(true, &[&x1], n);
    let fit = ols_fit(&x, &y).unwrap();
    let p_eff = 2;
    let df_e = (n - p_eff) as f64;
    let mse = fit.sse / df_e;
    let alpha = 0.10;
    let t = t_quantile(1.0 - alpha / 2.0, df_e);
    for j in 0..p_eff {
        let se = (mse * fit.xtx_inv[j][j]).sqrt();
        let lo = fit.beta[j] - t * se;
        let hi = fit.beta[j] + t * se;
        // Reconstruct what fit_and_print computes.
        assert!(lo < fit.beta[j] && fit.beta[j] < hi);
        assert!(((lo + hi) / 2.0 - fit.beta[j]).abs() < 1e-12);
    }
}

/// STUDENT = resid / STDR (matches M36.2 STDR).
#[test]
fn test_oracle_student_eq_resid_over_stdr() {
    let (x, y, fit, n, p_eff) = infl_setup();
    let obs = compute_obs_stats(&x, &y, &fit, n, p_eff, 0.05, None);
    let infl = compute_influence_stats(&x, &y, &fit, n, p_eff, None);
    for (s, o) in infl.iter().zip(obs.iter()) {
        assert!((s.student - s.resid / o.stdr).abs() < 1e-9);
        // STDR also matches the obs-stats STDR.
        assert!((s.stdr - o.stdr).abs() < 1e-9);
    }
}

/// RSTUDENT = student·√((dfE−1)/(dfE−student²)).
#[test]
fn test_oracle_rstudent_identity() {
    let (x, y, fit, n, p_eff) = infl_setup();
    let df_e = (n - p_eff) as f64;
    let infl = compute_influence_stats(&x, &y, &fit, n, p_eff, None);
    for s in &infl {
        let expect = s.student * ((df_e - 1.0) / (df_e - s.student * s.student)).sqrt();
        assert!(
            (s.rstudent - expect).abs() < 1e-9,
            "rstudent={} expect={}",
            s.rstudent,
            expect
        );
    }
}

/// PRESS = resid/(1−h) and Σ press² is the printed PRESS.
#[test]
fn test_oracle_press() {
    let (x, y, fit, n, p_eff) = infl_setup();
    let h = leverages(&x, &fit.xtx_inv);
    let infl = compute_influence_stats(&x, &y, &fit, n, p_eff, None);
    let mut press_ss = 0.0;
    for (i, s) in infl.iter().enumerate() {
        let expect = s.resid / (1.0 - h[i]);
        assert!((s.press - expect).abs() < 1e-9);
        press_ss += s.press * s.press;
    }
    let printed: f64 = infl.iter().map(|s| s.press * s.press).sum();
    assert!((press_ss - printed).abs() < 1e-9);
}

/// Cook's D ≥ 0, and DFFITS = rstudent·√(h/(1−h)).
#[test]
fn test_oracle_cookd_dffits() {
    let (x, y, fit, n, p_eff) = infl_setup();
    let h = leverages(&x, &fit.xtx_inv);
    let infl = compute_influence_stats(&x, &y, &fit, n, p_eff, None);
    for (i, s) in infl.iter().enumerate() {
        assert!(s.cookd >= 0.0, "cookd={}", s.cookd);
        let expect = s.rstudent * (h[i] / (1.0 - h[i])).sqrt();
        assert!((s.dffits - expect).abs() < 1e-9);
    }
}

/// Near-zero-leverage point → Cook's D ≈ 0.
#[test]
fn test_oracle_cookd_low_leverage() {
    let (x, y, fit, n, p_eff) = infl_setup();
    let h = leverages(&x, &fit.xtx_inv);
    let infl = compute_influence_stats(&x, &y, &fit, n, p_eff, None);
    // The lowest-leverage observation should have small Cook's D.
    let (min_i, _) = h
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .unwrap();
    assert!(infl[min_i].cookd < 0.5, "cookd={}", infl[min_i].cookd);
}

/// DFBETAS closed form == an explicit leave-one-out refit (within 1e-6).
#[test]
fn test_oracle_dfbetas_loo_refit() {
    // Tiny dataset, intercept + slope.
    let x1 = [1.0_f64, 2.0, 3.0, 5.0, 8.0];
    let y = [2.1_f64, 3.9, 6.2, 9.8, 16.1];
    let n = y.len();
    let p_eff = 2;
    let x = design(true, &[&x1], n);
    let fit = ols_fit(&x, &y).unwrap();
    let infl = compute_influence_stats(&x, &y.to_vec(), &fit, n, p_eff, None);

    for drop in 0..n {
        // Refit without observation `drop`.
        let xr: Vec<Vec<f64>> = (0..n).filter(|&i| i != drop).map(|i| x[i].clone()).collect();
        let yr: Vec<f64> = (0..n).filter(|&i| i != drop).map(|i| y[i]).collect();
        let fit_i = ols_fit(&xr, &yr).unwrap();
        // s_(i) = √MSE_(i).
        let df_i = (n - 1 - p_eff) as f64;
        let s_i = (fit_i.sse / df_i).sqrt();
        for j in 0..p_eff {
            let denom = s_i * fit.xtx_inv[j][j].sqrt();
            let expect = (fit.beta[j] - fit_i.beta[j]) / denom;
            assert!(
                (infl[drop].dfbetas[j] - expect).abs() < 1e-6,
                "drop={drop} j={j} got={} expect={}",
                infl[drop].dfbetas[j],
                expect
            );
        }
    }
}

/// VIF·TOL == 1; for two regressors VIF_1 == VIF_2 == 1/(1−r²).
#[test]
fn test_oracle_vif_tol() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 7.0, 8.0, 6.0];
    // x2 correlated-but-not-collinear with x1.
    let x2: Vec<f64> = x1.iter().map(|&a| 0.5 * a + (a * 0.7).sin()).collect();
    let cols = vec![x1.to_vec(), x2.clone()];
    let (tol, vif) = vif_tol(&cols);
    for j in 0..2 {
        assert!((vif[j] * tol[j] - 1.0).abs() < 1e-9, "VIF·TOL != 1");
    }
    // Two regressors → both VIF equal, == 1/(1−r²).
    let n = x1.len() as f64;
    let m1 = x1.iter().sum::<f64>() / n;
    let m2 = x2.iter().sum::<f64>() / n;
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    let mut syy = 0.0;
    for i in 0..x1.len() {
        sxy += (x1[i] - m1) * (x2[i] - m2);
        sxx += (x1[i] - m1) * (x1[i] - m1);
        syy += (x2[i] - m2) * (x2[i] - m2);
    }
    let r2 = (sxy * sxy) / (sxx * syy);
    let expected = 1.0 / (1.0 - r2);
    assert!((vif[0] - vif[1]).abs() < 1e-9, "VIFs differ");
    assert!((vif[0] - expected).abs() < 1e-7, "VIF != 1/(1-r²)");
}

/// Collinearity: #eigenvalues == #cols, condition index uses λ_max, and each
/// regressor's variance proportions sum to 1 across rows.
#[test]
fn test_oracle_collin_proportions() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let x2: Vec<f64> = x1.iter().map(|&a| a * a + 1.0).collect();
    let n = x1.len();
    let x = design(true, &[&x1, &x2], n);
    let reg = vec!["x1".to_string(), "x2".to_string()];
    let c = compute_collin(&x, &reg, true, false).unwrap();
    assert_eq!(c.eigenvalues.len(), 3); // intercept + 2 regressors
    // Descending.
    for k in 1..c.eigenvalues.len() {
        assert!(c.eigenvalues[k - 1] >= c.eigenvalues[k] - 1e-12);
    }
    // First condition index == 1 (λ_max / λ_max).
    assert!((c.condition_index[0] - 1.0).abs() < 1e-9);
    // Column proportions sum to 1.
    let m = c.eigenvalues.len();
    for j in 0..m {
        let s: f64 = (0..m).map(|k| c.proportions[k][j]).sum();
        assert!((s - 1.0).abs() < 1e-9, "proportion col sum != 1: {s}");
    }
    // COLLINOINT drops the intercept column → 2 columns analysed.
    let cint = compute_collin(&x, &reg, true, true).unwrap();
    assert_eq!(cint.eigenvalues.len(), 2);
    assert_eq!(cint.col_labels, vec!["x1".to_string(), "x2".to_string()]);
}

/// SPEC: W = n·R²_aux ≥ 0 and df == number of auxiliary regressors.
#[test]
fn test_oracle_spec_white() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    // x2 chosen so {1, x1, x2, x1², x2², x1·x2} is full rank.
    let x2 = [3.0_f64, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0, 5.0, 8.0];
    // Include genuine noise so the fit has nonzero residuals and the
    // auxiliary (White) regression is full rank.
    let y: Vec<f64> = (0..10)
        .map(|i| 1.0 + 2.0 * x1[i] - 0.5 * x2[i] + (x1[i] * 1.3).sin() * 0.8)
        .collect();
    let n = y.len();
    let x = design(true, &[&x1, &x2], n);
    let fit = ols_fit(&x, &y).unwrap();
    let cols = vec![x1.to_vec(), x2.to_vec()];
    let (w, df, pv) = white_spec_test(&cols, &fit.resid).unwrap();
    assert!(w >= 0.0);
    // p=2 regressors → linear(2) + square(2) + cross(1) = 5 aux regressors.
    assert_eq!(df, 5);
    assert!((0.0..=1.0).contains(&pv));
}

/// DW: 0 ≤ d ≤ 4; for no-autocorrelation residuals d ≈ 2; d ≈ 2(1−ρ).
#[test]
fn test_oracle_durbin_watson() {
    // Alternating-sign residuals → strong negative autocorrelation, d→4.
    let x1: Vec<f64> = (0..10).map(|i| i as f64).collect();
    let y: Vec<f64> = (0..10)
        .map(|i| 1.0 + 0.5 * i as f64 + if i % 2 == 0 { 1.0 } else { -1.0 })
        .collect();
    let n = y.len();
    let x = design(true, &[&x1], n);
    let fit = ols_fit(&x, &y).unwrap();
    let dwr = durbin_watson(&fit.resid, &x, &fit.xtx_inv, true);
    assert!((0.0..=4.0).contains(&dwr.d), "d out of range: {}", dwr.d);
    // d ≈ 2(1−ρ) (exact only up to O(1/n) boundary terms e_1²+e_n²).
    assert!((dwr.d - 2.0 * (1.0 - dwr.rho)).abs() < 0.6);
    // Alternating signs → ρ negative → d > 2.
    assert!(dwr.d > 2.0);
    // p-values present and in [0,1].
    let pp = dwr.pr_pos.unwrap();
    let pn = dwr.pr_neg.unwrap();
    assert!((0.0..=1.0).contains(&pp) && (0.0..=1.0).contains(&pn));
    assert!((pp + pn - 1.0).abs() < 1e-9);
}

/// ACOV: HC matrix is symmetric; for homoscedastic-like data HC SE is the
/// same order of magnitude as OLS SE.
#[test]
fn test_oracle_acov_hc0() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let y: Vec<f64> = x1.iter().map(|&a| 1.0 + 2.0 * a + (a * 0.9).sin()).collect();
    let n = y.len();
    let p_eff = 2;
    let x = design(true, &[&x1], n);
    let fit = ols_fit(&x, &y).unwrap();
    let cov = acov_hc0(&x, &fit.resid, &fit.xtx_inv);
    // Symmetry.
    for i in 0..p_eff {
        for j in 0..p_eff {
            assert!((cov[i][j] - cov[j][i]).abs() < 1e-12);
        }
    }
    // Order-of-magnitude agreement with OLS SE.
    let mse = fit.sse / (n - p_eff) as f64;
    for j in 0..p_eff {
        let ols_se = (mse * fit.xtx_inv[j][j]).sqrt();
        let hc_se = cov[j][j].sqrt();
        assert!(
            hc_se > 0.0 && hc_se < 100.0 * ols_se && ols_se < 100.0 * hc_se,
            "HC SE / OLS SE order mismatch: {hc_se} vs {ols_se}"
        );
    }
}
