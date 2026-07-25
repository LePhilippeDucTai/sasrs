//! PROC DISCRIM — Fisher's linear discriminant analysis (M27).
//!
//! Supports (pool=yes / METHOD=NORMAL):
//! - CLASS statement (group variable, char or numeric).
//! - VAR statement (numeric predictors).
//! - ID statement (label for the classification listing).
//! - PRIORS EQUAL (default) / PRIORS PROPORTIONAL.
//! - OUT= dataset with `_FROM_`, `_INTO_` and one `_<k>` posterior per class.
//!
//! Produces: header counts, Class Level Information, Within-Class Covariance
//! Matrix (per class), Pooled Within-Class Covariance Matrix, Pairwise Squared
//! Distances Between Groups, Linear Discriminant Function Coefficients,
//! Classification Results for Training Data, Error Count Estimates.
//!
//! Parse-accepted but not implemented (NOTE emitted): METHOD other than NORMAL,
//! POOL=NO/TEST (QDA deferred), OUTSTAT=, NOCLASSIFY, CROSSVALIDATE, SHORT.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, decode_column};
use crate::session::Session;
use crate::stat::invert_matrix;
use crate::token::TokenKind;
use crate::value::Value;


mod parse;
mod lda;
mod data;
mod report;
mod output;

pub use parse::parse;

use parse::*;
use lda::*;
use data::*;
use report::*;
use output::*;

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Priors {
    Equal,
    Proportional,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pool {
    Yes,
    No,
    Test,
}

#[derive(Debug, Clone)]
pub struct DiscrimAst {
    pub data: Option<DatasetRef>,
    pub out: Option<DatasetRef>,
    pub outstat: Option<DatasetRef>,
    pub method: Option<String>,
    pub pool: Pool,
    pub priors: Priors,
    pub noclassify: bool,
    pub crossvalidate: bool,
    pub short: bool,
    pub class_var: Option<String>,
    pub var_vars: Vec<String>,
    pub id_var: Option<String>,
}

use crate::procs::common::centered;

use crate::procs::common::value_label;

pub fn execute(ast: &DiscrimAst, session: &mut Session) -> Result<()> {
    // ── 1. Guards ──────────────────────────────────────────────────────────
    let class_name = check_options(ast, session)?;

    // ── 2. Read dataset ────────────────────────────────────────────────────
    let (ds, in_libref, in_table) = common::open_input(&ast.data, session)?;

    let n_read = ds.n_obs();
    session.log.note(&format!(
        "There were {} observations read from the data set {}.{}.",
        n_read, in_libref, in_table
    ));

    let p = ast.var_vars.len();

    // ── Find column indices + decode ───────────────────────────────────────
    let (class_col, var_cols, id_col) = resolve_and_decode(&ds, ast, class_name)?;

    // ── 3. Build complete observations grouped by class ────────────────────
    let (classes, kept) = collect_complete_obs(&class_col, &var_cols, n_read, p)?;
    let n_groups = classes.len();

    // Group rows per class.
    let mut class_obs: Vec<Vec<Vec<f64>>> = vec![Vec::new(); n_groups];
    for obs in &kept {
        class_obs[class_index_of(&classes, &obs.class)].push(obs.x.clone());
    }

    let n_used = kept.len();
    session
        .log
        .note(&format!("There were {} observations used.", n_used));

    // ── 4. Fit ─────────────────────────────────────────────────────────────
    let model = fit_lda(classes, &class_obs, &ast.priors, p)?;

    // ── 5. Listing ─────────────────────────────────────────────────────────
    session.listing.page_header();
    centered(session, "The DISCRIMINANT Procedure");
    session.listing.blank();

    print_counts_header(session, &model, p);
    print_class_level_info(session, class_name, &model);
    print_covariance_matrices(session, &ast.var_vars, &model);
    print_pairwise_distances(session, &model);
    print_discrim_coefficients(session, &ast.var_vars, &model);

    // ── 6. Classification ──────────────────────────────────────────────────
    let error_count = print_classification_results(session, ast, &model, &kept, &id_col);

    // ── 7. Error Count Estimates ───────────────────────────────────────────
    print_error_estimates(session, &model, &error_count);

    // ── 8. OUT= dataset ────────────────────────────────────────────────────
    if let Some(out_ref) = &ast.out {
        write_out_dataset(ast, session, &ds, &model, &var_cols, &class_col, out_ref, n_read)?;
    }

    Ok(())
}

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests;
