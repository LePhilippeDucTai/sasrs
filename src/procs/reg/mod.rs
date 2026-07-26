//! PROC REG — OLS linear regression (M25.1, extended M34.4).
//!
//! Implements PROC REG. Supports:
//! - Multiple MODEL statements (each with its own OUTPUT statement(s)).
//! - Intercept models and NOINT (no-intercept) models.
//! - SELECTION= FORWARD / BACKWARD / STEPWISE variable selection.
//!
//! Produces, per model, an ANOVA table, fit statistics, and parameter
//! estimates with t-tests. Optional OUTPUT statement writes predicted values
//! and residuals (using the final selected model when SELECTION= is given).

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::decode_column;
use crate::session::Session;
use crate::stat::linalg;
use crate::stat::{f_cdf, t_quantile};
use crate::token::TokenKind;
use crate::value::VarType;
use polars::prelude::{Column, DataFrame, NamedFrom, Series};

mod ast;
mod format;
mod run;
mod sections;

pub use ast::LinEq;
pub use ast::OutEst;
pub use ast::PlotPair;
pub use ast::PlotRequests;
pub use ast::PlotVar;
pub use ast::RegAst;
pub use ast::RegDataOptions;
pub use ast::RegModel;
pub use ast::RegModelEntry;
pub use ast::RegMtest;
pub use ast::RegOutput;
pub use ast::RegRestrict;
pub use ast::RegTest;
pub use ast::SelMethod;
pub use ast::Selection;

use format::*;
use run::*;
use sections::*;

mod diagnostics;

mod fit;

mod influence;

mod matrices;

mod output;

mod parse;

mod ridge;

mod selection;

mod hypothesis;

pub use self::parse::parse;

use self::diagnostics::*;

use self::fit::*;

use self::influence::*;

use self::matrices::*;

use self::output::*;

use self::ridge::*;

use self::selection::*;

use self::hypothesis::*;

use crate::procs::common::fmt_p;

// ───────────────────────── Stat helpers ─────────────────────────

use crate::procs::common::two_sided_p;

// ───────────────────────── Listing helpers ─────────────────────────

use crate::procs::common::centered;

// ───────────────────────── Execute ─────────────────────────

pub fn execute(ast: &RegAst, session: &mut Session) -> Result<()> {
    if ast.models.is_empty() {
        session.log.note("NOTE: No MODEL statement found.");
        return Ok(());
    }

    // --- M36.10: clean deferrals for the inherently-interactive / graphics
    // run-group statements. They are parsed and consumed; here we emit a single
    // NOTE per statement kind so the PROC never crashes on them.
    if ast.reweight_seen {
        session.log.note(
            "The REWEIGHT statement (interactive observation reweighting) is not supported in this build; it was ignored.",
        );
    }
    if ast.refit_seen {
        session.log.note(
            "The REFIT statement (interactive refit) is not supported in this build; it was ignored.",
        );
    }
    if ast.paint_seen {
        session.log.note(
            "The PAINT statement (interactive plot painting) is not supported in this build; it was ignored.",
        );
    }
    // ADD/DELETE are applied to the final model fit (not interactively between RUN
    // groups); note that when present so the non-interactive semantics are clear.
    if ast
        .models
        .iter()
        .any(|m| !m.add.is_empty() || !m.delete.is_empty())
    {
        session.log.note(
            "ADD/DELETE statements were applied to the final model fit; interactive editing between RUN groups is not supported in this build.",
        );
    }

    // --- 1. Resolve dataset (once per proc) ---
    let (ds, in_libref, in_table) = common::open_input(&ast.data_options.input, session)?;

    let n_read = ds.n_obs();
    session.log.note(&format!(
        "There were {} observations read from the data set {}.{}.",
        n_read, in_libref, in_table
    ));

    // --- M36.7: resolve WEIGHT / FREQ / ID / BY columns. Each is optional and,
    // when absent, the downstream path is byte-identical to the prior OLS code.
    let find_col = |nm: &str| -> Result<usize> {
        ds.vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(nm))
            .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", nm.to_uppercase())))
    };

    let weight_col: Option<Vec<crate::value::Value>> = match &ast.weight {
        Some(nm) => Some(decode_column(&ds, find_col(nm)?)?),
        None => None,
    };
    let freq_col: Option<Vec<crate::value::Value>> = match &ast.freq {
        Some(nm) => Some(decode_column(&ds, find_col(nm)?)?),
        None => None,
    };
    // ID variables: keep (display name, decoded column) for the diagnostic
    // listings. We support and print the first; others are carried.
    let mut id_cols: Vec<(String, Vec<crate::value::Value>)> = Vec::new();
    for nm in &ast.id {
        let idx = find_col(nm)?;
        id_cols.push((ds.vars[idx].name.clone(), decode_column(&ds, idx)?));
    }

    // --- BY processing: a single group spanning all rows when no BY (output
    // byte-identical). Otherwise contiguous, dataset-order groups via by_groups.
    let by_pairs: Vec<(String, bool)> = ast.by.iter().map(|n| (n.clone(), false)).collect();
    let by_cols = common::resolve_by_cols(&ds, &by_pairs)?;
    let by_values: Vec<Vec<crate::value::Value>> = by_cols
        .iter()
        .map(|c| decode_column(&ds, c.col_idx))
        .collect::<Result<_>>()?;
    let by_names: Vec<String> = by_cols.iter().map(|c| c.name.clone()).collect();
    let by_groups_list: Vec<(Vec<crate::value::Value>, Vec<usize>)> = if by_cols.is_empty() {
        vec![(Vec::new(), (0..n_read).collect())]
    } else {
        let descending: Vec<bool> = by_cols.iter().map(|c| c.descending).collect();
        let in_display = format!("{in_libref}.{in_table}");
        common::by_groups(&by_values, &descending, n_read, &by_names, &in_display)?
    };

    // --- 2. Per-BY-group, per-model loop ---
    // M36.8: OUTEST= accumulates one PARMS entry per model per BY group; written
    // once after the loop. None when no OUTEST= so the path stays byte-identical.
    let mut outest_entries: Vec<OutEstEntry> = Vec::new();
    let want_outest = ast.data_options.outest.is_some();
    for (by_key, grp_rows) in &by_groups_list {
        // BY heading (M36.7): rendered INSIDE each model's header block (after
        // "The REG Procedure", before "Model: MODELn"), so thread the label down
        // into run_model / fit_and_print rather than emitting it here. `None`
        // when there is no BY ⇒ header block byte-identical to the prior path.
        let by_heading: Option<String> = if by_names.is_empty() {
            None
        } else {
            Some(reg_by_heading_line(&by_names, by_key))
        };
        // SIMPLE / CORR PROC-level displays (M36.8): printed once per BY group,
        // over the FIRST model's analysis variables (regressors + dependent),
        // using the same listwise deletion as that model. Gated on the PROC
        // flags so a PROC without SIMPLE/CORR is byte-identical to before.
        if (ast.simple || ast.corr)
            && !ast.models.is_empty()
            && let Some((names, cols)) = gather_simple_corr(
                &ds,
                &ast.models[0].model,
                grp_rows,
                weight_col.as_deref(),
                freq_col.as_deref(),
            )
        {
            if ast.simple {
                print_simple_stats(&names, &cols, session);
            }
            if ast.corr {
                print_corr_matrix(&names, &cols, session);
            }
        }
        // OUTSSCP= (M36.8): one SSCP matrix per BY group, built from the FIRST
        // model's analysis variables (Intercept + regressors + dependent) over
        // the same complete-case rows. Gated so a PROC without OUTSSCP= is
        // byte-identical to before.
        if let Some(out) = &ast.data_options.outsscp
            && let Some((_, cols)) = gather_simple_corr(
                &ds,
                &ast.models[0].model,
                grp_rows,
                weight_col.as_deref(),
                freq_col.as_deref(),
            )
        {
            // `gather_simple_corr` returns the regressor columns then the
            // dependent column; `write_outsscp` consumes them in that order.
            let m0 = &ast.models[0].model;
            write_outsscp(
                out,
                &m0.regressors,
                m0.dependent(),
                &cols,
                !m0.noint,
                session,
            )?;
        }
        for (mi, entry) in ast.models.iter().enumerate() {
            let model_label = format!("Model: MODEL{}", mi + 1);
            run_model(
                ast,
                entry,
                &ds,
                &in_libref,
                &in_table,
                grp_rows,
                weight_col.as_deref(),
                freq_col.as_deref(),
                &id_cols,
                &model_label,
                by_heading.as_deref(),
                if want_outest {
                    Some(&mut outest_entries)
                } else {
                    None
                },
                session,
            )?;
        }
    }

    // OUTEST= (M36.8): write the accumulated parameter-estimates dataset.
    if let Some(spec) = &ast.data_options.outest {
        write_outest(spec, &outest_entries, session)?;
    }

    Ok(())
}

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests;
