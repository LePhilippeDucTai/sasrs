//! PROC UNIVARIATE (jalon M5).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc univariate data=a ; var v... ; [by ... ;] run ;`
//!
//! Sections du rapport par variable (fidèles au listing SAS) :
//! - Moments : N, Mean, Std Deviation, Skewness (définition SAS avec
//!   correction n/(n-1)(n-2)), Kurtosis (excès, formule SAS), Sum,
//!   Variance, Corrected SS, Uncorrected SS, Coeff Variation, Std Error.
//! - Basic Statistical Measures : mean/median/mode ; std/variance/
//!   range/IQR.
//! - Quantiles : 100% Max, 99%, 95%, 90%, 75% Q3, 50% Median, 25% Q1,
//!   10%, 5%, 1%, 0% Min — DÉFINITION 5 de SAS (empirique, moyenne aux
//!   discontinuités) et PAS l'interpolation linéaire par défaut de
//!   Polars : implémenter à la main sur la colonne triée non-missing.
//! - Extreme Observations : 5 plus basses / 5 plus hautes avec n° d'obs.
//!
//! Les missings sont exclus (compter et afficher la section Missing
//! Values si présents).
//!
//! ## WEIGHT statement (jalon WEIGHT)
//! `weight <var>;` — une seule variable numérique. Quand elle est présente,
//! les **Moments** et les mesures **Basic** mean/std/variance sont calculés
//! avec les formules pondérées (VARDEF=DF) :
//!   N = n (nb d'obs utilisables) ; Sum Weights = Σw_i ;
//!   Sum Observations = Σw_i x_i ; Mean = Σw_i x_i / Σw_i ;
//!   Variance = Σw_i(x_i−x̄_w)² / (n−1) ; Std = √Variance ;
//!   Corrected SS = Σw_i(x_i−x̄_w)² ; Uncorrected SS = Σw_i x_i² ;
//!   Coeff Variation = 100·Std/x̄_w ; Std Error Mean = Std/√(Σw_i).
//! Exclusions : valeur missing, poids missing, ou poids ≤ 0
//! (voir `common::partition_weighted`). Le chemin non-pondéré reste
//! BYTE-IDENTIQUE (la pondération ne s'active que si `ast.weight.is_some()`).
//!
//! ## Simplifications SAS documentées (WEIGHT)
//! - Skewness / Kurtosis pondérés : DIFFÉRÉ. Affichés à partir des valeurs
//!   NON pondérées (formules g1/g2 existantes) — divergence documentée.
//! - Quantiles pondérés (M33.2) : calculés via `weighted_quantile_def5`
//!   (analogue pondéré de la Définition 5 — position par poids cumulés). La
//!   section Quantiles (et Median/Q1/Q3/Range/IQR de Basic Measures) est donc
//!   désormais affichée avec les valeurs pondérées.
//! - Extreme Observations (M33.2) : affichées aussi sous WEIGHT. Les extrêmes
//!   listent les VALEURS brutes (non pondérées) avec leur n° d'obs, sur les
//!   observations utilisables (mêmes exclusions que `partition_weighted`).

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::num_var_meta;
use crate::procs::common::{
    self, by_groups, centered, decode_column, partition_weighted, phi_inv, probnorm,
    resolve_by_cols, sample_std,
};

use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{Value, VarType, format_best};
use polars::prelude::*;
use std::cmp::Ordering;

mod emit;
mod normality;
mod output;
mod parse;
mod plot;
mod stats;

pub use parse::parse;

use emit::*;
use normality::*;
use output::*;
use plot::*;
use stats::*;

pub struct UnivariateAst {
    pub data: Option<DatasetRef>,
    pub var: Vec<String>,
    /// BY variables (var, descending). Input must be sorted by the BY key.
    pub by: Vec<(String, bool)>,
    /// WEIGHT variable (single numeric var). When `Some`, the Moments and
    /// Basic Measures use the weighted formulas, and Quantiles use the weighted
    /// Definition 5 (`weighted_quantile_def5`); Extreme Observations list the
    /// raw extreme values (see file header).
    pub weight: Option<String>,
    pub output: Option<UnivariateOutput>,
    /// Tests for Normality requested (PROC option `normal` or `var x / normal`).
    /// When true and no WEIGHT is in effect, the "Tests for Normality" block is
    /// emitted after the Quantiles section. Default false → report is
    /// byte-identical to the pre-M21.3 output.
    pub normal: bool,
    /// Graphical statements (HISTOGRAM/QQPLOT/PROBPLOT/CDFPLOT/PPPLOT) seen, in
    /// source order with their target variable. When `ods_graphics.enabled` is
    /// false the rendering stays deferred (a single NOTE, as before M29.3);
    /// when enabled each plot is wired to the ODS GRAPHICS image infrastructure
    /// (M29.3) — an image under `--features graphics`, a deferral NOTE otherwise.
    pub plots: Vec<UnivariatePlot>,
}

/// A graphical statement requested in PROC UNIVARIATE (M29.3).
#[derive(Debug, Clone, PartialEq)]
pub struct UnivariatePlot {
    pub kind: UnivariatePlotKind,
    /// Target variable name (the first identifier after the keyword). `None`
    /// when the statement carries no explicit variable (e.g. `histogram;`).
    pub var: Option<String>,
}

/// Kind of UNIVARIATE graphical statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnivariatePlotKind {
    Histogram,
    QqPlot,
    ProbPlot,
    CdfPlot,
    PpPlot,
}

impl UnivariatePlotKind {
    /// Uppercase statement keyword (for NOTE messages).
    pub fn keyword(self) -> &'static str {
        match self {
            UnivariatePlotKind::Histogram => "HISTOGRAM",
            UnivariatePlotKind::QqPlot => "QQPLOT",
            UnivariatePlotKind::ProbPlot => "PROBPLOT",
            UnivariatePlotKind::CdfPlot => "CDFPLOT",
            UnivariatePlotKind::PpPlot => "PPPLOT",
        }
    }
}

/// OUTPUT OUT= specification: target dataset + (statistic keyword, output
/// variable names) pairs. Output names are paired positionally with the VAR
/// list.
pub struct UnivariateOutput {
    pub out: DatasetRef,
    /// (stat keyword lowercased, output var names in VAR-list order)
    pub specs: Vec<(String, Vec<String>)>,
}

/// SAS DEFINITION 5 quantile (default QNTLDEF=5) of a fraction `p` over the
/// already-sorted non-missing values `sorted` (ascending). Conceptually
/// 1-indexed `x[1..=n]`. Empty → None.
///
/// ```text
/// np = n * p
/// j  = floor(np)
/// g  = np - j
/// if g == 0:  Q = (x[j] + x[j+1]) / 2   // average at the discontinuity
/// else:       Q = x[j+1]
/// ```
/// with clamping for the edges (p=1 → max, p=0 → min) and index guards.
///
/// `pub(crate)` so PROC MEANS / SUMMARY can reuse the IDENTICAL Definition-5
/// percentile computation (M33.3) instead of re-implementing it.
pub(crate) fn quantile_def5(sorted: &[f64], p: f64) -> Option<f64> {
    let n = sorted.len();
    if n == 0 {
        return None;
    }
    // 1-indexed accessor: x(i) for i in 1..=n.
    let x = |i: usize| sorted[i - 1];

    if p <= 0.0 {
        return Some(x(1));
    }
    if p >= 1.0 {
        return Some(x(n));
    }

    let np = n as f64 * p;
    let j = np.floor() as usize; // integer part
    let g = np - j as f64; // fractional part

    if g == 0.0 {
        // Average at the discontinuity. j in 1..=n-1 here (np<n since p<1,
        // and np>0 since p>0 → j>=0; if j==0, g>0 so we are in the else arm).
        if j >= n {
            Some(x(n))
        } else {
            Some((x(j) + x(j + 1)) / 2.0)
        }
    } else if j == 0 {
        Some(x(1))
    } else if j >= n {
        Some(x(n))
    } else {
        Some(x(j + 1))
    }
}

// ─────────────────────────────── execute ──────────────────────────────────

pub fn execute(ast: &UnivariateAst, session: &mut Session) -> Result<()> {
    let (ds, display_name) = common::open_input_display(&ast.data, session)?;

    let n_obs = ds.n_obs();

    // Determine the analysis variable list: explicit `var`, else ALL numeric.
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
            .filter(|&i| ds.vars[i].ty == VarType::Num)
            .collect()
    };

    // Decode each analysis variable's column once.
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

    // --- BY processing: resolve, verify sortedness, partition into groups. ---
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
        by_groups(&by_values, &descending, n_obs, &by_names, &display_name)?
    };
    let by_names: Vec<String> = by_cols.iter().map(|c| c.name.clone()).collect();

    // M38.4 — ODS SELECT/EXCLUDE : les sections d'UNIVARIATE portent leurs
    // noms d'objets ODS SAS (Moments, BasicMeasures, TestsForNormality,
    // Quantiles, ExtremeObs, MissingValues). Si la liste de sélection ne
    // laisse passer aucune section, l'en-tête de page et les en-têtes BY sont
    // supprimés aussi (SAS ne produit pas de page vide). `MissingValues` entre
    // dans ce test même sans missing dans les données (léger sur-affichage de
    // l'en-tête dans ce cas limite, documenté).
    let proc_shows = session.ods_displays("Moments")
        || session.ods_displays("BasicMeasures")
        || session.ods_displays("Quantiles")
        || session.ods_displays("ExtremeObs")
        || session.ods_displays("MissingValues")
        || (ast.normal && session.ods_displays("TestsForNormality"));
    if proc_shows {
        session.listing.page_header();
        centered(session, "The UNIVARIATE Procedure");
    }

    for (by_key, grp_rows) in &by_groups_list {
        if proc_shows && !by_names.is_empty() {
            emit_by_heading(session, &by_names, by_key);
        }
        for (vi, &ci) in var_cols.iter().enumerate() {
            match &weight_values {
                Some(wv) => {
                    // Weighted path: usable (value, weight) pairs + excluded count.
                    let (pairs, n_missing) = partition_weighted(&var_values[vi], wv, grp_rows);
                    // Also collect the usable values with their 1-based obs
                    // numbers (in row order) for the Extreme Observations
                    // section — extremes report the raw VALUES, not weighted,
                    // so we mirror the exclusion rule of `partition_weighted`.
                    let mut obs_pairs: Vec<(f64, usize)> = Vec::with_capacity(grp_rows.len());
                    for &row in grp_rows {
                        let v = value_to_num(&var_values[vi][row]);
                        let w = value_to_num(&wv[row]);
                        if let (Some(vf), Some(wf)) = (v, w)
                            && !vf.is_nan()
                            && !wf.is_nan()
                            && wf > 0.0
                        {
                            obs_pairs.push((vf, row + 1));
                        }
                    }
                    emit_variable_weighted(
                        session,
                        &ds.vars[ci].name,
                        &pairs,
                        &obs_pairs,
                        n_missing,
                        grp_rows.len(),
                    );
                }
                None => {
                    // Drop missings into (value, 1-based obs number) pairs, in the
                    // group's row order.
                    let mut data: Vec<(f64, usize)> = Vec::with_capacity(grp_rows.len());
                    let mut n_missing = 0usize;
                    for &row in grp_rows {
                        match value_to_num(&var_values[vi][row]) {
                            Some(f) if !f.is_nan() => data.push((f, row + 1)),
                            _ => n_missing += 1,
                        }
                    }
                    emit_variable(
                        session,
                        &ds.vars[ci].name,
                        &data,
                        n_missing,
                        grp_rows.len(),
                        ast.normal,
                    )?;
                }
            }
        }
    }

    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        n_obs, display_name
    ));

    // --- Graphical statements (M29.3) ---
    if !ast.plots.is_empty() {
        if !session.ods_graphics.enabled {
            // ODS GRAPHICS off: rendering stays deferred (byte-identical to the
            // pre-M29.3 behaviour — a single NOTE for the whole PROC step).
            session
                .log
                .note("HISTOGRAM/QQPLOT: graphical output deferred to ODS GRAPHICS (M29).");
        } else {
            // ODS GRAPHICS on: wire each plot to the image infrastructure.
            for plot in &ast.plots {
                render_plot(session, plot, &var_cols, &var_values, &ds);
            }
        }
    }

    // --- OUTPUT OUT= ---
    if let Some(out) = &ast.output {
        write_output(
            session,
            &ds,
            &var_cols,
            &var_values,
            out,
            &by_cols,
            &by_groups_list,
        )?;
    }

    Ok(())
}

#[cfg(feature = "graphics")]
mod plot_graphics;

#[cfg(test)]
mod tests;
