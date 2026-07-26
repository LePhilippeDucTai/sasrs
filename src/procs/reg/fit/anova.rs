use super::*;

/// MQ5.2 — one model's ANOVA / fit-statistics numbers, computed once and
/// shared by the ANOVA-table and fit-statistics printers.
#[derive(Clone, Copy)]
pub(super) struct AnovaStats {
    pub(super) y_mean: f64,
    pub(super) ssm: f64,
    pub(super) sst: f64,
    pub(super) model_df: f64,
    pub(super) error_df: f64,
    pub(super) total_df: f64,
    pub(super) total_label: &'static str,
    pub(super) r2: f64,
    pub(super) adj_r2: f64,
    pub(super) msm: f64,
    pub(super) mse: f64,
    pub(super) f_stat: f64,
    pub(super) p_f: f64,
    pub(super) root_mse: f64,
    pub(super) cv: f64,
}

/// MQ5.2 — the ANOVA decomposition (corrected with an intercept, uncorrected
/// for NOINT), the derived F test, and the fit statistics.
#[allow(clippy::too_many_arguments)]
pub(super) fn compute_anova_stats(
    intercept: bool,
    y: &[f64],
    wts: &[f64],
    y_hat: &[f64],
    sse: f64,
    p: usize,
    restrict_q: usize,
    n_used: f64,
) -> AnovaStats {
    let n = y.len();
    let p_eff = p + intercept as usize;
    let sum_w: f64 = wts.iter().sum();
    // Weighted ("Dependent") mean ȳ_w = Σw_iy_i/Σw_i.
    let y_mean = {
        let sw: f64 = y.iter().zip(wts.iter()).map(|(yi, w)| w * yi).sum();
        if sum_w > 0.0 {
            sw / sum_w
        } else {
            y.iter().sum::<f64>() / n as f64
        }
    };

    // --- ANOVA decomposition ---
    let (ssm, sst, model_df, error_df, total_df, total_label, r2, adj_r2);
    if intercept {
        // Corrected (weighted) sums of squares: SST_w = Σ w_i (y_i−ȳ_w)².
        sst = y
            .iter()
            .zip(wts.iter())
            .map(|(yi, w)| w * (yi - y_mean) * (yi - y_mean))
            .sum();
        ssm = sst - sse;
        model_df = (p - restrict_q) as f64;
        error_df = n_used - p_eff as f64 + restrict_q as f64;
        total_df = n_used - 1.0;
        total_label = "Corrected Total";
        r2 = if sst > 0.0 { ssm / sst } else { f64::NAN };
        adj_r2 = if sst > 0.0 {
            1.0 - (1.0 - r2) * (n_used - 1.0) / error_df
        } else {
            f64::NAN
        };
    } else {
        // Uncorrected (weighted) sums of squares (NOINT).
        let sst_unc: f64 = y.iter().zip(wts.iter()).map(|(yi, w)| w * yi * yi).sum();
        let ssm_unc: f64 = y_hat
            .iter()
            .zip(wts.iter())
            .map(|(yh, w)| w * yh * yh)
            .sum();
        sst = sst_unc;
        ssm = ssm_unc;
        model_df = (p - restrict_q) as f64;
        error_df = n_used - p as f64 + restrict_q as f64;
        total_df = n_used;
        total_label = "Uncorrected Total";
        r2 = if sst > 0.0 { ssm / sst } else { f64::NAN };
        adj_r2 = if sst > 0.0 {
            1.0 - (1.0 - r2) * n_used / (n_used - p as f64)
        } else {
            f64::NAN
        };
    }

    let msm = if model_df > 0.0 {
        ssm / model_df
    } else {
        f64::NAN
    };
    let mse = sse / error_df;
    let f_stat = if mse > 0.0 { msm / mse } else { f64::NAN };
    let p_f = (1.0 - f_cdf(f_stat, model_df, error_df)).clamp(0.0, 1.0);

    let root_mse = mse.sqrt();
    let cv = if y_mean.abs() > 1e-15 {
        root_mse / y_mean.abs() * 100.0
    } else {
        f64::NAN
    };

    AnovaStats {
        y_mean,
        ssm,
        sst,
        model_df,
        error_df,
        total_df,
        total_label,
        r2,
        adj_r2,
        msm,
        mse,
        f_stat,
        p_f,
        root_mse,
        cv,
    }
}

// --- Standard errors / t / p for each beta ---
// For the restricted fit these come from the constrained covariance matrix
// computed in compute_restricted; otherwise from the usual MSE·(X'X)⁻¹.
pub(super) fn compute_beta_tests(
    restricted: Option<&Restricted>,
    fit: &OlsFit,
    beta: &[f64],
    mse: f64,
    p_eff: usize,
    error_df: f64,
) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    match restricted {
        Some(r) => (r.se_r.clone(), r.t_r.clone(), r.p_r.clone()),
        None => {
            let mut se_beta = Vec::with_capacity(p_eff);
            let mut t_beta = Vec::with_capacity(p_eff);
            let mut p_beta = Vec::with_capacity(p_eff);
            for j in 0..p_eff {
                let se = (mse * fit.xtx_inv[j][j]).sqrt();
                let t = beta[j] / se;
                let pv = two_sided_p(t, error_df);
                se_beta.push(se);
                t_beta.push(t);
                p_beta.push(pv);
            }
            (se_beta, t_beta, p_beta)
        }
    }
}
