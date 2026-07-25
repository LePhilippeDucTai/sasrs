//! PROC GENMOD — Generalized Linear Models via Newton-Raphson / IRLS (M26.2).
//!
//! Supports:
//! - DIST=POISSON (link=LOG, canonical)
//! - DIST=BINOMIAL (link=LOGIT, canonical)
//! - DIST=NORMAL (link=IDENTITY, canonical)
//! - DIST=GAMMA — deferred, parse OK but execute returns error
//! - FREQ statement (weighted observations).
//! - MODEL statement with EVENT= and DESCENDING options (Binomial).
//! - Produces: Model Information, Response Profile (Binomial only),
//!   Model Convergence Status, Criteria For Assessing Goodness Of Fit
//!   (Deviance/Pearson/LL/AIC/AICC/BIC), Analysis Of Maximum Likelihood
//!   Parameter Estimates (β/SE/Wald CI/Wald χ²/p), Scale parameter row.

use std::f64::consts::PI;

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::{chisq_sf, decode_column};
use crate::session::Session;
use crate::stat::invert_matrix;
use crate::token::TokenKind;
use crate::value::{format_best, Value};


mod link;
mod parse;
mod design;
mod fit;
mod report;
use link::*;
pub use parse::parse;
use design::*;
use fit::*;
use report::*;

// ───────────────────────── AST ─────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum Distribution {
    Poisson,
    Binomial,
    Normal,
    Gamma, // deferred — parse OK, execute errors
}

#[derive(Clone, Debug, PartialEq)]
pub enum LinkFunction {
    Log,
    Logit,
    Identity,
    /// Reciprocal (inverse) link: η = 1/μ, μ = 1/η — Gamma canonical link.
    Reciprocal,
}

#[derive(Debug, Clone)]
pub struct GenmodAst {
    pub data_options: GenmodDataOptions,
    pub model: Option<GenmodModel>,
    pub freq_var: Option<String>,
    pub class_vars: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GenmodDataOptions {
    pub input: Option<DatasetRef>,
}

#[derive(Debug, Clone)]
pub struct GenmodModel {
    pub response: String,
    pub event: Option<String>,
    pub descending: bool,
    pub predictors: Vec<String>,
    pub dist: Distribution,
    pub link: LinkFunction,
    pub noprint: bool,
    /// MODEL ... / SCALE=value — fix the dispersion at this value instead of
    /// estimating it (Normal/Gamma). `None` → estimate.
    pub scale: Option<f64>,
    /// MODEL ... / NOSCALE — hold the scale fixed at 1 (Normal/Gamma) rather
    /// than estimating it. Combined with SCALE= it fixes at the given value.
    pub noscale: bool,
}

// ───────────────────────── Formatting helpers ─────────────────────────

fn fmt4(v: f64) -> String {
    format!("{v:.4}")
}

use crate::procs::common::fmt_p_num as fmt_p_opt;


use crate::procs::common::centered;

// ───────────────────────── Value helpers ─────────────────────────

use crate::procs::common::value_label;


fn value_matches_event(v: &Value, event: &str) -> bool {
    match v {
        Value::Char(s) => s.trim_end() == event.trim(),
        Value::Num(f) => {
            if let Ok(ev_num) = event.trim().parse::<f64>() {
                (f - ev_num).abs() < 1e-15
            } else {
                format_best(*f, 12) == event.trim()
            }
        }
        Value::Missing(_) => false,
    }
}

// ───────────────────────── Matrix helpers ─────────────────────────

fn mat_vec(mat: &[Vec<f64>], vec: &[f64]) -> Vec<f64> {
    mat.iter()
        .map(|row| row.iter().zip(vec.iter()).map(|(a, b)| a * b).sum())
        .collect()
}

// ───────────────────────── Execute ─────────────────────────

pub fn execute(ast: &GenmodAst, session: &mut Session) -> Result<()> {
    // ── 1. Guards ──────────────────────────────────────────────────────────
    let model = ast.model.as_ref().ok_or_else(|| {
        SasError::runtime("MODEL statement required for PROC GENMOD")
    })?;

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
    let dist = &model.dist;
    let lf = &model.link;

    // ── Find column indices ────────────────────────────────────────────────
    let find_col = |nm: &str| -> Result<usize> {
        ds.vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(nm))
            .ok_or_else(|| {
                SasError::runtime(format!("Variable {} not found.", nm.to_uppercase()))
            })
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

    // ── Build design terms (CLASS reference-cell coding, ref = last level) ──
    let design_terms = build_design_terms(ast, &ds, predictors, &pred_idxs, &pred_cols)?;
    let n_design: usize = design_terms.iter().map(|t| t.n_cols()).sum();

    // ── 3. Prepare response for Binomial (determine event level) ──────────
    let BinomialResponse {
        event_level: binomial_event_level,
        event_label: binomial_event_label,
        nonevent_label: binomial_nonevent_label,
        n_event_total: binomial_n_event_total,
        n_nonevent_total: binomial_n_nonevent_total,
    } = prepare_binomial_response(model, &resp_col, &freq_col, n_read, resp_name)?;

    // ── 4. Listwise deletion + encoding ───────────────────────────────────
    let (y_vec, x_mat, freq_vec) = build_model_matrices(
        &design_terms,
        &pred_cols,
        &resp_col,
        &freq_col,
        dist,
        &binomial_event_level,
        n_read,
    );

    let n_total: f64 = freq_vec.iter().sum();
    let n_obs = y_vec.len();

    session.log.note(&format!(
        "There were {} observations used.",
        n_total as i64
    ));

    let p_param = 1 + n_design; // intercept + design columns

    if n_obs <= n_design {
        return Err(SasError::runtime(
            "Not enough observations for PROC GENMOD",
        ));
    }

    // ── 5-8. Listing: header, model info, class levels, response profile ──
    if !model.noprint {
        print_model_information(session, &in_libref, &in_table, resp_name, dist, lf, n_total);
        if !ast.class_vars.is_empty() {
            print_class_level_information(session, &design_terms);
        }
        if *dist == Distribution::Binomial {
            print_response_profile(
                session,
                resp_name,
                &binomial_event_label,
                &binomial_nonevent_label,
                binomial_n_event_total,
                binomial_n_nonevent_total,
            );
        }
        print_convergence_status(session);
    }

    // ── 9. IRLS / Newton-Raphson ──────────────────────────────────────────
    let (beta, h_inv, final_mu) =
        fit_irls(session, &y_vec, &x_mat, &freq_vec, dist, lf, n_total, p_param)?;

    // ── 10. Scale / Dispersion ────────────────────────────────────────────
    let scale = compute_scale(model, dist, &y_vec, &final_mu, &freq_vec, h_inv, n_total, p_param);

    // ── 11. SE, Wald chi², CI ─────────────────────────────────────────────
    let var_beta = &scale.var_beta;
    let se_beta: Vec<f64> = (0..p_param).map(|j| var_beta[j][j].sqrt()).collect();
    let wald_chi2: Vec<f64> = (0..p_param)
        .map(|j| (beta[j] / se_beta[j]).powi(2))
        .collect();
    let wald_p: Vec<f64> = wald_chi2.iter().map(|&w| chisq_sf(w, 1.0)).collect();

    // ── 12. Log-likelihood, GOF ───────────────────────────────────────────
    let crit = compute_fit_criteria(dist, &y_vec, &final_mu, &freq_vec, &scale, n_total, p_param);

    // ── 13-14. Listing: GOF table + parameter estimates ───────────────────
    if !model.noprint {
        print_gof(session, &crit);
        print_parameter_estimates(
            session,
            &design_terms,
            &beta,
            &se_beta,
            &wald_chi2,
            &wald_p,
            dist,
            &scale,
        );
    }

    Ok(())
}

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests;
