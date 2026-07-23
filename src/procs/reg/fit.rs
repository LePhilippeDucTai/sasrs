//! Cœur OLS : ajustement (pondéré ou non), leviers, stats séquentielles
//! (Type I/II, corrélations partielles) et impression du fit complet.

use super::*;

// ───────────────────────── OLS fit helper ─────────────────────────

/// Result of an ordinary-least-squares fit on a fully-numeric design matrix.
pub(super) struct OlsFit {
    /// Coefficient vector (one per column of X).
    pub(super) beta: Vec<f64>,
    /// Predicted values ŷ = Xβ.
    pub(super) y_hat: Vec<f64>,
    /// Residuals y − ŷ.
    pub(super) resid: Vec<f64>,
    /// Σ resid² (residual / error sum of squares).
    pub(super) sse: f64,
    /// (XᵀX)⁻¹.
    pub(super) xtx_inv: Vec<Vec<f64>>,
}

/// Fit OLS for the given design matrix `x` (rows are observations, columns are
/// regressors — the caller decides whether an intercept column is present) and
/// response `y`. Pure: no session / printing side effects.
pub(super) fn ols_fit(x: &[Vec<f64>], y: &[f64]) -> Result<OlsFit> {
    let beta = linalg::least_squares(x, y)?;
    let y_hat: Vec<f64> = x
        .iter()
        .map(|row| row.iter().zip(beta.iter()).map(|(xi, bi)| xi * bi).sum())
        .collect();
    let resid: Vec<f64> = y
        .iter()
        .zip(y_hat.iter())
        .map(|(yi, yhi)| yi - yhi)
        .collect();
    let sse: f64 = resid.iter().map(|r| r * r).sum();
    let xt = linalg::transpose(x);
    let xtx = linalg::matrix_mult(&xt, x);
    let xtx_inv = linalg::invert_matrix(&xtx)?;
    Ok(OlsFit {
        beta,
        y_hat,
        resid,
        sse,
        xtx_inv,
    })
}

/// M36.7 weighting context for weighted least squares / frequency replication.
pub(super) struct Weighting {
    /// Effective SS weight w_i·f_i per complete-case row (same order as y).
    pub(super) wf: Vec<f64>,
    /// Σ f_i — the observation count for n / degrees-of-freedom purposes (FREQ
    /// inflates this; WEIGHT alone leaves it equal to the row count).
    pub(super) total_n: f64,
}

/// Weighted-least-squares fit. Solves `X'WX β = X'Wy` with W = diag(wf_i) by
/// scaling each design row and `y` by √wf_i and reusing the OLS machinery, then
/// recomputes ŷ / residuals on the ORIGINAL scale and the weighted error sum of
/// squares SSEw = Σ wf_i e_i². The returned `OlsFit.xtx_inv` is `(X'WX)⁻¹`
/// (since the scaled cross-product is exactly X'WX) so all downstream SE /
/// covariance formulas use the weighted normal equations directly.
pub(super) fn weighted_ols_fit(x_mat: &[Vec<f64>], y: &[f64], wf: &[f64]) -> Result<OlsFit> {
    let n = y.len();
    let p = x_mat[0].len();
    let mut xs: Vec<Vec<f64>> = Vec::with_capacity(n);
    let mut ys: Vec<f64> = Vec::with_capacity(n);
    for i in 0..n {
        let s = wf[i].max(0.0).sqrt();
        let mut row = Vec::with_capacity(p);
        for j in 0..p {
            row.push(x_mat[i][j] * s);
        }
        xs.push(row);
        ys.push(y[i] * s);
    }
    // β and (X'WX)⁻¹ from the scaled normal equations.
    let scaled = ols_fit(&xs, &ys)?;
    let beta = scaled.beta;
    let xtx_inv = scaled.xtx_inv;
    // Original-scale predictions / residuals and the WEIGHTED SSE.
    let y_hat: Vec<f64> = x_mat
        .iter()
        .map(|row| row.iter().zip(beta.iter()).map(|(xi, bi)| xi * bi).sum())
        .collect();
    let resid: Vec<f64> = y
        .iter()
        .zip(y_hat.iter())
        .map(|(yi, yhi)| yi - yhi)
        .collect();
    let sse: f64 = resid
        .iter()
        .zip(wf.iter())
        .map(|(e, &w)| w * e * e)
        .sum();
    Ok(OlsFit {
        beta,
        y_hat,
        resid,
        sse,
        xtx_inv,
    })
}

/// Per-observation leverage h_i = x_iᵀ (X'X)⁻¹ x_i for every design row of
/// `x_mat`, given the already-computed `xtx_inv` (M36.2). Σ_i h_i == p_eff.
pub(super) fn leverages(x_mat: &[Vec<f64>], xtx_inv: &[Vec<f64>]) -> Vec<f64> {
    let p = xtx_inv.len();
    x_mat
        .iter()
        .map(|row| {
            // h = rowᵀ · (X'X)⁻¹ · row.
            let mut acc = 0.0;
            for a in 0..p {
                let mut inner = 0.0;
                for b in 0..p {
                    inner += xtx_inv[a][b] * row[b];
                }
                acc += row[a] * inner;
            }
            acc
        })
        .collect()
}

/// Compute SSE only for a candidate subset fit (used by SELECTION). Builds the
/// design matrix from `xcols` (columns of regressors, each length n) over the
/// `subset` of column indices, optionally prepending an intercept column.
/// Returns `None` if the fit is rank-deficient / not solvable.
pub(super) fn subset_sse(xcols: &[Vec<f64>], y: &[f64], subset: &[usize], intercept: bool) -> Option<f64> {
    let n = y.len();
    let mut x: Vec<Vec<f64>> = Vec::with_capacity(n);
    for i in 0..n {
        let mut row = Vec::with_capacity(subset.len() + intercept as usize);
        if intercept {
            row.push(1.0);
        }
        for &c in subset {
            row.push(xcols[c][i]);
        }
        x.push(row);
    }
    if x.is_empty() || x[0].is_empty() {
        return None;
    }
    ols_fit(&x, y).ok().map(|f| f.sse)
}

// ───────────────────────── Partial-SS / correlation stats (M36.5) ─────────────────────────

/// Per-design-column sums-of-squares & correlation statistics (M36.5). Every
/// vector is indexed by design column (column order == `fit.beta`: intercept
/// first when present, then the regressors in MODEL order). Only the requested
/// statistics are filled; unrequested ones are left as their `0.0` defaults and
/// never read.
pub(super) struct SeqStats {
    /// Type I (sequential) sum of squares per column.
    pub(super) ss1: Vec<f64>,
    /// Type II (partial) sum of squares per column.
    pub(super) ss2: Vec<f64>,
    /// Standardized estimate per column (intercept = 0).
    pub(super) stb: Vec<f64>,
    /// Squared partial correlation, Type I.
    pub(super) pcorr1: Vec<f64>,
    /// Squared partial correlation, Type II.
    pub(super) pcorr2: Vec<f64>,
    /// Squared semi-partial correlation, Type I.
    pub(super) scorr1: Vec<f64>,
    /// Squared semi-partial correlation, Type II.
    pub(super) scorr2: Vec<f64>,
    /// Sequential parameter estimate per column (coefficient of column j in the
    /// fit using columns 0..=j).
    pub(super) seqb: Vec<f64>,
}

/// Compute the M36.5 partial-SS / correlation statistics for the fitted model.
///
/// `x_mat` is the design matrix (column order == `fit.beta`: intercept first
/// when present, then the regressors), `y` the response, `fit` the OLS fit and
/// `mse = fit.sse / dfE`.
///
/// - **SS2** (Type II / partial): `β_j² / (X'X)⁻¹_{jj}` for every column,
///   intercept included (≡ t_j²·MSE).
/// - **SS1** (Type I / sequential): refit the model adding columns in design
///   order; `SS1_j` = SSE(cols 0..j) − SSE(cols 0..=j) = the increase in model
///   SS contributed by column j. For the first column the "before" SSE is the
///   uncorrected total Σy² (model with no columns predicts 0), so the
///   intercept's SS1 is the SS for the mean. `Σ SS1` over the regressors equals
///   the Model SS.
/// - **SEQB**: the coefficient of column j in the prefix fit using columns
///   0..=j (j is the last-added regressor). For the full model's last column
///   this equals its OLS β.
/// - **STB**: `β_j · sd(x_j)/sd(y)` (sample SDs); intercept = 0.
/// - **PCORR1** (Type I): `SS1_j / (SS1_j + SSE_incl_j)` where SSE_incl_j is the
///   residual SS of the prefix fit through column j (= SS1_j/SSE_before_j).
/// - **PCORR2** (Type II): `SS2_j / (SS2_j + SSE)`.
/// - **SCORR1** (Type I): `SS1_j / SST`.
/// - **SCORR2** (Type II): `SS2_j / SST`.
///
/// `sst` is the corrected total (intercept models) or uncorrected total (NOINT),
/// matching `fit_and_print`. All ratios are clamped to [0,1] for round-off
/// safety. Sequential fits are skipped (left at default) unless any Type I
/// statistic or SEQB is requested.
pub(super) fn compute_seq_stats(
    model: &RegModel,
    x_mat: &[Vec<f64>],
    y: &[f64],
    fit: &OlsFit,
    sst: f64,
    intercept: bool,
) -> SeqStats {
    let p_eff = x_mat[0].len();
    let n = y.len();
    let sse = fit.sse;

    let mut ss1 = vec![0.0; p_eff];
    let mut ss2 = vec![0.0; p_eff];
    let mut stb = vec![0.0; p_eff];
    let mut pcorr1 = vec![0.0; p_eff];
    let mut pcorr2 = vec![0.0; p_eff];
    let mut scorr1 = vec![0.0; p_eff];
    let mut scorr2 = vec![0.0; p_eff];
    let mut seqb = vec![0.0; p_eff];

    let need_type2 = model.ss2 || model.pcorr2 || model.scorr2;
    let need_type1 = model.ss1 || model.pcorr1 || model.scorr1 || model.seqb;

    // --- Type II (partial) SS and its derived correlations ---
    if need_type2 {
        for j in 0..p_eff {
            let cjj = fit.xtx_inv[j][j];
            let s = if cjj > 0.0 {
                fit.beta[j] * fit.beta[j] / cjj
            } else {
                0.0
            };
            ss2[j] = s;
            pcorr2[j] = if s + sse > 0.0 {
                (s / (s + sse)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            scorr2[j] = if sst > 0.0 {
                (s / sst).clamp(0.0, 1.0)
            } else {
                0.0
            };
        }
    }

    // --- Standardized estimates ---
    if model.stb {
        let sd_y = sample_sd(y);
        for j in 0..p_eff {
            // Intercept (column 0 when present) has STB = 0.
            let is_intercept = intercept && j == 0;
            if is_intercept {
                stb[j] = 0.0;
            } else {
                let col: Vec<f64> = (0..n).map(|i| x_mat[i][j]).collect();
                let sd_x = sample_sd(&col);
                stb[j] = if sd_y > 0.0 {
                    fit.beta[j] * sd_x / sd_y
                } else {
                    0.0
                };
            }
        }
    }

    // --- Type I (sequential) SS, SEQB and derived correlations ---
    if need_type1 {
        // SSE of the prefix model using columns 0..k (k columns). Column 0 of
        // this array (k=0) is the empty model: SSE = Σy² (uncorrected total).
        let mut sse_prefix = vec![0.0; p_eff + 1];
        sse_prefix[0] = y.iter().map(|v| v * v).sum();
        for k in 1..=p_eff {
            // Design matrix over columns 0..k.
            let mut xpre: Vec<Vec<f64>> = Vec::with_capacity(n);
            for i in 0..n {
                xpre.push(x_mat[i][0..k].to_vec());
            }
            match ols_fit(&xpre, y) {
                Ok(f) => {
                    sse_prefix[k] = f.sse;
                    // SEQB of column (k-1): the last coefficient of this fit.
                    seqb[k - 1] = f.beta[k - 1];
                }
                Err(_) => {
                    // Rank-deficient prefix: no reduction in SSE, SEQB undefined.
                    sse_prefix[k] = sse_prefix[k - 1];
                    seqb[k - 1] = f64::NAN;
                }
            }
        }
        for j in 0..p_eff {
            let before = sse_prefix[j];
            let after = sse_prefix[j + 1];
            let s = (before - after).max(0.0);
            ss1[j] = s;
            // PCORR1 = SS1_j / SSE_before_j (== SS1_j/(SS1_j+SSE_incl_j)).
            pcorr1[j] = if before > 0.0 {
                (s / before).clamp(0.0, 1.0)
            } else {
                0.0
            };
            scorr1[j] = if sst > 0.0 {
                (s / sst).clamp(0.0, 1.0)
            } else {
                0.0
            };
        }
    }

    SeqStats {
        ss1,
        ss2,
        stb,
        pcorr1,
        pcorr2,
        scorr1,
        scorr2,
        seqb,
    }
}

/// Sample standard deviation (divisor n−1). Returns 0 for fewer than 2 points.
pub(super) fn sample_sd(v: &[f64]) -> f64 {
    let n = v.len();
    if n < 2 {
        return 0.0;
    }
    let mean = v.iter().sum::<f64>() / n as f64;
    let ss: f64 = v.iter().map(|x| (x - mean) * (x - mean)).sum();
    (ss / (n as f64 - 1.0)).sqrt()
}

/// Fit-and-print the full output block for a model (ANOVA + fit statistics +
/// parameter estimates). This is the SINGLE printer shared by the default,
/// NOINT, and SELECTION-final paths, guaranteeing byte-identical output for the
/// default case. `reg_names` are the regressor names actually in the model (no
/// intercept entry); `fit` was computed on a design matrix whose column order
/// matches: [intercept?] then `reg_names`.
#[allow(clippy::too_many_arguments)]
pub(super) fn fit_and_print(
    model: &RegModel,
    dep_name: &str,
    reg_names: &[String],
    fit: &OlsFit,
    restricted: Option<&Restricted>,
    n_read: usize,
    n: usize,
    intercept: bool,
    model_label: &str,
    // `tolvif`: optional (tolerance, vif) per regressor (no intercept), in
    // `reg_names` order. `Some` when MODEL VIF and/or TOL is requested (M36.4).
    tolvif: Option<&(Vec<f64>, Vec<f64>)>,
    // `seqstats`: optional partial-SS / correlation statistics (M36.5), indexed
    // by design column (intercept first when present). `Some` when any of
    // SS1/SS2/STB/PCORR1/PCORR2/SCORR1/SCORR2/SEQB is requested.
    seqstats: Option<&SeqStats>,
    // `press_stat`: PRESS = Σ (resid_i/(1−h_i))² (M36.5). `Some` when MODEL PRESS
    // is requested; printed as a fit statistic.
    press_stat: Option<f64>,
    // `weighting`: M36.7 WLS/FREQ context. `Some` ⇒ weighted ANOVA (weighted
    // mean/SST, df from Σf_i, weighted-SSE-based MSE). `None` ⇒ plain OLS
    // (byte-identical default path).
    weighting: Option<&Weighting>,
    // M36.7: BY-group heading, emitted right after "The REG Procedure" line.
    // `None` ⇒ header block byte-identical to the prior (no-BY) path.
    by_heading: Option<&str>,
    session: &mut Session,
) {
    // When a restricted fit is present, the printed model (ANOVA, R², F, and
    // parameter estimates) reflects the RESTRICTed estimates β_r / SSE_r / df_r.
    let beta: &[f64] = match restricted {
        Some(r) => &r.beta_r,
        None => &fit.beta,
    };
    let sse = match restricted {
        Some(r) => r.sse_r,
        None => fit.sse,
    };
    let y_hat: &[f64] = match restricted {
        Some(r) => &r.y_hat_r,
        None => &fit.y_hat,
    };
    let resid: &[f64] = match restricted {
        Some(r) => &r.resid_r,
        None => &fit.resid,
    };

    // y vector reconstructed from ŷ + resid (avoids threading it in).
    let y: Vec<f64> = y_hat.iter().zip(resid.iter()).map(|(yh, r)| yh + r).collect();
    // Per-row weights (w_i·f_i). All-ones in the plain OLS / default path so the
    // weighted formulas below collapse to the original ones byte-for-byte.
    let ones = vec![1.0; n];
    let wts: &[f64] = match weighting {
        Some(w) => &w.wf,
        None => &ones,
    };
    // n_used drives degrees of freedom: Σf_i with FREQ (which inflates n/df);
    // the row count n when only WEIGHT (or neither) is present.
    let n_used: f64 = match weighting {
        Some(w) => w.total_n,
        None => n as f64,
    };
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

    let p = reg_names.len();
    let p_eff = p + intercept as usize;
    // Restricted error df = (n−p_eff)+qr; this raises the Error-line DF and
    // lowers the Model DF by the number of restrictions.
    let restrict_q = restricted.map(|r| r.lambda_rows.len()).unwrap_or(0);

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
        let sst_unc: f64 = y
            .iter()
            .zip(wts.iter())
            .map(|(yi, w)| w * yi * yi)
            .sum();
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

    let msm = if model_df > 0.0 { ssm / model_df } else { f64::NAN };
    let mse = sse / error_df;
    let f_stat = if mse > 0.0 { msm / mse } else { f64::NAN };
    let p_f = (1.0 - f_cdf(f_stat, model_df, error_df)).clamp(0.0, 1.0);

    let root_mse = mse.sqrt();
    let cv = if y_mean.abs() > 1e-15 {
        root_mse / y_mean.abs() * 100.0
    } else {
        f64::NAN
    };

    // --- Standard errors / t / p for each beta ---
    // For the restricted fit these come from the constrained covariance matrix
    // computed in compute_restricted; otherwise from the usual MSE·(X'X)⁻¹.
    let (se_beta, t_beta, p_beta): (Vec<f64>, Vec<f64>, Vec<f64>) = match restricted {
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
    };

    if model.noprint {
        return;
    }

    session.listing.page_header();
    centered(session, "The REG Procedure");
    if let Some(h) = by_heading {
        centered(session, h);
    }
    centered(session, model_label);
    centered(session, &format!("Dependent Variable: {}", dep_name));
    session.listing.blank();

    session.listing.write_line(&format!(
        "               Number of Observations Read         {}",
        n_read
    ));
    session.listing.write_line(&format!(
        "               Number of Observations Used         {}",
        n_used as usize
    ));
    session.listing.blank();
    session.listing.blank();

    centered(session, "Analysis of Variance");
    session.listing.blank();

    let anova_headers: Vec<String> = vec![
        "Source".into(),
        "DF".into(),
        "Sum of Squares".into(),
        "Mean Square".into(),
        "F Value".into(),
        "Pr > F".into(),
    ];
    let anova_aligns = vec![
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    let anova_rows: Vec<Vec<String>> = vec![
        vec![
            "Model".into(),
            format!("{}", model_df as usize),
            fmt5(ssm),
            fmt5(msm),
            fmt2(f_stat),
            fmt_p(Some(p_f)),
        ],
        vec![
            "Error".into(),
            format!("{}", error_df as usize),
            fmt5(sse),
            fmt5(mse),
            "".into(),
            "".into(),
        ],
        vec![
            total_label.into(),
            format!("{}", total_df as usize),
            fmt5(sst),
            "".into(),
            "".into(),
            "".into(),
        ],
    ];
    session
        .listing
        .write_table(&anova_headers, &anova_aligns, &anova_rows);
    session.listing.blank();
    session.listing.blank();

    // Fit statistics (written manually)
    session.listing.write_line(&format!(
        "Root MSE             {}    R-Square     {}",
        fmt5(root_mse),
        fmt_fit4(r2)
    ));
    session.listing.write_line(&format!(
        "Dependent Mean       {}    Adj R-Sq     {}",
        fmt5(y_mean),
        fmt_fit4(adj_r2)
    ));
    session
        .listing
        .write_line(&format!("Coeff Var            {}", fmt5(cv)));
    // PRESS statistic (M36.5): printed among the fit statistics when MODEL PRESS
    // is requested. This is independent of MODEL R, which prints its own
    // "Predicted Residual SS (PRESS)" line in the residual-analysis summary
    // block; both may appear and report the same value.
    if let Some(press) = press_stat {
        session
            .listing
            .write_line(&format!("PRESS                {}", fmt5(press)));
    }
    session.listing.blank();
    session.listing.blank();

    // Parameter estimates table. With RESTRICT statements a trailing Label
    // column carries the restriction expression; the unrestricted path keeps
    // the original 6-column layout byte-identical.
    let with_label = restricted.is_some();
    // CLB (M36.2): append two confidence-limit columns to the parameter table.
    let with_clb = model.clb;
    let clb_level = 100.0 * (1.0 - model.alpha);
    let t_crit = t_quantile(1.0 - model.alpha / 2.0, error_df);
    let mut pe_headers: Vec<String> = vec![
        "Variable".into(),
        "DF".into(),
        "Parameter Estimate".into(),
        "Standard Error".into(),
        "t Value".into(),
        "Pr > |t|".into(),
    ];
    let mut pe_aligns = vec![
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    if with_clb {
        pe_headers.push(format!("{}% Confidence Limits", fmt_level(clb_level)));
        pe_aligns.push(Align::Right);
        // The interval prints as two value columns under one spanning header;
        // emit a second (blank-titled) column to carry the upper limit.
        pe_headers.push(String::new());
        pe_aligns.push(Align::Right);
    }
    // VIF / TOL columns (M36.4). SAS orders Tolerance before Variance Inflation.
    let with_tol = model.tol && tolvif.is_some();
    let with_vif = model.vif && tolvif.is_some();
    if with_tol {
        pe_headers.push("Tolerance".into());
        pe_aligns.push(Align::Right);
    }
    if with_vif {
        pe_headers.push("Variance Inflation".into());
        pe_aligns.push(Align::Right);
    }
    // M36.5 partial-SS / correlation columns. SAS appends them in this order:
    // Type I SS, Type II SS, Standardized Estimate, Squared Partial Corr Type I,
    // Squared Partial Corr Type II, Squared Semi-partial Corr Type I, Squared
    // Semi-partial Corr Type II, Sequential Parameter Estimate.
    let with_seq = seqstats.is_some();
    if with_seq {
        if model.ss1 {
            pe_headers.push("Type I SS".into());
            pe_aligns.push(Align::Right);
        }
        if model.ss2 {
            pe_headers.push("Type II SS".into());
            pe_aligns.push(Align::Right);
        }
        if model.stb {
            pe_headers.push("Standardized Estimate".into());
            pe_aligns.push(Align::Right);
        }
        if model.pcorr1 {
            pe_headers.push("Squared Partial Corr Type I".into());
            pe_aligns.push(Align::Right);
        }
        if model.pcorr2 {
            pe_headers.push("Squared Partial Corr Type II".into());
            pe_aligns.push(Align::Right);
        }
        if model.scorr1 {
            pe_headers.push("Squared Semi-partial Corr Type I".into());
            pe_aligns.push(Align::Right);
        }
        if model.scorr2 {
            pe_headers.push("Squared Semi-partial Corr Type II".into());
            pe_aligns.push(Align::Right);
        }
        if model.seqb {
            pe_headers.push("Sequential Parameter Estimate".into());
            pe_aligns.push(Align::Right);
        }
    }
    if with_label {
        pe_headers.push("Label".into());
        pe_aligns.push(Align::Left);
    }
    let mut pe_rows: Vec<Vec<String>> = Vec::with_capacity(p_eff);
    for j in 0..p_eff {
        let var_name = if intercept {
            if j == 0 {
                "Intercept".to_string()
            } else {
                reg_names[j - 1].clone()
            }
        } else {
            reg_names[j].clone()
        };
        let mut row = vec![
            var_name,
            "1".into(),
            fmt5(beta[j]),
            fmt5(se_beta[j]),
            fmt2(t_beta[j]),
            fmt_p(Some(p_beta[j])),
        ];
        if with_clb {
            row.push(fmt5(beta[j] - t_crit * se_beta[j]));
            row.push(fmt5(beta[j] + t_crit * se_beta[j]));
        }
        if with_tol || with_vif {
            // Map design column j to a regressor index (intercept has none).
            let reg_idx = if intercept {
                if j == 0 {
                    None
                } else {
                    Some(j - 1)
                }
            } else {
                Some(j)
            };
            let (tv, vv) = tolvif.expect("tolvif present when columns requested");
            if with_tol {
                // Intercept row: Tolerance blank.
                match reg_idx {
                    Some(k) => row.push(fmt5(tv[k])),
                    None => row.push(String::new()),
                }
            }
            if with_vif {
                // Intercept row: SAS prints 0 for the intercept VIF.
                match reg_idx {
                    Some(k) => row.push(if vv[k].is_finite() {
                        fmt5(vv[k])
                    } else {
                        // Perfect collinearity → SAS prints a very large value;
                        // render a sentinel `.` for non-finite.
                        ".".to_string()
                    }),
                    None => row.push(fmt5(0.0)),
                }
            }
        }
        if let Some(ss) = seqstats {
            if model.ss1 {
                row.push(fmt5(ss.ss1[j]));
            }
            if model.ss2 {
                row.push(fmt5(ss.ss2[j]));
            }
            if model.stb {
                row.push(fmt5(ss.stb[j]));
            }
            if model.pcorr1 {
                row.push(fmt5(ss.pcorr1[j]));
            }
            if model.pcorr2 {
                row.push(fmt5(ss.pcorr2[j]));
            }
            if model.scorr1 {
                row.push(fmt5(ss.scorr1[j]));
            }
            if model.scorr2 {
                row.push(fmt5(ss.scorr2[j]));
            }
            if model.seqb {
                row.push(if ss.seqb[j].is_finite() {
                    fmt5(ss.seqb[j])
                } else {
                    ".".to_string()
                });
            }
        }
        if with_label {
            row.push(String::new());
        }
        pe_rows.push(row);
    }
    // Append RESTRICT rows: Variable="RESTRICT", DF=-1 (negative per SAS),
    // Estimate=λ_i, with the restriction expression in the Label column.
    if let Some(r) = restricted {
        for (label, lam, se, t, pv) in &r.lambda_rows {
            let mut row = vec![
                "RESTRICT".into(),
                "-1".into(),
                fmt5(*lam),
                fmt5(*se),
                fmt2(*t),
                fmt_p(Some(*pv)),
            ];
            if with_clb {
                // SAS leaves the confidence-limit cells blank for RESTRICT rows.
                row.push(String::new());
                row.push(String::new());
            }
            if with_tol {
                row.push(String::new());
            }
            if with_vif {
                row.push(String::new());
            }
            if with_seq {
                // SAS leaves the M36.5 partial-SS / correlation cells blank for
                // RESTRICT rows.
                for present in [
                    model.ss1,
                    model.ss2,
                    model.stb,
                    model.pcorr1,
                    model.pcorr2,
                    model.scorr1,
                    model.scorr2,
                    model.seqb,
                ] {
                    if present {
                        row.push(String::new());
                    }
                }
            }
            row.push(label.clone());
            pe_rows.push(row);
        }
    }
    session
        .listing
        .write_table(&pe_headers, &pe_aligns, &pe_rows);
}

/// Print the degenerate "no variables entered" case for SELECTION when the
/// selected set is empty.
pub(super) fn fit_and_print_empty(
    model: &RegModel,
    dep_name: &str,
    _n_read: usize,
    _n: usize,
    model_label: &str,
    by_heading: Option<&str>,
    session: &mut Session,
) {
    if model.noprint {
        return;
    }
    session.listing.page_header();
    centered(session, "The REG Procedure");
    if let Some(h) = by_heading {
        centered(session, h);
    }
    centered(session, model_label);
    centered(session, &format!("Dependent Variable: {}", dep_name));
    session.listing.blank();
    if model.noint {
        centered(
            session,
            "No variables met the entry criterion; no model was fit.",
        );
    } else {
        centered(
            session,
            "No variables met the entry criterion; intercept-only model.",
        );
    }
    session.listing.blank();
}
