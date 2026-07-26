//! Statistiques par observation (M36.2) et diagnostics d'influence (M36.3).

use super::*;

mod report;

pub(crate) use report::*;

/// Per-observation std errors and CL limits for one used row (M36.2).
#[derive(Clone)]
pub(crate) struct ObsStat {
    pub(super) y: f64,
    pub(super) y_hat: f64,
    pub(super) stdp: f64,
    pub(super) stdi: f64,
    pub(super) stdr: f64,
    pub(super) lclm: f64,
    pub(super) uclm: f64,
    pub(super) lcl: f64,
    pub(super) ucl: f64,
}

/// Reconstruct the response vector y = ŷ + resid from a fit (avoids threading
/// the y vector into helpers that already carry the fit).
pub(crate) fn reconstruct_y(fit: &OlsFit) -> Vec<f64> {
    fit.y_hat
        .iter()
        .zip(fit.resid.iter())
        .map(|(yh, r)| yh + r)
        .collect()
}

/// Compute the per-observation statistics for every used row from the OLS fit.
/// `mse = sse/dfE`, `h_i` the leverage, `t = t_quantile(1−α/2, dfE)`.
pub(crate) fn compute_obs_stats(
    x_mat: &[Vec<f64>],
    y: &[f64],
    fit: &OlsFit,
    n: usize,
    p_eff: usize,
    alpha: f64,
    weighting: Option<&Weighting>,
) -> Vec<ObsStat> {
    // df / MSE use Σf_i with FREQ; the weighted hat is h_i = w_i·x_iᵀ(X'WX)⁻¹x_i.
    let (df_e, wts): (f64, Option<&[f64]>) = match weighting {
        Some(w) => (w.total_n - p_eff as f64, Some(&w.wf)),
        None => ((n - p_eff) as f64, None),
    };
    let mse = fit.sse / df_e;
    let t = t_quantile(1.0 - alpha / 2.0, df_e);
    let h0 = leverages(x_mat, &fit.xtx_inv);
    let h: Vec<f64> = match wts {
        Some(w) => h0.iter().zip(w.iter()).map(|(hi, wi)| hi * wi).collect(),
        None => h0,
    };
    (0..n)
        .map(|i| {
            let hi = h[i];
            let stdp = (mse * hi).sqrt();
            let stdi = (mse * (1.0 + hi)).sqrt();
            let stdr = (mse * (1.0 - hi)).max(0.0).sqrt();
            let yh = fit.y_hat[i];
            ObsStat {
                y: y[i],
                y_hat: yh,
                stdp,
                stdi,
                stdr,
                lclm: yh - t * stdp,
                uclm: yh + t * stdp,
                lcl: yh - t * stdi,
                ucl: yh + t * stdi,
            }
        })
        .collect()
}

/// Per-observation influence diagnostics (M36.3). Reuses the same leverage /
/// MSE / dfE infrastructure as `compute_obs_stats` (no duplicate fit).
///
/// `dfbetas[i]` has one entry per parameter (column order matches `fit.beta`:
/// intercept first if present). When `dfE ≤ 1`, RSTUDENT / COVRATIO / DFFITS /
/// DFBETAS are undefined (their leave-one-out variance `MSE_(i)` has 0 df) and
/// are reported as `NaN`; callers render the SAS sentinel `.`.
pub(crate) struct InfluenceStat {
    pub(super) y: f64,
    pub(super) y_hat: f64,
    pub(super) resid: f64,
    stdp: f64,
    pub(super) stdr: f64,
    pub(super) h: f64,
    pub(super) student: f64,
    pub(super) rstudent: f64,
    pub(super) cookd: f64,
    pub(super) press: f64,
    pub(super) dffits: f64,
    pub(super) covratio: f64,
    /// One DFBETAS per parameter, same column order as `fit.beta`.
    pub(super) dfbetas: Vec<f64>,
}

/// Compute the full influence-diagnostic set for every used row. `c = (X'X)⁻¹Xᵀ`
/// (p_eff × n) drives DFBETAS via the closed form
/// `DFBETAS_{ij} = (rstudent_i · c_{ji}) / √(Σ_k c_{jk}²)` — no leave-one-out
/// refits.
pub(crate) fn compute_influence_stats(
    x_mat: &[Vec<f64>],
    y: &[f64],
    fit: &OlsFit,
    n: usize,
    p_eff: usize,
    weighting: Option<&Weighting>,
) -> Vec<InfluenceStat> {
    let (df_e, wts): (f64, Option<&[f64]>) = match weighting {
        Some(w) => (w.total_n - p_eff as f64, Some(&w.wf)),
        None => ((n - p_eff) as f64, None),
    };
    let mse = fit.sse / df_e;
    let h0 = leverages(x_mat, &fit.xtx_inv);
    let h: Vec<f64> = match wts {
        Some(w) => h0.iter().zip(w.iter()).map(|(hi, wi)| hi * wi).collect(),
        None => h0,
    };

    // c = (X'X)⁻¹ Xᵀ  →  p_eff × n. Row j, col i is c_{ji}.
    let xt = linalg::transpose(x_mat); // p_eff × n
    let c = linalg::matrix_mult(&fit.xtx_inv, &xt); // (p_eff×p_eff)·(p_eff×n)
    // Row norms √(Σ_k c_{jk}²) for the DFBETAS denominator (= √((X'X)⁻¹_{jj})).
    let c_row_norm: Vec<f64> = (0..p_eff)
        .map(|j| c[j].iter().map(|v| v * v).sum::<f64>().sqrt())
        .collect();

    (0..n)
        .map(|i| {
            let hi = h[i];
            let yh = fit.y_hat[i];
            let resid = fit.resid[i];
            let one_minus_h = 1.0 - hi;
            let stdp = (mse * hi).sqrt();
            let stdr = (mse * one_minus_h).max(0.0).sqrt();
            // STUDENT = resid / STDR.
            let student = if stdr > 0.0 { resid / stdr } else { f64::NAN };
            // Leave-one-out MSE_(i): undefined when dfE ≤ 1.
            let (rstudent, mse_i_ok) = if df_e > 1.0 && one_minus_h > 0.0 {
                let mse_i = (df_e * mse - resid * resid / one_minus_h) / (df_e - 1.0);
                if mse_i > 0.0 {
                    (resid / (mse_i * one_minus_h).sqrt(), true)
                } else {
                    (f64::NAN, false)
                }
            } else {
                (f64::NAN, false)
            };
            // Cook's D = (student²/p)·(h/(1−h)).
            let cookd = if one_minus_h > 0.0 && p_eff > 0 {
                (student * student / p_eff as f64) * (hi / one_minus_h)
            } else {
                f64::NAN
            };
            let press = if one_minus_h != 0.0 {
                resid / one_minus_h
            } else {
                f64::NAN
            };
            let dffits = if mse_i_ok && one_minus_h > 0.0 {
                rstudent * (hi / one_minus_h).sqrt()
            } else {
                f64::NAN
            };
            // COVRATIO = 1 / ( ((dfE−1+rstudent²)/dfE)^p · (1−h) ).
            let covratio = if mse_i_ok && one_minus_h > 0.0 {
                let base = (df_e - 1.0 + rstudent * rstudent) / df_e;
                1.0 / (base.powi(p_eff as i32) * one_minus_h)
            } else {
                f64::NAN
            };
            // DFBETAS_{ij} = c_{ji}·rstudent_i / (√(1−h_i)·√((X'X)⁻¹_{jj})).
            // Here √(Σ_k c_{jk}²) = √((X'X)⁻¹_{jj}) since c·cᵀ = (X'X)⁻¹.
            // The extra √(1−h_i) converts e_i/s_(i) into rstudent_i (which
            // carries its own √(1−h_i)); see derivation in the milestone notes.
            let dfbetas: Vec<f64> = (0..p_eff)
                .map(|j| {
                    if mse_i_ok && c_row_norm[j] > 0.0 && one_minus_h > 0.0 {
                        rstudent * c[j][i] / (c_row_norm[j] * one_minus_h.sqrt())
                    } else {
                        f64::NAN
                    }
                })
                .collect();

            InfluenceStat {
                y: y[i],
                y_hat: yh,
                resid,
                stdp,
                stdr,
                h: hi,
                student,
                rstudent,
                cookd,
                press,
                dffits,
                covratio,
                dfbetas,
            }
        })
        .collect()
}
