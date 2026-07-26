use super::*;

// ═════════════════════ General fixed-effects design ═════════════════════

use crate::stat::optim::{nelder_mead, polish_coord};

// ═════════════════════ General covariance V(θ) + REML ═════════════════════

/// The kind of covariance model being optimized in the general path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GenCov {
    /// RANDOM intercept VC/CS with a general fixed design (params: σ²_u, σ²_e).
    RandomVc,
    /// REPEATED TYPE=AR(1) with SUBJECT (params: ρ, σ²).
    RepeatedAr1,
    /// REPEATED TYPE=UN with SUBJECT (t(t+1)/2 params).
    RepeatedUn { t: usize },
}

/// Build V(θ) for the general path.
/// `subj_of[i]` is the subject index of observation i; `within_idx[i]` is the
/// position of obs i within its subject (0-based, in order of appearance).
pub(super) fn build_v_gen(
    cov: GenCov,
    theta: &[f64],
    n: usize,
    subj_of: &[usize],
    within_idx: &[usize],
) -> Vec<Vec<f64>> {
    let mut v = vec![vec![0.0; n]; n];
    match cov {
        GenCov::RandomVc => {
            let s2u = theta[0];
            let s2e = theta[1];
            for i in 0..n {
                for j in 0..n {
                    let mut val = 0.0;
                    if subj_of[i] == subj_of[j] {
                        val += s2u;
                    }
                    if i == j {
                        val += s2e;
                    }
                    v[i][j] = val;
                }
            }
        }
        GenCov::RepeatedAr1 => {
            let rho = theta[0];
            let s2 = theta[1];
            for i in 0..n {
                for j in 0..n {
                    if subj_of[i] == subj_of[j] {
                        let d = (within_idx[i] as i64 - within_idx[j] as i64).unsigned_abs();
                        v[i][j] = s2 * rho.powi(d as i32);
                    }
                }
            }
        }
        GenCov::RepeatedUn { t } => {
            // Build the t×t SPD block from packed params (row-major lower).
            let block = un_block(theta, t);
            for i in 0..n {
                for j in 0..n {
                    if subj_of[i] == subj_of[j] {
                        v[i][j] = block[within_idx[i]][within_idx[j]];
                    }
                }
            }
        }
    }
    v
}

/// Number of free covariance parameters for a covariance model.
pub(super) fn n_cov_params(cov: GenCov) -> usize {
    match cov {
        GenCov::RandomVc => 2,
        GenCov::RepeatedAr1 => 2,
        GenCov::RepeatedUn { t } => t * (t + 1) / 2,
    }
}

/// Map an unconstrained parameter vector `u` to the natural θ for the model,
/// enforcing bounds: σ²>0 via exp, ρ∈(−1,1) via tanh, UN via Cholesky factor.
pub(super) fn unconstrained_to_theta(cov: GenCov, u: &[f64]) -> Vec<f64> {
    match cov {
        GenCov::RandomVc => vec![u[0].exp(), u[1].exp()],
        GenCov::RepeatedAr1 => vec![u[0].tanh(), u[1].exp()],
        GenCov::RepeatedUn { t } => {
            // u parameterizes a lower-triangular Cholesky factor L (with positive
            // diagonal via exp); θ = packed lower of L Lᵀ in UN order.
            let mut l = vec![vec![0.0; t]; t];
            let mut k = 0;
            for r in 0..t {
                for c in 0..=r {
                    if r == c {
                        l[r][c] = u[k].exp();
                    } else {
                        l[r][c] = u[k];
                    }
                    k += 1;
                }
            }
            let mut theta = Vec::with_capacity(t * (t + 1) / 2);
            for r in 0..t {
                for c in 0..=r {
                    let mut s = 0.0;
                    for p in 0..=c.min(r) {
                        s += l[r][p] * l[c][p];
                    }
                    theta.push(s);
                }
            }
            theta
        }
    }
}

/// Evaluate −2·log(RE)ML at θ. Returns (neg2, β̂, (X'V⁻¹X)⁻¹).
pub(super) fn neg2_loglik_gen(
    y: &[f64],
    x: &[Vec<f64>],
    v: &[Vec<f64>],
    method: Method,
) -> Result<(f64, Vec<f64>, Vec<Vec<f64>>)> {
    let n = y.len();
    let p = x[0].len();
    let v_inv = invert_matrix(v)?;
    let log_det_v = log_det_spd(v)?;

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
    let neg2 = match method {
        Method::Reml => (n as f64 - p as f64) * two_pi.ln() + log_det_v + log_det_xtvix + quad,
        Method::Ml => n as f64 * two_pi.ln() + log_det_v + quad,
    };
    Ok((neg2, beta, xtvix_inv))
}

/// Result of a general mixed fit.
pub(super) struct GenFit {
    /// Natural covariance parameters θ.
    pub(super) theta: Vec<f64>,
    pub(super) beta: Vec<f64>,
    pub(super) cov_beta: Vec<Vec<f64>>,
    pub(super) neg2ll: f64,
    pub(super) neg2_start: f64,
    pub(super) iters: usize,
    pub(super) converged: bool,
}

/// Nelder-Mead minimisation of −2·log(RE)ML over the unconstrained parameters,
/// with simplex restarts and a final coordinate-descent polish so the estimate
/// reaches ≈4-decimal accuracy on the flat profiled-likelihood surface.
pub(super) fn fit_gen(
    y: &[f64],
    x: &[Vec<f64>],
    cov: GenCov,
    subj_of: &[usize],
    within_idx: &[usize],
    method: Method,
    u0: &[f64],
) -> Result<GenFit> {
    let n = y.len();

    let eval = |u: &[f64]| -> f64 {
        let theta = unconstrained_to_theta(cov, u);
        let v = build_v_gen(cov, &theta, n, subj_of, within_idx);
        match neg2_loglik_gen(y, x, &v, method) {
            Ok((neg2, _, _)) => {
                if neg2.is_finite() {
                    neg2
                } else {
                    1e30
                }
            }
            Err(_) => 1e30,
        }
    };

    let neg2_start = eval(u0);

    // Repeatedly run Nelder-Mead, re-initialising the simplex around the
    // current best vertex. Restarts are the standard cure for NM stalling on
    // flat/valley surfaces. Shrink the initial step each restart so later runs
    // refine locally.
    let mut u_best = u0.to_vec();
    let mut f_best = neg2_start;
    let mut total_iters = 0usize;
    let mut converged = false;
    let mut step = 0.5_f64;
    for restart in 0..6 {
        let (u_r, f_r, it, conv) = nelder_mead(&eval, &u_best, step, 2000, 1e-12, 1e-10);
        total_iters += it;
        if f_r <= f_best {
            f_best = f_r;
            u_best = u_r;
        }
        converged = conv;
        // Stop early once two successive restarts no longer move the optimum.
        if restart >= 2 && conv {
            break;
        }
        step *= 0.3;
    }

    // Final coordinate-descent polish to squeeze out residual flat-surface
    // error; cheap (a few dozen evals) and robust for VC/CS/AR(1)/UN.
    polish_coord(&eval, &mut u_best, &mut f_best, 1e-9);

    let theta = unconstrained_to_theta(cov, &u_best);
    let v = build_v_gen(cov, &theta, n, subj_of, within_idx);
    let (neg2ll, beta, cov_beta) = neg2_loglik_gen(y, x, &v, method)?;

    Ok(GenFit {
        theta,
        beta,
        cov_beta,
        neg2ll,
        neg2_start,
        iters: total_iters,
        converged,
    })
}

/// Covariance model + initial unconstrained parameters for the optimizer.
pub(super) fn initial_cov_params(plan: &Plan, y: &[f64], max_obs: usize) -> (GenCov, Vec<f64>) {
    let n_used = y.len();
    match plan {
        Plan::RandomVc(_, _) => {
            // Use the variance of y as a scale.
            let mean = y.iter().sum::<f64>() / n_used as f64;
            let var =
                y.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n_used as f64 - 1.0).max(1.0);
            let v0 = var.max(1e-3);
            (GenCov::RandomVc, vec![(v0 / 2.0).ln(), (v0 / 2.0).ln()])
        }
        Plan::Repeated(CovType::Ar1, _) => {
            let mean = y.iter().sum::<f64>() / n_used as f64;
            let var =
                y.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n_used as f64 - 1.0).max(1.0);
            // u[0]=atanh(0.1)≈0.1, u[1]=ln(var).
            (GenCov::RepeatedAr1, vec![0.1_f64, var.max(1e-3).ln()])
        }
        Plan::Repeated(CovType::Un, _) => {
            let t = max_obs;
            // Initial L = diag(sqrt(var)) → u diagonal = 0.5*ln(var), off-diag 0.
            let mean = y.iter().sum::<f64>() / n_used as f64;
            let var = (y.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
                / (n_used as f64 - 1.0).max(1.0))
            .max(1e-3);
            let mut u = Vec::new();
            for r in 0..t {
                for c in 0..=r {
                    if r == c {
                        u.push(0.5 * var.ln());
                    } else {
                        u.push(0.0);
                    }
                }
            }
            (GenCov::RepeatedUn { t }, u)
        }
        Plan::Repeated(_, _) => unreachable!(),
    }
}
