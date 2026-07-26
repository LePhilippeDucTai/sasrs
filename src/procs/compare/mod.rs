//! PROC COMPARE (jalon M21.1).
//!
//! # Syntaxe
//! ```sas
//! proc compare base=lib.x compare=lib.y [out=lib.z] [novalues] [briefsummary];
//! run;
//! ```
//!
//! - BASE= et COMPARE= obligatoires.
//! - Compare structure (variables communes, types) et valeurs ligne à ligne.
//! - NOVALUES : omet la section "Values Comparison".
//! - BRIEFSUMMARY : rapport condensé (seulement les totaux).
//! - OUT= : dataset des différences (variables STAT, _BASE_, _COMP_, _TYPE_).
//!
//! # Algorithme de comparaison
//!
//! Valeurs via `sas_cmp` (de `Value`) :
//! - Missing ordinaire (.) = missing ordinaire → égal.
//! - Char : comparaison trim trailing blanks.
//! - Numériques : différence considérée si |base - compare| > 0.0. Pas de
//!   tolérance fuzzy en v1 (documenter si besoin ultérieur).
//!
//! # Rapport listing
//! 1. "Data Set Summary"  : NObs + NVars de chaque dataset.
//! 2. "Variables Summary" : en commun, seulement dans BASE, seulement dans COMPARE.
//! 3. "Observation Summary" : nb obs comparées, nb avec différences.
//! 4. "Values Comparison" : pour chaque variable numérique commune, max |diff|.
//!    (Absente si NOVALUES.)
//!
//! # Déviation v1
//! - Tolérance numérique : zéro (différence si valeurs f64 divergent, même
//!   d'un epsilon machine). La tolérance CRITERION= est reportée à v2.
//! - OUT= : créé, colonne _TYPE_ ∈ {BASE, COMPARE, DIF} + colonnes des
//!   variables communes. Conformité SAS OUT= approximative (format SAS exact
//!   différé).

use std::cmp::Ordering;
use std::collections::HashMap;

use polars::prelude::*;

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::num_to_value;
use crate::parser::StatementStream;
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{Value, VarType};

mod analyze;
mod output;
mod parse;
mod report;

pub use parse::CompareAst;
pub use parse::parse;

use analyze::*;
use output::*;
use report::*;

/// Execute PROC COMPARE.
pub fn execute(ast: &CompareAst, session: &mut Session) -> Result<()> {
    // ── Load BASE dataset ────────────────────────────────────────────────────
    let base_display = ast.base.display();
    let base_ds = read_input(session, &ast.base)?;

    // ── Load COMPARE dataset ─────────────────────────────────────────────────
    let comp_display = ast.compare.display();
    let comp_ds = read_input(session, &ast.compare)?;

    let base_nobs = base_ds.n_obs();
    let base_nvars = base_ds.n_vars();
    let comp_nobs = comp_ds.n_obs();
    let comp_nvars = comp_ds.n_vars();

    // ── Variable analysis ────────────────────────────────────────────────────
    let (only_base, only_comp, common_vars) = analyze_variables(&base_ds, &comp_ds);
    let matching_vars: Vec<&CommonVar> = common_vars.iter().filter(|cv| cv.type_match).collect();

    // ── Observation comparison ───────────────────────────────────────────────
    let n_compared = base_nobs.min(comp_nobs);
    let need_out = ast.out.is_some();
    let (n_with_diffs, var_diffs, out_rows) =
        compare_observations(&base_ds, &comp_ds, &matching_vars, n_compared, need_out);

    // ── Render listing ───────────────────────────────────────────────────────
    session.listing.page_header();
    let ctx = ReportCtx {
        base_display: &base_display,
        comp_display: &comp_display,
        base_nobs,
        base_nvars,
        comp_nobs,
        comp_nvars,
        only_base: &only_base,
        only_comp: &only_comp,
        common_vars: &common_vars,
        n_matching: matching_vars.len(),
        n_compared,
        n_with_diffs,
        var_diffs: &var_diffs,
    };
    if !ast.briefsummary {
        print_full_report(session, ast, &ctx);
    } else {
        print_brief_report(session, &ctx);
    }

    // ── NOTE log ────────────────────────────────────────────────────────────
    if n_with_diffs == 0 {
        session.log.note(
            "No unequal values were found. All values compared are exactly equal.",
        );
    } else {
        session.log.note(&format!(
            "There were {} observations with at least one unequal value.",
            n_with_diffs
        ));
    }

    // ── Write OUT= dataset ───────────────────────────────────────────────────
    if let Some(ref out_ref) = ast.out {
        write_out_dataset(session, out_ref, &out_rows, &matching_vars, &base_ds)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
