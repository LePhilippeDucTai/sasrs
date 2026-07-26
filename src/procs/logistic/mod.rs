//! PROC LOGISTIC — logistic regression via Newton-Raphson MLE (M26.1, M34.6).
//!
//! Supports:
//! - Binary response (two levels) and ordinal response (>2 ordered levels,
//!   proportional-odds cumulative-logit model) (M34.6).
//! - FREQ statement (weighted observations).
//! - MODEL statement with DESCENDING option and EVENT= option.
//! - CLASS variables: reference-cell (PARAM=REF, ref=last level) coding (M34.6).
//!   NOTE: SAS's default CLASS parameterization is EFFECT coding; we implement
//!   reference-cell coding (the GLM/`PARAM=REF` convention) instead. This is a
//!   documented deviation — the design columns and the resulting parameter
//!   estimates therefore match `PROC LOGISTIC ... (PARAM=REF REF=LAST)`.
//! - LINK= option: LOGIT (default), CLOGLOG, PROBIT (M34.6).
//! - OUTPUT OUT= PREDICTED=/P= XBETA= statement (M34.6).
//! - Produces: Model Information, Class Level Information (when CLASS present),
//!   Response Profile, Model Fit Statistics, Global Null tests (LR, Score, Wald),
//!   Analysis of ML Estimates, and (logit only) Odds Ratio Estimates.

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::fmt4;
use crate::procs::common::num_var_meta;
use crate::procs::common::{chisq_sf, decode_column, probnorm};
use crate::session::Session;
use crate::stat::invert_matrix;
use crate::stat::{dot, matrix_vec_mult};
use crate::token::TokenKind;
use crate::value::{Value, VarType, format_best};
use polars::prelude::{Column, DataFrame, NamedFrom, Series};

mod binary;
mod design;
mod ordinal;
mod ordinal_report;
mod output;
mod parse;
mod report;

pub use parse::parse;

use binary::*;
use design::*;
use ordinal::*;
use ordinal_report::*;
use output::*;
use report::*;

/// Link function for the (binary) logistic model. `Logit` is the default and
/// reproduces the canonical-link IRLS exactly; the other two branch only where
/// the mean/derivative differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Logit,
    Cloglog,
    Probit,
}

impl Link {
    /// Mean μ = g⁻¹(η).
    fn mean(self, eta: f64) -> f64 {
        match self {
            Link::Logit => 1.0 / (1.0 + (-eta).exp()),
            Link::Probit => probnorm(eta),
            Link::Cloglog => 1.0 - (-(eta.exp())).exp(),
        }
    }

    /// dμ/dη.
    fn dmu_deta(self, eta: f64) -> f64 {
        match self {
            Link::Logit => {
                let p = 1.0 / (1.0 + (-eta).exp());
                p * (1.0 - p)
            }
            // φ(η): standard normal pdf.
            Link::Probit => (-0.5 * eta * eta).exp() / (2.0 * std::f64::consts::PI).sqrt(),
            // exp(η − exp(η)).
            Link::Cloglog => (eta - eta.exp()).exp(),
        }
    }
}

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone)]
pub struct LogisticAst {
    pub data_options: LogisticDataOptions,
    pub class_vars: Vec<String>,
    pub model: Option<LogisticModel>,
    pub freq_var: Option<String>,
    pub outputs: Vec<LogisticOutput>,
}

#[derive(Debug, Clone)]
pub struct LogisticDataOptions {
    pub input: Option<DatasetRef>,
}

#[derive(Debug, Clone)]
pub struct LogisticModel {
    pub response: String,
    pub event: Option<String>,
    pub descending: bool,
    pub predictors: Vec<String>,
    pub noprint: bool,
    pub link: Link,
}

/// `OUTPUT OUT=ds PREDICTED=name | P=name [XBETA=name]`.
#[derive(Debug, Clone)]
pub struct LogisticOutput {
    pub out: DatasetRef,
    pub predicted: Option<String>,
    pub xbeta: Option<String>,
}

// ───────────────────────── Formatting helpers ─────────────────────────

use crate::procs::common::fmt_p_num as fmt_p_opt;

use crate::procs::common::centered;

// ───────────────────────── Value → display string ─────────────────────────

use crate::procs::common::value_label;

/// Compare a Value against an event string. Returns true if they match.
/// For numeric values, try parsing the event string as f64.
/// For char values, compare trimmed strings.
fn value_matches_event(v: &Value, event: &str) -> bool {
    match v {
        Value::Char(s) => s.trim_end() == event.trim(),
        Value::Num(f) => {
            // Try numeric parse
            if let Ok(ev_num) = event.trim().parse::<f64>() {
                (f - ev_num).abs() < 1e-15
            } else {
                // Fallback: compare string representations
                format_best(*f, 12) == event.trim()
            }
        }
        Value::Missing(_) => false,
    }
}

// ───────────────────────── Matrix helpers ─────────────────────────

pub fn execute(ast: &LogisticAst, session: &mut Session) -> Result<()> {
    // ── 1. Guards ──────────────────────────────────────────────────────────
    let model = ast
        .model
        .as_ref()
        .ok_or_else(|| SasError::runtime("MODEL statement required"))?;

    // ── 2. Read dataset ────────────────────────────────────────────────────
    let (ds, in_libref, in_table) = common::open_input(&ast.data_options.input, session)?;

    let n_read = ds.n_obs();
    session.log.note(&format!(
        "There were {} observations read from the data set {}.{}.",
        n_read, in_libref, in_table
    ));

    let resp_name = &model.response;
    let predictors = &model.predictors;
    let nb_preds = predictors.len();

    // ── Find column indices ────────────────────────────────────────────────
    let find_col = |nm: &str| -> Result<usize> {
        ds.vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(nm))
            .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", nm.to_uppercase())))
    };

    let resp_idx = find_col(resp_name)?;
    let mut pred_idxs: Vec<usize> = Vec::with_capacity(nb_preds);
    for nm in predictors {
        pred_idxs.push(find_col(nm)?);
    }
    let freq_idx: Option<usize> = if let Some(fv) = &ast.freq_var {
        Some(find_col(fv)?)
    } else {
        None
    };

    // ── Decode columns ─────────────────────────────────────────────────────
    let resp_col = decode_column(&ds, resp_idx)?;
    let mut pred_cols: Vec<Vec<Value>> = Vec::with_capacity(nb_preds);
    for &idx in &pred_idxs {
        pred_cols.push(decode_column(&ds, idx)?);
    }
    let freq_col: Option<Vec<Value>> = if let Some(fi) = freq_idx {
        Some(decode_column(&ds, fi)?)
    } else {
        None
    };

    // ── Build design (CLASS expansion via reference-cell coding) ───────────
    let design = build_design(&ast.class_vars, predictors, &pred_cols, n_read)?;

    // ── 3. Determine response levels (sorted by sas_cmp) ───────────────────
    let levels = crate::procs::lincom::class_levels(resp_col.iter().take(n_read));

    if levels.len() < 2 {
        return Err(SasError::runtime(format!(
            "Response variable {} must have at least 2 non-missing levels (found {}).",
            resp_name.to_uppercase(),
            levels.len()
        )));
    }

    // ── Ordinal branch: >2 ordered response levels → proportional-odds. ────
    if levels.len() > 2 {
        if model.link != Link::Logit {
            return Err(SasError::runtime(
                "Ordinal (multi-level) response is only supported with LINK=LOGIT.",
            ));
        }
        return execute_ordinal(
            ast, session, model, &ds, &in_libref, &in_table, resp_name, &resp_col, &pred_cols,
            &freq_col, &levels, &design, n_read,
        );
    }

    // ── 3b. Binary: determine event level (P(Y=event) is modeled) ──────────
    let (event_level, event_label, nonevent_label) =
        determine_event_level(model, &levels, resp_name)?;

    // Number of non-intercept design columns (CLASS-expanded).
    let nb_cols = design.n_cols();

    // ── 4. Listwise deletion + encoding ───────────────────────────────────
    let (y_vec, x_mat, freq_vec, complete_mask) = build_model_matrices(
        &design,
        &pred_cols,
        &resp_col,
        &freq_col,
        &event_level,
        n_read,
    );

    let n_total: f64 = freq_vec.iter().sum();
    let n_event_total: f64 = y_vec.iter().zip(freq_vec.iter()).map(|(y, w)| y * w).sum();
    let n_nonevent_total = n_total - n_event_total;
    let n_obs = y_vec.len();

    // ── 5. Manual NOTE ────────────────────────────────────────────────────
    session
        .log
        .note(&format!("There were {} observations used.", n_total as i64));

    if n_obs <= nb_cols + 1 {
        return Err(SasError::runtime(
            "Not enough observations for logistic regression",
        ));
    }

    // Model-description string: "binary logit" stays byte-identical for the
    // default link; other links read "binary <link-name>".
    let model_desc = match model.link {
        Link::Logit => "binary logit".to_string(),
        Link::Probit => "binary probit".to_string(),
        Link::Cloglog => "binary cloglog".to_string(),
    };

    // ── 6-8. Listing: header, model info, class levels, response profile ──
    if !model.noprint {
        print_model_information(session, &in_libref, &in_table, resp_name, &model_desc);
        write_class_level_info(session, &design);
        print_response_profile(
            session,
            resp_name,
            &event_label,
            &nonevent_label,
            n_event_total,
            n_nonevent_total,
        );
    }

    // ── 9. NR/IRLS Algorithm ─────────────────────────────────────────────
    let p_bar = n_event_total / n_total;
    let p_param = 1 + nb_cols; // number of parameters (with intercept)
    let link = model.link;
    let BinaryFit {
        beta,
        converged,
        var_beta,
        se_beta,
        wald_chi2,
        wald_p,
        final_p,
    } = fit_binary(session, &y_vec, &x_mat, &freq_vec, link, p_bar, p_param)?;

    // ── 11. Log-likelihoods ───────────────────────────────────────────────
    let log_l: f64 = (0..n_obs)
        .map(|i| {
            let pi = final_p[i];
            let fi = freq_vec[i];
            fi * (y_vec[i] * pi.ln() + (1.0 - y_vec[i]) * (1.0 - pi).ln())
        })
        .sum();

    let log_l_null: f64 = (0..n_obs)
        .map(|i| {
            let fi = freq_vec[i];
            fi * (y_vec[i] * p_bar.ln() + (1.0 - y_vec[i]) * (1.0 - p_bar).ln())
        })
        .sum();

    let neg2log_l = -2.0 * log_l;
    let neg2log_l_null = -2.0 * log_l_null;

    // ── 12. AIC / SC ─────────────────────────────────────────────────────
    let aic = neg2log_l + 2.0 * p_param as f64;
    let sc = neg2log_l + p_param as f64 * n_total.ln();
    let aic_null = neg2log_l_null + 2.0; // 1 parameter (intercept only)
    let sc_null = neg2log_l_null + n_total.ln();

    // ── 13. Global tests under H₀: BETA=0 ────────────────────────────────
    // LR chi2
    let lr_chi2 = neg2log_l_null - neg2log_l;
    let lr_p = chisq_sf(lr_chi2, nb_cols as f64);

    let wald_chi2_global = wald_global_test(&beta, &var_beta, nb_cols)?;
    let wald_global_p = chisq_sf(wald_chi2_global, nb_cols as f64);

    let score_chi2 = score_global_test(&y_vec, &x_mat, &freq_vec, p_bar, n_total, nb_cols)?;
    let score_p = chisq_sf(score_chi2, nb_cols as f64);

    // ── Listing (remaining sections) ─────────────────────────────────────
    if !model.noprint {
        print_convergence_status(session, converged);
        print_model_fit_statistics(
            session,
            aic_null,
            aic,
            sc_null,
            sc,
            neg2log_l_null,
            neg2log_l,
        );
        print_global_tests(
            session,
            nb_cols,
            lr_chi2,
            lr_p,
            score_chi2,
            score_p,
            wald_chi2_global,
            wald_global_p,
        );
        print_parameter_estimates(session, &design, &beta, &se_beta, &wald_chi2, &wald_p);
        // Odds ratios are only meaningful (exp(β) = odds multiplier) for the
        // logit link, so the table is omitted for PROBIT/CLOGLOG (documented).
        if nb_cols > 0 && model.link == Link::Logit {
            print_odds_ratios(session, &design, &beta, &se_beta);
        }
    }

    // ── OUTPUT OUT= ────────────────────────────────────────────────────────
    // Binary: predicted = P(Y = event). xbeta = linear predictor η.
    let preds_out: Vec<f64> = (0..n_obs).map(|i| final_p[i]).collect();
    let xbeta_out: Vec<f64> = x_mat
        .iter()
        .map(|xi| xi.iter().zip(beta.iter()).map(|(x, b)| x * b).sum())
        .collect();
    write_outputs(
        &ast.outputs,
        &ds,
        &complete_mask,
        &preds_out,
        &xbeta_out,
        session,
    )?;

    Ok(())
}

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests;
