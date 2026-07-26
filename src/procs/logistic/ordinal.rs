use super::*;

// ───────────────────────── Ordinal (cumulative-logit) ─────────────────────────

/// Proportional-odds cumulative-logit model for an ordered response with k>2
/// levels: intercepts α_1 < … < α_{k−1} plus a SHARED slope vector β, fit by
/// Newton-Raphson on the cumulative-logit log-likelihood.
///
/// Parameter layout in `theta`: [α_1, …, α_{k−1}, β_1, …, β_m].
/// Cumulative model: P(Y ≤ j) = logit⁻¹(α_j + x'β) using ordered categories
/// 1..k (category 1 = lowest in `sas_cmp` order). SAS by default orders
/// FORMATTED values ascending and models P(Y ≤ j); DESCENDING reverses.
///
/// Deferrals (documented):
/// - The "Score Test for the Proportional Odds Assumption" is NOT computed; a
///   deferral NOTE is emitted instead.
/// - OUTPUT predicted = P(Y in lowest modeled cumulative category) = P(Y = 1).
#[allow(clippy::too_many_arguments)]
///
/// Listwise deletion + design build for the ordinal model: category indices
/// (1..=k), design rows (NO intercept column), frequencies, and the
/// complete-case mask (for OUTPUT OUT=).
pub(super) fn build_ordinal_matrices(
    design: &Design,
    pred_cols: &[Vec<Value>],
    resp_col: &[Value],
    freq_col: &Option<Vec<Value>>,
    ordered_levels: &[&Value],
    n_read: usize,
    nb_cols: usize,
) -> (Vec<usize>, Vec<Vec<f64>>, Vec<f64>, Vec<bool>) {
    let mut cat_vec: Vec<usize> = Vec::new(); // 1..=k
    let mut x_mat: Vec<Vec<f64>> = Vec::new(); // design columns (NO intercept)
    let mut freq_vec: Vec<f64> = Vec::new();
    let mut complete_mask: Vec<bool> = vec![false; n_read];

    for i in 0..n_read {
        if resp_col[i].is_missing() {
            continue;
        }
        let w = if let Some(fc) = freq_col {
            match value_to_num(&fc[i]) {
                Some(f) if !f.is_nan() && f > 0.0 => f,
                _ => continue,
            }
        } else {
            1.0
        };
        let mut row: Vec<f64> = Vec::with_capacity(nb_cols);
        let mut ok = true;
        for eff in &design.effects {
            let col = &pred_cols[eff.pred_col_idx];
            if eff.is_class {
                let v = &col[i];
                if v.is_missing() {
                    ok = false;
                    break;
                }
                for lv in &eff.levels {
                    row.push(if v.sas_cmp(lv) == std::cmp::Ordering::Equal {
                        1.0
                    } else {
                        0.0
                    });
                }
            } else {
                match value_to_num(&col[i]) {
                    Some(v) if !v.is_nan() => row.push(v),
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
        }
        if !ok {
            continue;
        }
        // Category index (1..=k) in the ordered scheme.
        let cat = ordered_levels
            .iter()
            .position(|lv| lv.sas_cmp(&resp_col[i]) == std::cmp::Ordering::Equal)
            .map(|p| p + 1);
        let cat = match cat {
            Some(c) => c,
            None => continue,
        };
        cat_vec.push(cat);
        x_mat.push(row);
        freq_vec.push(w);
        complete_mask[i] = true;
    }

    (cat_vec, x_mat, freq_vec, complete_mask)
}

/// Newton-Raphson proportional-odds fit results.
pub(super) struct OrdinalFit {
    pub(super) theta: Vec<f64>,
    pub(super) converged: bool,
    pub(super) se: Vec<f64>,
    pub(super) wald: Vec<f64>,
    pub(super) wald_p: Vec<f64>,
}

/// Newton-Raphson on the cumulative-logit log-likelihood.
///
/// P(Y ≤ j) = σ(α_j + x'β). Initialise α at the empirical cumulative
/// logits and β = 0.
#[allow(clippy::too_many_arguments)]
pub(super) fn fit_ordinal(
    session: &mut Session,
    cat_vec: &[usize],
    x_mat: &[Vec<f64>],
    freq_vec: &[f64],
    n_total: f64,
    n_int: usize,
    nb_cols: usize,
    k: usize,
) -> OrdinalFit {
    let n_obs = cat_vec.len();
    let n_par = n_int + nb_cols;

    let mut theta = vec![0.0_f64; n_par];
    {
        // Empirical cumulative proportions (weighted).
        let mut cum = vec![0.0_f64; k];
        for i in 0..n_obs {
            cum[cat_vec[i] - 1] += freq_vec[i];
        }
        let mut running = 0.0;
        for j in 0..n_int {
            running += cum[j];
            let p = (running / n_total).clamp(1e-6, 1.0 - 1e-6);
            theta[j] = (p / (1.0 - p)).ln();
        }
    }

    let sigma = |z: f64| 1.0 / (1.0 + (-z).exp());
    let mut converged = false;
    for _iter in 0..25 {
        let mut grad = vec![0.0_f64; n_par];
        let mut hess = vec![vec![0.0_f64; n_par]; n_par];

        for i in 0..n_obs {
            let xi = &x_mat[i];
            let fi = freq_vec[i];
            let c = cat_vec[i]; // 1..=k
            let xb: f64 = xi
                .iter()
                .zip(theta[n_int..].iter())
                .map(|(x, b)| x * b)
                .sum();

            // Cumulative probs γ_j = P(Y ≤ j) = σ(α_j + xβ), j = 1..k-1
            // with γ_0 = 0, γ_k = 1. Probability of category c is γ_c − γ_{c-1}.
            let gamma = |j: usize| -> f64 {
                if j == 0 {
                    0.0
                } else if j >= k {
                    1.0
                } else {
                    sigma(theta[j - 1] + xb)
                }
            };
            let g_c = gamma(c);
            let g_cm1 = gamma(c - 1);
            let prob = (g_c - g_cm1).max(1e-12);

            // d σ(η)/dη = σ(1−σ).
            let dsig = |j: usize| -> f64 {
                if j == 0 || j >= k {
                    0.0
                } else {
                    let s = sigma(theta[j - 1] + xb);
                    s * (1.0 - s)
                }
            };
            let d_c = dsig(c);
            let d_cm1 = dsig(c - 1);

            // ∂logL/∂α_j: only α_{c} and α_{c-1} contribute.
            // ∂prob/∂α_{c} = d_c ; ∂prob/∂α_{c-1} = −d_{c-1}.
            let inv = fi / prob;
            if c <= n_int {
                grad[c - 1] += inv * d_c;
            }
            if c > 1 {
                grad[c - 2] += inv * (-d_cm1);
            }
            // ∂prob/∂β = (d_c − d_{c-1}) * x.
            let dprob_db = d_c - d_cm1;
            for (m, &xm) in xi.iter().enumerate() {
                grad[n_int + m] += inv * dprob_db * xm;
            }

            // Gauss-Newton / Fisher-style Hessian approximation: −(1/prob²)
            // outer product of ∂prob (expected-information style), summed.
            // Build the gradient-of-prob vector once.
            let mut dp = vec![0.0_f64; n_par];
            if c <= n_int {
                dp[c - 1] += d_c;
            }
            if c > 1 {
                dp[c - 2] += -d_cm1;
            }
            for (m, &xm) in xi.iter().enumerate() {
                dp[n_int + m] += dprob_db * xm;
            }
            let coef = fi / (prob * prob);
            for a in 0..n_par {
                if dp[a] == 0.0 {
                    continue;
                }
                for b in 0..n_par {
                    hess[a][b] -= coef * dp[a] * dp[b];
                }
            }
        }

        let neg_hess: Vec<Vec<f64>> = hess
            .iter()
            .map(|r| r.iter().map(|v| -v).collect())
            .collect();
        let inv = match invert_matrix(&neg_hess) {
            Ok(m) => m,
            Err(_) => break,
        };
        let delta = mat_vec(&inv, &grad);
        for j in 0..n_par {
            theta[j] += delta[j];
        }
        let max_delta = delta.iter().map(|d| d.abs()).fold(0.0_f64, f64::max);
        let max_t = theta.iter().map(|b| b.abs()).fold(0.0_f64, f64::max);
        if max_delta / (1.0 + max_t) < 1e-8 {
            converged = true;
            break;
        }
    }

    if !converged {
        session.log.note(
            "PROC LOGISTIC (ordinal): iteration limit reached without convergence \
             (possible separation).",
        );
    }

    // Variance-covariance for standard errors (final information).
    let var = ordinal_varcov(x_mat, cat_vec, freq_vec, &theta, n_int, nb_cols, k);
    let se: Vec<f64> = (0..n_par)
        .map(|j| var.get(j).map(|r| r[j]).unwrap_or(f64::NAN).max(0.0).sqrt())
        .collect();
    let wald: Vec<f64> = (0..n_par).map(|j| (theta[j] / se[j]).powi(2)).collect();
    let wald_p: Vec<f64> = wald.iter().map(|&w| chisq_sf(w, 1.0)).collect();

    OrdinalFit {
        theta,
        converged,
        se,
        wald,
        wald_p,
    }
}

/// Final-iterate variance-covariance for the ordinal model (inverse of the
/// observed information). Returns an `n_par × n_par` matrix; on inversion
/// failure returns NaNs so SEs degrade gracefully rather than panicking.
#[allow(clippy::too_many_arguments)]
pub(super) fn ordinal_varcov(
    x_mat: &[Vec<f64>],
    cat_vec: &[usize],
    freq_vec: &[f64],
    theta: &[f64],
    n_int: usize,
    nb_cols: usize,
    k: usize,
) -> Vec<Vec<f64>> {
    let n_par = n_int + nb_cols;
    let sigma = |z: f64| 1.0 / (1.0 + (-z).exp());
    let mut hess = vec![vec![0.0_f64; n_par]; n_par];
    for i in 0..x_mat.len() {
        let xi = &x_mat[i];
        let fi = freq_vec[i];
        let c = cat_vec[i];
        let xb: f64 = xi
            .iter()
            .zip(theta[n_int..].iter())
            .map(|(x, b)| x * b)
            .sum();
        let gamma = |j: usize| -> f64 {
            if j == 0 {
                0.0
            } else if j >= k {
                1.0
            } else {
                sigma(theta[j - 1] + xb)
            }
        };
        let dsig = |j: usize| -> f64 {
            if j == 0 || j >= k {
                0.0
            } else {
                let s = sigma(theta[j - 1] + xb);
                s * (1.0 - s)
            }
        };
        let prob = (gamma(c) - gamma(c - 1)).max(1e-12);
        let d_c = dsig(c);
        let d_cm1 = dsig(c - 1);
        let mut dp = vec![0.0_f64; n_par];
        if c <= n_int {
            dp[c - 1] += d_c;
        }
        if c > 1 {
            dp[c - 2] += -d_cm1;
        }
        let dprob_db = d_c - d_cm1;
        for (m, &xm) in xi.iter().enumerate() {
            dp[n_int + m] += dprob_db * xm;
        }
        let coef = fi / (prob * prob);
        for a in 0..n_par {
            if dp[a] == 0.0 {
                continue;
            }
            for b in 0..n_par {
                hess[a][b] += coef * dp[a] * dp[b];
            }
        }
    }
    invert_matrix(&hess).unwrap_or_else(|_| vec![vec![f64::NAN; n_par]; n_par])
}
