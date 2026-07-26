//! PROC MEANS / SUMMARY (jalon M5).
//!
//! # Plan du fichier — voir PLAN.md  (difficulté : ÉLEVÉE — _TYPE_)
//!
//! `proc means data=a [noprint] [stats...] ; class c1 c2 ; var v1 v2 ;
//! output out=b [stat(var)=name...] ; run ;`  — SUMMARY = MEANS noprint
//! par défaut.
//!
//! ## Sémantique à répliquer
//! - Stats défaut du rapport : N, Mean, Std Dev, Minimum, Maximum.
//!   Stats demandables : n nmiss mean std min max sum range stderr cv
//!   median.  SAS EXCLUT les missings : chaque stat est calculée sur les
//!   valeurs numériques NON missing du groupe (helper `compute`).
//! - CLASS sans OUTPUT : rapport par combinaison de classes.
//! - OUTPUT OUT= avec CLASS : produit TOUTES les combinaisons de
//!   sous-ensembles de classes — `_TYPE_` = masque binaire (bit le plus
//!   à droite = dernière variable CLASS), `_FREQ_` = effectif. Ordre :
//!   _TYPE_ croissant puis valeurs de classes. Lignes des classes non
//!   actives → missing.
//! - VAR absent : toutes les numériques hors CLASS/BY.
//! - Rapport listing : table par variable x stat, en-tête style SAS
//!   ("The MEANS Procedure").
//!
//! ## Choix de rendu (documenté pour l'orchestrateur)
//! - Titre centré "The MEANS Procedure" via `page_header()` puis une ligne
//!   centrée.
//! - Sans CLASS : une table, colonne `Variable` puis une colonne par stat
//!   demandée (défaut : N, Mean, Std Dev, Minimum, Maximum). Une ligne par
//!   variable analysée.
//! - Avec CLASS : une table COMBINÉE — colonnes de tête = chaque variable
//!   CLASS, puis colonne `Variable`, puis une colonne par stat. Une ligne
//!   par (combinaison de classes × variable). Les combinaisons de classes
//!   sont ordonnées par `sas_cmp`.
//!
//! ## WEIGHT statement (jalon WEIGHT)
//! `weight <var>;` — une seule variable numérique. Quand elle est présente,
//! toutes les stats passent par `compute_weighted` (analogue pondéré de
//! `compute`). Le chemin non-pondéré reste BYTE-IDENTIQUE : `compute_weighted`
//! n'est appelé que si `ast.weight.is_some()`. Fonctionne avec CLASS et BY
//! (poids partitionnés par groupe), et OUTPUT OUT= utilise les stats pondérées.
//!
//! Formules pondérées (VARDEF=DF) — n = nb d'obs utilisables, w_i poids, x_i :
//!   SumWgt = Σw_i ; Sum = Σw_i x_i ; Mean = Σw_i x_i / Σw_i ;
//!   CSS_w = Σw_i(x_i−x̄_w)² ; Variance = CSS_w/(n−1) ; Std = √Variance ;
//!   StdErr = Std/√(Σw_i) (SAS pondère l'erreur-type par √ΣW) ;
//!   CV = 100·Std/x̄_w ; Min/Max = min/max NON pondérés de x_i ; N = n ;
//!   NMiss = nb d'obs exclues (valeur missing, poids missing, ou poids ≤ 0).
//! Exclusions : voir `common::partition_weighted`.
//!
//! ## Simplifications SAS documentées (WEIGHT)
//! - MEDIAN avec WEIGHT : la vraie médiane pondérée de SAS est complexe ;
//!   DIFFÉRÉ. Ici MEDIAN est calculée NON pondérée (médiane simple des x_i
//!   utilisables) — divergence assumée et documentée.

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::num_var_meta;
use crate::procs::common::{
    by_groups, decode_column, partition_numeric, partition_weighted, resolve_by_cols, sample_std,
    t_quantile,
};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{Value, VarType, format_best};
use polars::prelude::*;
use std::cmp::Ordering;

mod output;
mod parse;
mod report;
mod stats;
mod types;

pub use parse::parse;

// `parse_single_var` et `parse_by_list` ont été déplacés vers `procs::common`
// (M31.2). Ré-export `pub(crate)` pour les appelants existants
// (`means` lui-même, `univariate`, `rank` via `means::parse_by_list`).
pub(crate) use crate::procs::common::{parse_by as parse_by_list, parse_single_var};
pub use report::stat_header;
pub use stats::compute;
pub use stats::compute_weighted;

use output::*;
use parse::*;
use report::*;
use types::*;

pub struct MeansAst {
    pub data: Option<DatasetRef>,
    pub summary: bool,
    pub noprint: bool,
    pub stats: Vec<String>,
    pub class: Vec<String>,
    pub var: Vec<String>,
    /// BY variables (var, descending). Outer grouping; input must be sorted.
    pub by: Vec<(String, bool)>,
    /// WEIGHT variable (single numeric var). When `Some`, all statistics are
    /// computed through the weighted code path (see `compute_weighted`).
    pub weight: Option<String>,
    /// Confidence level alpha for CLM/LCLM/UCLM (SAS default 0.05). Only the
    /// CI statistics consult it; it never affects the default output.
    pub alpha: f64,
    /// PRINTALLTYPES PROC option (M33.3). When false (default), the printed
    /// table shows only the all-CLASS-combined `_TYPE_`; when true, every
    /// generated `_TYPE_` subtable is printed.
    pub printalltypes: bool,
    /// WAYS values (M33.3): each requests the `_TYPE_` rows whose number of
    /// active CLASS variables equals the value. Empty → no WAYS restriction.
    pub ways: Vec<usize>,
    /// TYPES specifications (M33.3): each entry is a set of CLASS variable
    /// names (a specific crossing, e.g. `(a*b)`). Empty → no TYPES restriction.
    pub types: Vec<Vec<String>>,
    pub output: Option<MeansOutput>,
}

pub struct MeansOutput {
    pub out: DatasetRef,
    /// (stat, var source, nom de sortie)
    pub specs: Vec<(String, String, String)>,
}

/// Execute PROC MEANS / SUMMARY. Called by `procs::execute_proc`.
pub fn execute(ast: &MeansAst, session: &mut Session) -> Result<()> {
    let (ds, in_libref, in_table) = crate::procs::common::open_input(&ast.data, session)?;

    let n_obs = ds.n_obs();

    // Resolve CLASS column indices (validate existence).
    let mut class_cols: Vec<usize> = Vec::with_capacity(ast.class.len());
    for cname in &ast.class {
        match ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(cname))
        {
            Some(i) => class_cols.push(i),
            None => {
                return Err(SasError::runtime(format!(
                    "Variable {} not found.",
                    cname.to_uppercase()
                )));
            }
        }
    }

    // Determine the VAR list: explicit `var`, else all NUMERIC variables not
    // in CLASS.
    let var_cols: Vec<usize> = if !ast.var.is_empty() {
        let mut v = Vec::with_capacity(ast.var.len());
        for vname in &ast.var {
            match ds
                .vars
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(vname))
            {
                Some(i) => v.push(i),
                None => {
                    return Err(SasError::runtime(format!(
                        "Variable {} not found.",
                        vname.to_uppercase()
                    )));
                }
            }
        }
        v
    } else {
        (0..ds.vars.len())
            .filter(|&i| ds.vars[i].ty == VarType::Num && !class_cols.contains(&i))
            .collect()
    };

    // Decode CLASS columns and VAR columns once each.
    let class_values: Vec<Vec<Value>> = class_cols
        .iter()
        .map(|&ci| decode_column(&ds, ci))
        .collect::<Result<_>>()?;
    let var_values: Vec<Vec<Value>> = var_cols
        .iter()
        .map(|&ci| decode_column(&ds, ci))
        .collect::<Result<_>>()?;

    // Resolve & decode the WEIGHT column once (None → unweighted path,
    // byte-identical to before).
    let weight_values: Option<Vec<Value>> = match &ast.weight {
        Some(wname) => {
            let wi = ds
                .vars
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(wname))
                .ok_or_else(|| {
                    SasError::runtime(format!("Variable {} not found.", wname.to_uppercase()))
                })?;
            Some(decode_column(&ds, wi)?)
        }
        None => None,
    };

    // Default report stats when none requested.
    let report_stats: Vec<String> = if ast.stats.is_empty() {
        vec![
            "n".into(),
            "mean".into(),
            "std".into(),
            "min".into(),
            "max".into(),
        ]
    } else {
        ast.stats.clone()
    };

    // --- BY processing: resolve, verify sortedness, partition into groups. ---
    // No BY → a single group spanning all rows (output byte-identical).
    let by_cols = resolve_by_cols(&ds, &ast.by)?;
    let by_values: Vec<Vec<Value>> = by_cols
        .iter()
        .map(|c| decode_column(&ds, c.col_idx))
        .collect::<Result<_>>()?;
    let by_groups_list: Vec<(Vec<Value>, Vec<usize>)> = if by_cols.is_empty() {
        vec![(Vec::new(), (0..n_obs).collect())]
    } else {
        let descending: Vec<bool> = by_cols.iter().map(|c| c.descending).collect();
        let by_names: Vec<String> = by_cols.iter().map(|c| c.name.clone()).collect();
        let in_display = format!("{in_libref}.{in_table}");
        by_groups(&by_values, &descending, n_obs, &by_names, &in_display)?
    };
    let by_names: Vec<String> = by_cols.iter().map(|c| c.name.clone()).collect();

    // --- WAYS / TYPES restriction (M33.3): the set of _TYPE_ values to keep,
    // or None for "no restriction" (default path). ---
    let k = class_cols.len();
    let allowed = allowed_types(ast, &ast.class, k)?;

    // Which _TYPE_ subtables the listing prints. SAS default: ONLY the highest
    // _TYPE_ (all CLASS crossed). PRINTALLTYPES (or any WAYS/TYPES request)
    // prints each selected _TYPE_ as its own subtable. Without CLASS there is a
    // single _TYPE_=0 table either way (byte-identical default).
    let print_types: Vec<u64> = if k == 0 {
        vec![0]
    } else if ast.printalltypes || allowed.is_some() {
        // Every selected _TYPE_, ascending. With no WAYS/TYPES but
        // PRINTALLTYPES → all 2^k types.
        let mut v: Vec<u64> = match &allowed {
            Some(set) => set.iter().copied().collect(),
            None => (0u32..(1u32 << k))
                .map(|mask| {
                    let active: Vec<usize> = (0..k).filter(|&i| (mask >> i) & 1 == 1).collect();
                    type_mask(&active, k)
                })
                .collect(),
        };
        v.sort_unstable();
        v.dedup();
        v
    } else {
        // Default: only the highest _TYPE_ (all CLASS active) → byte-identical.
        vec![type_mask(&(0..k).collect::<Vec<_>>(), k)]
    };

    // --- Report ---
    if !ast.noprint {
        // Title printed once per proc invocation.
        session.listing.page_header();
        let title = "The MEANS Procedure";
        let ls = session.listing.ls();
        let pad = ls.saturating_sub(title.len()) / 2;
        session
            .listing
            .write_line(&format!("{}{}", " ".repeat(pad), title));
        session.listing.blank();

        for (by_key, grp_rows) in &by_groups_list {
            if !by_names.is_empty() {
                emit_by_heading(session, &by_names, by_key);
            }
            // Default single-type path stays byte-identical: when k==0 or the
            // only printed type is the full crossing AND neither PRINTALLTYPES
            // nor WAYS/TYPES is active, use the original combined-table emitter.
            let full_type = type_mask(&(0..k).collect::<Vec<_>>(), k);
            if !ast.printalltypes && allowed.is_none() && print_types == [full_type] {
                emit_report_group(
                    session,
                    &ds,
                    &class_cols,
                    &class_values,
                    &var_cols,
                    &var_values,
                    weight_values.as_deref(),
                    &report_stats,
                    ast.alpha,
                    grp_rows,
                );
            } else {
                for &ty in &print_types {
                    emit_report_type(
                        session,
                        &ds,
                        &class_cols,
                        &class_values,
                        &var_cols,
                        &var_values,
                        weight_values.as_deref(),
                        &report_stats,
                        ast.alpha,
                        grp_rows,
                        ty,
                    );
                }
            }
        }
    }

    // --- OUTPUT OUT= ---
    if let Some(out) = &ast.output {
        write_output(
            session,
            &ds,
            &class_cols,
            &class_values,
            &var_values,
            &var_cols,
            weight_values.as_deref(),
            out,
            &by_cols,
            &by_groups_list,
            ast.alpha,
            allowed.as_ref(),
        )?;
    }

    // --- ODS OUTPUT Summary= (M22.3) ---
    // Capture la table ODS "Summary" comme dataset si `ODS OUTPUT Summary=...`
    // est actif. Inactif par défaut (registre vide) → aucun effet, listing
    // byte-identique. La table Summary = une ligne par variable de VAR, avec
    // colonnes Variable + une par statistique du rapport.
    if let Some(target) = session.ods_output_target("Summary") {
        write_ods_summary(
            session,
            &ds,
            &var_cols,
            &var_values,
            weight_values.as_deref(),
            &report_stats,
            ast.alpha,
            &target,
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
