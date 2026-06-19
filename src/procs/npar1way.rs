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
//! - NOTE: no continuity correction is applied (matches the M24.3 numeric spec).
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
//! - ODS OUTPUT / OUT= dataset writing → deferred (parse `output out=`, store, but
//!   no dataset emitted in v1)
//!
//! ### Tests
//! - Wilcoxon: known data (simple pairs), tie handling, p-value vs. R::wilcox.test
//! - Kruskal-Wallis: 3+ groups, tie handling, p-value vs. R::kruskal.test
//! - Missing-value handling: excluded obs count, NOTE verification
//! - Listing format: columns, rounding, headers
//! - ODS output: dataset structure, column names, values
//! - Error paths: 1-level class, invalid scoring, invalid CLASS var type

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::decode_column;
use crate::session::Session;
use crate::stat::{chisq_cdf, probnorm};
use crate::token::TokenKind;
use crate::value::{Value, VarType};

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
                Err(SasError::parse(
                    format!("expected a number after {opt}="),
                    ts.peek().span,
                ))
            }
        }
        _ => Err(SasError::parse(
            format!("expected a number for {opt}="),
            tok.span,
        )),
    }
}

/// Parse PROC NPAR1WAY statement and its options.
///
/// Called AFTER "proc npar1way" was consumed. Consumes through `run;` / `quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<NparAst> {
    let mut input: Option<DatasetRef> = None;
    let mut output: Option<DatasetRef> = None;
    let mut proc_options = NparProcOptions::default();
    let mut wilcoxon = false;
    let mut kruskal = false;

    // --- PROC NPAR1WAY statement options, until `;` ---
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
        } else if ts.peek().is_kw("alpha") {
            ts.next();
            expect_eq(ts, "ALPHA")?;
            proc_options.alpha = parse_num_value(ts, "ALPHA")?;
        } else if ts.peek().is_kw("wilcoxon") {
            ts.next();
            wilcoxon = true;
        } else if ts.peek().is_kw("kruskal") || ts.peek().is_kw("kruskalwallis") {
            ts.next();
            kruskal = true;
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!("Unknown PROC NPAR1WAY option: {}", name.to_uppercase()),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC NPAR1WAY statement.",
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; ---
    let mut var_vars: Vec<String> = Vec::new();
    let mut class_var: Option<String> = None;

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
                    "The CLASS statement of PROC NPAR1WAY accepts exactly one variable.",
                ));
            }
            class_var = Some(names.into_iter().next().unwrap());
        } else if ts.peek().is_kw("by") {
            return Err(SasError::runtime(
                "BY processing is not yet implemented in PROC NPAR1WAY.",
            ));
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

    let class_var = class_var.ok_or_else(|| {
        SasError::runtime("PROC NPAR1WAY requires a CLASS statement.")
    })?;

    // SAS default: with no test option, run Wilcoxon (k=2) and Kruskal-Wallis
    // (k≥2). Enabling both flags reproduces that behaviour.
    if !wilcoxon && !kruskal {
        wilcoxon = true;
        kruskal = true;
    }

    Ok(NparAst {
        data_options: NparDataOptions { input, output },
        proc_options,
        var_vars,
        class_var,
        test_options: NparTestOptions {
            wilcoxon,
            kruskal,
            scores: NparScores::Normal,
        },
    })
}

// ───────────────────────── numeric core ─────────────────────────

/// Wilcoxon two-sample rank-sum statistics (computed only for k=2).
#[derive(Debug, Clone)]
struct WilcoxonResult {
    /// Rank sum of group 0 (the first group in sas_cmp order).
    w: f64,
    /// Expected value of `w` under H0.
    ew: f64,
    /// Variance of `w` (tie-corrected).
    var_w: f64,
    /// Standardized statistic `(w - ew) / sqrt(var_w)`.
    z: f64,
    /// Two-sided normal-approximation p-value.
    p: f64,
}

/// Kruskal-Wallis statistics (always computed for k≥2).
#[derive(Debug, Clone)]
struct KruskalResult {
    /// Tie-corrected H statistic.
    h: f64,
    /// Degrees of freedom (k-1).
    df: usize,
    /// Upper-tail chi-square p-value.
    p: f64,
}

/// Combined non-parametric analysis of one VAR across the CLASS groups.
#[derive(Debug, Clone)]
struct NparResult {
    /// Total non-missing observations.
    n: usize,
    /// Tie-correction factor `1 - Σ(t³-t)/(n³-n)`.
    tie_factor: f64,
    /// Wilcoxon result (only when `k == 2`).
    wilcoxon: Option<WilcoxonResult>,
    /// Kruskal-Wallis result.
    kruskal: KruskalResult,
}

/// Assign mid-ranks (1-based) to a slice of values, averaging ties.
///
/// Returns a vector `ranks` aligned with `values` (same order), and the list of
/// tie-group sizes (for the tie correction).
fn midranks(values: &[f64]) -> (Vec<f64>, Vec<usize>) {
    let n = values.len();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| values[a].partial_cmp(&values[b]).unwrap_or(std::cmp::Ordering::Equal));

    let mut ranks = vec![0.0_f64; n];
    let mut tie_sizes: Vec<usize> = Vec::new();
    let mut i = 0usize;
    while i < n {
        let mut j = i + 1;
        while j < n && values[idx[j]] == values[idx[i]] {
            j += 1;
        }
        // Positions i..j (0-based) share the same value; ranks are i+1..=j.
        let group = j - i;
        // Average of ranks (i+1)..=j = (i+1 + j) / 2.
        let midrank = ((i + 1) + j) as f64 / 2.0;
        for &k in &idx[i..j] {
            ranks[k] = midrank;
        }
        if group > 1 {
            tie_sizes.push(group);
        }
        i = j;
    }
    (ranks, tie_sizes)
}

/// Core numeric routine. `groups[i]` holds the missing-excluded numeric values
/// of CLASS level `i`, in sas_cmp order. Pools all observations, ranks them with
/// mid-ranks, then computes the Kruskal-Wallis statistic (always) and the
/// Wilcoxon rank-sum statistic (when there are exactly two groups).
fn analyze(groups: &[Vec<f64>]) -> NparResult {
    let k = groups.len();
    // Flatten, keeping track of which group each pooled value belongs to.
    let mut pooled: Vec<f64> = Vec::new();
    let mut owner: Vec<usize> = Vec::new();
    for (gi, g) in groups.iter().enumerate() {
        for &v in g {
            pooled.push(v);
            owner.push(gi);
        }
    }
    let n = pooled.len();

    let (ranks, tie_sizes) = midranks(&pooled);

    // Tie correction: 1 - Σ(t³ - t) / (n³ - n).
    let nf = n as f64;
    let denom = nf * nf * nf - nf;
    let tie_factor = if denom > 0.0 {
        let s: f64 = tie_sizes
            .iter()
            .map(|&t| {
                let tf = t as f64;
                tf * tf * tf - tf
            })
            .sum();
        1.0 - s / denom
    } else {
        1.0
    };

    // Rank sums and sizes per group.
    let mut rank_sum = vec![0.0_f64; k];
    let mut n_i = vec![0usize; k];
    for r in 0..n {
        rank_sum[owner[r]] += ranks[r];
        n_i[owner[r]] += 1;
    }

    // Kruskal-Wallis: H = [12/(n(n+1))] Σ R_i²/n_i - 3(n+1), corrected by /tie_factor.
    let kruskal = {
        let h_raw = if n >= 1 {
            let mut s = 0.0_f64;
            for i in 0..k {
                if n_i[i] > 0 {
                    s += rank_sum[i] * rank_sum[i] / n_i[i] as f64;
                }
            }
            12.0 / (nf * (nf + 1.0)) * s - 3.0 * (nf + 1.0)
        } else {
            f64::NAN
        };
        let h = if tie_factor > 0.0 { h_raw / tie_factor } else { h_raw };
        let df = k.saturating_sub(1);
        let p = if df >= 1 && h.is_finite() {
            (1.0 - chisq_cdf(h, df as f64)).clamp(0.0, 1.0)
        } else {
            f64::NAN
        };
        KruskalResult { h, df, p }
    };

    // Wilcoxon two-sample (only for k == 2).
    let wilcoxon = if k == 2 && n_i[0] > 0 && n_i[1] > 0 {
        let na = n_i[0] as f64;
        let nb = n_i[1] as f64;
        let w = rank_sum[0];
        let ew = na * (nf + 1.0) / 2.0;
        // Var(W) = n_A n_B (n+1) / 12, tie-corrected.
        let var_w = na * nb * (nf + 1.0) / 12.0 * tie_factor;
        let (z, p) = if var_w > 0.0 {
            let z = (w - ew) / var_w.sqrt();
            let cdf = probnorm(z);
            let p = (2.0 * cdf.min(1.0 - cdf)).clamp(0.0, 1.0);
            (z, p)
        } else {
            (f64::NAN, f64::NAN)
        };
        Some(WilcoxonResult { w, ew, var_w, z, p })
    } else {
        None
    };

    NparResult {
        n,
        tie_factor,
        wilcoxon,
        kruskal,
    }
}

// ───────────────────────── formatting ─────────────────────────

/// Format a statistic to 4 decimals; NaN → ".".
fn fmt4(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.4}")
    } else {
        ".".to_string()
    }
}

/// Format a p-value SAS-style: `<.0001`, else 4 decimals; NaN → ".".
fn fmt_p(p: f64) -> String {
    if !p.is_finite() {
        ".".to_string()
    } else if p < 0.0001 {
        "<.0001".to_string()
    } else {
        format!("{p:.4}")
    }
}

// ───────────────────────── execute ─────────────────────────

/// Resolve `data=` or `_LAST_` into a concrete DatasetRef.
fn resolve_input(ast: &NparAst, session: &Session) -> Result<DatasetRef> {
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

/// Execute PROC NPAR1WAY and produce the listing.
pub fn execute(ast: &NparAst, session: &mut Session) -> Result<()> {
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

    // Resolve the CLASS column.
    let class_idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(&ast.class_var))
        .ok_or_else(|| {
            SasError::runtime(format!("Variable {} not found.", ast.class_var.to_uppercase()))
        })?;
    let class_vals = decode_column(&ds, class_idx)?;

    // Distinct non-missing CLASS levels (sas_cmp ordering).
    let mut levels: Vec<Value> = Vec::new();
    for &r in &all_rows {
        let v = &class_vals[r];
        if v.is_missing() {
            continue;
        }
        if !levels
            .iter()
            .any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
        {
            levels.push(v.clone());
        }
    }
    levels.sort_by(|a, b| a.sas_cmp(b));
    if levels.len() < 2 {
        return Err(SasError::runtime(format!(
            "The CLASS variable {} must have at least 2 levels.",
            ast.class_var.to_uppercase()
        )));
    }
    let k = levels.len();

    // Resolve VAR columns: explicit, else all numeric (excluding CLASS).
    let find_col = |nm: &str| -> Result<usize> {
        ds.vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(nm))
            .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", nm.to_uppercase())))
    };
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
            .filter(|&i| i != class_idx && ds.vars[i].ty == VarType::Num)
            .collect()
    };

    // Listing header.
    session.listing.page_header();
    centered(session, "The NPAR1WAY Procedure");
    session.listing.blank();

    for &c in &var_cols {
        let col = decode_column(&ds, c)?;
        // Partition the non-missing numeric values by CLASS level.
        let mut groups: Vec<Vec<f64>> = vec![Vec::new(); k];
        for &r in &all_rows {
            let lv = &class_vals[r];
            if lv.is_missing() {
                continue;
            }
            let gi = levels
                .iter()
                .position(|l| l.sas_cmp(lv) == std::cmp::Ordering::Equal);
            let Some(gi) = gi else { continue };
            if let Some(x) = value_to_num(&col[r]) {
                if !x.is_nan() {
                    groups[gi].push(x);
                }
            }
        }

        let res = analyze(&groups);
        let vname = ds.vars[c].name.clone();

        centered(session, &format!("One-Way Analysis of {vname}"));
        session.listing.blank();

        if res.n == 0 {
            session
                .listing
                .write_line(&format!("No non-missing observations for {vname}."));
            session.listing.blank();
            continue;
        }

        // Wilcoxon (k == 2).
        if ast.test_options.wilcoxon {
            if let Some(w) = &res.wilcoxon {
                centered(session, "Wilcoxon Two-Sample Test");
                session.listing.blank();
                let headers: Vec<String> = vec![
                    "Statistic".into(),
                    "Mean Under H0".into(),
                    "Std Dev Under H0".into(),
                    "Z".into(),
                    "Pr > |Z|".into(),
                ];
                let aligns = vec![
                    Align::Right,
                    Align::Right,
                    Align::Right,
                    Align::Right,
                    Align::Right,
                ];
                let rows = vec![vec![
                    fmt4(w.w),
                    fmt4(w.ew),
                    fmt4(w.var_w.sqrt()),
                    fmt4(w.z),
                    fmt_p(w.p),
                ]];
                session.listing.write_table(&headers, &aligns, &rows);
                session.listing.blank();
            }
        }

        // Kruskal-Wallis (always, including k == 2 — SAS does the same).
        if ast.test_options.kruskal {
            centered(session, "Kruskal-Wallis Test");
            session.listing.blank();
            let headers: Vec<String> =
                vec!["Chi-Square".into(), "DF".into(), "Pr > ChiSq".into()];
            let aligns = vec![Align::Right, Align::Right, Align::Right];
            let rows = vec![vec![
                fmt4(res.kruskal.h),
                format!("{}", res.kruskal.df),
                fmt_p(res.kruskal.p),
            ]];
            session.listing.write_table(&headers, &aligns, &rows);
            if res.tie_factor < 1.0 {
                session.listing.write_line(&format!(
                    "Average scores were used for ties (tie correction factor = {}).",
                    fmt4(res.tie_factor)
                ));
            }
            session.listing.blank();
        }
    }

    // Log NOTE (plural-invariant phrasing).
    session.log.note(&format!(
        "There were {} observations read from the data set {}.{}.",
        n_obs, in_libref, in_table
    ));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wilcoxon_basic() {
        // Group A = [1,2,3], Group B = [4,5,6]; no ties.
        // Ranks of A = 1+2+3 = 6; E(W) = 10.5; Var(W) = 5.25; Z ≈ -1.9642.
        let res = analyze(&[vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0]]);
        assert_eq!(res.n, 6);
        assert!((res.tie_factor - 1.0).abs() < 1e-12, "tie_factor={}", res.tie_factor);
        let w = res.wilcoxon.expect("wilcoxon present for k=2");
        assert!((w.w - 6.0).abs() < 1e-12, "W={}", w.w);
        assert!((w.ew - 10.5).abs() < 1e-12, "E(W)={}", w.ew);
        assert!((w.var_w - 5.25).abs() < 1e-12, "Var(W)={}", w.var_w);
        assert!((w.z - (-1.9642)).abs() < 1e-3, "Z={}", w.z);
        assert!(w.p < 0.06, "p={}", w.p);
    }

    #[test]
    fn test_kruskal_three_groups() {
        // Three groups, no ties: H ≈ 1.143, df = 2.
        let res = analyze(&[vec![1.0, 4.0], vec![2.0, 5.0], vec![3.0, 6.0]]);
        assert_eq!(res.n, 6);
        assert!((res.tie_factor - 1.0).abs() < 1e-12);
        assert_eq!(res.kruskal.df, 2);
        assert!((res.kruskal.h - 1.143).abs() < 1e-2, "H={}", res.kruskal.h);
        // k=3 → no Wilcoxon.
        assert!(res.wilcoxon.is_none());
    }

    #[test]
    fn test_ties_correction() {
        // Group A = [1,2], Group B = [2,3]. Sorted: 1, 2, 2, 3.
        // Mid-ranks: 1 -> 1; the two 2.0 -> 2.5; 3 -> 4.
        // tie group {2,2}: t=2 → Σ(t³-t)=6; n=4 → n³-n=60; tie_factor = 1 - 6/60 = 0.9.
        // W (rank sum of A = [1.0, 2.5]) = 3.5; Var(W)_corrected = (20/12)*0.9 = 1.5.
        let res = analyze(&[vec![1.0, 2.0], vec![2.0, 3.0]]);
        assert_eq!(res.n, 4);
        assert!((res.tie_factor - 0.9).abs() < 1e-12, "tie_factor={}", res.tie_factor);
        let w = res.wilcoxon.expect("wilcoxon present for k=2");
        assert!((w.w - 3.5).abs() < 1e-12, "W={}", w.w);
        assert!((w.var_w - 1.5).abs() < 1e-12, "Var(W)_corrected={}", w.var_w);
        // Confirm the correction actually changed the variance from the uncorrected 20/12.
        assert!((w.var_w - 20.0 / 12.0).abs() > 1e-6, "variance should be tie-corrected");
    }
}
