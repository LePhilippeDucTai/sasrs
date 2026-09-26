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
//! ## WEIGHT statement (jalon WEIGHT, J03-P2)
//! `weight <var>;` — une seule variable numérique. Quand elle est présente,
//! les **Moments** et les mesures **Basic** mean/std/variance sont calculés
//! avec les formules pondérées :
//!   N = nb d'obs à x non manquant (défaut) ; Sum Weights = Σw_i ;
//!   Sum Observations = Σw_i x_i ; Mean = Σw_i x_i / Σw_i ;
//!   Variance = Σw_i(x_i−x̄_w)² / (Σw_i − 1) (VARDEF=DF, diviseur W−1)
//!   ou Σw_i(x_i−x̄_w)² / (Σw_i − Σw_i²/Σw_i) (VARDEF=WEIGHT) ;
//!   Std = √Variance ; Corrected SS = Σw_i(x_i−x̄_w)² ;
//!   Uncorrected SS = Σw_i x_i² ; Coeff Variation = 100·Std/x̄_w ;
//!   Std Error Mean = Std/√(Σw_i).
//!
//! Exclusions (SAS, PROC UNIVARIATE par défaut SANS EXCLNPWGT) : un poids
//! nul ou négatif est converti en 0 — l'observation RESTE comptée dans N
//! mais pèse 0 dans SUMWGT/moments et n'entre pas dans les quantiles
//! pondérés ; un x manquant est compté dans NMiss (règles
//! `common::partition_weighted_lax`). Avec l'option **EXCLNPWGT**, les
//! observations à poids nul, négatif ou manquant sont EXCLUSES de l'analyse
//! (N compris — `common::partition_weighted_strict`, mêmes règles que PROC
//! MEANS). Référence :
//! https://support.sas.com/documentation/cdl/en/procstat/63104/HTML/default/procstat_univariate_sect021.htm
//!
//! ## OUTPUT OUT= sous WEIGHT (J03-P2)
//! Les statistiques de `OUTPUT OUT=` (mean, std, var, sum, n, nmiss,
//! quantiles…) suivent les MÊMES formules pondérées que le listing, avec
//! les mêmes règles N/NMiss selon EXCLNPWGT.
//!
//! ## Tests de normalité sous WEIGHT (J03-P2)
//! L'option NORMAL « is not available with the WEIGHT statement » (doc SAS
//! ci-dessus) : demandée avec un WEIGHT, elle produit une NOTE dans le log
//! et aucune section Tests for Normality.
//!
//! ## Simplifications SAS documentées (WEIGHT)
//! - Quantiles pondérés : calculés via `common::weighted_quantile_def5`
//!   (analogue pondéré de la Définition 5 — position par poids cumulés,
//!   partagé avec PROC MEANS depuis J03-P2). La section Quantiles (et
//!   Median/Q1/Q3/Range/IQR de Basic Measures) est affichée avec les
//!   valeurs pondérées.
//! - Mode / Extreme Observations : affichés à partir des valeurs BRUTES
//!   (non pondérées) — SAS : « The weight variable does not change how the
//!   procedure determines the range, mode, extreme values ».

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::num_var_meta;
use crate::procs::common::{
    self, VarDef, by_groups, centered, decode_column, partition_weighted, partition_weighted_lax,
    partition_weighted_strict, phi_inv, probnorm, resolve_by_cols, sample_std, weighted_mean_css,
    weighted_quantile_def5, weighted_variance,
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
    /// Definition 5 (`common::weighted_quantile_def5`); Extreme Observations
    /// list the raw extreme values (see file header).
    pub weight: Option<String>,
    /// VARDEF= divisor for the weighted variance (J03-P2). Default DF
    /// (divisor Σw − 1); WEIGHT divides by Σw − Σw²/Σw.
    pub vardef: VarDef,
    /// EXCLNPWGT PROC option (J03-P2): exclude observations with
    /// nonpositive/missing weights from the analysis (N included). Default
    /// false → a zero/negative weight counts 0 in the weighted formulas but
    /// the observation stays in N (SAS default).
    pub exclnpwgt: bool,
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
    /// NOPRINT (J02-P4): suppress every report section, BY heading, fitted
    /// table and plot — OUTPUT OUT= datasets and log NOTEs are still produced.
    /// Default false → report identical to the pre-J02-P4 output.
    pub noprint: bool,
}

/// A graphical statement requested in PROC UNIVARIATE (M29.3).
#[derive(Debug, Clone, PartialEq)]
pub struct UnivariatePlot {
    pub kind: UnivariatePlotKind,
    /// Target variable name (the first identifier after the keyword). `None`
    /// when the statement carries no explicit variable (e.g. `histogram;`).
    pub var: Option<String>,
    /// `/ NORMAL` was requested on the statement (M45.2): fit a normal
    /// distribution to the plotted variable — emit the "Fitted Normal
    /// Distribution" parameters table in the listing, and (under
    /// `--features graphics`) overlay the fitted curve / reference line.
    pub normal: bool,
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
    let proc_shows = !ast.noprint
        && (session.ods_displays("Moments")
            || session.ods_displays("BasicMeasures")
            || session.ods_displays("Quantiles")
            || session.ods_displays("ExtremeObs")
            || session.ods_displays("MissingValues")
            // J03-P2 — sous WEIGHT l'option NORMAL n'est pas disponible
            // (NOTE dans le log) : la section ne s'affiche jamais, elle
            // n'entre pas dans la porte d'affichage non plus.
            || (ast.normal
                && weight_values.is_none()
                && session.ods_displays("TestsForNormality")));
    if proc_shows {
        session.listing.page_header();
        centered(session, "The UNIVARIATE Procedure");
    }

    for (by_key, grp_rows) in &by_groups_list {
        if proc_shows && !by_names.is_empty() {
            emit_by_heading(session, &by_names, by_key);
        }
        for (vi, &ci) in var_cols.iter().enumerate() {
            // NOPRINT (J02-P4): nothing reaches the listing, but the decoding
            // above (and the OUTPUT below) still run.
            if !proc_shows {
                break;
            }
            match &weight_values {
                Some(wv) => {
                    // J03-P2 — partition selon EXCLNPWGT : par défaut (lax)
                    // les poids nuls/négatifs deviennent 0 mais l'observation
                    // reste dans N ; avec EXCLNPWGT (strict) elles sont
                    // exclues de l'analyse, N compris.
                    let (pairs, n_missing) = if ast.exclnpwgt {
                        partition_weighted_strict(&var_values[vi], wv, grp_rows)
                    } else {
                        partition_weighted_lax(&var_values[vi], wv, grp_rows)
                    };
                    // Extreme Observations : valeurs BRUTES (non pondérées —
                    // SAS : les poids ne changent pas les extrêmes). En mode
                    // lax ce sont toutes les obs à x non manquant ; en mode
                    // EXCLNPWGT, les seules obs utilisables.
                    let mut obs_pairs: Vec<(f64, usize)> = Vec::with_capacity(grp_rows.len());
                    for &row in grp_rows {
                        if let Some(vf) = value_to_num(&var_values[vi][row])
                            && !vf.is_nan()
                            && (!ast.exclnpwgt || {
                                let w = value_to_num(&wv[row]);
                                matches!(w, Some(wf) if !wf.is_nan() && wf > 0.0)
                            })
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
                        ast.vardef,
                        ast.normal,
                    )?;
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

        // M45.2 — « Fitted Normal Distribution » : une table de paramètres par
        // instruction graphique portant `/ NORMAL`, dans l'ordre des
        // instructions et par groupe BY (comme SAS). C'est du listing : elle
        // sort que ODS GRAPHICS soit ON ou OFF.
        if proc_shows {
            for plot in ast.plots.iter().filter(|p| p.normal) {
                let Some(vi) = plot_var_index(plot, &var_cols, &ds) else {
                    continue;
                };
                let Some((mu, sigma)) =
                    fitted_normal_params(&var_values[vi], weight_values.as_deref(), grp_rows)
                else {
                    continue;
                };
                emit_fitted_normal(session, &ds.vars[var_cols[vi]].name, mu, sigma);
            }
        }
    }

    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        n_obs, display_name
    ));

    // --- Graphical statements (M29.3) ---
    // J03-P9: NOPRINT suppresses printed output only — ODS Graphics images
    // are still produced (ODS Graphics: Procedures Guide), so the graphical
    // gate no longer tests `ast.noprint`. The listing tables above remain
    // suppressed under NOPRINT and OUTPUT OUT= is unaffected.
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
            weight_values.as_deref(),
            ast.vardef,
            ast.exclnpwgt,
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
