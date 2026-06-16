//! PROC NPAR1WAY — non-parametric one-way tests (M24.3).
//!
//! ## Plan d'implémentation (M24.3 — Opus, moyen-élevé)
//!
//! Non-parametric alternatives to ANOVA for testing equality of k>1 populations.
//! Wilcoxon rank-sum (2 samples), Kruskal-Wallis (k>1), score test methods.
//! CLASS required (exact 1, multi-level). VAR optional (default all numeric).
//! Output: test statistics, p-values, ranks (optionally), sample statistics.
//!
//! ### Architecture
//!
//! `NparAst { data_options, proc_options, var_vars, class_var, test_options }`
//! - Single CLASS variable (k levels, n total obs)
//! - Multiple VAR (analyzed separately)
//! - Tests: WILCOXON (2-sample), KRUSKAL, SCORES (normal/van-der-Waerden/Savage/median)
//!
//! ### Wilcoxon test (2-sample: rank-sum)
//! - Samples A (n_A obs) vs B (n_B obs), k=2
//! - Null hypothesis: distributions identical (shift alternative)
//! - Procedure:
//!   1. Pool all n = n_A + n_B observations, sort by VAR value
//!   2. Assign ranks 1..n (midrank ties), missing → excluded + NOTE "N pairs"
//!   3. W = sum of ranks in group A (by convention, smaller group)
//!   4. Expected W_0 = n_A(n+1)/2, Var(W) = n_A·n_B·(n+1)/12 (adjust for ties via tie_correction)
//!   5. Z = (W - W_0) / √Var(W) ≈ N(0,1) for large n
//!   6. 2-tailed p: Pr(|Z| > |z|) via `stat::probnorm`
//!   7. Exact p-value (if n ≤ 20) via permutation enumeration (expensive, defer v1)
//! - Output: Z, Pr>|Z|
//!
//! ### Kruskal-Wallis test (k-sample: rank-sum)
//! - k≥2 groups with sizes n_1, ..., n_k, total n
//! - Null: all k distributions identical
//! - Procedure:
//!   1. Pool, rank 1..n (midrank ties)
//!   2. R_i = sum of ranks in group i
//!   3. H = [12 / (n(n+1))] · Σ R_i² / n_i - 3(n+1) (Kruskal-Wallis statistic)
//!   4. H ≈ χ²(df=k-1) under null (adjust for ties via tie_correction)
//!   5. p-value: Pr(χ² > H) via `stat::chisq_cdf` (survival prob)
//! - Output: H statistic, DF (k-1), Pr>H (chi-square tail probability)
//!
//! ### Score tests (van der Waerden, Savage, median)
//! - Generalization: assign scores φ(ranks) to observations, test rank sums
//! - **Normal scores** (van der Waerden): φ(r) = Φ⁻¹(r / (n+1))
//! - **Median scores**: φ(r) = 1 if r > n/2 else 0 (counting above/below median)
//! - **Savage scores**: φ(r) = 1/(n+1-r) (exponential decay)
//! - Procedure: replace ranks with scores, recompute H (or Z for k=2)
//! - p-value: χ²(df=k-1) or normal Z ≈ N(0,1) depending on score type
//! - NOTE: implementation deferred; skeleton parse only, error "not yet implemented"
//!
//! ### Options
//! - **CLASS** var (required; multi-level automatic)
//! - **VAR** variables (default = all numeric)
//! - **WILCOXON** (exact/normal approximation) — test for 2-sample; auto-selected if k=2
//! - **KRUSKAL** (default if k>2) — test statistic reported
//! - **ALPHA=** (default 0.05; NOT used for p-value, only for CI — deferred)
//! - **OUT=** dataset (ODS: column tests, test names, statistics, p-values)
//! - Scoring methods: NORMAL (default), SAVAGE, MEDIAN (NOT IN v1; parse but error)
//! - (Deferred) **BY** support
//!
//! ### Parsers
//! - `parse_npar1way(ts: &mut TokenStream) -> Result<NparAst>` : top-level proc parser
//! - `parse_npar_options` : CLASS, VAR, WILCOXON, KRUSKAL, ALPHA, OUT=, scoring
//! - Reject: multiple CLASS, scoring method ≠ default (NOT YET IMPLEMENTED error)
//!
//! ### Execution
//! - `execute_npar1way(ast: NparAst, session: &mut Session) -> Result<Option<LastDataset>>`
//! - Read DATA= dataset
//! - Validate CLASS var (≥2 distinct non-missing values)
//! - For each VAR:
//!   - Decode column to Vec<Value>
//!   - Exclude missing: build row indices + class membership
//!   - If k=2: Wilcoxon test (Z + p-value via normal approx)
//!   - If k>2: Kruskal-Wallis test (H + df + p-value via χ²)
//!   - Write listing row: Var, Test, Statistic, p-value
//! - Handle ties: tie_correction factor = Σ t_i³ - Σ t_i / (n³ - n), where t_i = tie group size
//! - Missing values: NOTE "N pairs" (for Wilcoxon) or "observations analyzed"
//! - If OUT=: write ODS dataset with one row per VAR per test
//! - Emit NOTEs: "The NPAR1WAY Procedure", "One-Way Non-Parametric Analysis", etc.
//!
//! ### Error handling
//! - CLASS with 1 level → error "Class variable must have ≥2 levels"
//! - Non-numeric VAR → skip with NOTE
//! - All missing for a VAR → skip with NOTE
//! - Scoring method ≠ default → error "NORMAL scores are default; others not yet implemented"
//! - BY support → error "BY groups not yet implemented" (defer M24.3)
//!
//! ### Special cases (documented)
//! - Exact Wilcoxon p-value (n ≤ 20) → deferred (permutation test)
//! - Median/Savage scores → deferred (parse but reject)
//! - Confidence intervals → deferred (Hodges-Lehmann, etc.)
//! - Trend tests (linear contrast) → deferred (M24.x)
//! - STRATA for stratification → deferred
//!
//! ### Tests
//! - Wilcoxon: known data (simple pairs), tie handling, p-value vs. R::wilcox.test
//! - Kruskal-Wallis: 3+ groups, tie handling, p-value vs. R::kruskal.test
//! - Missing-value handling: excluded obs count, NOTE verification
//! - Listing format: columns, rounding, headers
//! - ODS output: dataset structure, column names, values
//! - Error paths: 1-level class, invalid scoring, invalid CLASS var type

use crate::ast::DatasetRef;
use crate::error::Result;
use crate::parser::StatementStream;
use crate::session::Session;

#[derive(Debug, Clone)]
pub struct NparAst {
    pub data_options: NparDataOptions,
    pub proc_options: NparProcOptions,
    pub var_vars: Vec<String>,
    pub class_var: String,
    pub test_options: NparTestOptions,
}

#[derive(Debug, Clone)]
pub struct NparDataOptions {
    pub input: Option<DatasetRef>,
    pub output: Option<DatasetRef>,
}

#[derive(Debug, Clone)]
pub struct NparProcOptions {
    pub alpha: f64,
}

#[derive(Debug, Clone)]
pub struct NparTestOptions {
    pub wilcoxon: bool,
    pub kruskal: bool,
    pub scores: NparScores,
}

#[derive(Debug, Clone, Copy)]
pub enum NparScores {
    Normal,
    Savage,
    Median,
}

impl Default for NparProcOptions {
    fn default() -> Self {
        NparProcOptions { alpha: 0.05 }
    }
}

/// Parse PROC NPAR1WAY statement and its options.
pub fn parse(ts: &mut StatementStream) -> Result<NparAst> {
    todo!("Implement parse_npar1way — parse PROC NPAR1WAY statement and options")
}

/// Execute PROC NPAR1WAY and produce listing + ODS output.
pub fn execute(ast: &NparAst, session: &mut Session) -> Result<()> {
    todo!("Implement execute_npar1way — execute non-parametric tests and write listing/output")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wilcoxon_basic() {
        todo!("Test Wilcoxon rank-sum: 2 samples, simple data")
    }

    #[test]
    fn test_kruskal_three_groups() {
        todo!("Test Kruskal-Wallis: 3 groups")
    }

    #[test]
    fn test_ties_correction() {
        todo!("Test tie group correction in variance")
    }
}
