//! PROC GLM — General Linear Model for one-way CLASS designs (M25.3).
//!
//! Extends PROC ANOVA with:
//! - `/SOLUTION` option: parameter estimates (intercept + CLASS level effects)
//! - LSMEANS statement: least-squares means with SEs and Pr > |t|
//! - ESTIMATE statement: user-defined linear combinations of CLASS means
//! - CONTRAST statement: F-tests for linear combinations (same as ESTIMATE but gives F)
//!
//! For now, only one-way CLASS designs (single effect in MODEL) are supported.
//! ESTIMATE and CONTRAST are computed on the one-way path only, for its MODEL
//! effect; elsewhere, and for their options other than E, they are an ERROR
//! (J02-P8, see `lincomb_unsupported` and `parse::parse_lincomb_options`).

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::num_var_meta;
use crate::procs::common::{decode_column, sample_std};
use crate::procs::common::{fmt2, fmt5, fmt6};
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
mod output;
mod parse;
use design::*;
use multiway::*;
use multiway_report::*;
use oneway::*;
use oneway_means::*;
use oneway_report::*;
pub use output::GlmOutput;
use output::*;
pub use parse::parse;

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone)]
pub struct GlmAst {
    pub data_options: GlmDataOptions,
    pub class_vars: Vec<String>,
    pub model: Option<GlmModel>,
    /// J02-P12 — one entry per LSMEANS statement (its effect names), in
    /// source order: all of them are produced (SAS/STAT 9.4, LSMEANS
    /// statement: « multiple LSMEANS statements can be used »). Base :
    /// only the last statement was kept.
    pub lsmeans_stmts: Vec<Vec<String>>,
    pub estimates: Vec<GlmEstimate>,
    pub contrasts: Vec<GlmContrast>,
    /// J02-P12 — one entry per MEANS statement, in source order (base:
    /// only the last statement was kept).
    pub means_stmts: Vec<Vec<String>>,
    /// J02-P2 (issue #17) — OUTPUT statement (`output out=<ds> p= r=;`).
    pub output: Option<GlmOutput>,
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

    // J02-P8 — ESTIMATE/CONTRAST that the executed path cannot produce are
    // rejected (already at parse time; kept here for ASTs built directly).
    // J02-P12 — a non-estimable ESTIMATE is rejected the same way.
    let lincombs = ast
        .estimates
        .iter()
        .map(|e| ("ESTIMATE", &e.effect, &e.coefficients))
        .chain(
            ast.contrasts
                .iter()
                .map(|c| ("CONTRAST", &c.effect, &c.coefficients)),
        );
    for (stmt, effect, coefficients) in lincombs {
        if let Some(msg) = lincomb_unsupported(stmt, effect, model, &ast.class_vars) {
            return Err(SasError::runtime(msg));
        }
        if stmt == "ESTIMATE"
            && let Some(msg) = estimate_nonestimable(effect, coefficients, model, &ast.class_vars)
        {
            return Err(SasError::runtime(msg));
        }
    }

    // Branch: the existing one-way path is taken ONLY for a single main effect
    // over a single CLASS variable with no interaction. Anything else (interaction
    // term, multiple effect terms, or multiple CLASS vars) goes to the general
    // multiway engine. This keeps the one-way path byte-identical.
    if uses_multiway_engine(model, &ast.class_vars) {
        // J02-P2 (issue #17) — OUTPUT OUT= est implémenté pour le dessin une
        // voie uniquement ; refus explicite plutôt qu'honneur silencieux.
        if ast.output.is_some() {
            return Err(SasError::runtime(
                "The OUTPUT statement of PROC GLM is only supported for one-way models in sasrs.",
            ));
        }
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
    // J02-P2 (issue #17) — OUTPUT OUT= : une seule variable dépendante.
    if ast.output.is_some() && model.dependents.len() > 1 {
        return Err(SasError::runtime(
            "The OUTPUT statement of PROC GLM supports a single dependent variable in sasrs.",
        ));
    }
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
        // J02-P12 — one table per LSMEANS statement whose effects contain the
        // MODEL effect (base: only the last statement was kept).
        for stmt in &ast.lsmeans_stmts {
            let show_lsmeans = stmt.iter().any(|v| v.eq_ignore_ascii_case(eff));

            if show_lsmeans {
                print_oneway_lsmeans(session, dep_var, eff, &stats);
            }
        }

        print_oneway_contrasts(session, ast, eff, &stats)?;
        print_oneway_estimates(session, ast, eff, &stats);

        // --- MEANS section ---
        // J02-P12 — one table per MEANS statement (base: only the last kept).
        for stmt in &ast.means_stmts {
            let show_means = stmt.iter().any(|m| m.eq_ignore_ascii_case(eff));

            if show_means {
                print_oneway_means(session, eff, &stats);
            }
        }

        // J02-P2 (issue #17) — statement OUTPUT OUT= : valeurs ajustées
        // (moyennes de groupe) et résidus en dataset, après le rendu.
        if let Some(oreq) = &ast.output {
            write_output(session, &ds, dep_var, eff, &stats, oreq)?;
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
    }

    Ok(())
}

/// True when PROC GLM runs the general multi-way engine: an interaction
/// term, several effect terms or several CLASS variables.
pub(super) fn uses_multiway_engine(model: &GlmModel, class_vars: &[String]) -> bool {
    let has_interaction = model.effect_terms.iter().any(|t| t.len() > 1);
    has_interaction || model.effect_terms.len() > 1 || class_vars.len() > 1
}

/// J02-P8 — ERROR text for an ESTIMATE/CONTRAST statement (`stmt`) on
/// `effect` that sasrs cannot produce for this MODEL, `None` when it is
/// produced. The multi-way engine computes no ESTIMATE/CONTRAST (they used to
/// be skipped without output nor NOTE); the one-way path only computes the
/// coefficients of its single MODEL effect (any other effect, INTERCEPT
/// included, used to be dropped without a word). A MODEL without effect or
/// without CLASS variable keeps its own execution ERROR.
pub(super) fn lincomb_unsupported(
    stmt: &str,
    effect: &str,
    model: &GlmModel,
    class_vars: &[String],
) -> Option<String> {
    let article = if stmt == "ESTIMATE" { "An" } else { "A" };
    let tail = "is not supported in PROC GLM; it can affect results and cannot be ignored.";
    let model_effect = model.effects.first()?;
    if class_vars.is_empty() {
        return None;
    }
    if uses_multiway_engine(model, class_vars) {
        return Some(format!(
            "{article} {stmt} statement in a model with several effects or CLASS variables \
             {tail}"
        ));
    }
    if effect.eq_ignore_ascii_case(model_effect) {
        return None;
    }
    Some(format!(
        "{article} {stmt} statement on the effect {}, which is not the MODEL effect {}, {tail}",
        effect.to_uppercase(),
        model_effect.to_uppercase()
    ))
}

/// J02-P12 — ERROR text for an ESTIMATE statement of the one-way path,
/// without INTERCEPT, whose coefficients do not sum to zero: that linear
/// function is not estimable, and sasrs used to print Σ cᵢ ȳᵢ anyway (the
/// value under one particular generalized inverse). `None` when the ESTIMATE
/// is estimable (or out of scope of this check).
///
/// SAS/STAT 9.4 (GLM) references:
/// - ESTIMATE statement: « The linear function is checked for
///   estimability »;
/// - « Specification of ESTIMATE Expressions » → « A Check for
///   Estimability »: L is « declared nonestimable » when ABS(L − LH)
///   exceeds the SINGULAR= tolerance (default 1e-4);
/// - Chapter 15, « Estimability »: Lβ is estimable iff L lies in the row
///   space of X. For a one-way model the rows of X span {(1, e₁) … (1, e_k)},
///   so L = (0, c₁…c_k) — an ESTIMATE without INTERCEPT — is estimable iff
///   Σ cᵢ = 0.
///
/// The published doc does not fix the SAS rendering of a non-estimable
/// estimate (none of the pages above prints « Non-est »), so the J02-P12
/// decision is an ERROR rather than an invented output. The tolerance is
/// relative, like SAS's ε=1e-4 on the projected residual.
pub(super) fn estimate_nonestimable(
    effect: &str,
    coefficients: &[f64],
    model: &GlmModel,
    class_vars: &[String],
) -> Option<String> {
    if class_vars.is_empty() || uses_multiway_engine(model, class_vars) {
        // The multiway path and other effects are already rejected by
        // `lincomb_unsupported`.
        return None;
    }
    let model_effect = model.effects.first()?;
    if !effect.eq_ignore_ascii_case(model_effect) {
        return None;
    }
    let sum: f64 = coefficients.iter().sum();
    let scale: f64 = 1.0 + coefficients.iter().map(|c| c.abs()).sum::<f64>();
    if sum.abs() <= 1e-4 * scale {
        return None;
    }
    let tail = "is not supported in PROC GLM; it can affect results and cannot be ignored.";
    Some(format!(
        "An ESTIMATE statement whose coefficients do not sum to zero (not estimable without \
         INTERCEPT) {tail}"
    ))
}

// NOTE (M37.1): `lsmean_coef_vector` was extracted into
// `crate::procs::lincom::LinCombEngine` (as a private method) together with its
// `row_dummies` helper. The multiway LSMEANS path now delegates to the engine.

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests;
