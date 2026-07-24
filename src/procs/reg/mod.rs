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

mod diagnostics;
mod fit;
mod influence;
mod matrices;
mod output;
mod parse;
mod ridge;
mod selection;
mod tests_stmt;

pub use self::parse::parse;

use self::diagnostics::*;
use self::fit::*;
use self::influence::*;
use self::matrices::*;
use self::output::*;
use self::ridge::*;
use self::selection::*;
use self::tests_stmt::*;

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone)]
pub struct RegAst {
    pub data_options: RegDataOptions,
    /// All MODEL statements, in source order. Each carries the OUTPUT
    /// statement(s) that followed it (SAS associates an OUTPUT with the
    /// MODEL it follows).
    pub models: Vec<RegModelEntry>,
    /// M29.3 — an explicit `PLOTS ...;` statement was seen. Its complex forms
    /// are deferred (a NOTE); the simple residuals-vs-predicted diagnostic is
    /// driven automatically from `ods_graphics.enabled`, not from this flag.
    pub plots_requested: bool,
    /// M36.11 — parsed `PLOTS=(…)` request set (PROC- or MODEL-level diagnostic
    /// panel selection). Defaults to "no explicit request"; `PLOTS=NONE`
    /// suppresses even the automatic diagnostic image.
    pub plot_requests: PlotRequests,
    /// M36.11 — traditional `PLOT y*x …;` statement requests, in source order.
    pub plot_statements: Vec<PlotPair>,
    /// M36.7 — `WEIGHT var;` weight variable (weighted least squares).
    pub weight: Option<String>,
    /// M36.7 — `FREQ var;` frequency variable (replication counts).
    pub freq: Option<String>,
    /// M36.7 — `BY var1 var2 …;` by-group processing variables.
    pub by: Vec<String>,
    /// M36.7 — `ID var1 …;` identification variables for diagnostic listings.
    pub id: Vec<String>,
    /// M36.8 — PROC-statement `SIMPLE` option: print descriptive statistics for
    /// all model variables.
    pub simple: bool,
    /// M36.8 — PROC-statement `CORR` option: print the correlation matrix among
    /// all model variables.
    pub corr: bool,
    /// M36.10 — `VAR v1 v2 …;`: variables declared for later interactive editing.
    /// Recorded only (used by SAS to make variables available to ADD between RUN
    /// groups); does not affect a non-interactive fit.
    pub var_list: Vec<String>,
    /// M36.10 — a `REWEIGHT …;` statement was seen (interactive observation
    /// reweighting). Deferred: parsed, a NOTE emitted at execute time.
    pub reweight_seen: bool,
    /// M36.10 — a `REFIT;` statement was seen (interactive refit). Deferred.
    pub refit_seen: bool,
    /// M36.10 — a `PAINT …;` statement was seen (interactive plot painting).
    /// Deferred (graphics-related).
    pub paint_seen: bool,
}

/// M36.11 — the parsed `PLOTS=(…)` diagnostic-panel request set. Each plot
/// family is an independent bool; `none` (PLOTS=NONE) and `all` (PLOTS=ALL)
/// are recorded explicitly so the executor can both suppress (NONE) and expand
/// (ALL) the automatic diagnostics. The struct's `Default` is "no explicit
/// PLOTS= seen" (every field false), which preserves the pre-M36.11 behaviour.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlotRequests {
    /// PLOTS=DIAGNOSTICS — the residual/leverage/QQ diagnostics panel.
    pub diagnostics: bool,
    /// PLOTS=RESIDUALS — residual-by-regressor scatter(s).
    pub residuals: bool,
    /// PLOTS=FIT — fit plot (regression line + CLM/CLI bands, single regressor).
    pub fit: bool,
    /// PLOTS=ALL — request every diagnostic family.
    pub all: bool,
    /// PLOTS=NONE — suppress all plots, including the automatic diagnostic image.
    pub none: bool,
    /// `PLOTS(UNPACK)=…` — render panel components as separate images. Recorded
    /// only (panel-vs-separate is a rendering detail; we always emit separate
    /// images), so this is informational.
    pub unpack: bool,
    /// `PLOTS(ONLY)=…` — render only the explicitly requested plots (suppress the
    /// default DIAGNOSTICS panel that PLOTS= would otherwise add). Recorded.
    pub only: bool,
    /// An explicit `PLOTS=` (with a value other than NONE) was seen.
    pub explicit: bool,
}

impl PlotRequests {
    /// True when at least one renderable plot family was requested (ALL expands
    /// to all three families). NONE always yields false.
    fn any(&self) -> bool {
        if self.none {
            return false;
        }
        self.all || self.diagnostics || self.residuals || self.fit
    }
}

/// M36.11 — one term of a `keyword.`-or-variable axis in a traditional `PLOT`
/// statement. `PREDICTED.`/`P.` and `RESIDUAL.`/`R.` map to the fitted-value and
/// residual special variables; everything else is a plain model variable name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlotVar {
    /// A model variable, by (uppercased) name.
    Named(String),
    /// `PREDICTED.` / `P.` — fitted value ŷ.
    Predicted,
    /// `RESIDUAL.` / `R.` — residual y − ŷ.
    Residual,
}

/// M36.11 — one `y*x` pair from a traditional `PLOT` statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlotPair {
    pub y: PlotVar,
    pub x: PlotVar,
}

#[derive(Debug, Clone)]
pub struct RegModelEntry {
    pub model: RegModel,
    pub outputs: Vec<RegOutput>,
    /// TEST statements that followed this MODEL (M36.1).
    pub tests: Vec<RegTest>,
    /// RESTRICT statements that followed this MODEL (M36.1).
    pub restricts: Vec<RegRestrict>,
    /// MTEST statements that followed this MODEL — multivariate hypothesis tests
    /// across the model responses (M36.10).
    pub mtests: Vec<RegMtest>,
    /// Regressors added via `ADD x …;` after this MODEL (M36.10, run-group
    /// editing). Applied to the final fit.
    pub add: Vec<String>,
    /// Regressors removed via `DELETE x …;` after this MODEL (M36.10).
    pub delete: Vec<String>,
}

/// A linear equation over regressor names (and the keyword `INTERCEPT`),
/// normalised so that every term is moved to the left-hand side:
///   Σ coef_i · var_i = rhs
/// where `var_i` is an uppercased regressor name (or the literal `"INTERCEPT"`)
/// and `rhs` is the net constant after moving variables left / constants right.
#[derive(Debug, Clone, PartialEq)]
pub struct LinEq {
    /// (coefficient, uppercased variable name). The intercept maps to the
    /// reserved name `"INTERCEPT"`.
    pub terms: Vec<(f64, String)>,
    /// The net constant on the right-hand side.
    pub rhs: f64,
}

/// A `[label:] TEST eq [, eq ...];` statement (M36.1).
#[derive(Debug, Clone)]
pub struct RegTest {
    pub label: Option<String>,
    pub equations: Vec<LinEq>,
}

/// A `RESTRICT eq [, eq ...];` statement (M36.1).
#[derive(Debug, Clone)]
pub struct RegRestrict {
    pub equations: Vec<LinEq>,
}

#[derive(Debug, Clone)]
pub struct RegDataOptions {
    pub input: Option<DatasetRef>,
    /// M36.8 — `OUTEST=ds`: parameter-estimates output dataset (+ modifiers).
    pub outest: Option<OutEst>,
    /// M36.8 — `OUTSSCP=ds`: sums-of-squares-and-crossproducts output dataset.
    pub outsscp: Option<DatasetRef>,
    /// M36.9 — `RIDGE=value-list`: ridge-regression constants. Empty ⇒ no ridge.
    pub ridge: Vec<f64>,
    /// M36.9 — `PCOMIT=value-list`: incomplete-principal-component drop counts.
    /// Empty ⇒ no IPC regression.
    pub pcomit: Vec<f64>,
    /// M36.9 — `OUTVIF` (valid with RIDGE): emit `_TYPE_="RIDGEVIF"` rows in
    /// OUTEST= carrying the per-k ridge VIF values.
    pub outvif: bool,
}

/// M36.8 — OUTEST= request with its modifiers.
#[derive(Debug, Clone)]
pub struct OutEst {
    pub out: DatasetRef,
    /// COVOUT → emit `_TYPE_="COV"` rows (covariance matrix MSE·(X'X)⁻¹).
    pub covout: bool,
    /// OUTSEB → emit a `_TYPE_="SEB"` row with the parameter standard errors.
    pub outseb: bool,
    /// EDF → emit `_IN_`/`_P_`/`_EDF_` degrees-of-freedom columns.
    pub edf: bool,
    /// TABLEOUT → emit `_LB_`/`_UB_` confidence-bound columns (estimate subset).
    pub tableout: bool,
}

#[derive(Debug, Clone)]
pub struct RegModel {
    /// The dependent (response) variable(s). SAS PROC REG permits several
    /// responses on the LHS of MODEL (`model y1 y2 = x1 x2;`) for use by MTEST
    /// (M36.10). The single-response code paths read `dependent()` (the first
    /// response); a single-response model therefore behaves exactly as before.
    pub dependents: Vec<String>,
    pub regressors: Vec<String>,
    pub noint: bool,
    pub noprint: bool,
    /// SELECTION= option (FORWARD / BACKWARD / STEPWISE), if requested.
    pub selection: Option<Selection>,
    /// Significance level α (default 0.05) → 100(1−α)% intervals (M36.2).
    pub alpha: f64,
    /// CLB → confidence limits on the parameter estimates (M36.2).
    pub clb: bool,
    /// CLM → per-observation mean confidence limits in Output Statistics.
    pub clm: bool,
    /// CLI → per-observation individual prediction limits in Output Statistics.
    pub cli: bool,
    /// R → residual-analysis "Output Statistics" listing (M36.3).
    pub r: bool,
    /// INFLUENCE → influence-diagnostics listing (M36.3).
    pub influence: bool,
    /// VIF → Variance Inflation column in the parameter table (M36.4).
    pub vif: bool,
    /// TOL → Tolerance column in the parameter table (M36.4).
    pub tol: bool,
    /// COLLIN → Collinearity Diagnostics table, intercept included (M36.4).
    pub collin: bool,
    /// COLLINOINT → Collinearity Diagnostics table, intercept excluded (M36.4).
    pub collinoint: bool,
    /// SPEC → White's test of first and second moment specification (M36.4).
    pub spec: bool,
    /// DW → Durbin-Watson statistic block (M36.4).
    pub dw: bool,
    /// DWPROB → Durbin-Watson with positive/negative autocorrelation p-values
    /// (implies DW). (M36.4)
    pub dwprob: bool,
    /// ACOV / HCC → heteroscedasticity-consistent (White HC0) covariance of the
    /// estimates plus HC standard errors. ACOV and HCC are synonyms; either sets
    /// this flag. (M36.4)
    pub acov: bool,
    /// SS1 → Type I (sequential) sum of squares column (M36.5).
    pub ss1: bool,
    /// SS2 → Type II (partial) sum of squares column (M36.5).
    pub ss2: bool,
    /// STB → standardized parameter-estimate column (M36.5).
    pub stb: bool,
    /// PCORR1 → squared partial correlation Type I column (M36.5).
    pub pcorr1: bool,
    /// PCORR2 → squared partial correlation Type II column (M36.5).
    pub pcorr2: bool,
    /// SCORR1 → squared semi-partial correlation Type I column (M36.5).
    pub scorr1: bool,
    /// SCORR2 → squared semi-partial correlation Type II column (M36.5).
    pub scorr2: bool,
    /// SEQB → sequential parameter-estimate column (M36.5).
    pub seqb: bool,
    /// PRESS → print the PRESS statistic as a model fit statistic (M36.5).
    pub press_opt: bool,
    /// M36.8 — XPX: print the X'X crossproducts matrix (augmented with X'Y/Y'Y).
    pub xpx: bool,
    /// M36.8 — I: print the (X'X)⁻¹ inverse matrix (augmented with estimates/SSE).
    pub inv: bool,
    /// M36.8 — COVB: print Covariance of Estimates = MSE·(X'X)⁻¹.
    pub covb: bool,
    /// M36.8 — CORRB: print Correlation of Estimates.
    pub corrb: bool,
}

impl RegModel {
    /// The primary (first) response variable. All single-response code paths use
    /// this so a one-response MODEL is byte-identical to the pre-M36.10 behaviour.
    pub fn dependent(&self) -> &str {
        &self.dependents[0]
    }
}

/// A `[label:] MTEST [equations] [/ options];` statement (M36.10). Performs the
/// multivariate test of a linear hypothesis across all model responses. With no
/// equations the default hypothesis tests that every non-intercept coefficient
/// is jointly zero (the overall multivariate regression test).
#[derive(Debug, Clone)]
pub struct RegMtest {
    pub label: Option<String>,
    /// Linear-hypothesis equations over the regressors (reusing `LinEq`/`build_lc`).
    /// Empty ⇒ the default "all regressors = 0" hypothesis.
    pub equations: Vec<LinEq>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelMethod {
    Forward,
    Backward,
    Stepwise,
    /// All-subsets, grouped by model size, ranked by R² (M36.6).
    RSquare,
    /// All-subsets, ranked overall by adjusted R² (M36.6).
    AdjRsq,
    /// All-subsets, ranked overall by Mallows' C(p) (M36.6).
    Cp,
    /// Stepwise maximum-R²-improvement (M36.6).
    MaxR,
    /// Stepwise minimum-R²-improvement (M36.6).
    MinR,
    /// No selection — fit the full model (M36.6).
    None,
}

#[derive(Debug, Clone, Copy)]
pub struct Selection {
    pub method: SelMethod,
    pub slentry: f64,
    pub slstay: f64,
    /// BEST=b — keep only the top `b` models in all-subsets tables (M36.6).
    pub best: Option<usize>,
    /// INCLUDE=k — force the first `k` regressors (MODEL order) into every
    /// model considered (M36.6).
    pub include: usize,
    /// START=k — smallest subset size to enumerate / build (M36.6).
    pub start: Option<usize>,
    /// STOP=k — largest subset size to enumerate / build (M36.6).
    pub stop: Option<usize>,
    /// DETAILS — emit the per-step detail tables (M36.6; parsed, gated).
    pub details: bool,
    /// STB — add standardized estimates to printed estimates (M36.6).
    pub stb: bool,
}

#[derive(Debug, Clone)]
pub struct RegOutput {
    pub out: DatasetRef,
    pub predicted: Option<String>,
    pub residual: Option<String>,
    /// M36.2 — std errors / prediction limits requested as output columns.
    pub stdp: Option<String>,
    pub stdi: Option<String>,
    pub stdr: Option<String>,
    pub lcl: Option<String>,
    pub ucl: Option<String>,
    pub lclm: Option<String>,
    pub uclm: Option<String>,
    /// M36.3 — influence/observation diagnostics requested as output columns.
    pub student: Option<String>,
    pub rstudent: Option<String>,
    pub cookd: Option<String>,
    pub h: Option<String>,
    pub press: Option<String>,
    pub dffits: Option<String>,
    pub covratio: Option<String>,
    /// DFBETAS= prefix. SAS does not accept a single name (DFBETAS is
    /// per-parameter); when given a prefix we emit one column per parameter
    /// named `<prefix>_<var>` (Intercept first if present).
    pub dfbetas: Option<String>,
}


// ───────────────────────── Formatting helpers ─────────────────────────

fn fmt5(v: f64) -> String {
    format!("{v:.5}")
}

fn fmt2(v: f64) -> String {
    format!("{v:.2}")
}

fn fmt_fit4(v: f64) -> String {
    format!("{v:.4}")
}

/// Format a confidence level (e.g. 95, 90, or 97.5) without a trailing `.0`.
fn fmt_level(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        // Trim trailing zeros from a fixed-precision rendering.
        let s = format!("{:.4}", v);
        let s = s.trim_end_matches('0').trim_end_matches('.');
        s.to_string()
    }
}

use crate::procs::common::fmt_p;


// ───────────────────────── Stat helpers ─────────────────────────

use crate::procs::common::two_sided_p;


// ───────────────────────── Listing helpers ─────────────────────────

use crate::procs::common::centered;

// ───────────────────────── VarMeta helper ─────────────────────────

fn num_var_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}


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
        if (ast.simple || ast.corr) && !ast.models.is_empty() {
            if let Some((names, cols)) = gather_simple_corr(
                &ds,
                &ast.models[0].model,
                grp_rows,
                weight_col.as_deref(),
                freq_col.as_deref(),
            ) {
                if ast.simple {
                    print_simple_stats(&names, &cols, session);
                }
                if ast.corr {
                    print_corr_matrix(&names, &cols, session);
                }
            }
        }
        // OUTSSCP= (M36.8): one SSCP matrix per BY group, built from the FIRST
        // model's analysis variables (Intercept + regressors + dependent) over
        // the same complete-case rows. Gated so a PROC without OUTSSCP= is
        // byte-identical to before.
        if let Some(out) = &ast.data_options.outsscp {
            if let Some((_, cols)) = gather_simple_corr(
                &ds,
                &ast.models[0].model,
                grp_rows,
                weight_col.as_deref(),
                freq_col.as_deref(),
            ) {
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

/// Build a PROC REG BY-group heading line (`<var>=<value> ...`), matching the
/// standard SAS BY-line used by the other procs (M36.7). Centered and emitted by
/// the per-model header path so it lands after "The REG Procedure".
fn reg_by_heading_line(by_names: &[String], by_key: &[crate::value::Value]) -> String {
    let parts: Vec<String> = by_names
        .iter()
        .zip(by_key)
        .map(|(name, v)| format!("{}={}", name, by_value_cell(v)))
        .collect();
    parts.join(" ")
}

/// Render a BY-key cell value for the heading line (M36.7).
fn by_value_cell(v: &crate::value::Value) -> String {
    match v {
        crate::value::Value::Num(f) => crate::value::format_best(*f, 12),
        crate::value::Value::Missing(k) => k.display(),
        crate::value::Value::Char(s) => s.trim_end().to_string(),
    }
}

/// Render an ID cell value for the diagnostic-listing leading column (M36.7).
fn id_value_cell(v: &crate::value::Value) -> String {
    match v {
        crate::value::Value::Num(f) => crate::value::format_best(*f, 12),
        crate::value::Value::Missing(k) => k.display(),
        crate::value::Value::Char(s) => s.trim_end().to_string(),
    }
}

/// Run a single MODEL statement: resolve columns, do listwise deletion, then
/// dispatch to the default/NOINT path or the SELECTION path. Writes any OUTPUT
/// dataset associated with the model.
#[allow(clippy::too_many_arguments)]
fn run_model(
    ast: &RegAst,
    entry: &RegModelEntry,
    ds: &SasDataset,
    in_libref: &str,
    in_table: &str,
    rows: &[usize],
    weight_col: Option<&[crate::value::Value]>,
    freq_col: Option<&[crate::value::Value]>,
    id_cols: &[(String, Vec<crate::value::Value>)],
    model_label: &str,
    // M36.7: BY-group heading line, threaded down so it can be emitted inside
    // the per-model header block (after "The REG Procedure"). `None` when no BY.
    by_heading: Option<&str>,
    // M36.8: OUTEST= accumulator. `Some` when a PROC OUTEST= was requested; this
    // model pushes its PARMS/cov/se data here for the per-PROC writer. `None`
    // keeps the OUTEST-free path byte-identical.
    outest_accum: Option<&mut Vec<OutEstEntry>>,
    session: &mut Session,
) -> Result<()> {
    let _ = (in_libref, in_table);
    let model = &entry.model;
    // M36.10 run-group editing (ADD/DELETE) — see `effective_regressors`.
    let regressors_owned: Option<Vec<String>> = effective_regressors(entry);
    let regressors: &[String] = match &regressors_owned {
        Some(v) => v.as_slice(),
        None => &model.regressors,
    };
    let p = regressors.len();
    let n_read = rows.len();

    // --- Resolve + decode the regressor columns (shared across responses) ---
    let reg_cols = decode_regressor_columns(ds, regressors)?;

    // M36.10 multi-response MODEL (`model y1 y2 = x;`): SAS PROC REG prints a
    // SEPARATE univariate regression analysis for EACH dependent, in MODEL
    // order, then the MTEST tables once. We loop the existing univariate
    // fit+print path once per response. With a single response (`dependents`
    // has one element) the loop body runs exactly once and the emitted output —
    // headers, spacing, NOTE lines, OUTEST rows, OUTPUT datasets — is
    // byte-identical to the prior single-response path.
    //
    // Per-response OUTPUT/diagnostics: each response's OUTPUT dataset, R /
    // INFLUENCE / CLM / CLI listings, OUTEST row, and printed matrices are
    // produced from that response's own fit, exactly as a univariate run would.
    // OUTSSCP / SIMPLE / CORR are PROC-level displays driven off the FIRST
    // model's variables and are emitted once in the caller, unchanged.
    let mut outest_accum = outest_accum;
    // MTEST runs ONCE after all per-response blocks. It needs the selected design
    // (intercept + chosen regressors). With no SELECTION this is identical for
    // every response; with SELECTION the prior code used the (sole) response's
    // choice, so we capture the FIRST response's selection here for parity.
    let mut mtest_inputs: Option<(Vec<usize>, Vec<String>, bool)> = None;
    for (resp_i, dep_name) in model.dependents.iter().enumerate() {
        let dep_name: &str = dep_name.as_str();
        let dep_idx = find_col(ds, dep_name)?;
        if ds.vars[dep_idx].ty != VarType::Num {
            return Err(SasError::runtime(format!(
                "Dependent variable {} must be numeric.",
                dep_name.to_uppercase()
            )));
        }
        let dep_col = decode_column(ds, dep_idx)?;

    // --- M36.7 WEIGHT/FREQ bookkeeping + listwise deletion — see
    // `gather_complete_cases`. When no WEIGHT and no FREQ are present,
    // `weighting` stays inactive and the whole analysis is byte-identical to
    // the prior OLS path.
    let CaseData {
        xcols,
        y_vec,
        complete_mask,
        weighting,
        id_used,
    } = gather_complete_cases(ds, rows, weight_col, freq_col, id_cols, &dep_col, &reg_cols);
    let id_first: Option<&[String]> = if id_cols.is_empty() {
        None
    } else {
        Some(&id_used)
    };

    let n = y_vec.len();
    session
        .log
        .note(&format!("There were {} observations used.", n));

    let intercept = !model.noint;

    // --- SELECTION path: choose the final regressor subset, then fit/print it.
    let selected: Vec<usize> = if let Some(sel) = model.selection {
        match run_selection(&sel, &xcols, &y_vec, regressors, intercept, model, session) {
            Some(s) => s,
            None => {
                // Nothing entered (FORWARD/STEPWISE) — fit the intercept-only
                // model (or note for NOINT) and finish, no OUTPUT. With multiple
                // responses, move on to the next dependent (`continue`); with a
                // single response this ends the only iteration just as the prior
                // `return Ok(())` did, so the output is unchanged.
                fit_and_print_empty(
                    model,
                    dep_name,
                    &FitReportOptions {
                        n_read,
                        n,
                        model_label,
                        by_heading,
                        ..Default::default()
                    },
                    session,
                );
                continue;
            }
        }
    } else {
        (0..p).collect()
    };

    // Build the final design matrix over the selected columns.
    let sel_p = selected.len();
    let p_eff = sel_p + intercept as usize;

    if n <= p_eff {
        return Err(SasError::runtime("Not enough observations for regression"));
    }

    let mut x_mat: Vec<Vec<f64>> = Vec::with_capacity(n);
    for i in 0..n {
        let mut row = Vec::with_capacity(p_eff);
        if intercept {
            row.push(1.0);
        }
        for &c in &selected {
            row.push(xcols[c][i]);
        }
        x_mat.push(row);
    }

    let sel_reg_names: Vec<String> = selected.iter().map(|&c| regressors[c].clone()).collect();

    // Capture the first response's selected design for the post-loop MTEST.
    if resp_i == 0 {
        mtest_inputs = Some((selected.clone(), sel_reg_names.clone(), intercept));
    }

    // Weighted-least-squares fit when WEIGHT/FREQ is active; plain OLS
    // otherwise (byte-identical default path).
    let fit_result = match &weighting {
        Some(w) => weighted_ols_fit(&x_mat, &y_vec, &w.wf),
        None => ols_fit(&x_mat, &y_vec),
    };
    let fit = match fit_result {
        Ok(f) => f,
        Err(e) => {
            session.log.error(&format!("Regression failed: {}", e));
            return Err(e);
        }
    };

    // --- RESTRICT (M36.1): re-estimate under the linear constraints. The model
    // is printed as the restricted fit; TEST then operates on that fit. When
    // there are no RESTRICT statements this stays None and the OLS path is
    // byte-identical to before.
    let restricted = if entry.restricts.is_empty() {
        None
    } else {
        compute_restricted(
            &entry.restricts,
            &sel_reg_names,
            intercept,
            &x_mat,
            &y_vec,
            &fit,
            n,
        )?
    };

    // --- VIF / TOL (M36.4): per-regressor tolerance & variance inflation,
    // computed from the selected regressor columns (no intercept). Only built
    // when requested so the default path stays allocation-free / byte-identical.
    let sel_cols: Vec<Vec<f64>> = selected.iter().map(|&c| xcols[c].clone()).collect();
    let tolvif = if (model.vif || model.tol) && !model.noprint {
        Some(vif_tol(&sel_cols))
    } else {
        None
    };

    // --- Partial-SS / correlation statistics (M36.5). Computed on the OLS fit
    // (not the restricted fit). Only built when any SS1/SS2/STB/PCORR/SCORR/SEQB
    // option is requested, so the default / RESTRICT paths stay byte-identical.
    let want_seq = model.ss1
        || model.ss2
        || model.stb
        || model.pcorr1
        || model.pcorr2
        || model.scorr1
        || model.scorr2
        || model.seqb;
    // SST consistent with fit_and_print: corrected total (intercept) or
    // uncorrected total Σy² (NOINT).
    let sst_seq = if intercept {
        let ybar = y_vec.iter().sum::<f64>() / n as f64;
        y_vec.iter().map(|v| (v - ybar) * (v - ybar)).sum()
    } else {
        y_vec.iter().map(|v| v * v).sum()
    };
    let seqstats = if want_seq && !model.noprint {
        Some(compute_seq_stats(
            model, &x_mat, &y_vec, &fit, sst_seq, intercept,
        ))
    } else {
        None
    };

    // PRESS statistic (M36.5) — see `compute_press_stat`.
    let press_stat = compute_press_stat(model, &x_mat, &fit, weighting.as_ref());

    // --- RIDGE= / PCOMIT= (M36.9): when requested, SAS replaces the ordinary
    // parameter-estimates analysis with the ridge / incomplete-principal-
    // component results. Entirely gated on the new PROC options so the default
    // path is byte-identical. Ridge/IPC operate on the standardized model and
    // require an intercept (SAS centers); NOINT ⇒ clean NOTE + skip.
    let want_ridge = !ast.data_options.ridge.is_empty();
    let want_ipc = !ast.data_options.pcomit.is_empty();
    let ridge_ipc_active = want_ridge || want_ipc;
    // Back-transformed ridge/IPC rows to attach to this model's OUTEST entry.
    let mut ridge_ipc_rows: Vec<RidgeIpcRow> = Vec::new();
    // Report context + optional statistics shared by both printers (the
    // ridge/IPC one only reads the header context).
    let fit_opts = FitReportOptions {
        n_read,
        n,
        intercept,
        model_label,
        restricted: restricted.as_ref(),
        tolvif: tolvif.as_ref(),
        seqstats: seqstats.as_ref(),
        press_stat,
        weighting: weighting.as_ref(),
        by_heading,
    };
    if ridge_ipc_active {
        if !intercept {
            session.log.note(
                "RIDGE/PCOMIT regression requires an intercept; the NOINT model is skipped.",
            );
        } else {
            ridge_ipc_rows = fit_and_print_ridge_ipc(
                ast,
                model,
                dep_name,
                &sel_reg_names,
                &sel_cols,
                &y_vec,
                &fit_opts,
                session,
            );
        }
    } else {
        fit_and_print(model, dep_name, &sel_reg_names, &fit, &fit_opts, session);
    }

    // --- OUTEST= (M36.8): record this fit for the per-PROC parameter-estimates
    // dataset. Built from the existing fit (no refit). `_MODEL_` uses the bare
    // "MODELn" label (the model_label is "Model: MODELn").
    if let Some(accum) = outest_accum.as_deref_mut() {
        let n_used: f64 = weighting.as_ref().map(|w| w.total_n).unwrap_or(n as f64);
        let bare_label = model_label
            .strip_prefix("Model: ")
            .unwrap_or(model_label)
            .to_string();
        let mut entry = build_outest_entry(
            &bare_label,
            dep_name,
            &sel_reg_names,
            &fit,
            intercept,
            n_used,
            model.alpha,
        );
        // M36.9: attach back-transformed ridge / IPC rows so the per-PROC writer
        // emits the `_TYPE_`=RIDGE/RIDGEVIF/IPC rows alongside the PARMS row.
        entry.ridge_ipc = ridge_ipc_rows;
        accum.push(entry);
    }

    // MQ5.2 — shared per-response context for the post-fit option sections.
    let rf = RespFit {
        model,
        dep_name,
        x_mat: &x_mat,
        y_vec: &y_vec,
        sel_cols: &sel_cols,
        sel_reg_names: &sel_reg_names,
        fit: &fit,
        intercept,
        n,
        p_eff,
        weighting: weighting.as_ref(),
        id_first,
    };
    // --- Printed matrices (M36.8): XPX / I / COVB / CORRB — see
    // `print_matrix_options`.
    print_matrix_options(&rf, session);

    // --- Diagnostics / observation statistics (M36.2-M36.4) — see
    // `print_diagnostic_options`.
    print_diagnostic_options(&rf, session);

    // --- TEST (M36.1): operate on the model as fitted (restricted if present).
    run_test_section(entry, &rf, restricted.as_ref(), session)?;

    // --- OUTPUT dataset(s) for this model (complete cases only) ---
    write_outputs(
        entry,
        ds,
        &complete_mask,
        n,
        &fit,
        &x_mat,
        p_eff,
        model.alpha,
        &sel_reg_names,
        intercept,
        weighting.as_ref(),
        session,
    )?;

    // --- Diagnostics / PLOTS rendering (M29.3, M36.11) — see
    // `render_model_plots`.
    render_model_plots(ast, &rf, session);
    } // end per-response loop

    // --- MTEST (M36.10): multivariate hypothesis tests across all responses.
    // Self-contained: gathers a fresh multivariate response/design matrix with
    // listwise deletion across every response + the selected regressors, fits
    // the multivariate coefficient matrix, and prints the four MANOVA statistics
    // per MTEST. Printed ONCE, after every per-response univariate block. Only
    // entered when MTEST statements are present, so the single-response /
    // no-MTEST path is byte-identical.
    let run_mtest = !entry.mtests.is_empty() && !model.noprint;
    if let Some((selected, sel_reg_names, intercept)) = mtest_inputs.as_ref().filter(|_| run_mtest)
    {
        run_mtests(
            &entry.mtests,
            model,
            ds,
            rows,
            selected,
            sel_reg_names,
            *intercept,
            session,
        )?;
    }

    Ok(())
}

/// M36.10 run-group editing: ADD/DELETE adjust the regressor set for the
/// final fit. With neither present this returns `None` and the caller borrows
/// `model.regressors` directly (byte-identical). ADD appends
/// not-already-present names (MODEL order preserved, additions last); DELETE
/// removes matching names.
fn effective_regressors(entry: &RegModelEntry) -> Option<Vec<String>> {
    let model = &entry.model;
    if entry.add.is_empty() && entry.delete.is_empty() {
        None
    } else {
        let mut regs: Vec<String> = model.regressors.clone();
        for a in &entry.add {
            if !regs.iter().any(|r| r.eq_ignore_ascii_case(a)) {
                regs.push(a.clone());
            }
        }
        if !entry.delete.is_empty() {
            regs.retain(|r| !entry.delete.iter().any(|d| d.eq_ignore_ascii_case(r)));
        }
        Some(regs)
    }
}

/// Find a dataset column by (case-insensitive) name.
fn find_col(ds: &SasDataset, nm: &str) -> Result<usize> {
    ds.vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(nm))
        .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", nm.to_uppercase())))
}

/// Regressor resolution + decode is shared across all responses (the design
/// is common to every dependent on the MODEL LHS), so it is hoisted above the
/// per-response loop of `run_model`.
fn decode_regressor_columns(
    ds: &SasDataset,
    regressors: &[String],
) -> Result<Vec<Vec<crate::value::Value>>> {
    let p = regressors.len();
    let mut reg_idxs: Vec<usize> = Vec::with_capacity(p);
    for nm in regressors {
        let idx = find_col(ds, nm)?;
        if ds.vars[idx].ty != VarType::Num {
            return Err(SasError::runtime(format!(
                "Regressor {} must be numeric.",
                nm.to_uppercase()
            )));
        }
        reg_idxs.push(idx);
    }
    // --- Decode regressor columns (shared) ---
    let mut reg_cols: Vec<Vec<crate::value::Value>> = Vec::with_capacity(p);
    for &idx in &reg_idxs {
        reg_cols.push(decode_column(ds, idx)?);
    }
    Ok(reg_cols)
}

/// MQ5.2 — one response's complete-case data: the listwise-deleted regressor
/// columns and response vector, the complete-row mask, the M36.7 weighting
/// context, and the first ID variable's per-row display values.
struct CaseData {
    xcols: Vec<Vec<f64>>,
    y_vec: Vec<f64>,
    complete_mask: Vec<bool>,
    weighting: Option<Weighting>,
    id_used: Vec<String>,
}

/// MQ5.2 — gather one response's complete cases (M36.7 WEIGHT/FREQ/ID
/// bookkeeping + listwise deletion over the regressors and the response).
fn gather_complete_cases(
    ds: &SasDataset,
    rows: &[usize],
    weight_col: Option<&[crate::value::Value]>,
    freq_col: Option<&[crate::value::Value]>,
    id_cols: &[(String, Vec<crate::value::Value>)],
    dep_col: &[crate::value::Value],
    reg_cols: &[Vec<crate::value::Value>],
) -> CaseData {
    let p = reg_cols.len();
    // --- M36.7 weighting bookkeeping. `wf` accumulates the effective SS weight
    // w_i·f_i for each complete-case row; `total_n` accumulates Σf_i (FREQ
    // inflates the observation count / df, WEIGHT does not). `id_used` carries
    // the first ID variable's per-row display value when ID is given. When no
    // WEIGHT and no FREQ are present, `weighting` stays inactive and the whole
    // analysis is byte-identical to the prior OLS path.
    let has_weight = weight_col.is_some();
    let has_freq = freq_col.is_some();
    let mut wf: Vec<f64> = Vec::new();
    let mut total_n: f64 = 0.0;
    let mut id_used: Vec<String> = Vec::new();

    // --- Build regressor columns (numeric) and y vector (listwise deletion) ---
    // xcols[c] is the c-th regressor over the complete-case rows.
    let mut xcols: Vec<Vec<f64>> = vec![Vec::new(); p];
    let mut y_vec: Vec<f64> = Vec::new();
    let mut complete_mask: Vec<bool> = vec![false; ds.n_obs()];

    for &i in rows {
        // FREQ: truncate to integer; exclude obs with f_i < 1 or missing.
        let fi: f64 = match freq_col {
            Some(col) => match value_to_num(&col[i]) {
                Some(v) if !v.is_nan() => {
                    let t = v.trunc();
                    if t < 1.0 {
                        continue;
                    }
                    t
                }
                _ => continue,
            },
            None => 1.0,
        };
        // WEIGHT: exclude obs with w_i ≤ 0 or missing weight.
        let wi: f64 = match weight_col {
            Some(col) => match value_to_num(&col[i]) {
                Some(v) if !v.is_nan() && v > 0.0 => v,
                _ => continue,
            },
            None => 1.0,
        };
        let yi = match value_to_num(&dep_col[i]) {
            Some(v) if !v.is_nan() => v,
            _ => continue,
        };
        let mut row_vals = Vec::with_capacity(p);
        let mut ok = true;
        for rc in reg_cols {
            match value_to_num(&rc[i]) {
                Some(v) if !v.is_nan() => row_vals.push(v),
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            for (c, v) in row_vals.into_iter().enumerate() {
                xcols[c].push(v);
            }
            y_vec.push(yi);
            wf.push(wi * fi);
            total_n += fi;
            if let Some((_, col)) = id_cols.first() {
                id_used.push(id_value_cell(&col[i]));
            }
            complete_mask[i] = true;
        }
    }

    // Effective weighting context. Active when WEIGHT or FREQ is present. When
    // inactive the OLS path runs exactly as before (byte-identical). `total_n`
    // (Σf_i) is the observation count that drives df / n bookkeeping: FREQ
    // changes n and df, WEIGHT does not.
    let weighting = if has_weight || has_freq {
        Some(Weighting {
            wf: wf.clone(),
            total_n,
        })
    } else {
        None
    };
    CaseData {
        xcols,
        y_vec,
        complete_mask,
        weighting,
        id_used,
    }
}

/// PRESS statistic (M36.5): Σ wf_i·(resid_i/(1−h_i))². With WEIGHT/FREQ active
/// (M36.7) the leverage is the WEIGHTED one (h_i·w_i) and each term carries
/// wf_i, matching the weighted PRESS in the MODEL R residual summary and
/// STUDENT/Cook's D (which already use the weighted leverage). With no
/// weighting `wf` is all-ones and h is the plain OLS leverage, so this is
/// byte-identical to before.
fn compute_press_stat(
    model: &RegModel,
    x_mat: &[Vec<f64>],
    fit: &OlsFit,
    weighting: Option<&Weighting>,
) -> Option<f64> {
    if model.press_opt && !model.noprint {
        let h0 = leverages(&x_mat, &fit.xtx_inv);
        let ones = vec![1.0; h0.len()];
        let wf: &[f64] = weighting.as_ref().map(|w| w.wf.as_slice()).unwrap_or(&ones);
        let h: Vec<f64> = h0
            .iter()
            .zip(wf.iter())
            .map(|(&hi, &wi)| hi * wi)
            .collect();
        let press: f64 = fit
            .resid
            .iter()
            .zip(h.iter())
            .zip(wf.iter())
            .map(|((e, &hi), &wi)| {
                let d = 1.0 - hi;
                if d != 0.0 {
                    let p = e / d;
                    wi * p * p
                } else {
                    0.0
                }
            })
            .sum();
        Some(press)
    } else {
        None
    }
}

/// MQ5.2 — one response's fitted-model context, shared by the post-fit
/// option sections (printed matrices, diagnostics, TEST, plots).
struct RespFit<'a> {
    model: &'a RegModel,
    dep_name: &'a str,
    x_mat: &'a [Vec<f64>],
    y_vec: &'a [f64],
    sel_cols: &'a [Vec<f64>],
    sel_reg_names: &'a [String],
    fit: &'a OlsFit,
    intercept: bool,
    n: usize,
    p_eff: usize,
    weighting: Option<&'a Weighting>,
    id_first: Option<&'a [String]>,
}

/// MQ5.2 — the XPX / I / COVB / CORRB printed-matrices section of one
/// response's report.
fn print_matrix_options(rf: &RespFit, session: &mut Session) {
    let &RespFit {
        model,
        dep_name,
        x_mat,
        y_vec,
        sel_reg_names,
        fit,
        intercept,
        n,
        p_eff,
        weighting,
        ..
    } = rf;
    // --- Printed matrices (M36.8): XPX / I / COVB / CORRB. Each is gated on its
    // MODEL option (and !noprint), computed from the existing fit (no refit), so
    // a MODEL without any of these options is byte-identical to before. MSE uses
    // the same error df as the ANOVA table (Σf_i − p_eff with FREQ active).
    if (model.xpx || model.inv || model.covb || model.corrb) && !model.noprint {
        let n_used: f64 = weighting.as_ref().map(|w| w.total_n).unwrap_or(n as f64);
        let error_df = n_used - p_eff as f64;
        let mse = if error_df > 0.0 { fit.sse / error_df } else { f64::NAN };
        if model.xpx {
            let xpx = build_xpx(&x_mat, &y_vec);
            print_xpx(&xpx, &sel_reg_names, dep_name, intercept, session);
        }
        if model.inv {
            print_inverse(
                &fit.xtx_inv,
                &fit.beta,
                fit.sse,
                &sel_reg_names,
                dep_name,
                intercept,
                session,
            );
        }
        if model.covb || model.corrb {
            print_estimate_matrices(
                model,
                &fit.xtx_inv,
                mse,
                &sel_reg_names,
                intercept,
                session,
            );
        }
    }
}

/// MQ5.2 — the collinearity / SPEC / DW / ACOV diagnostics and the CLM/CLI /
/// R / INFLUENCE observation-statistics sections of one response's report.
fn print_diagnostic_options(rf: &RespFit, session: &mut Session) {
    let &RespFit {
        model,
        dep_name,
        x_mat,
        y_vec,
        sel_cols,
        sel_reg_names,
        fit,
        intercept,
        n,
        p_eff,
        weighting,
        id_first,
    } = rf;
    // --- Collinearity / specification / autocorrelation diagnostics (M36.4).
    // All gated on the corresponding flags (and !noprint), so a MODEL without
    // any of these options is byte-identical to before.
    if (model.collin || model.collinoint) && !model.noprint {
        if model.collin {
            if let Ok(c) = compute_collin(&x_mat, &sel_reg_names, intercept, false) {
                print_collin(&c, false, session);
            }
        }
        if model.collinoint {
            if let Ok(c) = compute_collin(&x_mat, &sel_reg_names, intercept, true) {
                print_collin(&c, true, session);
            }
        }
    }
    if model.spec && !model.noprint {
        print_spec_test(&sel_cols, &fit.resid, session);
    }
    if model.dw && !model.noprint {
        let dwr = durbin_watson(&fit.resid, &x_mat, &fit.xtx_inv, model.dwprob);
        print_durbin_watson(&dwr, session);
    }
    if model.acov && !model.noprint {
        let cov = acov_hc0(&x_mat, &fit.resid, &fit.xtx_inv);
        print_acov(
            &cov,
            &fit.beta,
            &sel_reg_names,
            intercept,
            (n - p_eff) as f64,
            session,
        );
    }

    // --- Output Statistics (M36.2): per-observation CLM / CLI limits. Driven
    // off the (unrestricted) OLS fit, gated on the CLM/CLI model options.
    if (model.clm || model.cli) && !model.noprint {
        print_output_statistics(
            model, dep_name, &x_mat, &y_vec, &fit, n, p_eff, weighting, id_first,
            session,
        );
    }

    // --- Residual / influence diagnostics (M36.3): MODEL R and INFLUENCE.
    // Computed lazily once off the OLS fit, shared by both listings.
    if (model.r || model.influence) && !model.noprint {
        let infl = compute_influence_stats(&x_mat, &y_vec, &fit, n, p_eff, weighting);
        if model.r {
            print_r_statistics(model, &infl, id_first, weighting, session);
        }
        if model.influence {
            print_influence_statistics(&infl, &sel_reg_names, intercept, id_first, session);
        }
    }
}

/// MQ5.2 — the TEST section of one response's report: operates on the model
/// as fitted (restricted if present).
fn run_test_section(
    entry: &RegModelEntry,
    rf: &RespFit,
    restricted: Option<&Restricted>,
    session: &mut Session,
) -> Result<()> {
    let &RespFit {
        dep_name,
        x_mat,
        sel_reg_names,
        fit,
        intercept,
        n,
        ..
    } = rf;
    if !entry.tests.is_empty() {
        let (t_beta, t_xtx, t_sse, t_dfe) = match &restricted {
            Some(r) => (&r.beta_r, &fit.xtx_inv, r.sse_r, r.df_r),
            None => (
                &fit.beta,
                &fit.xtx_inv,
                fit.sse,
                (n - x_mat[0].len()) as f64,
            ),
        };
        run_tests(
            &entry.tests,
            &sel_reg_names,
            intercept,
            dep_name,
            t_beta,
            t_xtx,
            t_sse,
            t_dfe,
            x_mat[0].len(),
            session,
        )?;
    }
    Ok(())
}

/// MQ5.2 — the diagnostics / PLOTS rendering section of one response's
/// report (M29.3 deferral NOTE, M36.11 request set + PLOT statements).
fn render_model_plots(ast: &RegAst, rf: &RespFit, session: &mut Session) {
    let &RespFit {
        model,
        dep_name,
        x_mat,
        y_vec,
        sel_reg_names,
        fit,
        intercept,
        n,
        p_eff,
        weighting,
        ..
    } = rf;
    // --- Diagnostics (M29.3) ---
    if ast.plots_requested {
        session.log.note("PLOTS options deferred in PROC REG.");
    }
    // M36.11 — the automatic residuals-vs-predicted diagnostic fires when ODS
    // graphics is on, UNLESS PLOTS=NONE was requested (which suppresses it).
    if session.ods_graphics.enabled && !ast.plot_requests.none {
        let y_hat = fit.y_hat.clone();
        let resid = fit.resid.clone();
        reg_diagnostic_plot(session, &y_hat, &resid);
    }
    // M36.11 — explicit PLOTS=(…) request set and traditional PLOT statements.
    // Under the default build each requested plot is a clean deferral NOTE;
    // under `--features graphics` each is rendered. NONE suppresses everything.
    if ast.plot_requests.any() {
        render_plot_requests(
            &ast.plot_requests,
            &x_mat,
            &y_vec,
            &fit,
            n,
            p_eff,
            model.alpha,
            &sel_reg_names,
            intercept,
            weighting,
            session,
        );
    }
    if !ast.plot_statements.is_empty() {
        render_plot_statements(
            &ast.plot_statements,
            dep_name,
            &x_mat,
            &fit,
            &sel_reg_names,
            intercept,
            session,
        );
    }
}

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests;
