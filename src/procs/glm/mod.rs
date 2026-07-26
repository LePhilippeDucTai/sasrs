//! PROC GLM — General Linear Model for one-way CLASS designs (M25.3).
//!
//! Extends PROC ANOVA with:
//! - `/SOLUTION` option: parameter estimates (intercept + CLASS level effects)
//! - LSMEANS statement: least-squares means with SEs and Pr > |t|
//! - ESTIMATE statement: user-defined linear combinations of CLASS means
//! - CONTRAST statement: F-tests for linear combinations (same as ESTIMATE but gives F)
//!
//! For now, only one-way CLASS designs (single effect in MODEL) are supported.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::{decode_column, sample_std};
use crate::session::Session;
use crate::stat::{f_cdf, student_t_cdf};
use crate::token::TokenKind;
use crate::value::{Value, VarType};

mod design;
mod multiway;
mod multiway_report;
mod oneway;
mod oneway_means;
mod oneway_report;
mod parse;
use design::*;
use multiway::*;
use multiway_report::*;
use oneway::*;
use oneway_means::*;
use oneway_report::*;
pub use parse::parse;

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone)]
pub struct GlmAst {
    pub data_options: GlmDataOptions,
    pub class_vars: Vec<String>,
    pub model: Option<GlmModel>,
    pub lsmeans_vars: Vec<String>,
    pub estimates: Vec<GlmEstimate>,
    pub contrasts: Vec<GlmContrast>,
    pub means_vars: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GlmDataOptions {
    pub input: Option<DatasetRef>,
}

#[derive(Debug, Clone)]
pub struct GlmModel {
    pub dependents: Vec<String>,
    /// Legacy flat list of effect variable names (one-way path). Kept intact for
    /// byte-identity of the existing snapshot. For `a b a*b` this is `["a","b","a*b"]`.
    pub effects: Vec<String>,
    /// Structured effect terms for the multiway engine. Each term is the list of
    /// CLASS variable names it involves: main effect = 1 elt, `a*b` = `["a","b"]`.
    pub effect_terms: Vec<Vec<String>>,
    pub solution: bool,
    pub noprint: bool,
}

#[derive(Debug, Clone)]
pub struct GlmEstimate {
    pub label: String,
    pub effect: String,
    pub coefficients: Vec<f64>,
}

#[derive(Debug, Clone)]
pub struct GlmContrast {
    pub label: String,
    pub effect: String,
    pub coefficients: Vec<f64>,
}

// ───────────────────────── Formatting ─────────────────────────

fn fmt5(v: f64) -> String {
    format!("{v:.5}")
}

fn fmt2(v: f64) -> String {
    format!("{v:.2}")
}

fn fmt6(v: f64) -> String {
    format!("{v:.6}")
}

use crate::procs::common::fmt_p;

// ───────────────────────── Listing helpers ─────────────────────────

use crate::procs::common::centered;

// ───────────────────────── Execute ─────────────────────────

pub fn execute(ast: &GlmAst, session: &mut Session) -> Result<()> {
    // Guard: MODEL required
    let model = match &ast.model {
        Some(m) => m,
        None => {
            session.log.note("No MODEL statement found in PROC GLM.");
            return Ok(());
        }
    };

    // Pre-check: at least one effect and at least one class var
    if model.effects.is_empty() || ast.class_vars.is_empty() {
        return Err(SasError::runtime(
            "MODEL statement requires at least one CLASS effect.",
        ));
    }

    // Branch: the existing one-way path is taken ONLY for a single main effect
    // over a single CLASS variable with no interaction. Anything else (interaction
    // term, multiple effect terms, or multiple CLASS vars) goes to the general
    // multiway engine. This keeps the one-way path byte-identical.
    let has_interaction = model.effect_terms.iter().any(|t| t.len() > 1);
    let is_multiway = has_interaction || model.effect_terms.len() > 1 || ast.class_vars.len() > 1;
    if is_multiway {
        return execute_multiway(ast, model, session);
    }

    // --- 1. Resolve dataset ---
    let (ds, in_libref, in_table) = common::open_input(&ast.data_options.input, session)?;

    let n_obs = ds.n_obs();
    session.log.note(&format!(
        "There were {} observations read from the data set {}.{}.",
        n_obs, in_libref, in_table
    ));

    // --- 2. Validate CLASS vars ---
    for class_var in &ast.class_vars {
        let found = ds
            .vars
            .iter()
            .any(|m| m.name.eq_ignore_ascii_case(class_var));
        if !found {
            return Err(SasError::runtime(format!(
                "Variable {} not found.",
                class_var.to_uppercase()
            )));
        }
    }

    // --- 3-4. Listing header + Class Level Information ---
    print_class_level_info_oneway(session, ast, &ds, n_obs)?;

    // --- 5. Per-dependent variable loop ---
    for dep_var in &model.dependents {
        // For one-way GLM, use the first effect as the CLASS grouping variable
        let eff = &model.effects[0];

        let stats = compute_oneway_stats(session, &ds, dep_var, eff, n_obs)?;

        // --- Dependent Variable header ---
        centered(session, &format!("Dependent Variable: {}", dep_var));
        session.listing.blank();

        print_oneway_anova_and_fit(session, dep_var, eff, &stats);

        // --- Parameter Estimates (if /SOLUTION) ---
        if model.solution && stats.k >= 1 {
            print_oneway_solution(session, eff, &stats);
        }

        // --- LSMEANS ---
        let show_lsmeans = !ast.lsmeans_vars.is_empty()
            && ast.lsmeans_vars.iter().any(|v| v.eq_ignore_ascii_case(eff));

        if show_lsmeans {
            print_oneway_lsmeans(session, dep_var, eff, &stats);
        }

        print_oneway_contrasts(session, ast, eff, &stats)?;
        print_oneway_estimates(session, ast, eff, &stats);

        // --- MEANS section ---
        let show_means = !ast.means_vars.is_empty()
            && ast.means_vars.iter().any(|m| m.eq_ignore_ascii_case(eff));

        if show_means {
            print_oneway_means(session, eff, &stats);
        }
    }

    Ok(())
}

/// General multi-way / interaction GLM engine.
fn execute_multiway(ast: &GlmAst, model: &GlmModel, session: &mut Session) -> Result<()> {
    // --- 1. Resolve dataset ---
    let (ds, in_libref, in_table) = common::open_input(&ast.data_options.input, session)?;

    let n_obs = ds.n_obs();
    session.log.note(&format!(
        "There were {} observations read from the data set {}.{}.",
        n_obs, in_libref, in_table
    ));

    // --- 2. Validate CLASS vars and effect variables ---
    for class_var in &ast.class_vars {
        let found = ds
            .vars
            .iter()
            .any(|m| m.name.eq_ignore_ascii_case(class_var));
        if !found {
            return Err(SasError::runtime(format!(
                "Variable {} not found.",
                class_var.to_uppercase()
            )));
        }
    }
    // Every variable appearing in an effect term must be a CLASS variable.
    for term in &model.effect_terms {
        for v in term {
            if !ast.class_vars.iter().any(|c| c.eq_ignore_ascii_case(v)) {
                return Err(SasError::runtime(format!(
                    "Variable {} not found.",
                    v.to_uppercase()
                )));
            }
        }
    }

    // Decode each CLASS column once (canonical name from metadata).
    let mut class_cols: Vec<(String, Vec<Value>)> = Vec::new();
    for class_var in &ast.class_vars {
        let col_idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(class_var))
            .unwrap();
        let col = decode_column(&ds, col_idx)?;
        class_cols.push((ds.vars[col_idx].name.clone(), col));
    }

    // --- 3-4. Listing header + Class Level Information ---
    print_class_level_info_multiway(session, &class_cols, n_obs);

    // Map each effect term (Vec of class-var names) to indices into `class_cols`.
    let term_factor_idxs: Vec<Vec<usize>> = model
        .effect_terms
        .iter()
        .map(|term| {
            term.iter()
                .map(|v| {
                    class_cols
                        .iter()
                        .position(|(n, _)| n.eq_ignore_ascii_case(v))
                        .unwrap()
                })
                .collect()
        })
        .collect();

    // --- 5. Per-dependent variable loop ---
    for dep_var in &model.dependents {
        let fit = fit_multiway(
            session,
            &ds,
            dep_var,
            &class_cols,
            &term_factor_idxs,
            model,
            n_obs,
        )?;

        // --- Dependent Variable header ---
        centered(session, &format!("Dependent Variable: {}", dep_var));
        session.listing.blank();

        print_multiway_anova_and_fit(session, dep_var, &fit);

        // Need (X'X)^-1 for SOLUTION / LSMEANS standard errors.
        let (beta, xtx_inv, lincom_coding, lincom_engine) = build_lincom_engine(&fit);

        // --- SOLUTION (parameter estimates) ---
        if model.solution {
            print_multiway_solution(session, model, &fit, &lincom_engine, &beta, &xtx_inv);
        }

        // --- LSMEANS (main effects only) ---
        print_multiway_lsmeans(
            session,
            ast,
            dep_var,
            &fit,
            &lincom_engine,
            &lincom_coding,
            &beta,
            &xtx_inv,
        );

        note_skipped_contrasts(session, ast, model, &fit.factors);
    }

    Ok(())
}

// NOTE (M37.1): `lsmean_coef_vector` was extracted into
// `crate::procs::lincom::LinCombEngine` (as a private method) together with its
// `row_dummies` helper. The multiway LSMEANS path now delegates to the engine.

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests;
