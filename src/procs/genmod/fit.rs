use super::*;

/// Résultat d'un ajustement IRLS : `(β, (X'WX)⁻¹ à convergence, μ ajusté)`.
type IrlsFit = (Vec<f64>, Vec<Vec<f64>>, Vec<f64>);

/// Données d'ajustement d'un modèle GENMOD : réponse, plan, fréquences et
/// tailles dérivées. Constantes pendant tout l'ajustement, elles étaient
/// jusqu'ici passées une par une à `fit_irls` PUIS à `compute_scale`
/// (8 paramètres positionnels chacun, dont trois `&[f64]` interchangeables à
/// la compilation).
pub(super) struct GenmodData<'a> {
    pub(super) y: &'a [f64],
    pub(super) x: &'a [Vec<f64>],
    pub(super) freq: &'a [f64],
    /// Somme des fréquences (≠ nombre de lignes quand FREQ= est utilisé).
    pub(super) n_total: f64,
    /// Nombre de paramètres du modèle.
    pub(super) p_param: usize,
}

/// IRLS / Newton-Raphson fit. Returns `(β, (X'WX)⁻¹ at convergence, fitted μ)`.
pub(super) fn fit_irls(
    session: &mut Session,
    data: &GenmodData<'_>,
    dist: &Distribution,
    lf: &LinkFunction,
) -> Result<IrlsFit> {
    let (y_vec, x_mat, freq_vec, n_total, p_param) =
        (data.y, data.x, data.freq, data.n_total, data.p_param);
    let n_obs = y_vec.len();

    // Initialize β
    let y_mean: f64 = {
        let sum_wy: f64 = y_vec.iter().zip(freq_vec.iter()).map(|(y, w)| y * w).sum();
        sum_wy / n_total
    };

    let eta0 = match lf {
        LinkFunction::Log => y_mean.max(1e-10).ln(),
        LinkFunction::Logit => {
            let p = y_mean.clamp(1e-10, 1.0 - 1e-10);
            (p / (1.0 - p)).ln().clamp(-10.0, 10.0)
        }
        LinkFunction::Identity => y_mean,
        // η = 1/μ; guard ȳ > 0 (Gamma response is positive).
        LinkFunction::Reciprocal => 1.0 / y_mean.max(1e-10),
    };

    let mut beta: Vec<f64> = vec![0.0; p_param];
    beta[0] = eta0;

    // IRLS iterations (max 50)
    let mut converged = false;
    for _iter in 0..50 {
        // Compute H = X'WX and score = X'W(z - Xβ) = X'(y - μ)/V * deta_dmu⁻¹
        // Actually: score_j = Σ freq_i * x_ij * (y_i - mu_i) / (V(mu_i) * deta_dmu(mu_i))
        // H_jk = Σ freq_i * x_ij * x_ik / (V(mu_i) * deta_dmu(mu_i)^2)
        let mut score: Vec<f64> = vec![0.0; p_param];
        let mut hessian: Vec<Vec<f64>> = vec![vec![0.0; p_param]; p_param];

        for i in 0..n_obs {
            let xi = &x_mat[i];
            let eta: f64 = xi.iter().zip(beta.iter()).map(|(x, b)| x * b).sum();
            let mu = inv_link(eta, lf);
            let v = variance(mu, dist);
            let dg = deta_dmu(mu, lf);
            let w_irls = freq_vec[i] / (v * dg * dg);

            // Score: X' w_irls * (z_i - eta_i) where z_i = eta + (y-mu)*dg
            // = X' freq_i * (y_i - mu_i) / (V * dg)
            let resid_adj = freq_vec[i] * (y_vec[i] - mu) / (v * dg);
            for j in 0..p_param {
                score[j] += xi[j] * resid_adj;
            }

            // Hessian H = X'WX (positive definite)
            for j in 0..p_param {
                for k in 0..p_param {
                    hessian[j][k] += xi[j] * xi[k] * w_irls;
                }
            }
        }

        // Solve H·δ = score
        let h_inv = invert_matrix(&hessian)?;
        let delta = matrix_vec_mult(&h_inv, &score);

        // Update β with step-halving to keep μ valid (μ>0 for Gamma/Poisson,
        // 0<μ<1 for Binomial). The reciprocal link in particular can drive μ
        // negative; halve the step until every fitted μ is in range.
        let mu_in_range = |b: &[f64]| -> bool {
            for xi in x_mat.iter() {
                let eta: f64 = xi.iter().zip(b.iter()).map(|(x, c)| x * c).sum();
                let mu = inv_link(eta, lf);
                let ok = match dist {
                    Distribution::Binomial => mu > 0.0 && mu < 1.0,
                    _ => mu > 0.0,
                };
                if !ok || !mu.is_finite() {
                    return false;
                }
            }
            true
        };

        let mut step = 1.0_f64;
        let mut trial = beta.clone();
        for _ in 0..40 {
            for j in 0..p_param {
                trial[j] = beta[j] + step * delta[j];
            }
            if mu_in_range(&trial) {
                break;
            }
            step *= 0.5;
        }
        beta = trial;

        // GCONV convergence check (scaled by the realized step length)
        let max_delta = delta
            .iter()
            .map(|d| (step * d).abs())
            .fold(0.0_f64, f64::max);
        let max_beta = beta.iter().map(|b| b.abs()).fold(0.0_f64, f64::max);
        let gconv = max_delta / (1.0 + max_beta);
        if gconv < 1e-8 {
            converged = true;
            break;
        }
    }

    if !converged {
        // NOTE (not a panic): report non-convergence and stop this PROC cleanly.
        session
            .log
            .note("PROC GENMOD failed to converge within the iteration limit.");
        return Err(SasError::runtime("PROC GENMOD failed to converge"));
    }

    // ── Final H = X'WX at convergence ────────────────────────────────────
    let mut final_hessian: Vec<Vec<f64>> = vec![vec![0.0; p_param]; p_param];
    let mut final_mu: Vec<f64> = Vec::with_capacity(n_obs);

    for i in 0..n_obs {
        let xi = &x_mat[i];
        let eta: f64 = xi.iter().zip(beta.iter()).map(|(x, b)| x * b).sum();
        let mu = inv_link(eta, lf);
        let v = variance(mu, dist);
        let dg = deta_dmu(mu, lf);
        let w_irls = freq_vec[i] / (v * dg * dg);
        final_mu.push(mu);
        for j in 0..p_param {
            for k in 0..p_param {
                final_hessian[j][k] += xi[j] * xi[k] * w_irls;
            }
        }
    }

    let h_inv = invert_matrix(&final_hessian)?;

    Ok((beta, h_inv, final_mu))
}

/// Scale/dispersion estimate: printed Scale row value, its DF and SE, the
/// dispersion φ, and Var(β̂) = φ·H⁻¹.
pub(super) struct ScaleInfo {
    pub(super) scale_est: f64,
    pub(super) scale_df: i64,
    pub(super) disp_phi: f64,
    pub(super) se_scale: f64,
    pub(super) var_beta: Vec<Vec<f64>>,
}

/// Estimate (or fix) the scale/dispersion parameter.
///
/// Poisson, Binomial: scale=1 (fixed, DF=0).
/// Normal: φ̂ = MSE = SSE/(n−p); reported Scale = √φ̂ = σ̂, DF=n−p.
/// Gamma:  Pearson dispersion φ̂ = (1/(n−p)) Σ (y−μ)²/μ²; reported Scale is
///         the estimate of 1/φ (SAS GENMOD reports the Gamma "Scale" in the
///         1/φ form). The exact SAS default uses the ML (digamma) scale; we
///         approximate it by the moment-based Pearson dispersion — the two
///         agree asymptotically but differ in finite samples. DF=n−p.
///
/// NOSCALE / SCALE=: when NOSCALE, the dispersion is held FIXED rather than
/// estimated (φ=1 by default, or derived from SCALE=); when SCALE=v is given
/// the reported Scale is fixed at v (for Normal v=σ ⇒ φ=v²; for Gamma the
/// value is the 1/φ form ⇒ φ=1/v). A fixed scale carries DF=0.
///
/// `disp_phi` is the dispersion φ used to scale Var(β̂)=φ·H⁻¹ and the scaled
/// criteria. `scale_est` is the value printed on the Scale row.
pub(super) fn compute_scale(
    model: &GenmodModel,
    dist: &Distribution,
    data: &GenmodData<'_>,
    final_mu: &[f64],
    h_inv: Vec<Vec<f64>>,
) -> ScaleInfo {
    let (y_vec, freq_vec, n_total, p_param) = (data.y, data.freq, data.n_total, data.p_param);
    let n_obs = y_vec.len();

    let scale_est: f64;
    let scale_df: i64;
    let disp_phi: f64;
    let var_beta: Vec<Vec<f64>>;

    let df_err = (n_total as i64) - (p_param as i64);
    let fixed_scale = model.noscale || model.scale.is_some();

    match dist {
        Distribution::Normal => {
            if fixed_scale {
                // SCALE= is σ for Normal (default 1 under NOSCALE alone).
                let sigma = model.scale.unwrap_or(1.0);
                scale_est = sigma;
                disp_phi = sigma * sigma;
                scale_df = 0;
            } else {
                let sse: f64 = y_vec
                    .iter()
                    .zip(final_mu.iter())
                    .zip(freq_vec.iter())
                    .map(|((y, mu), w)| w * (y - mu) * (y - mu))
                    .sum();
                let mse = sse / (df_err as f64);
                scale_est = mse.sqrt();
                disp_phi = mse;
                scale_df = df_err;
            }
            var_beta = h_inv
                .iter()
                .map(|row| row.iter().map(|v| disp_phi * v).collect())
                .collect();
        }
        Distribution::Gamma => {
            // Pearson dispersion φ̂ = (1/(n−p)) Σ w (y−μ)²/μ².
            let pearson_disp: f64 = {
                let s: f64 = (0..n_obs)
                    .map(|i| {
                        let y = y_vec[i];
                        let mu = final_mu[i];
                        freq_vec[i] * (y - mu) * (y - mu) / (mu * mu)
                    })
                    .sum();
                s / (df_err as f64)
            };
            if fixed_scale {
                // For Gamma the printed Scale is the 1/φ form; SCALE=v ⇒ φ=1/v.
                let s = model.scale.unwrap_or(1.0);
                scale_est = s;
                disp_phi = if s != 0.0 { 1.0 / s } else { 1.0 };
                scale_df = 0;
            } else {
                disp_phi = pearson_disp;
                scale_est = if pearson_disp != 0.0 {
                    1.0 / pearson_disp
                } else {
                    0.0
                };
                scale_df = df_err;
            }
            var_beta = h_inv
                .iter()
                .map(|row| row.iter().map(|v| disp_phi * v).collect())
                .collect();
        }
        _ => {
            // Poisson / Binomial: dispersion fixed at 1 (φ=1), Scale row = 1.
            // SCALE= is accepted but only rescales the scaled criteria below; the
            // SEs are left at the φ=1 model (overdispersion adjustment of SEs is
            // deferred).
            scale_est = 1.0;
            scale_df = 0;
            disp_phi = 1.0;
            var_beta = h_inv;
        }
    }

    // Scale SE (Normal/Gamma, when estimated): SE(scale) ≈ scale / sqrt(2·df).
    let se_scale: f64 =
        if (*dist == Distribution::Normal || *dist == Distribution::Gamma) && scale_df > 0 {
            scale_est / (2.0 * scale_df as f64).sqrt()
        } else {
            0.0
        };

    ScaleInfo {
        scale_est,
        scale_df,
        disp_phi,
        se_scale,
        var_beta,
    }
}

/// Goodness-of-fit criteria: log-likelihood, deviance, Pearson χ², their
/// scaled forms, and the information criteria.
pub(super) struct FitCriteria {
    pub(super) log_lik: f64,
    pub(super) deviance: f64,
    pub(super) pearson: f64,
    pub(super) scaled_deviance: f64,
    pub(super) scaled_pearson: f64,
    pub(super) df_gof: i64,
    pub(super) aic: f64,
    pub(super) aicc: f64,
    pub(super) bic: f64,
}

/// Compute the Criteria For Assessing Goodness Of Fit values.
pub(super) fn compute_fit_criteria(
    dist: &Distribution,
    y_vec: &[f64],
    final_mu: &[f64],
    freq_vec: &[f64],
    scale: &ScaleInfo,
    n_total: f64,
    p_param: usize,
) -> FitCriteria {
    let n_obs = y_vec.len();
    let scale_est = scale.scale_est;
    let disp_phi = scale.disp_phi;

    let log_lik: f64 = match dist {
        Distribution::Poisson => (0..n_obs)
            .map(|i| {
                let mu = final_mu[i];
                let y = y_vec[i];
                let fi = freq_vec[i];
                fi * (y * mu.ln() - mu)
            })
            .sum(),
        Distribution::Binomial => (0..n_obs)
            .map(|i| {
                let mu = final_mu[i].clamp(1e-15, 1.0 - 1e-15);
                let y = y_vec[i];
                let fi = freq_vec[i];
                fi * (y * mu.ln() + (1.0 - y) * (1.0 - mu).ln())
            })
            .sum(),
        Distribution::Normal => {
            let sigma2 = scale_est * scale_est;
            (0..n_obs)
                .map(|i| {
                    let y = y_vec[i];
                    let mu = final_mu[i];
                    let fi = freq_vec[i];
                    -0.5 * fi * ((y - mu) * (y - mu) / sigma2 + (2.0 * PI * sigma2).ln())
                })
                .sum()
        }
        Distribution::Gamma => {
            // Full Gamma log-likelihood with dispersion φ (shape ν = 1/φ):
            //   Σ fi·[ (ν−1)·ln y − ν·y/μ − ν·ln μ + ν·ln ν − ln Γ(ν) ].
            let nu = if disp_phi > 0.0 { 1.0 / disp_phi } else { 1.0 };
            (0..n_obs)
                .map(|i| {
                    let y = y_vec[i].max(1e-300);
                    let mu = final_mu[i].max(1e-300);
                    let fi = freq_vec[i];
                    fi * ((nu - 1.0) * y.ln() - nu * y / mu - nu * mu.ln() + nu * nu.ln()
                        - crate::stat::ln_gamma(nu))
                })
                .sum()
        }
    };

    let deviance: f64 = match dist {
        Distribution::Poisson => (0..n_obs)
            .map(|i| {
                let y = y_vec[i];
                let mu = final_mu[i];
                let fi = freq_vec[i];
                let t1 = if y > 0.0 { y * (y / mu).ln() } else { 0.0 };
                fi * 2.0 * (t1 - (y - mu))
            })
            .sum(),
        Distribution::Binomial => (0..n_obs)
            .map(|i| {
                let y = y_vec[i];
                let mu = final_mu[i].clamp(1e-15, 1.0 - 1e-15);
                let fi = freq_vec[i];
                fi * dev_contribution_binom(y, mu)
            })
            .sum(),
        Distribution::Normal => (0..n_obs)
            .map(|i| {
                let y = y_vec[i];
                let mu = final_mu[i];
                let fi = freq_vec[i];
                fi * (y - mu) * (y - mu)
            })
            .sum(),
        // Gamma deviance = 2·Σ fi·[ −ln(y/μ) + (y−μ)/μ ].
        Distribution::Gamma => (0..n_obs)
            .map(|i| {
                let y = y_vec[i].max(1e-300);
                let mu = final_mu[i].max(1e-300);
                let fi = freq_vec[i];
                fi * 2.0 * (-(y / mu).ln() + (y - mu) / mu)
            })
            .sum(),
    };

    let pearson: f64 = (0..n_obs)
        .map(|i| {
            let y = y_vec[i];
            let mu = final_mu[i];
            let fi = freq_vec[i];
            let v = variance(mu, dist);
            fi * (y - mu) * (y - mu) / v
        })
        .sum();

    let df_gof = (n_total as i64) - (p_param as i64);

    // Scaled deviance and scaled Pearson: divide by the dispersion φ. For Normal
    // φ=σ²=scale_est² and for Poisson/Binomial φ=1, so the values match the
    // previous scale_est² form byte-for-byte; for Gamma φ is the dispersion
    // (scale_est=1/φ), so dividing by φ (not scale_est²) is the correct scaling.
    let scaled_deviance = deviance / disp_phi;
    let scaled_pearson = pearson / disp_phi;

    // Information criteria — n_params excludes Scale for Poisson/Binomial
    // (scale fixed, not estimated), but for Normal Scale is estimated via
    // a moment estimator (not MLE parameter in the LL); SAS GENMOD uses p_param
    // (just regression params) for AIC/BIC of all three distributions.
    let n_params = p_param; // intercept + predictors (no scale term)
    let aic = -2.0 * log_lik + 2.0 * (n_params as f64);
    let aicc =
        aic + 2.0 * (n_params as f64) * (n_params as f64 + 1.0) / (n_total - n_params as f64 - 1.0);
    let bic = -2.0 * log_lik + (n_params as f64) * n_total.ln();

    FitCriteria {
        log_lik,
        deviance,
        pearson,
        scaled_deviance,
        scaled_pearson,
        df_gof,
        aic,
        aicc,
        bic,
    }
}
