//! PROC TTEST — t-tests for hypothesis testing (M24.2).
//!
//! ## Plan d'implémentation (M24.2 — Opus, élevé)
//!
//! One-sample, two-sample (independent/paired), CLASS statement, Satterthwaite
//! test for unequal variances. Confidence intervals for means. Output listing
//! with test statistics, p-values, degrees of freedom, pooled/separate variance
//! estimates. ODS OUTPUT → datasets (Paired, TTest, ConfLimits, Equality).
//!
//! ### Architecture
//!
//! `TTestAst { data_options, proc_options, var_vars, class_var, paired_vars, output_options }`
//! with separate execution paths for 1-sample, 2-independent (CLASS), and paired.
//!
//! ### One-sample t-test
//! - Statement: `PROC TTEST DATA=ds H0=value ALPHA=p;` (default H0=0, ALPHA=0.05)
//! - VAR statement (required): `var x y z;` → test each vs H0
//! - t = (mean - H0) / (s / √n), df = n-1
//! - 2-tailed p-value: Pr(|T| > |t|) via `stat::student_t_cdf`
//! - 100(1-α)% CI for mean: mean ± t_{α/2,df} · s/√n
//! - Output row per variable: Variable, DF, Mean, StdDev, StdErr, Minimum, Maximum,
//!   t, Pr>|t|, 95% Lower/Upper CL Mean
//!
//! Listing section: "One-Sample t Tests"
//! Output dataset Ttest (if ODS OUTPUT ttest=ds):
//!   - Variable (char), N (num), Mean, StdDev, StdErr, DF, t, Probt
//!
//! ### Two-sample t-test (CLASS)
//! - Statement: `CLASS var;` where var has 2 distinct non-missing values
//! - Error if >2 classes (reject gracefully with NOTE)
//! - Assumption: groups A, B with n_A, n_B, s²_A, s²_B, x̄_A, x̄_B
//! - **Pooled variance** (default): s²_p = [(n_A-1)s²_A + (n_B-1)s²_B] / (n_A+n_B-2)
//!   t_pool = (x̄_A - x̄_B) / √(s²_p·(1/n_A + 1/n_B)), df = n_A+n_B-2
//! - **Satterthwaite** (ALPHA=p with EQUAL=NO): unequal variance correction
//!   t_satt = (x̄_A - x̄_B) / √(s²_A/n_A + s²_B/n_B)
//!   df = (s²_A/n_A + s²_B/n_B)² / [(s²_A/n_A)²/(n_A-1) + (s²_B/n_B)²/(n_B-1)]
//!   (Welch-Satterthwaite formula)
//! - **Folding F-test for equal variances** (optional): printed under listing
//!   F = max(s²_A, s²_B) / min(s²_A, s²_B), df1 = n_max-1, df2 = n_min-1
//!   Prob > F = 2 · min(F_CDF, 1 - F_CDF) for 2-tailed
//! - p-value: Pr(|T| > |t|) using pooled or Satterthwaite df
//! - CI: (x̄_A - x̄_B) ± t_{α/2,df} · SE(diff)
//!
//! Listing section: "Two-Sample t Tests" with columns:
//!   - Variable, Class1, Class2, Method (pooled/Satterthwaite), N, Mean, StdDev, SE, t, Pr>|t|, 95% CL
//! Subscript: "Equality of Variances" row if requested (F test)
//!
//! Output dataset Ttest (if ODS):
//!   - Variable (char), Class (char), N, Mean, StdDev, StdErr, DF, t, Probt, LowerCLMean, UpperCLMean
//!
//! ### Paired t-test
//! - Statement: `PAIRED x*y z*w;` → paired differences per VAR pair
//! - Error if datasets have different n (one pair is NA → exclude obs, WARN)
//! - Differences: d_i = x_i - y_i (per pair specification)
//! - One-sample t test on differences: t = d̄ / (s_d / √n), df = n-1
//! - p-value: 2-tailed Pr(|T| > |t|)
//! - CI: d̄ ± t_{α/2,df} · s_d/√n
//! - Also output: N_pairs, Mean_diff, StdDev_diff, StdErr_diff, t, Pr>|t|, CL
//!
//! Listing section: "Paired t Tests" with columns per pair:
//!   - Variable, N, Mean Diff, StdDev, StdErr, DF, t, Pr>|t|, 95% CL Diff
//!
//! Output dataset Ttest (if ODS):
//!   - Variable (char), N, MeanDiff, StdDevDiff, StdErrDiff, DF, t, Probt, LowerCLDiff, UpperCLDiff
//!
//! ### Options
//! - **ALPHA=** (default 0.05): significance level for CIs
//! - **H0=** (default 0): null hypothesis mean (1-sample)
//! - **EQUAL=YES|NO** (default YES for pooled): Satterthwaite test
//! - **CI=** (default 95): confidence level as percentage (e.g., CI=90 → α=0.10)
//! - **SIDES=2|U|L** (2-tailed, upper, lower; default 2)
//! - **CLASS** var (2-level classification, automatic detection)
//! - **PAIRED** pair1*pair2 ... (paired comparisons)
//! - **OUT=** dataset (ODS OUTPUT TTEST equivalent; also write Paired, ConfLimits, Equality if applicable)
//! - **VAR** variables (default = all numeric)
//!
//! ### Parsers
//! - `parse_ttest(ts: &mut TokenStream) -> Result<TTestAst>` : top-level proc parser
//! - `parse_ttest_options` : ALPHA, H0, EQUAL, etc.
//! - `parse_class` : CLASS var; single var, required ≥2 distinct values
//! - `parse_paired` : PAIRED x*y z*w; VAR-like syntax but pairs
//! - `parse_var` : VAR variables; defaults to all numeric if omitted
//!
//! ### Execution
//! - `execute_ttest(ast: TTestAst, session: &mut Session) -> Result<Option<LastDataset>>`
//! - Read DATA= dataset via `provider.read()` + `SasDataset`
//! - Filter VAR to numeric columns
//! - If CLASS: validate 2 distinct values, split into groups, compute pooled/Satt stats
//! - If PAIRED: align pairs, compute differences, execute 1-sample test
//! - Otherwise: execute 1-sample test on each VAR
//! - Write listing via `listing.page_header()`, `listing.write_table()`
//! - If OUT=: create ODS output dataset(s) with stats
//! - Emit NOTEs: "N observations read...", "The TTEST Procedure", variable counts
//!
//! ### Error handling
//! - Missing values → excluded from analysis (note "NMiss" in listing)
//! - Constant variable (s²=0) → note "StdDev=.", t=missing, p=missing (no test)
//! - Non-numeric VAR → note and skip
//! - CLASS with >2 values → error "Class variable must have exactly 2 levels"
//! - PAIRED with length mismatch → error or exclude incomplete pairs
//! - ALPHA not in (0, 1) → error "ALPHA must be in (0, 1)"
//!
//! ### Special cases (documented as limits)
//! - BY groups (M24.2) : parser recognizes BY but defers (note "not yet implemented")
//! - Multiple CLASS variables → parser rejects (requires single var)
//! - Confidence level asymmetry (lower/upper CI separate) → differed
//! - Non-parametric alternatives (WILCOXON) → M24.3 (PROC NPAR1WAY)
//!
//! ### Tests
//! - 1-sample: t=0 (mean=H0), t>0, t<0, CI coverage
//! - 2-sample: pooled, Satterthwaite, F test, CI
//! - Paired: simple pair, multiple pairs, missing-value handling
//! - Edge cases: n=1, n=2, s²=0, all missing, alpha edge
//! - Listing format verification (columns, headers, rounding)
//! - ODS output dataset structure (column names, types, values)

use crate::ast::DatasetRef;
use crate::error::Result;
use crate::parser::StatementStream;
use crate::session::Session;

#[derive(Debug, Clone)]
pub struct TTestAst {
    pub data_options: TTestDataOptions,
    pub proc_options: TTestProcOptions,
    pub var_vars: Vec<String>,
    pub class_var: Option<String>,
    pub paired_vars: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct TTestDataOptions {
    pub input: Option<DatasetRef>,
    pub output: Option<DatasetRef>,
}

#[derive(Debug, Clone)]
pub struct TTestProcOptions {
    pub h0: f64,
    pub alpha: f64,
    pub ci: f64,
    pub equal: bool,
    pub sides: TTestSides,
}

#[derive(Debug, Clone, Copy)]
pub enum TTestSides {
    TwoTailed,
    Upper,
    Lower,
}

impl Default for TTestProcOptions {
    fn default() -> Self {
        TTestProcOptions {
            h0: 0.0,
            alpha: 0.05,
            ci: 95.0,
            equal: true,
            sides: TTestSides::TwoTailed,
        }
    }
}

/// Parse PROC TTEST statement and its options.
pub fn parse(ts: &mut StatementStream) -> Result<TTestAst> {
    todo!("Implement parse_ttest — parse PROC TTEST statement and options")
}

/// Execute PROC TTEST and produce listing + ODS output.
pub fn execute(ast: &TTestAst, session: &mut Session) -> Result<()> {
    todo!("Implement execute_ttest — execute t-tests and write listing/output")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_one_sample_basic() {
        todo!("Test 1-sample t-test: H0=0, simple VAR")
    }

    #[test]
    fn test_two_sample_pooled() {
        todo!("Test 2-sample pooled variance")
    }

    #[test]
    fn test_paired_simple() {
        todo!("Test paired t-test: simple pair")
    }
}
