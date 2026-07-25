use super::*;

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
    pub(super) fn any(&self) -> bool {
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
