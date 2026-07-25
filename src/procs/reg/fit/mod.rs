//! Cœur OLS : ajustement (pondéré ou non), leviers, stats séquentielles
//! (Type I/II, corrélations partielles) et impression du fit complet.

use super::*;


mod anova;
mod report;
mod estimates;

use anova::*;
use report::*;
use estimates::*;


mod ols;

pub(crate) use ols::*;

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

/// Context + optional statistics for the model-report printers
/// (`fit_and_print`, `fit_and_print_empty`, `fit_and_print_ridge_ipc`).
/// Groups the header context and the per-milestone optional blocks that had
/// accreted as positional parameters (M36.x). `Default` gives the plain
/// no-option path (all `None`), so partial call sites can use
/// `..Default::default()`.
#[derive(Default)]
pub(super) struct FitReportOptions<'a> {
    /// Number of observations read (header "Number of Observations Read").
    pub(super) n_read: usize,
    /// Number of complete-case rows used in the fit.
    pub(super) n: usize,
    /// Whether the model includes an intercept (i.e. not NOINT).
    pub(super) intercept: bool,
    /// "Model: MODELn" heading line.
    pub(super) model_label: &'a str,
    /// RESTRICT re-estimate (M36.1). `Some` ⇒ the printed model (ANOVA, R², F,
    /// parameter estimates) reflects the restricted fit.
    pub(super) restricted: Option<&'a Restricted>,
    /// Optional (tolerance, vif) per regressor (no intercept), in `reg_names`
    /// order. `Some` when MODEL VIF and/or TOL is requested (M36.4).
    pub(super) tolvif: Option<&'a (Vec<f64>, Vec<f64>)>,
    /// Optional partial-SS / correlation statistics (M36.5), indexed by design
    /// column (intercept first when present). `Some` when any of
    /// SS1/SS2/STB/PCORR1/PCORR2/SCORR1/SCORR2/SEQB is requested.
    pub(super) seqstats: Option<&'a SeqStats>,
    /// PRESS = Σ (resid_i/(1−h_i))² (M36.5). `Some` when MODEL PRESS is
    /// requested; printed as a fit statistic.
    pub(super) press_stat: Option<f64>,
    /// M36.7 WLS/FREQ context. `Some` ⇒ weighted ANOVA (weighted mean/SST, df
    /// from Σf_i, weighted-SSE-based MSE). `None` ⇒ plain OLS (byte-identical
    /// default path).
    pub(super) weighting: Option<&'a Weighting>,
    /// M36.7: BY-group heading, emitted right after "The REG Procedure" line.
    /// `None` ⇒ header block byte-identical to the prior (no-BY) path.
    pub(super) by_heading: Option<&'a str>,
}

/// Fit-and-print the full output block for a model (ANOVA + fit statistics +
/// parameter estimates). This is the SINGLE printer shared by the default,
/// NOINT, and SELECTION-final paths, guaranteeing byte-identical output for the
/// default case. `reg_names` are the regressor names actually in the model (no
/// intercept entry); `fit` was computed on a design matrix whose column order
/// matches: [intercept?] then `reg_names`.
pub(super) fn fit_and_print(
    model: &RegModel,
    dep_name: &str,
    reg_names: &[String],
    fit: &OlsFit,
    opts: &FitReportOptions,
    session: &mut Session,
) {
    let &FitReportOptions {
        n_read,
        n,
        intercept,
        model_label,
        restricted,
        tolvif,
        seqstats,
        press_stat,
        weighting,
        by_heading,
    } = opts;
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

    let p = reg_names.len();
    let p_eff = p + intercept as usize;
    // Restricted error df = (n−p_eff)+qr; this raises the Error-line DF and
    // lowers the Model DF by the number of restrictions.
    let restrict_q = restricted.map(|r| r.lambda_rows.len()).unwrap_or(0);

    // --- ANOVA decomposition + fit statistics — see `compute_anova_stats`.
    let stats = compute_anova_stats(intercept, &y, wts, y_hat, sse, p, restrict_q, n_used);
    let AnovaStats { error_df, mse, .. } = stats;

    // --- Standard errors / t / p for each beta — see `compute_beta_tests`.
    let (se_beta, t_beta, p_beta) = compute_beta_tests(restricted, fit, beta, mse, p_eff, error_df);

    if model.noprint {
        return;
    }

    print_report_header(n_read, n_used, model_label, by_heading, dep_name, session);
    print_anova_table(&stats, sse, session);
    print_fit_stats(&stats, press_stat, session);
    // Parameter estimates table — see `print_parameter_estimates`.
    print_parameter_estimates(
        model,
        reg_names,
        intercept,
        &PeTableCtx {
            beta,
            se_beta: &se_beta,
            t_beta: &t_beta,
            p_beta: &p_beta,
            error_df,
            p_eff,
            restricted,
            tolvif,
            seqstats,
        },
        session,
    );
}

/// Print the degenerate "no variables entered" case for SELECTION when the
/// selected set is empty.
pub(super) fn fit_and_print_empty(
    model: &RegModel,
    dep_name: &str,
    opts: &FitReportOptions,
    session: &mut Session,
) {
    if model.noprint {
        return;
    }
    session.listing.page_header();
    centered(session, "The REG Procedure");
    if let Some(h) = opts.by_heading {
        centered(session, h);
    }
    centered(session, opts.model_label);
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
