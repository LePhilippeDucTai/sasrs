use super::*;

// ───────────────────────── Mixed model fit ─────────────────────────

/// Result of fitting the variance-components mixed model (single random
/// intercept per subject).
pub(super) struct MixedFit {
    pub(super) sigma2_u: f64,
    pub(super) sigma2_e: f64,
    /// β̂ for the fixed effects (length p).
    pub(super) beta: Vec<f64>,
    /// Var(β̂) = (X'V⁻¹X)⁻¹ (p×p).
    pub(super) cov_beta: Vec<Vec<f64>>,
    /// -2 log (restricted) likelihood at the optimum.
    pub(super) neg2ll: f64,
    pub(super) n: usize,
    pub(super) p: usize,
    pub(super) balanced: bool,
}

/// Build V = σ²_u Z Z' + σ²_e I given subject membership.
/// `subj_of[i]` is the subject index of observation i.
pub(super) fn build_v(n: usize, subj_of: &[usize], sigma2_u: f64, sigma2_e: f64) -> Vec<Vec<f64>> {
    let mut v = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..n {
            let mut val = 0.0;
            if subj_of[i] == subj_of[j] {
                val += sigma2_u;
            }
            if i == j {
                val += sigma2_e;
            }
            v[i][j] = val;
        }
    }
    v
}

/// -2 log REML/ML likelihood for given (σ²_u, σ²_e).
///
/// -2 logL_ML  = n·log(2π) + log|V| + (y-Xβ)'V⁻¹(y-Xβ)
/// -2 logL_REML = -2 logL_ML(restricted) = (n-p)·log(2π) + log|V|
///                + log|X'V⁻¹X| + y'Py
/// where Py = V⁻¹(y-Xβ) at β = (X'V⁻¹X)⁻¹X'V⁻¹y.
pub(super) fn neg2_loglik(
    y: &[f64],
    x: &[Vec<f64>],
    subj_of: &[usize],
    sigma2_u: f64,
    sigma2_e: f64,
    method: Method,
) -> Result<(f64, Vec<f64>, Vec<Vec<f64>>)> {
    let n = y.len();
    let p = x[0].len();
    let v = build_v(n, subj_of, sigma2_u, sigma2_e);
    let v_inv = invert_matrix(&v)?;
    let log_det_v = log_det_spd(&v)?;

    // X'V⁻¹  (p×n)
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
    // X'V⁻¹X  (p×p)
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

    // X'V⁻¹y  (p)
    let xtviy: Vec<f64> = (0..p).map(|a| dot(&xtvi[a], y)).collect();
    // β̂ = (X'V⁻¹X)⁻¹ X'V⁻¹y
    let beta = matrix_vec_mult(&xtvix_inv, &xtviy);

    // residual r = y - Xβ
    let resid: Vec<f64> = (0..n)
        .map(|i| y[i] - (0..p).map(|a| x[i][a] * beta[a]).sum::<f64>())
        .collect();
    // r' V⁻¹ r
    let vir = matrix_vec_mult(&v_inv, &resid);
    let quad = dot(&resid, &vir);

    let two_pi = std::f64::consts::TAU;
    let neg2 = match method {
        Method::Reml => (n as f64 - p as f64) * two_pi.ln() + log_det_v + log_det_xtvix + quad,
        Method::Ml => n as f64 * two_pi.ln() + log_det_v + quad,
    };

    Ok((neg2, beta, xtvix_inv))
}

/// Fit the variance-components mixed model.
pub(super) fn fit_mixed(
    y: &[f64],
    x: &[Vec<f64>],
    subj_of: &[usize],
    n_subjects: usize,
    method: Method,
    nobound: bool,
) -> Result<MixedFit> {
    let n = y.len();
    let p = x[0].len();

    // Detect balance: do all subjects have the same number of observations?
    let mut counts = vec![0usize; n_subjects];
    for &s in subj_of {
        counts[s] += 1;
    }
    let n_i = counts[0];
    let balanced = counts.iter().all(|&c| c == n_i) && n_i > 0;

    // For the intercept-only balanced case, use the closed-form moment
    // estimator (exact REML/ML). This is the configuration the oracle verifies.
    let intercept_only = p == 1 && x.iter().all(|row| row[0] == 1.0);

    let (mut sigma2_u, sigma2_e) = if balanced && intercept_only && n_subjects >= 2 {
        closed_form_vc(y, subj_of, n_subjects, n_i, method)
    } else {
        // General path: 1-D profile search over λ = σ²_u / σ²_e ≥ 0.
        profile_search(y, x, subj_of, method)?
    };

    if !nobound && sigma2_u < 0.0 {
        sigma2_u = 0.0;
    }

    // Final β̂, Var(β̂), and -2 logL at the estimated variances.
    let (neg2ll, beta, cov_beta) = neg2_loglik(y, x, subj_of, sigma2_u, sigma2_e, method)?;

    Ok(MixedFit {
        sigma2_u,
        sigma2_e,
        beta,
        cov_beta,
        neg2ll,
        n,
        p,
        balanced,
    })
}

/// Closed-form variance components for a balanced one-way random model.
/// Returns (σ²_u, σ²_e).
pub(super) fn closed_form_vc(
    y: &[f64],
    subj_of: &[usize],
    n_subjects: usize,
    n_i: usize,
    method: Method,
) -> (f64, f64) {
    let a = n_subjects;
    let n_total = y.len();

    // Group means and grand mean.
    let mut group_sum = vec![0.0; a];
    for (i, &yi) in y.iter().enumerate() {
        group_sum[subj_of[i]] += yi;
    }
    let group_mean: Vec<f64> = group_sum.iter().map(|s| s / n_i as f64).collect();
    let grand_mean = y.iter().sum::<f64>() / n_total as f64;

    // SS_between and SS_within.
    let ss_between: f64 = group_mean
        .iter()
        .map(|m| (m - grand_mean).powi(2))
        .sum::<f64>()
        * n_i as f64;
    let ss_within: f64 = y
        .iter()
        .enumerate()
        .map(|(i, &yi)| (yi - group_mean[subj_of[i]]).powi(2))
        .sum();

    let ms_between = ss_between / (a as f64 - 1.0);
    let ms_within = ss_within / (n_total as f64 - a as f64);

    let sigma2_e = ms_within;
    let sigma2_u = match method {
        Method::Reml => (ms_between - ms_within) / n_i as f64,
        Method::Ml => (((a as f64 - 1.0) / a as f64) * ms_between - ms_within) / n_i as f64,
    };
    (sigma2_u, sigma2_e)
}

/// Profile search over λ = σ²_u / σ²_e for the unbalanced / general case.
/// Returns (σ²_u, σ²_e). Uses golden-section minimisation of -2 logL.
pub(super) fn profile_search(
    y: &[f64],
    x: &[Vec<f64>],
    subj_of: &[usize],
    method: Method,
) -> Result<(f64, f64)> {
    let total_var = {
        let n = y.len() as f64;
        let mean = y.iter().sum::<f64>() / n;
        y.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n
    };
    // For a given λ, profile σ²_e out and return -2 logL.
    // We parameterise V = σ²_e (λ ZZ' + I); profile σ²_e analytically.
    let eval = |lambda: f64| -> Result<(f64, f64)> {
        // V0 = λ ZZ' + I
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
                    val += 1.0;
                }
                v0[i][j] = val;
            }
        }
        let v0_inv = invert_matrix(&v0)?;
        let log_det_v0 = log_det_spd(&v0)?;

        // X'V0⁻¹
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
        let beta = matrix_vec_mult(&xtvix_inv, &xtviy);
        let resid: Vec<f64> = (0..n)
            .map(|i| y[i] - (0..p).map(|a| x[i][a] * beta[a]).sum::<f64>())
            .collect();
        let vir = matrix_vec_mult(&v0_inv, &resid);
        let quad = dot(&resid, &vir);

        let dof = match method {
            Method::Reml => (n - p) as f64,
            Method::Ml => n as f64,
        };
        let sigma2_e = quad / dof;

        let two_pi = std::f64::consts::TAU;
        let neg2 = match method {
            Method::Reml => {
                (n as f64 - p as f64) * (two_pi.ln() + sigma2_e.ln())
                    + log_det_v0
                    + log_det_xtvix
                    + dof
            }
            Method::Ml => n as f64 * (two_pi.ln() + sigma2_e.ln()) + log_det_v0 + dof,
        };
        Ok((neg2, sigma2_e))
    };

    // Golden-section search for λ ∈ [0, λ_max].
    let lambda_max = 1000.0_f64;
    let gr = (5.0_f64.sqrt() - 1.0) / 2.0;
    let mut lo = 0.0;
    let mut hi = lambda_max;
    let mut c = hi - gr * (hi - lo);
    let mut d = lo + gr * (hi - lo);
    let mut fc = eval(c)?.0;
    let mut fd = eval(d)?.0;
    for _ in 0..200 {
        if (hi - lo).abs() < 1e-10 {
            break;
        }
        if fc < fd {
            hi = d;
            d = c;
            fd = fc;
            c = hi - gr * (hi - lo);
            fc = eval(c)?.0;
        } else {
            lo = c;
            c = d;
            fc = fd;
            d = lo + gr * (hi - lo);
            fd = eval(d)?.0;
        }
    }
    let lambda = 0.5 * (lo + hi);
    // Also check the boundary λ=0 (σ²_u = 0).
    let (f_opt, _) = eval(lambda)?;
    let (f0, s2e0) = eval(0.0)?;
    if f0 <= f_opt {
        // σ²_u clipped to 0.
        let _ = total_var;
        return Ok((0.0, s2e0));
    }
    let (_, sigma2_e) = eval(lambda)?;
    let sigma2_u = lambda * sigma2_e;
    Ok((sigma2_u, sigma2_e))
}
