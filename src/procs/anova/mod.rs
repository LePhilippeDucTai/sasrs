//! PROC ANOVA — One-way ANOVA for balanced designs (M25.2).
//!
//! Implements CLASS statement, MODEL statement (multiple dependents, one CLASS
//! effect), MEANS statement. Produces Class Level Information, ANOVA table,
//! fit statistics, Type I SS, Type III SS, and optional MEANS table.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::{decode_column, sample_std};
use crate::session::Session;
use crate::stat::f_cdf;
use crate::token::TokenKind;
use crate::value::{Value, VarType};


mod oneway;
mod design;
mod multiway;
mod multiway_report;
use oneway::*;
use design::*;
use multiway::*;
use multiway_report::*;

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone)]
pub struct AnovaAst {
    pub data_options: AnovaDataOptions,
    pub class_vars: Vec<String>,
    pub model: Option<AnovaModel>,
    pub means_vars: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AnovaDataOptions {
    pub input: Option<DatasetRef>,
}

#[derive(Debug, Clone)]
pub struct AnovaModel {
    pub dependents: Vec<String>,
    /// Raw effect tokens as written (e.g. `["a", "b", "a*b"]`). The one-way path
    /// only ever inspects `effects[0]`.
    pub effects: Vec<String>,
    /// Structured effect terms: each term is the list of CLASS var names it
    /// references. A main effect is a 1-element vec; `a*b` is `["a","b"]`.
    pub terms: Vec<Vec<String>>,
    pub noprint: bool,
}

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC ANOVA. Called AFTER `proc anova` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<AnovaAst> {
    let mut input: Option<DatasetRef> = None;

    // PROC ANOVA statement options until `;`
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            input = Some(common::parse_dataset_opt(ts, "DATA")?);
        } else {
            // Skip unknown proc-level options
            ts.next();
        }
    }

    // Sub-statements until run;/quit;
    let mut class_vars: Vec<String> = Vec::new();
    let mut model: Option<AnovaModel> = None;
    let mut means_vars: Vec<String> = Vec::new();

    common::parse_proc_body(ts, |ts, kw| {
        if kw == "class" {
            ts.next();
            class_vars = ts.parse_name_list()?;
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "model" {
            ts.next();
            // Read dependents: idents before `=` (the `=` itself is consumed).
            let dependents = common::parse_model_lhs(ts);
            // Read effects: idents after `=` until `/` or `;`. Idents joined by
            // `*` form a single interaction term (e.g. `a*b`).
            let (effects, terms) = common::parse_effect_terms(ts);
            let mut noprint = false;
            if ts.peek().kind == TokenKind::Slash {
                ts.next();
                // Parse options until semi
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    if ts.peek().is_kw("noprint") {
                        noprint = true;
                    }
                    ts.next();
                }
            }
            ts.expect_semi()?;
            model = Some(AnovaModel {
                dependents,
                effects,
                terms,
                noprint,
            });
            Ok(true)
        } else if kw == "means" {
            ts.next();
            means_vars = ts.parse_name_list()?;
            ts.expect_semi()?;
            Ok(true)
        } else {
            Ok(false)
        }
    })?;

    Ok(AnovaAst {
        data_options: AnovaDataOptions { input },
        class_vars,
        model,
        means_vars,
    })
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

pub fn execute(ast: &AnovaAst, session: &mut Session) -> Result<()> {
    // Guard: MODEL required
    let model = match &ast.model {
        Some(m) => m,
        None => {
            session.log.note("No MODEL statement found in PROC ANOVA.");
            return Ok(());
        }
    };

    // Pre-check: at least one effect and at least one class var
    if model.effects.is_empty() || ast.class_vars.is_empty() {
        return Err(SasError::runtime(
            "MODEL statement requires at least one CLASS effect.",
        ));
    }

    // Validate every effect term references only declared CLASS variables.
    for term in &model.terms {
        for part in term {
            let is_class = ast
                .class_vars
                .iter()
                .any(|c| c.eq_ignore_ascii_case(part));
            if !is_class {
                return Err(SasError::runtime(format!(
                    "Variable {} not found in CLASS list.",
                    part.to_uppercase()
                )));
            }
        }
    }

    // Decide one-way vs multi-way. The one-way path (byte-identical to the
    // existing snapshot) is taken ONLY when the model is a single main effect
    // referencing exactly one CLASS variable and no interaction is present.
    let distinct_class_used: std::collections::BTreeSet<String> = model
        .terms
        .iter()
        .flatten()
        .map(|s| s.to_uppercase())
        .collect();
    let is_multiway = model.terms.len() > 1
        || model.terms.iter().any(|t| t.len() > 1)
        || distinct_class_used.len() > 1;
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
        // For one-way ANOVA, use the first effect as the CLASS grouping variable
        let eff = &model.effects[0];

        let stats = compute_oneway_stats(session, &ds, dep_var, eff, n_obs)?;

        // Listing — Dependent Variable header
        centered(session, &format!("Dependent Variable: {}", dep_var));
        session.listing.blank();

        print_oneway_anova_and_fit(session, dep_var, eff, &stats);

        // MEANS section — only if means_vars contains `eff`
        let show_means = !ast.means_vars.is_empty()
            && ast
                .means_vars
                .iter()
                .any(|m| m.eq_ignore_ascii_case(eff));

        if show_means {
            print_oneway_means(session, eff, &stats);
        }
    }

    Ok(())
}

/// General multi-way ANOVA (interactions, multiple CLASS vars), reference-cell
/// coding with Type I (sequential) and Type III (partial) sums of squares.
fn execute_multiway(ast: &AnovaAst, model: &AnovaModel, session: &mut Session) -> Result<()> {
    // --- 1. Resolve dataset ---
    let (ds, in_libref, in_table) = common::open_input(&ast.data_options.input, session)?;

    let n_obs = ds.n_obs();
    session.log.note(&format!(
        "There were {} observations read from the data set {}.{}.",
        n_obs, in_libref, in_table
    ));

    // --- 2. Validate CLASS vars exist ---
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

    // Decode all CLASS columns once, keyed by uppercase name.
    let mut class_cols: std::collections::HashMap<String, (String, Vec<Value>)> =
        std::collections::HashMap::new();
    for class_var in &ast.class_vars {
        let col_idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(class_var))
            .unwrap();
        let col = decode_column(&ds, col_idx)?;
        class_cols.insert(
            class_var.to_uppercase(),
            (ds.vars[col_idx].name.clone(), col),
        );
    }

    // --- 3-4. Listing header + Class Level Information ---
    print_class_level_info_multiway(session, ast, &class_cols, n_obs);

    // Distinct CLASS vars referenced by the model, uppercased.
    let used_classes: Vec<String> = {
        let mut seen: Vec<String> = Vec::new();
        for part in model.terms.iter().flatten() {
            let up = part.to_uppercase();
            if !seen.contains(&up) {
                seen.push(up);
            }
        }
        seen
    };

    // --- 5. Per-dependent loop ---
    for dep_var in &model.dependents {
        let fit = fit_multiway(
            session,
            &ds,
            dep_var,
            &class_cols,
            &used_classes,
            model,
            n_obs,
        )?;

        // --- Listing: Dependent Variable header ---
        centered(session, &format!("Dependent Variable: {}", dep_var));
        session.listing.blank();

        print_multiway_anova_and_fit(session, dep_var, model, &fit);

        // MEANS: main-effect marginal cell means for each requested CLASS var.
        print_multiway_means(session, ast, &fit, &class_cols, &used_classes);
    }

    Ok(())
}

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests;
