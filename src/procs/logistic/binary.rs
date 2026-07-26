// MQ7.2c — `needless_range_loop` assumé dans ce module : l'indice EST le
// langage du domaine (`a[i][j] * b[j][k]`, parcours colonne-major, triangle
// d'une matrice symétrique). La forme itérateur y coûte plus en lisibilité
// qu'elle n'en rend, et la revue a préféré garder les indices explicites.
#![allow(clippy::needless_range_loop)]

use super::*;

/// Newton-Raphson binary fit results.
pub(super) struct BinaryFit {
    pub(super) beta: Vec<f64>,
    pub(super) converged: bool,
    pub(super) var_beta: Vec<Vec<f64>>,
    pub(super) se_beta: Vec<f64>,
    pub(super) wald_chi2: Vec<f64>,
    pub(super) wald_p: Vec<f64>,
    pub(super) final_p: Vec<f64>,
}

/// Newton-Raphson / IRLS fit of the binary model, with the final Hessian,
/// variance-covariance matrix and per-parameter Wald statistics.
pub(super) fn fit_binary(
    session: &mut Session,
    y_vec: &[f64],
    x_mat: &[Vec<f64>],
    freq_vec: &[f64],
    link: Link,
    p_bar: f64,
    p_param: usize,
) -> Result<BinaryFit> {
    let n_obs = y_vec.len();
    let mut beta: Vec<f64> = vec![0.0; p_param];
    beta[0] = (p_bar / (1.0 - p_bar)).ln();

    let mut converged = false;
    for _iter in 0..50 {
        // Compute predictions
        let mut score: Vec<f64> = vec![0.0; p_param];
        let mut hessian: Vec<Vec<f64>> = vec![vec![0.0; p_param]; p_param];

        for i in 0..n_obs {
            let xi = &x_mat[i];
            let eta: f64 = xi.iter().zip(beta.iter()).map(|(x, b)| x * b).sum();
            let fi = freq_vec[i];

            // `s_resid` is the per-obs score multiplier for x[j]; `wi` the
            // Fisher weight for the Hessian. For LINK=LOGIT these reduce to the
            // canonical (y−μ) and μ(1−μ), reproducing the original code exactly.
            let (s_resid, wi) = if link == Link::Logit {
                let pi = (1.0 / (1.0 + (-eta).exp())).clamp(1e-10, 1.0 - 1e-10);
                (y_vec[i] - pi, pi * (1.0 - pi))
            } else {
                let mu = link.mean(eta).clamp(1e-10, 1.0 - 1e-10);
                let dmu = link.dmu_deta(eta).max(1e-12);
                let var = mu * (1.0 - mu);
                (dmu * (y_vec[i] - mu) / var, dmu * dmu / var)
            };

            // Score (gradient)
            for j in 0..p_param {
                score[j] += fi * xi[j] * s_resid;
            }

            // Hessian = -X'WX (negative information)
            for j in 0..p_param {
                for k in 0..p_param {
                    hessian[j][k] -= fi * xi[j] * xi[k] * wi;
                }
            }
        }

        // Newton step: delta = -H^{-1} * score = solve(-H, score)
        // Negate hessian to get positive definite matrix
        let neg_hessian: Vec<Vec<f64>> = hessian
            .iter()
            .map(|row| row.iter().map(|v| -v).collect())
            .collect();

        let neg_h_inv = invert_matrix(&neg_hessian)?;
        let delta = mat_vec(&neg_h_inv, &score);

        // Update beta
        for j in 0..p_param {
            beta[j] += delta[j];
        }

        // Convergence check (GCONV)
        let max_delta = delta.iter().map(|d| d.abs()).fold(0.0_f64, f64::max);
        let max_beta = beta.iter().map(|b| b.abs()).fold(0.0_f64, f64::max);
        let gconv = max_delta / (1.0 + max_beta);
        if gconv < 1e-8 {
            converged = true;
            break;
        }
    }

    if !converged {
        // Quasi-complete or complete separation, or slow convergence: warn but
        // proceed with the last iterate rather than panicking.
        session.log.note(
            "PROC LOGISTIC: the maximum likelihood estimate may not exist \
             (possible separation); iteration limit reached.",
        );
    }

    // ── 9b. Final Hessian and variance-covariance matrix ─────────────────
    let mut final_hessian: Vec<Vec<f64>> = vec![vec![0.0; p_param]; p_param];
    let mut final_p: Vec<f64> = Vec::with_capacity(n_obs);

    for i in 0..n_obs {
        let xi = &x_mat[i];
        let eta: f64 = xi.iter().zip(beta.iter()).map(|(x, b)| x * b).sum();
        let (pi, wi) = if link == Link::Logit {
            let pi = (1.0 / (1.0 + (-eta).exp())).clamp(1e-10, 1.0 - 1e-10);
            (pi, pi * (1.0 - pi))
        } else {
            let mu = link.mean(eta).clamp(1e-10, 1.0 - 1e-10);
            let dmu = link.dmu_deta(eta).max(1e-12);
            (mu, dmu * dmu / (mu * (1.0 - mu)))
        };
        let fi = freq_vec[i];
        final_p.push(pi);
        for j in 0..p_param {
            for k in 0..p_param {
                final_hessian[j][k] -= fi * xi[j] * xi[k] * wi;
            }
        }
    }

    let neg_final_hessian: Vec<Vec<f64>> = final_hessian
        .iter()
        .map(|row| row.iter().map(|v| -v).collect())
        .collect();
    let var_beta = invert_matrix(&neg_final_hessian)?;

    // Standard errors, Wald chi-squares, p-values for each parameter
    let se_beta: Vec<f64> = (0..p_param).map(|j| var_beta[j][j].sqrt()).collect();
    let wald_chi2: Vec<f64> = (0..p_param)
        .map(|j| (beta[j] / se_beta[j]).powi(2))
        .collect();
    let wald_p: Vec<f64> = wald_chi2.iter().map(|&w| chisq_sf(w, 1.0)).collect();

    Ok(BinaryFit {
        beta,
        converged,
        var_beta,
        se_beta,
        wald_chi2,
        wald_p,
        final_p,
    })
}

/// Global Wald test: β_c' * Σ_c^{-1} * β_c where Σ_c = submatrix for predictors.
pub(super) fn wald_global_test(beta: &[f64], var_beta: &[Vec<f64>], nb_cols: usize) -> Result<f64> {
    let wald_chi2_global = if nb_cols > 0 {
        let sigma_c = submatrix_predictors(var_beta, nb_cols);
        let sigma_c_inv = invert_matrix(&sigma_c)?;
        let beta_c: Vec<f64> = (1..=nb_cols).map(|j| beta[j]).collect();
        let tmp = mat_vec(&sigma_c_inv, &beta_c);
        dot(&beta_c, &tmp)
    } else {
        0.0
    };
    Ok(wald_chi2_global)
}

/// Global score test (under H₀: β_c=0, β₀=logit(p̄)). Computed with the
/// canonical (logit) Fisher information; for non-logit links it is a close
/// Rao-score approximation to the global association test.
pub(super) fn score_global_test(
    y_vec: &[f64],
    x_mat: &[Vec<f64>],
    freq_vec: &[f64],
    p_bar: f64,
    n_total: f64,
    nb_cols: usize,
) -> Result<f64> {
    let n_obs = y_vec.len();
    let score_chi2 = if nb_cols > 0 {
        // Score_c_j = Σ freq_i * x_ij * (y_i - p̄) for j = predictors
        let mut score_c: Vec<f64> = vec![0.0; nb_cols];
        // Score_0 = Σ freq_i * (y_i - p̄)
        let mut score_0: f64 = 0.0;
        // I_cc_jk = p̄*(1-p̄) * Σ freq_i * x_ij * x_ik  (j,k predictors)
        let mut i_cc: Vec<Vec<f64>> = vec![vec![0.0; nb_cols]; nb_cols];
        // I_00 = p̄*(1-p̄) * n_total
        let i_00 = p_bar * (1.0 - p_bar) * n_total;
        // I_0c_j = p̄*(1-p̄) * Σ freq_i * x_ij  (j predictor)
        let mut i_0c: Vec<f64> = vec![0.0; nb_cols];

        for i in 0..n_obs {
            let fi = freq_vec[i];
            let xi = &x_mat[i];
            let resid = y_vec[i] - p_bar;
            score_0 += fi * resid;
            for j in 0..nb_cols {
                score_c[j] += fi * xi[j + 1] * resid;
                i_0c[j] += fi * xi[j + 1];
                for k in 0..nb_cols {
                    i_cc[j][k] += fi * xi[j + 1] * xi[k + 1];
                }
            }
        }
        // Apply p̄*(1-p̄) to I matrices
        let pb_var = p_bar * (1.0 - p_bar);
        for j in 0..nb_cols {
            i_0c[j] *= pb_var;
            for k in 0..nb_cols {
                i_cc[j][k] *= pb_var;
            }
        }

        // Schur complement: I_cc|0 = I_cc - I_c0 * I_00^{-1} * I_0c
        // (I_c0 = I_0c^T for scalar I_00)
        let i_00_inv = 1.0 / i_00;
        let mut i_cc_schur = i_cc.clone();
        for j in 0..nb_cols {
            for k in 0..nb_cols {
                i_cc_schur[j][k] -= i_0c[j] * i_00_inv * i_0c[k];
            }
        }

        // Score_c|0 = Score_c - (I_c0 / I_00) * Score_0
        let mut score_c_schur = score_c.clone();
        for j in 0..nb_cols {
            score_c_schur[j] -= (i_0c[j] / i_00) * score_0;
        }

        // χ²_Score = Score_c|0' * I_cc|0^{-1} * Score_c|0
        let i_cc_schur_inv = invert_matrix(&i_cc_schur)?;
        let tmp = mat_vec(&i_cc_schur_inv, &score_c_schur);
        dot(&score_c_schur, &tmp)
    } else {
        0.0
    };
    Ok(score_chi2)
}
