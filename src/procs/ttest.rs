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
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::parser::StatementStream;
use crate::procs::common::{decode_column, partition_numeric, sample_std};
use crate::session::Session;
use crate::stat::{f_cdf, student_t_cdf};
use crate::token::TokenKind;
use crate::value::{Value, VarType};
use polars::prelude::{Column, DataFrame, NamedFrom, Series};

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

fn expect_eq(ts: &mut StatementStream, opt: &str) -> Result<()> {
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            format!("expected '=' after {opt}"),
            ts.peek().span,
        ));
    }
    ts.next();
    Ok(())
}

/// Parse a numeric option value (`=<num>`); the `=` must already be consumed.
fn parse_num_value(ts: &mut StatementStream, opt: &str) -> Result<f64> {
    let tok = ts.peek().clone();
    match tok.kind {
        TokenKind::Num(n) => {
            ts.next();
            Ok(n)
        }
        TokenKind::Minus => {
            ts.next();
            if let TokenKind::Num(n) = ts.peek().kind {
                ts.next();
                Ok(-n)
            } else {
                Err(SasError::parse(format!("expected a number after {opt}="), ts.peek().span))
            }
        }
        _ => Err(SasError::parse(
            format!("expected a number for {opt}="),
            tok.span,
        )),
    }
}

/// Parse PROC TTEST statement and its options.
///
/// Called AFTER "proc ttest" was consumed. Consumes through `run;` / `quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<TTestAst> {
    let mut input: Option<DatasetRef> = None;
    let mut output: Option<DatasetRef> = None;
    let mut proc_options = TTestProcOptions::default();

    // --- PROC TTEST statement options, until `;` ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            ts.next();
            expect_eq(ts, "DATA")?;
            input = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("out") {
            ts.next();
            expect_eq(ts, "OUT")?;
            output = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("h0") {
            ts.next();
            expect_eq(ts, "H0")?;
            proc_options.h0 = parse_num_value(ts, "H0")?;
        } else if ts.peek().is_kw("alpha") {
            ts.next();
            expect_eq(ts, "ALPHA")?;
            proc_options.alpha = parse_num_value(ts, "ALPHA")?;
        } else if ts.peek().is_kw("ci") {
            ts.next();
            expect_eq(ts, "CI")?;
            proc_options.ci = parse_num_value(ts, "CI")?;
        } else if ts.peek().is_kw("equal") {
            ts.next();
            expect_eq(ts, "EQUAL")?;
            let v = ts
                .peek()
                .ident()
                .map(str::to_string)
                .ok_or_else(|| SasError::parse("expected YES or NO after EQUAL=", ts.peek().span))?;
            ts.next();
            proc_options.equal = !v.eq_ignore_ascii_case("no");
        } else if ts.peek().is_kw("sides") {
            ts.next();
            expect_eq(ts, "SIDES")?;
            let v = ts
                .peek()
                .ident()
                .map(str::to_string)
                .ok_or_else(|| SasError::parse("expected 2, U or L after SIDES=", ts.peek().span))?;
            ts.next();
            proc_options.sides = match v.to_ascii_uppercase().as_str() {
                "U" => TTestSides::Upper,
                "L" => TTestSides::Lower,
                _ => TTestSides::TwoTailed,
            };
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!("Unknown PROC TTEST option: {}", name.to_uppercase()),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse("Unexpected token on PROC TTEST statement.", span));
        }
    }

    // --- sub-statements until run;/quit; ---
    let mut var_vars: Vec<String> = Vec::new();
    let mut class_var: Option<String> = None;
    let mut paired_vars: Vec<(String, String)> = Vec::new();

    loop {
        while ts.peek().kind == TokenKind::Semi {
            ts.next();
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("run") || ts.peek().is_kw("quit") {
            ts.next();
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            break;
        }

        if ts.peek().is_kw("var") {
            ts.next();
            var_vars = ts.parse_name_list()?;
            ts.expect_semi()?;
        } else if ts.peek().is_kw("class") {
            ts.next();
            let names = ts.parse_name_list()?;
            ts.expect_semi()?;
            if names.len() != 1 {
                return Err(SasError::runtime(
                    "The CLASS statement of PROC TTEST accepts exactly one variable.",
                ));
            }
            class_var = Some(names.into_iter().next().unwrap());
        } else if ts.peek().is_kw("paired") {
            ts.next();
            // `paired x*y z*w;` — each pair is name '*' name.
            loop {
                if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
                    break;
                }
                let left = ts
                    .peek()
                    .ident()
                    .map(str::to_string)
                    .ok_or_else(|| SasError::parse("expected a variable name in PAIRED", ts.peek().span))?;
                ts.next();
                if ts.peek().kind != TokenKind::Star {
                    return Err(SasError::parse(
                        "expected '*' between paired variables",
                        ts.peek().span,
                    ));
                }
                ts.next();
                let right = ts
                    .peek()
                    .ident()
                    .map(str::to_string)
                    .ok_or_else(|| SasError::parse("expected a variable name after '*' in PAIRED", ts.peek().span))?;
                ts.next();
                paired_vars.push((left, right));
            }
            ts.expect_semi()?;
        } else if ts.peek().is_kw("by") {
            ts.next();
            ts.skip_to_semi();
            // BY processing is recognized but deferred.
        } else if ts.peek().is_kw("output") {
            // `output out=<ref>;`
            ts.next();
            if ts.peek().is_kw("out") {
                ts.next();
                expect_eq(ts, "OUT")?;
                output = Some(ts.parse_dataset_ref()?);
            }
            ts.skip_to_semi();
        } else {
            // Unknown sub-statement: skip it (recovery).
            ts.skip_to_semi();
        }
    }

    Ok(TTestAst {
        data_options: TTestDataOptions { input, output },
        proc_options,
        var_vars,
        class_var,
        paired_vars,
    })
}

// ───────────────────────── numeric core ─────────────────────────

/// One-sample (or paired-difference) t-test result over a complete numeric
/// sample. `t`/`p` are `None` when the test is undefined (n < 2 or zero std).
#[derive(Debug, Clone)]
struct OneSampleResult {
    n: usize,
    mean: f64,
    std: Option<f64>,
    se: Option<f64>,
    min: f64,
    max: f64,
    df: f64,
    t: Option<f64>,
    p: Option<f64>,
}

/// One-sample t-test of `values` against `h0` at significance `alpha`.
fn one_sample(values: &[f64], h0: f64, alpha: f64) -> OneSampleResult {
    let n = values.len();
    let mean = if n > 0 {
        values.iter().sum::<f64>() / n as f64
    } else {
        f64::NAN
    };
    let std = sample_std(values);
    let df = (n as f64 - 1.0).max(0.0);
    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    // alpha drives the (currently unreported) confidence interval; reserved for
    // a future CL column. Kept in the signature for API stability.
    let _ = alpha;
    let (se, t, p) = match std {
        Some(s) if n >= 2 && s > 0.0 => {
            let se = s / (n as f64).sqrt();
            let t = (mean - h0) / se;
            let p = two_sided_p(t, df);
            (Some(se), Some(t), Some(p))
        }
        Some(_) if n >= 2 => {
            // Constant sample (zero std): test undefined.
            (Some(0.0), None, None)
        }
        _ => (None, None, None),
    };

    OneSampleResult {
        n,
        mean,
        std,
        se,
        min: if n > 0 { min } else { f64::NAN },
        max: if n > 0 { max } else { f64::NAN },
        df,
        t,
        p,
    }
}

/// Two-method (Pooled + Satterthwaite) two-sample t-test, plus the folded
/// F-test for equality of variances. Groups `a` and `b` are the complete
/// numeric samples for the two CLASS levels in display order (a first).
#[derive(Debug, Clone)]
struct TwoSampleResult {
    n_a: usize,
    n_b: usize,
    /// Pooled: (t, df, p)
    pooled: Option<(f64, f64, f64)>,
    /// Satterthwaite: (t, df, p)
    satterthwaite: Option<(f64, f64, f64)>,
    /// Folded F test for equal variances: (F, df1, df2, p)
    f_test: Option<(f64, f64, f64, f64)>,
}

fn two_sample(a: &[f64], b: &[f64]) -> TwoSampleResult {
    let n_a = a.len();
    let n_b = b.len();
    let naf = n_a as f64;
    let nbf = n_b as f64;
    let mean_a = if n_a > 0 { a.iter().sum::<f64>() / naf } else { f64::NAN };
    let mean_b = if n_b > 0 { b.iter().sum::<f64>() / nbf } else { f64::NAN };
    let std_a = sample_std(a);
    let std_b = sample_std(b);

    let (pooled, satterthwaite) = match (std_a, std_b) {
        (Some(sa), Some(sb)) if n_a >= 2 && n_b >= 2 => {
            let va = sa * sa;
            let vb = sb * sb;
            // Pooled.
            let sp2 = ((naf - 1.0) * va + (nbf - 1.0) * vb) / (naf + nbf - 2.0);
            let se_pool = (sp2 * (1.0 / naf + 1.0 / nbf)).sqrt();
            let pooled = if se_pool > 0.0 {
                let df = naf + nbf - 2.0;
                let t = (mean_a - mean_b) / se_pool;
                Some((t, df, two_sided_p(t, df)))
            } else {
                None
            };
            // Satterthwaite.
            let se_satt = (va / naf + vb / nbf).sqrt();
            let satt = if se_satt > 0.0 {
                let num = (va / naf + vb / nbf).powi(2);
                let den = (va / naf).powi(2) / (naf - 1.0) + (vb / nbf).powi(2) / (nbf - 1.0);
                let df = num / den;
                let t = (mean_a - mean_b) / se_satt;
                Some((t, df, two_sided_p(t, df)))
            } else {
                None
            };
            (pooled, satt)
        }
        _ => (None, None),
    };

    let f_test = match (std_a, std_b) {
        (Some(sa), Some(sb)) if n_a >= 2 && n_b >= 2 && sa > 0.0 && sb > 0.0 => {
            let va = sa * sa;
            let vb = sb * sb;
            // Numerator df corresponds to the group with the LARGER variance.
            let (f, df1, df2) = if va >= vb {
                (va / vb, naf - 1.0, nbf - 1.0)
            } else {
                (vb / va, nbf - 1.0, naf - 1.0)
            };
            let cdf = f_cdf(f, df1, df2);
            let p = 2.0 * cdf.min(1.0 - cdf);
            Some((f, df1, df2, p.clamp(0.0, 1.0)))
        }
        _ => None,
    };

    TwoSampleResult {
        n_a,
        n_b,
        pooled,
        satterthwaite,
        f_test,
    }
}

/// Two-sided p-value for a t statistic with `df` degrees of freedom.
fn two_sided_p(t: f64, df: f64) -> f64 {
    (2.0 * (1.0 - student_t_cdf(t.abs(), df))).clamp(0.0, 1.0)
}

// ───────────────────────── formatting ─────────────────────────

/// Format a t-statistic / mean / CI value to 4 decimals; None → ".".
fn fmt4(v: Option<f64>) -> String {
    match v {
        Some(f) => format!("{f:.4}"),
        None => ".".to_string(),
    }
}

/// Format a two-sided p-value SAS-style: `<.0001`, else 4 decimals; None → ".".
fn fmt_p(p: Option<f64>) -> String {
    match p {
        None => ".".to_string(),
        Some(v) => {
            if v < 0.0001 {
                "<.0001".to_string()
            } else {
                format!("{v:.4}")
            }
        }
    }
}

// ───────────────────────── execute ─────────────────────────

/// Resolve `data=` or `_LAST_` into a concrete DatasetRef.
fn resolve_input(ast: &TTestAst, session: &Session) -> Result<DatasetRef> {
    match &ast.data_options.input {
        Some(r) => Ok(r.clone()),
        None => {
            let last = session.last_dataset.clone().ok_or_else(|| {
                SasError::runtime("There is no default input data set (_LAST_ is undefined).")
            })?;
            let parts: Vec<&str> = last.splitn(2, '.').collect();
            if parts.len() == 2 {
                Ok(DatasetRef {
                    libref: Some(parts[0].to_string()),
                    name: parts[1].to_string(),
                })
            } else {
                Ok(DatasetRef {
                    libref: None,
                    name: last,
                })
            }
        }
    }
}

/// Write a centered line within LINESIZE.
fn centered(session: &mut Session, text: &str) {
    let ls = session.listing.ls();
    let pad = ls.saturating_sub(text.len()) / 2;
    session
        .listing
        .write_line(&format!("{}{}", " ".repeat(pad), text));
}

/// Execute PROC TTEST and produce listing + ODS output.
pub fn execute(ast: &TTestAst, session: &mut Session) -> Result<()> {
    let in_ref = resolve_input(ast, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }

    let n_obs = ds.n_obs();
    let all_rows: Vec<usize> = (0..n_obs).collect();

    // Helper: resolve a variable name to a column index (any type).
    let find_col = |nm: &str| -> Result<usize> {
        ds.vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(nm))
            .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", nm.to_uppercase())))
    };

    // Resolve numeric VAR columns: explicit, else all numeric in dataset order.
    let var_cols: Vec<usize> = if !ast.var_vars.is_empty() {
        let mut out = Vec::with_capacity(ast.var_vars.len());
        for nm in &ast.var_vars {
            let i = find_col(nm)?;
            if ds.vars[i].ty != VarType::Num {
                return Err(SasError::runtime(format!(
                    "Variable {} in the VAR list is not numeric.",
                    nm.to_uppercase()
                )));
            }
            out.push(i);
        }
        out
    } else {
        (0..ds.vars.len())
            .filter(|&i| ds.vars[i].ty == VarType::Num)
            .collect()
    };

    // Listing header.
    session.listing.page_header();
    centered(session, "The TTEST Procedure");
    session.listing.blank();

    let alpha = ast.proc_options.alpha;

    if let Some(class_name) = &ast.class_var {
        execute_two_sample(ast, session, &ds, &var_cols, class_name, &all_rows, alpha)?;
    } else if !ast.paired_vars.is_empty() {
        execute_paired(ast, session, &ds, &all_rows, alpha, &find_col)?;
    } else {
        execute_one_sample(ast, session, &ds, &var_cols, &all_rows, alpha)?;
    }

    Ok(())
}

/// Mode 1 — one-sample t tests on every VAR variable.
fn execute_one_sample(
    ast: &TTestAst,
    session: &mut Session,
    ds: &SasDataset,
    var_cols: &[usize],
    all_rows: &[usize],
    alpha: f64,
) -> Result<()> {
    let h0 = ast.proc_options.h0;
    centered(session, "One-Sample t Tests");
    session.listing.blank();

    let headers: Vec<String> = vec![
        "Variable".into(),
        "N".into(),
        "Mean".into(),
        "Std Dev".into(),
        "Std Err".into(),
        "Minimum".into(),
        "Maximum".into(),
        "t Value".into(),
        "Pr > |t|".into(),
    ];
    let aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right, Align::Right, Align::Right, Align::Right, Align::Right, Align::Right];

    let mut rows: Vec<Vec<String>> = Vec::with_capacity(var_cols.len());
    let mut ods_rows: Vec<(String, OneSampleResult)> = Vec::new();
    for &c in var_cols {
        let col = decode_column(ds, c)?;
        let (xs, _nmiss) = partition_numeric(&col, all_rows);
        let r = one_sample(&xs, h0, alpha);
        rows.push(vec![
            ds.vars[c].name.clone(),
            format!("{}", r.n),
            fmt4(if r.n > 0 { Some(r.mean) } else { None }),
            fmt4(r.std),
            fmt4(r.se),
            fmt4(if r.n > 0 { Some(r.min) } else { None }),
            fmt4(if r.n > 0 { Some(r.max) } else { None }),
            fmt4(r.t),
            fmt_p(r.p),
        ]);
        ods_rows.push((ds.vars[c].name.clone(), r));
    }
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();

    // ODS OUTPUT TTest + OUT= dataset.
    maybe_write_one_sample_output(ast, session, &ods_rows)?;
    Ok(())
}

/// Mode 2 — two-sample t tests defined by a CLASS variable with 2 levels.
fn execute_two_sample(
    ast: &TTestAst,
    session: &mut Session,
    ds: &SasDataset,
    var_cols: &[usize],
    class_name: &str,
    all_rows: &[usize],
    _alpha: f64,
) -> Result<()> {
    let class_idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(class_name))
        .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", class_name.to_uppercase())))?;
    let class_vals = decode_column(ds, class_idx)?;

    // Collect distinct non-missing class levels (sas_cmp comparison + order).
    let mut levels: Vec<Value> = Vec::new();
    for &r in all_rows {
        let v = &class_vals[r];
        if v.is_missing() {
            continue;
        }
        if !levels.iter().any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal) {
            levels.push(v.clone());
        }
    }
    levels.sort_by(|a, b| a.sas_cmp(b));
    if levels.len() != 2 {
        return Err(SasError::runtime(format!(
            "The CLASS variable {} must have exactly 2 levels.",
            class_name.to_uppercase()
        )));
    }

    let level_label = |v: &Value| -> String {
        match v {
            Value::Char(s) => s.trim_end().to_string(),
            Value::Num(f) => format!("{f}"),
            Value::Missing(_) => ".".to_string(),
        }
    };
    let label_a = level_label(&levels[0]);
    let label_b = level_label(&levels[1]);

    centered(session, "Two-Sample t Tests");
    session.listing.blank();

    let headers: Vec<String> = vec![
        "Variable".into(),
        "Method".into(),
        "DF".into(),
        "t Value".into(),
        "Pr > |t|".into(),
    ];
    let aligns = vec![Align::Left, Align::Left, Align::Right, Align::Right, Align::Right];

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut feq_rows: Vec<Vec<String>> = Vec::new();
    let mut ods: Vec<(String, TwoSampleResult)> = Vec::new();

    for &c in var_cols {
        let col = decode_column(ds, c)?;
        let mut a: Vec<f64> = Vec::new();
        let mut b: Vec<f64> = Vec::new();
        for &r in all_rows {
            let lv = &class_vals[r];
            if lv.is_missing() {
                continue;
            }
            let group = if lv.sas_cmp(&levels[0]) == std::cmp::Ordering::Equal {
                Some(&mut a)
            } else if lv.sas_cmp(&levels[1]) == std::cmp::Ordering::Equal {
                Some(&mut b)
            } else {
                None
            };
            if let Some(g) = group {
                if let Some(x) = crate::missing::value_to_num(&col[r]) {
                    if !x.is_nan() {
                        g.push(x);
                    }
                }
            }
        }
        let res = two_sample(&a, &b);
        let vname = ds.vars[c].name.clone();

        let (pt, pdf, pp) = match res.pooled {
            Some((t, df, p)) => (Some(t), Some(df), Some(p)),
            None => (None, None, None),
        };
        rows.push(vec![
            vname.clone(),
            "Pooled".into(),
            fmt4(pdf),
            fmt4(pt),
            fmt_p(pp),
        ]);
        let (st, sdf, sp) = match res.satterthwaite {
            Some((t, df, p)) => (Some(t), Some(df), Some(p)),
            None => (None, None, None),
        };
        rows.push(vec![
            vname.clone(),
            "Satterthwaite".into(),
            fmt4(sdf),
            fmt4(st),
            fmt_p(sp),
        ]);

        if let Some((f, df1, df2, p)) = res.f_test {
            feq_rows.push(vec![
                vname.clone(),
                format!("{}", df1 as usize),
                format!("{}", df2 as usize),
                fmt4(Some(f)),
                fmt_p(Some(p)),
            ]);
        } else {
            feq_rows.push(vec![vname.clone(), ".".into(), ".".into(), ".".into(), ".".into()]);
        }
        ods.push((vname, res));
    }

    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();

    // Equality of Variances section.
    centered(session, "Equality of Variances");
    session.listing.blank();
    let feq_headers: Vec<String> = vec![
        "Variable".into(),
        "Num DF".into(),
        "Den DF".into(),
        "F Value".into(),
        "Pr > F".into(),
    ];
    let feq_aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right, Align::Right];
    session.listing.write_table(&feq_headers, &feq_aligns, &feq_rows);
    session.listing.blank();

    maybe_write_two_sample_output(ast, session, &ods, &label_a, &label_b)?;
    Ok(())
}

/// Mode 3 — paired t tests on each (x, y) difference.
fn execute_paired(
    ast: &TTestAst,
    session: &mut Session,
    ds: &SasDataset,
    all_rows: &[usize],
    alpha: f64,
    find_col: &dyn Fn(&str) -> Result<usize>,
) -> Result<()> {
    centered(session, "Paired t Tests");
    session.listing.blank();

    let headers: Vec<String> = vec![
        "Variable".into(),
        "N Pairs".into(),
        "Mean".into(),
        "Std Dev".into(),
        "Std Err".into(),
        "DF".into(),
        "t Value".into(),
        "Pr > |t|".into(),
    ];
    let aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right, Align::Right, Align::Right, Align::Right, Align::Right];

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut ods: Vec<(String, OneSampleResult)> = Vec::new();

    for (xn, yn) in &ast.paired_vars {
        let xi = find_col(xn)?;
        let yi = find_col(yn)?;
        if ds.vars[xi].ty != VarType::Num || ds.vars[yi].ty != VarType::Num {
            return Err(SasError::runtime(format!(
                "PAIRED variables {} and {} must be numeric.",
                xn.to_uppercase(),
                yn.to_uppercase()
            )));
        }
        let xc = decode_column(ds, xi)?;
        let yc = decode_column(ds, yi)?;
        let mut diffs: Vec<f64> = Vec::new();
        for &r in all_rows {
            match (
                crate::missing::value_to_num(&xc[r]),
                crate::missing::value_to_num(&yc[r]),
            ) {
                (Some(x), Some(y)) if !x.is_nan() && !y.is_nan() => diffs.push(x - y),
                _ => {}
            }
        }
        let res = one_sample(&diffs, 0.0, alpha);
        let label = format!("{}-{}", ds.vars[xi].name, ds.vars[yi].name);
        rows.push(vec![
            label.clone(),
            format!("{}", res.n),
            fmt4(if res.n > 0 { Some(res.mean) } else { None }),
            fmt4(res.std),
            fmt4(res.se),
            fmt4(if res.n >= 1 { Some(res.df) } else { None }),
            fmt4(res.t),
            fmt_p(res.p),
        ]);
        ods.push((label, res));
    }

    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();

    maybe_write_paired_output(ast, session, &ods)?;
    Ok(())
}

// ───────────────────────── ODS / OUT= datasets ─────────────────────────

fn num_var_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}

fn char_var_meta(name: &str, length: usize) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Char,
        length,
        format: None,
        label: None,
    }
}

/// Resolve the destination for the TTest output (ODS OUTPUT TTest, else OUT=).
fn output_target(ast: &TTestAst, session: &Session) -> Option<DatasetRef> {
    session
        .ods_output_target("TTest")
        .or_else(|| ast.data_options.output.clone())
}

fn write_out_dataset(session: &mut Session, target: &DatasetRef, out_ds: SasDataset) -> Result<()> {
    let out_libref = target.libref_or_work();
    let out_table = target.name.to_uppercase();
    let display = format!("{out_libref}.{out_table}");
    let n_rows = out_ds.n_obs();
    let n_vars = out_ds.vars.len();
    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(display.clone());
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_rows, n_vars
    ));
    Ok(())
}

fn maybe_write_one_sample_output(
    ast: &TTestAst,
    session: &mut Session,
    rows: &[(String, OneSampleResult)],
) -> Result<()> {
    let Some(target) = output_target(ast, session) else {
        return Ok(());
    };
    let var: Vec<Option<String>> = rows.iter().map(|(n, _)| Some(n.clone())).collect();
    let n: Vec<Option<f64>> = rows.iter().map(|(_, r)| Some(r.n as f64)).collect();
    let mean: Vec<Option<f64>> = rows.iter().map(|(_, r)| (r.n > 0).then_some(r.mean)).collect();
    let std: Vec<Option<f64>> = rows.iter().map(|(_, r)| r.std).collect();
    let stderr: Vec<Option<f64>> = rows.iter().map(|(_, r)| r.se).collect();
    let df: Vec<Option<f64>> = rows.iter().map(|(_, r)| Some(r.df)).collect();
    let tval: Vec<Option<f64>> = rows.iter().map(|(_, r)| r.t).collect();
    let probt: Vec<Option<f64>> = rows.iter().map(|(_, r)| r.p).collect();

    let columns: Vec<Column> = vec![
        Series::new("Variable".into(), var).into(),
        Series::new("N".into(), n).into(),
        Series::new("Mean".into(), mean).into(),
        Series::new("StdDev".into(), std).into(),
        Series::new("StdErr".into(), stderr).into(),
        Series::new("DF".into(), df).into(),
        Series::new("tValue".into(), tval).into(),
        Series::new("Probt".into(), probt).into(),
    ];
    let vars = vec![
        char_var_meta("Variable", 32),
        num_var_meta("N"),
        num_var_meta("Mean"),
        num_var_meta("StdDev"),
        num_var_meta("StdErr"),
        num_var_meta("DF"),
        num_var_meta("tValue"),
        num_var_meta("Probt"),
    ];
    let df_out = DataFrame::new(columns)?;
    write_out_dataset(session, &target, SasDataset { df: df_out, vars })
}

fn maybe_write_paired_output(
    ast: &TTestAst,
    session: &mut Session,
    rows: &[(String, OneSampleResult)],
) -> Result<()> {
    // Paired output shares the one-sample layout (difference statistics).
    maybe_write_one_sample_output(ast, session, rows)
}

fn maybe_write_two_sample_output(
    ast: &TTestAst,
    session: &mut Session,
    rows: &[(String, TwoSampleResult)],
    label_a: &str,
    label_b: &str,
) -> Result<()> {
    let Some(target) = output_target(ast, session) else {
        return Ok(());
    };
    // One row per variable per method (Pooled, Satterthwaite).
    let mut var: Vec<Option<String>> = Vec::new();
    let mut method: Vec<Option<String>> = Vec::new();
    let mut n1: Vec<Option<f64>> = Vec::new();
    let mut n2: Vec<Option<f64>> = Vec::new();
    let mut df: Vec<Option<f64>> = Vec::new();
    let mut tval: Vec<Option<f64>> = Vec::new();
    let mut probt: Vec<Option<f64>> = Vec::new();
    let _ = (label_a, label_b);
    for (name, r) in rows {
        for (m, res) in [("Pooled", &r.pooled), ("Satterthwaite", &r.satterthwaite)] {
            var.push(Some(name.clone()));
            method.push(Some(m.to_string()));
            n1.push(Some(r.n_a as f64));
            n2.push(Some(r.n_b as f64));
            match res {
                Some((t, d, p)) => {
                    df.push(Some(*d));
                    tval.push(Some(*t));
                    probt.push(Some(*p));
                }
                None => {
                    df.push(None);
                    tval.push(None);
                    probt.push(None);
                }
            }
        }
    }
    let columns: Vec<Column> = vec![
        Series::new("Variable".into(), var).into(),
        Series::new("Method".into(), method).into(),
        Series::new("N1".into(), n1).into(),
        Series::new("N2".into(), n2).into(),
        Series::new("DF".into(), df).into(),
        Series::new("tValue".into(), tval).into(),
        Series::new("Probt".into(), probt).into(),
    ];
    let vars = vec![
        char_var_meta("Variable", 32),
        char_var_meta("Method", 13),
        num_var_meta("N1"),
        num_var_meta("N2"),
        num_var_meta("DF"),
        num_var_meta("tValue"),
        num_var_meta("Probt"),
    ];
    let df_out = DataFrame::new(columns)?;
    write_out_dataset(session, &target, SasDataset { df: df_out, vars })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_one_sample_basic() {
        // [1,2,3,4,5] vs H0=0: n=5, mean=3, s≈1.5811, se≈0.7071, t≈4.2426, df=4.
        let values = [1.0, 2.0, 3.0, 4.0, 5.0];
        let r = one_sample(&values, 0.0, 0.05);
        assert_eq!(r.n, 5);
        assert!((r.mean - 3.0).abs() < 1e-12);
        assert!((r.std.unwrap() - 2.5_f64.sqrt()).abs() < 1e-9, "std={:?}", r.std);
        assert!((r.se.unwrap() - 0.5_f64.sqrt()).abs() < 1e-9, "se={:?}", r.se);
        assert!((r.df - 4.0).abs() < 1e-12);
        let t = r.t.unwrap();
        assert!((t - 4.2426).abs() < 1e-4, "t={t}");
        let p = r.p.unwrap();
        assert!(p < 0.015, "p={p}");
    }

    #[test]
    fn test_two_sample_pooled() {
        // A=[1,2,3] (mean 2, s 1), B=[5,6,7] (mean 6, s 1).
        let a = [1.0, 2.0, 3.0];
        let b = [5.0, 6.0, 7.0];
        let r = two_sample(&a, &b);
        assert_eq!(r.n_a, 3);
        assert_eq!(r.n_b, 3);
        let (tp, dfp, _pp) = r.pooled.unwrap();
        assert!((tp - (-4.8990)).abs() < 1e-4, "t_pool={tp}");
        assert!((dfp - 4.0).abs() < 1e-12, "df_pool={dfp}");
        let (ts, dfs, _ps) = r.satterthwaite.unwrap();
        assert!((ts.abs() - 4.8990).abs() < 1e-4, "t_satt={ts}");
        assert!((dfs - 4.0).abs() < 1e-6, "df_satt={dfs}");
        let (f, _df1, _df2, _pf) = r.f_test.unwrap();
        assert!((f - 1.0).abs() < 1e-12, "F={f}");
    }

    #[test]
    fn test_paired_simple() {
        // x=[2,4,6], y=[1,2,3]: diffs=[1,2,3], mean=2, s=1, se≈0.5774, t≈3.4641, df=2.
        let diffs = [1.0, 2.0, 3.0];
        let r = one_sample(&diffs, 0.0, 0.05);
        assert_eq!(r.n, 3);
        assert!((r.mean - 2.0).abs() < 1e-12);
        assert!((r.std.unwrap() - 1.0).abs() < 1e-12);
        assert!((r.se.unwrap() - 1.0 / 3.0_f64.sqrt()).abs() < 1e-9);
        assert!((r.df - 2.0).abs() < 1e-12);
        let t = r.t.unwrap();
        assert!((t - 3.4641).abs() < 1e-4, "t={t}");
    }

    // --- parser + executor smoke tests ---

    use crate::dataset::{SasDataset, VarMeta};
    use crate::source::SourceFile;
    use polars::df;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn num_meta(name: &str) -> VarMeta {
        VarMeta { name: name.into(), ty: VarType::Num, length: 8, format: None, label: None }
    }
    fn char_meta(name: &str) -> VarMeta {
        VarMeta { name: name.into(), ty: VarType::Char, length: 1, format: None, label: None }
    }

    fn parse_ttest(src: &str) -> Result<TTestAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // proc
        ts.next(); // ttest
        parse(&mut ts)
    }

    #[test]
    fn parse_options_and_statements() {
        let ast = parse_ttest(
            "proc ttest data=a h0=5 alpha=0.10 sides=u equal=no; var x y; class g; run;",
        )
        .unwrap();
        assert_eq!(ast.data_options.input.as_ref().unwrap().name, "a");
        assert!((ast.proc_options.h0 - 5.0).abs() < 1e-12);
        assert!((ast.proc_options.alpha - 0.10).abs() < 1e-12);
        assert!(!ast.proc_options.equal);
        assert!(matches!(ast.proc_options.sides, TTestSides::Upper));
        assert_eq!(ast.var_vars, vec!["x", "y"]);
        assert_eq!(ast.class_var.as_deref(), Some("g"));
    }

    #[test]
    fn parse_paired_pairs() {
        let ast = parse_ttest("proc ttest data=a; paired x*y z*w; run;").unwrap();
        assert_eq!(
            ast.paired_vars,
            vec![("x".to_string(), "y".to_string()), ("z".to_string(), "w".to_string())]
        );
    }

    #[test]
    fn parse_unknown_option_errors() {
        let r = parse_ttest("proc ttest data=a bogus; run;");
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("BOGUS"));
    }

    #[test]
    fn execute_two_sample_listing() {
        let mut session = make_session();
        let df = df![
            "g" => ["A", "A", "A", "B", "B", "B"],
            "x" => [1.0_f64, 2.0, 3.0, 5.0, 6.0, 7.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x")] };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

        let ast = TTestAst {
            data_options: TTestDataOptions {
                input: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
                output: None,
            },
            proc_options: TTestProcOptions::default(),
            var_vars: vec!["x".into()],
            class_var: Some("g".into()),
            paired_vars: vec![],
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("The TTEST Procedure"), "{listing}");
        assert!(listing.contains("Two-Sample t Tests"), "{listing}");
        assert!(listing.contains("Pooled"), "{listing}");
        assert!(listing.contains("Satterthwaite"), "{listing}");
        assert!(listing.contains("Equality of Variances"), "{listing}");
    }

    #[test]
    fn execute_one_sample_and_paired_listing() {
        let mut session = make_session();
        let df = df![
            "x" => [2.0_f64, 4.0, 6.0],
            "y" => [1.0_f64, 2.0, 3.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("y")] };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

        // One-sample on all numeric vars.
        let ast1 = TTestAst {
            data_options: TTestDataOptions {
                input: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
                output: None,
            },
            proc_options: TTestProcOptions::default(),
            var_vars: vec![],
            class_var: None,
            paired_vars: vec![],
        };
        execute(&ast1, &mut session).unwrap();
        let l1 = session.listing.into_string();
        assert!(l1.contains("One-Sample t Tests"), "{l1}");

        // Paired x*y.
        let mut session2 = make_session();
        let df2 = df![
            "x" => [2.0_f64, 4.0, 6.0],
            "y" => [1.0_f64, 2.0, 3.0]
        ]
        .unwrap();
        let ds2 = SasDataset { df: df2, vars: vec![num_meta("x"), num_meta("y")] };
        session2.libs.get("WORK").unwrap().write("T", &ds2).unwrap();
        let ast2 = TTestAst {
            data_options: TTestDataOptions {
                input: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
                output: Some(DatasetRef { libref: Some("WORK".into()), name: "OUT".into() }),
            },
            proc_options: TTestProcOptions::default(),
            var_vars: vec![],
            class_var: None,
            paired_vars: vec![("x".into(), "y".into())],
        };
        execute(&ast2, &mut session2).unwrap();
        let l2 = session2.listing.into_string();
        assert!(l2.contains("Paired t Tests"), "{l2}");
        // OUT= dataset written.
        let (out, _) = session2.libs.get("WORK").unwrap().read("OUT").unwrap();
        assert_eq!(out.n_obs(), 1);
        assert_eq!(session2.last_dataset.as_deref(), Some("WORK.OUT"));
    }
}
