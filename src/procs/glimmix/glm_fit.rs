use super::*;

// ───────────────────────── No-random GLM (IRLS) fit ─────────────────────────

/// Result of the no-random fixed-effects fit.
pub(super) struct GlmFit {
    pub(super) beta: Vec<f64>,
    /// Var(β̂) = scale * (X'WX)⁻¹.
    pub(super) cov_beta: Vec<Vec<f64>>,
    /// Fitted means μ_i.
    pub(super) mu: Vec<f64>,
    pub(super) iterations: usize,
}

/// IRLS for the fixed-effects-only GLM (mirrors PROC GENMOD), with FREQ
/// weighting. For NORMAL, scale = MSE; for Poisson/Binary, scale = 1.
pub(super) fn fit_glm(
    y: &[f64],
    x: &[Vec<f64>],
    freq: &[f64],
    dist: Distribution,
    lf: LinkFunction,
) -> Result<GlmFit> {
    let n = y.len();
    let p = x[0].len();
    let n_total: f64 = freq.iter().sum();

    // Initialise β via η0 on the (weighted) mean response.
    let y_mean: f64 = y.iter().zip(freq).map(|(yi, w)| yi * w).sum::<f64>() / n_total;
    let eta0 = match lf {
        LinkFunction::Log => y_mean.max(1e-10).ln(),
        LinkFunction::Logit => {
            let pp = y_mean.clamp(1e-10, 1.0 - 1e-10);
            (pp / (1.0 - pp)).ln().clamp(-10.0, 10.0)
        }
        _ => y_mean,
    };
    let mut beta = vec![0.0; p];
    beta[0] = eta0;

    let mut iterations = 0;
    let mut converged = false;
    for it in 0..50 {
        iterations = it + 1;
        let mut score = vec![0.0; p];
        let mut hess = vec![vec![0.0; p]; p];
        for i in 0..n {
            let eta: f64 = dot(&x[i], &beta);
            let mu = inv_link(eta, lf);
            let v = variance(mu, dist);
            let d = dmu_deta(eta, lf);
            let w = freq[i] * d * d / v;
            let resid_adj = freq[i] * (y[i] - mu) * d / v;
            for j in 0..p {
                score[j] += x[i][j] * resid_adj;
                for k in 0..p {
                    hess[j][k] += x[i][j] * x[i][k] * w;
                }
            }
        }
        let hinv = invert_matrix(&hess)?;
        let delta = mat_vec(&hinv, &score);
        for j in 0..p {
            beta[j] += delta[j];
        }
        let max_delta = delta.iter().map(|d| d.abs()).fold(0.0_f64, f64::max);
        let max_beta = beta.iter().map(|b| b.abs()).fold(0.0_f64, f64::max);
        if max_delta / (1.0 + max_beta) < 1e-10 {
            converged = true;
            break;
        }
    }
    if !converged {
        return Err(SasError::runtime("PROC GLIMMIX failed to converge."));
    }

    // Final H = X'WX and μ at convergence.
    let mut hess = vec![vec![0.0; p]; p];
    let mut mu = Vec::with_capacity(n);
    for i in 0..n {
        let eta: f64 = dot(&x[i], &beta);
        let m = inv_link(eta, lf);
        let v = variance(m, dist);
        let d = dmu_deta(eta, lf);
        let w = freq[i] * d * d / v;
        mu.push(m);
        for j in 0..p {
            for k in 0..p {
                hess[j][k] += x[i][j] * x[i][k] * w;
            }
        }
    }
    let hinv = invert_matrix(&hess)?;

    // Scale: Normal → MSE; others → 1 (the oracle demands GENMOD scale-1 SEs).
    let scale = if dist == Distribution::Normal {
        let sse: f64 = (0..n).map(|i| freq[i] * (y[i] - mu[i]).powi(2)).sum();
        let dfe = (n_total - p as f64).max(1.0);
        sse / dfe
    } else {
        1.0
    };
    let cov_beta: Vec<Vec<f64>> = hinv
        .iter()
        .map(|row| row.iter().map(|v| scale * v).collect())
        .collect();

    Ok(GlmFit {
        beta,
        cov_beta,
        mu,
        iterations,
    })
}

// ───────────────────────── Variance-components mixed fit ─────────────────────

/// Fit y = Xβ + Zu + ε with V = σ²_u ZZ' + σ²_e I (single random intercept).
/// Returns (σ²_u, σ²_e, β, Var(β), -2 Res LogLik). Used for NORMAL/IDENTITY
/// (closed-form REML) and as the WMME solver inside the PQL loop (working data).
pub(super) fn fit_vc(
    y: &[f64],
    x: &[Vec<f64>],
    subj_of: &[usize],
    n_subjects: usize,
    weights: Option<&[f64]>,
) -> Result<(f64, f64, Vec<f64>, Vec<Vec<f64>>, f64)> {
    let p = x[0].len();

    // Balance detection for the closed-form intercept-only path.
    let mut counts = vec![0usize; n_subjects];
    for &s in subj_of {
        counts[s] += 1;
    }
    let n_i = counts[0];
    let balanced = counts.iter().all(|&c| c == n_i) && n_i > 0;
    let intercept_only = p == 1 && x.iter().all(|row| row[0] == 1.0);
    let unweighted = weights.is_none();

    let (mut sigma2_u, sigma2_e) =
        if unweighted && balanced && intercept_only && n_subjects >= 2 {
            closed_form_vc(y, subj_of, n_subjects, n_i)
        } else {
            profile_search(y, x, subj_of, weights)?
        };
    if sigma2_u < 0.0 {
        sigma2_u = 0.0;
    }

    let (neg2, beta, cov) = neg2_reml(y, x, subj_of, sigma2_u, sigma2_e, weights)?;
    Ok((sigma2_u, sigma2_e, beta, cov, neg2))
}

/// Closed-form REML variance components, balanced one-way random intercept.
pub(super) fn closed_form_vc(y: &[f64], subj_of: &[usize], n_subjects: usize, n_i: usize) -> (f64, f64) {
    let a = n_subjects;
    let n_total = y.len();
    let mut group_sum = vec![0.0; a];
    for (i, &yi) in y.iter().enumerate() {
        group_sum[subj_of[i]] += yi;
    }
    let group_mean: Vec<f64> = group_sum.iter().map(|s| s / n_i as f64).collect();
    let grand_mean = y.iter().sum::<f64>() / n_total as f64;
    let ss_between: f64 =
        group_mean.iter().map(|m| (m - grand_mean).powi(2)).sum::<f64>() * n_i as f64;
    let ss_within: f64 = y
        .iter()
        .enumerate()
        .map(|(i, &yi)| (yi - group_mean[subj_of[i]]).powi(2))
        .sum();
    let ms_between = ss_between / (a as f64 - 1.0);
    let ms_within = ss_within / (n_total as f64 - a as f64);
    let sigma2_e = ms_within;
    let sigma2_u = (ms_between - ms_within) / n_i as f64;
    (sigma2_u, sigma2_e)
}

/// Build V = σ²_u ZZ' + σ²_e diag(1/w_i). When weights are given, the residual
/// variance is σ²_e scaled by 1/w_i (working-variate pseudo-likelihood).
pub(super) fn build_v(
    n: usize,
    subj_of: &[usize],
    sigma2_u: f64,
    sigma2_e: f64,
    weights: Option<&[f64]>,
) -> Vec<Vec<f64>> {
    let mut v = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..n {
            let mut val = 0.0;
            if subj_of[i] == subj_of[j] {
                val += sigma2_u;
            }
            if i == j {
                let wi = weights.map(|w| w[i]).unwrap_or(1.0).max(1e-12);
                val += sigma2_e / wi;
            }
            v[i][j] = val;
        }
    }
    v
}

/// -2 Res Log Likelihood for given variances, plus β̂ and Var(β̂).
pub(super) fn neg2_reml(
    y: &[f64],
    x: &[Vec<f64>],
    subj_of: &[usize],
    sigma2_u: f64,
    sigma2_e: f64,
    weights: Option<&[f64]>,
) -> Result<(f64, Vec<f64>, Vec<Vec<f64>>)> {
    let n = y.len();
    let p = x[0].len();
    let v = build_v(n, subj_of, sigma2_u, sigma2_e, weights);
    let v_inv = invert_matrix(&v)?;
    let log_det_v = log_det_spd(&v)?;

    let mut xtvi = vec![vec![0.0; n]; p];
    for a in 0..p {
        for j in 0..n {
            let mut s = 0.0;
            for i in 0..n {
                s += x[i][a] * v_inv[i][j];
            }
            xtvi[a][j] = s;
        }
    }
    let mut xtvix = vec![vec![0.0; p]; p];
    for a in 0..p {
        for b in 0..p {
            let mut s = 0.0;
            for j in 0..n {
                s += xtvi[a][j] * x[j][b];
            }
            xtvix[a][b] = s;
        }
    }
    let xtvix_inv = invert_matrix(&xtvix)?;
    let log_det_xtvix = log_det_spd(&xtvix)?;
    let xtviy: Vec<f64> = (0..p).map(|a| dot(&xtvi[a], y)).collect();
    let beta = mat_vec(&xtvix_inv, &xtviy);
    let resid: Vec<f64> = (0..n)
        .map(|i| y[i] - (0..p).map(|a| x[i][a] * beta[a]).sum::<f64>())
        .collect();
    let vir = mat_vec(&v_inv, &resid);
    let quad = dot(&resid, &vir);

    let two_pi = std::f64::consts::TAU;
    let neg2 = (n as f64 - p as f64) * two_pi.ln() + log_det_v + log_det_xtvix + quad;
    Ok((neg2, beta, xtvix_inv))
}

/// Golden-section profile over λ = σ²_u/σ²_e for the general / weighted case.
pub(super) fn profile_search(
    y: &[f64],
    x: &[Vec<f64>],
    subj_of: &[usize],
    weights: Option<&[f64]>,
) -> Result<(f64, f64)> {
    let eval = |lambda: f64| -> Result<f64> {
        let n = y.len();
        let p = x[0].len();
        // V0 = λ ZZ' + diag(1/w_i)
        let mut v0 = vec![vec![0.0; n]; n];
        for i in 0..n {
            for j in 0..n {
                let mut val = 0.0;
                if subj_of[i] == subj_of[j] {
                    val += lambda;
                }
                if i == j {
                    let wi = weights.map(|w| w[i]).unwrap_or(1.0).max(1e-12);
                    val += 1.0 / wi;
                }
                v0[i][j] = val;
            }
        }
        let v0_inv = invert_matrix(&v0)?;
        let log_det_v0 = log_det_spd(&v0)?;
        let mut xtvi = vec![vec![0.0; n]; p];
        for a in 0..p {
            for j in 0..n {
                let mut s = 0.0;
                for i in 0..n {
                    s += x[i][a] * v0_inv[i][j];
                }
                xtvi[a][j] = s;
            }
        }
        let mut xtvix = vec![vec![0.0; p]; p];
        for a in 0..p {
            for b in 0..p {
                let mut s = 0.0;
                for j in 0..n {
                    s += xtvi[a][j] * x[j][b];
                }
                xtvix[a][b] = s;
            }
        }
        let xtvix_inv = invert_matrix(&xtvix)?;
        let log_det_xtvix = log_det_spd(&xtvix)?;
        let xtviy: Vec<f64> = (0..p).map(|a| dot(&xtvi[a], y)).collect();
        let beta = mat_vec(&xtvix_inv, &xtviy);
        let resid: Vec<f64> = (0..n)
            .map(|i| y[i] - (0..p).map(|a| x[i][a] * beta[a]).sum::<f64>())
            .collect();
        let vir = mat_vec(&v0_inv, &resid);
        let quad = dot(&resid, &vir);
        let dof = (n - p) as f64;
        let sigma2_e = quad / dof;
        let two_pi = std::f64::consts::TAU;
        let neg2 =
            (n as f64 - p as f64) * (two_pi.ln() + sigma2_e.ln()) + log_det_v0 + log_det_xtvix + dof;
        Ok(neg2)
    };
    // σ²_e for a given λ.
    let sigma2_e_of = |lambda: f64| -> Result<f64> {
        let n = y.len();
        let p = x[0].len();
        let mut v0 = vec![vec![0.0; n]; n];
        for i in 0..n {
            for j in 0..n {
                let mut val = 0.0;
                if subj_of[i] == subj_of[j] {
                    val += lambda;
                }
                if i == j {
                    let wi = weights.map(|w| w[i]).unwrap_or(1.0).max(1e-12);
                    val += 1.0 / wi;
                }
                v0[i][j] = val;
            }
        }
        let v0_inv = invert_matrix(&v0)?;
        let mut xtvi = vec![vec![0.0; n]; p];
        for a in 0..p {
            for j in 0..n {
                let mut s = 0.0;
                for i in 0..n {
                    s += x[i][a] * v0_inv[i][j];
                }
                xtvi[a][j] = s;
            }
        }
        let mut xtvix = vec![vec![0.0; p]; p];
        for a in 0..p {
            for b in 0..p {
                let mut s = 0.0;
                for j in 0..n {
                    s += xtvi[a][j] * x[j][b];
                }
                xtvix[a][b] = s;
            }
        }
        let xtvix_inv = invert_matrix(&xtvix)?;
        let xtviy: Vec<f64> = (0..p).map(|a| dot(&xtvi[a], y)).collect();
        let beta = mat_vec(&xtvix_inv, &xtviy);
        let resid: Vec<f64> = (0..n)
            .map(|i| y[i] - (0..p).map(|a| x[i][a] * beta[a]).sum::<f64>())
            .collect();
        let vir = mat_vec(&v0_inv, &resid);
        let quad = dot(&resid, &vir);
        Ok(quad / (n - p) as f64)
    };

    let lambda_max = 1000.0_f64;
    let gr = (5.0_f64.sqrt() - 1.0) / 2.0;
    let mut lo = 0.0;
    let mut hi = lambda_max;
    let mut c = hi - gr * (hi - lo);
    let mut d = lo + gr * (hi - lo);
    let mut fc = eval(c)?;
    let mut fd = eval(d)?;
    for _ in 0..200 {
        if (hi - lo).abs() < 1e-10 {
            break;
        }
        if fc < fd {
            hi = d;
            d = c;
            fd = fc;
            c = hi - gr * (hi - lo);
            fc = eval(c)?;
        } else {
            lo = c;
            c = d;
            fc = fd;
            d = lo + gr * (hi - lo);
            fd = eval(d)?;
        }
    }
    let lambda = 0.5 * (lo + hi);
    let f_opt = eval(lambda)?;
    let f0 = eval(0.0)?;
    if f0 <= f_opt {
        return Ok((0.0, sigma2_e_of(0.0)?));
    }
    let sigma2_e = sigma2_e_of(lambda)?;
    Ok((lambda * sigma2_e, sigma2_e))
}
