use super::*;

// ═════════════════ General covariance V(θ) = R (AR(1)/UN) + weights ══════════
//
// Mirror of the PROC MIXED general optimizer, specialised to a within-subject
// repeated structure R (AR(1) or UN). The working-variate weights from the RSPL
// linearisation enter by inflating the diagonal of R by 1/w_i (so the Normal,
// no-weight case is the exact LMM that PROC MIXED's REPEATED path reports).

/// The within-subject repeated covariance model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RepCov {
    /// AR(1): params (ρ, σ²).
    Ar1,
    /// UN: t(t+1)/2 params for a t×t SPD block.
    Un { t: usize },
}

/// Build V(θ) = R for the repeated structure, with the working weights folded
/// into the diagonal (R_ii ← R_ii + extra/w_i is NOT used; instead the standard
/// GLMM working covariance is R∘ where the residual block is scaled by the
/// pseudo-variance 1/w_i). For the un-weighted Normal case `weights=None` gives
/// the plain repeated covariance.
pub(super) fn build_v_rep(
    cov: RepCov,
    theta: &[f64],
    n: usize,
    subj_of: &[usize],
    within_idx: &[usize],
    weights: Option<&[f64]>,
) -> Vec<Vec<f64>> {
    let mut v = vec![vec![0.0; n]; n];
    match cov {
        RepCov::Ar1 => {
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
        RepCov::Un { t } => {
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
    // Fold working weights into the diagonal (pseudo-residual variance 1/w_i).
    if let Some(w) = weights {
        for i in 0..n {
            let wi = w[i].max(1e-12);
            v[i][i] += 1.0 / wi;
        }
    }
    v
}

/// Map unconstrained params `u` to natural θ: AR(1) via (tanh, exp); UN via a
/// Cholesky factor with positive (exp) diagonal so the block is SPD.
pub(super) fn unconstrained_to_theta_rep(cov: RepCov, u: &[f64]) -> Vec<f64> {
    match cov {
        RepCov::Ar1 => vec![u[0].tanh(), u[1].exp()],
        RepCov::Un { t } => {
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
                    for q in 0..=c.min(r) {
                        s += l[r][q] * l[c][q];
                    }
                    theta.push(s);
                }
            }
            theta
        }
    }
}

pub(super) fn n_rep_params(cov: RepCov) -> usize {
    match cov {
        RepCov::Ar1 => 2,
        RepCov::Un { t } => t * (t + 1) / 2,
    }
}

/// Evaluate −2·log REML at V. Returns (neg2, β̂, (X'V⁻¹X)⁻¹). REML only (the
/// repeated structure here is for the Normal/pseudo-likelihood working model).
pub(super) fn neg2_reml_gen(
    y: &[f64],
    x: &[Vec<f64>],
    v: &[Vec<f64>],
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
    let neg2 = (n as f64 - p as f64) * two_pi.ln() + log_det_v + log_det_xtvix + quad;
    Ok((neg2, beta, xtvix_inv))
}

/// Result of fitting the repeated-structure (RE)ML working model.
pub(super) struct RepFit {
    pub(super) theta: Vec<f64>,
    pub(super) beta: Vec<f64>,
    pub(super) cov_beta: Vec<Vec<f64>>,
    pub(super) neg2: f64,
}

/// Fit the weighted LMM with repeated covariance R (AR(1)/UN) via Nelder-Mead
/// with restarts + coordinate polish (mirrors the PROC MIXED general optimizer).
pub(super) fn fit_rep(
    y: &[f64],
    x: &[Vec<f64>],
    cov: RepCov,
    subj_of: &[usize],
    within_idx: &[usize],
    weights: Option<&[f64]>,
) -> Result<RepFit> {
    let n = y.len();
    let np = n_rep_params(cov);

    let eval = |u: &[f64]| -> f64 {
        let theta = unconstrained_to_theta_rep(cov, u);
        let v = build_v_rep(cov, &theta, n, subj_of, within_idx, weights);
        match neg2_reml_gen(y, x, &v) {
            Ok((neg2, _, _)) if neg2.is_finite() => neg2,
            _ => 1e30,
        }
    };

    // Start: ρ≈0.1 (atanh), σ²≈Var(y) (log). For UN, diagonal ≈ Var(y).
    let var_y = {
        let m = y.iter().sum::<f64>() / n as f64;
        (y.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (n.max(1) as f64)).max(1e-6)
    };
    let u0: Vec<f64> = match cov {
        RepCov::Ar1 => vec![0.1_f64.atanh(), var_y.ln()],
        RepCov::Un { t } => {
            let mut u = Vec::with_capacity(np);
            for r in 0..t {
                for c in 0..=r {
                    if r == c {
                        u.push((var_y.sqrt()).ln());
                    } else {
                        u.push(0.0);
                    }
                }
            }
            u
        }
    };

    let mut u_best = u0.clone();
    let mut f_best = eval(&u0);
    let mut step = 0.5_f64;
    for restart in 0..6 {
        let (u_r, f_r, _iters, conv) = nelder_mead(&eval, &u_best, step, 2000, 1e-12, 1e-10);
        if f_r <= f_best {
            f_best = f_r;
            u_best = u_r;
        }
        if restart >= 2 && conv {
            break;
        }
        step *= 0.3;
    }
    polish_coord(&eval, &mut u_best, &mut f_best, 1e-9);

    let theta = unconstrained_to_theta_rep(cov, &u_best);
    let v = build_v_rep(cov, &theta, n, subj_of, within_idx, weights);
    let (neg2, beta, cov_beta) = neg2_reml_gen(y, x, &v)?;
    Ok(RepFit {
        theta,
        beta,
        cov_beta,
        neg2,
    })
}

/// Build the named covariance-parameter rows from a repeated-structure θ.
/// AR(1): rows AR(1) and Residual (σ²). UN: rows UN(i,j) in SAS packed order.
pub(super) fn cov_parms_from_rep(cov: RepCov, theta: &[f64]) -> Vec<CovParm> {
    match cov {
        RepCov::Ar1 => vec![
            CovParm {
                name: "AR(1)".to_string(),
                show_subject: true,
                estimate: theta[0],
            },
            CovParm {
                name: "Residual".to_string(),
                show_subject: false,
                estimate: theta[1],
            },
        ],
        RepCov::Un { t } => {
            let mut rows = Vec::with_capacity(theta.len());
            let mut k = 0;
            for r in 0..t {
                for c in 0..=r {
                    rows.push(CovParm {
                        name: format!("UN({},{})", r + 1, c + 1),
                        show_subject: true,
                        estimate: theta[k],
                    });
                    k += 1;
                }
            }
            rows
        }
    }
}

/// Fit a GLMM with a within-subject repeated covariance R (AR(1)/UN) via the
/// RSPL working-variate loop. For Normal/Identity the loop converges in one step
/// (the working response equals y, weights are 1), reproducing the exact REML
/// reported by PROC MIXED's REPEATED path.
#[allow(clippy::too_many_arguments)]
pub(super) fn fit_rspl_rep(
    y: &[f64],
    x: &[Vec<f64>],
    freq: &[f64],
    subj_of: &[usize],
    within_idx: &[usize],
    cov: RepCov,
    dist: Distribution,
    lf: LinkFunction,
) -> Result<GlimmixFit> {
    let n = y.len();

    if dist == Distribution::Normal && lf == LinkFunction::Identity {
        // Exact weighted (here un-weighted) LMM: no PQL iteration needed.
        let rep = fit_rep(y, x, cov, subj_of, within_idx, None)?;
        let mu = (0..n).map(|i| dot(&x[i], &rep.beta)).collect();
        let residual = match cov {
            RepCov::Ar1 => rep.theta[1],
            RepCov::Un { .. } => rep.theta[0],
        };
        return Ok(GlimmixFit {
            beta: rep.beta.clone(),
            cov_beta: rep.cov_beta,
            mu,
            sigma2_u: None,
            sigma2_e: residual,
            neg2: rep.neg2,
            iterations: 1,
            cov_parms: Some(cov_parms_from_rep(cov, &rep.theta)),
        });
    }

    // Non-normal: RSPL loop with R as the working covariance.
    let glm0 = fit_glm(y, x, freq, dist, lf)?;
    let mut beta = glm0.beta.clone();
    let mut last = fit_rep(
        &{
            // initial working response at β (u≡0)
            let mut z = vec![0.0; n];
            for i in 0..n {
                let eta = dot(&x[i], &beta);
                let mu = inv_link(eta, lf);
                let d = dmu_deta(eta, lf).max(1e-12);
                z[i] = eta + (y[i] - mu) / d;
            }
            z
        },
        x,
        cov,
        subj_of,
        within_idx,
        Some(&{
            let mut w = vec![0.0; n];
            for i in 0..n {
                let eta = dot(&x[i], &beta);
                let mu = inv_link(eta, lf);
                let d = dmu_deta(eta, lf).max(1e-12);
                let v = variance(mu, dist);
                w[i] = freq[i] * d * d / v;
            }
            w
        }),
    )?;
    beta = last.beta.clone();
    let mut iterations = 1;

    for it in 1..50 {
        iterations = it + 1;
        let mut z = vec![0.0; n];
        let mut w = vec![0.0; n];
        for i in 0..n {
            let eta = dot(&x[i], &beta);
            let mu = inv_link(eta, lf);
            let d = dmu_deta(eta, lf).max(1e-12);
            let v = variance(mu, dist);
            w[i] = freq[i] * d * d / v;
            z[i] = eta + (y[i] - mu) / d;
        }
        let rep = fit_rep(&z, x, cov, subj_of, within_idx, Some(&w))?;
        let diff: f64 = rep
            .beta
            .iter()
            .zip(&beta)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt();
        let norm_old: f64 = beta.iter().map(|b| b * b).sum::<f64>().sqrt();
        beta = rep.beta.clone();
        last = rep;
        if diff / (1.0 + norm_old) < 1e-6 {
            break;
        }
    }

    let mu = (0..n).map(|i| inv_link(dot(&x[i], &beta), lf)).collect();
    let residual = match cov {
        RepCov::Ar1 => last.theta[1],
        RepCov::Un { .. } => last.theta[0],
    };
    Ok(GlimmixFit {
        beta,
        cov_beta: last.cov_beta,
        mu,
        sigma2_u: None,
        sigma2_e: residual,
        neg2: last.neg2,
        iterations,
        cov_parms: Some(cov_parms_from_rep(cov, &last.theta)),
    })
}
