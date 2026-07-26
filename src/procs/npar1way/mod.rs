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
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, decode_column, phi_inv};
use crate::session::Session;
use crate::stat::{chisq_cdf, probnorm};
use crate::token::TokenKind;
use crate::value::{Value, VarType, format_best};
use polars::prelude::{Column, DataFrame, NamedFrom, Series};

mod exact;
mod output;
mod parse;
mod ranks;
mod report;
mod scores;

pub use parse::parse;

use exact::*;
use output::*;
use ranks::*;
use report::*;
use scores::*;

#[derive(Debug, Clone)]
pub struct NparAst {
    pub data_options: NparDataOptions,
    pub proc_options: NparProcOptions,
    pub var_vars: Vec<String>,
    pub class_var: String,
    pub test_options: NparTestOptions,
    /// BY variables (name, descending). Empty when no BY statement.
    pub by: Vec<(String, bool)>,
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
    /// Median score test (MEDIAN option).
    pub median: bool,
    /// Savage score test (SAVAGE option).
    pub savage: bool,
    /// Normal / van der Waerden score test (NORMAL or VW option).
    pub normal: bool,
    /// Exact Wilcoxon permutation test (EXACT option / sub-statement).
    pub exact: bool,
    pub scores: NparScores,
}

#[derive(Debug, Clone, Copy)]
pub enum NparScores {
    Normal,
    Savage,
    Median,
}

/// A linear-rank score method (the generic framework instantiation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScoreKind {
    /// Rank scores. The listing/OUT= Wilcoxon path uses the closed-form
    /// `analyze()`; this variant drives the generic framework's self-checks and
    /// keeps the score machinery total over every method.
    #[cfg_attr(not(test), allow(dead_code))]
    Wilcoxon,
    Median,
    Savage,
    Normal,
}

impl Default for NparProcOptions {
    fn default() -> Self {
        NparProcOptions { alpha: 0.05 }
    }
}

// ───────────────────────── execute ─────────────────────────

/// Resolve `data=` or `_LAST_` into a concrete DatasetRef.
use crate::procs::common::centered;

/// Execute PROC NPAR1WAY and produce the listing.
pub fn execute(ast: &NparAst, session: &mut Session) -> Result<()> {
    let (ds, in_libref, in_table) = common::open_input(&ast.data_options.input, session)?;

    let n_obs = ds.n_obs();
    let all_rows: Vec<usize> = (0..n_obs).collect();

    // Resolve the CLASS column.
    let class_idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(&ast.class_var))
        .ok_or_else(|| {
            SasError::runtime(format!(
                "Variable {} not found.",
                ast.class_var.to_uppercase()
            ))
        })?;
    let class_vals = decode_column(&ds, class_idx)?;

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

    // Pre-decode every VAR column once.
    let var_decoded: Vec<Vec<Value>> = var_cols
        .iter()
        .map(|&c| decode_column(&ds, c))
        .collect::<Result<_>>()?;

    // --- BY processing: resolve, verify sortedness, partition into groups. ---
    // No BY → a single group spanning all rows (output byte-identical).
    let by_cols = common::resolve_by_cols(&ds, &ast.by)?;
    let by_names: Vec<String> = by_cols.iter().map(|c| c.name.clone()).collect();
    let by_groups_list: Vec<(Vec<Value>, Vec<usize>)> = if by_cols.is_empty() {
        vec![(Vec::new(), all_rows.clone())]
    } else {
        let by_values: Vec<Vec<Value>> = by_cols
            .iter()
            .map(|c| decode_column(&ds, c.col_idx))
            .collect::<Result<_>>()?;
        let descending: Vec<bool> = by_cols.iter().map(|c| c.descending).collect();
        let in_display = format!("{in_libref}.{in_table}");
        common::by_groups(&by_values, &descending, n_obs, &by_names, &in_display)?
    };

    // Listing header.
    session.listing.page_header();
    centered(session, "The NPAR1WAY Procedure");
    session.listing.blank();

    // Accumulator for the OUT= dataset (one row per VAR per BY group).
    let mut out_rows: Vec<OutRow> = Vec::new();

    for (by_key, grp_rows) in &by_groups_list {
        if !by_names.is_empty() {
            emit_by_heading(session, &by_names, by_key);
        }

        // Distinct non-missing CLASS levels within this BY group (sas_cmp order).
        let levels = crate::procs::lincom::class_levels(grp_rows.iter().map(|&r| &class_vals[r]));
        if levels.len() < 2 {
            return Err(SasError::runtime(format!(
                "The CLASS variable {} must have at least 2 levels.",
                ast.class_var.to_uppercase()
            )));
        }
        let k = levels.len();

        for (vi, &c) in var_cols.iter().enumerate() {
            let col = &var_decoded[vi];
            // Partition the non-missing numeric values by CLASS level.
            let mut groups: Vec<Vec<f64>> = vec![Vec::new(); k];
            for &r in grp_rows {
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

            let mut out_row = OutRow::new(by_key.clone(), vname.clone());

            // Wilcoxon (k == 2).
            if ast.test_options.wilcoxon {
                if let Some(w) = &res.wilcoxon {
                    centered(session, "Wilcoxon Two-Sample Test");
                    session.listing.blank();
                    write_two_sample_table(session, w.w, w.ew, w.var_w.sqrt(), w.z, w.p);
                    session.listing.blank();
                    out_row.wil = Some((w.w, w.z, w.p, normal_p1(w.z)));

                    // Exact Wilcoxon (k == 2 only).
                    if ast.test_options.exact {
                        match exact_wilcoxon(&groups) {
                            Some(ex) => {
                                write_exact_block(session, &ex);
                                session.listing.blank();
                                out_row.exact = Some((ex.p_lower, ex.p_two));
                            }
                            None => {
                                session.log.note(&format!(
                                    "The exact Wilcoxon test was not computed for {vname} \
                                     because the sample size exceeds the limit of {EXACT_N_CAP}."
                                ));
                            }
                        }
                    }
                }
            }

            // Kruskal-Wallis (Wilcoxon-score one-way χ²).
            if ast.test_options.kruskal {
                centered(session, "Kruskal-Wallis Test");
                session.listing.blank();
                write_one_way_table(session, res.kruskal.h, res.kruskal.df, res.kruskal.p);
                if res.tie_factor < 1.0 {
                    session.listing.write_line(&format!(
                        "Average scores were used for ties (tie correction factor = {}).",
                        fmt4(res.tie_factor)
                    ));
                }
                session.listing.blank();
                out_row.kw = Some((res.kruskal.h, res.kruskal.df, res.kruskal.p));
            }

            // Additional score methods (MEDIAN / SAVAGE / NORMAL-VW).
            let score_specs: [(bool, ScoreKind, &str, &str); 3] = [
                (
                    ast.test_options.median,
                    ScoreKind::Median,
                    "Median Two-Sample Test",
                    "Median One-Way Analysis",
                ),
                (
                    ast.test_options.savage,
                    ScoreKind::Savage,
                    "Savage Two-Sample Test",
                    "Savage One-Way Analysis",
                ),
                (
                    ast.test_options.normal,
                    ScoreKind::Normal,
                    "Van der Waerden Two-Sample Test",
                    "Van der Waerden One-Way Analysis",
                ),
            ];
            for (enabled, kind, two_title, one_title) in score_specs {
                if !enabled {
                    continue;
                }
                let sa = score_analysis(&groups, kind);
                let two = score_two_sample(&sa);
                if let Some(t) = &two {
                    centered(session, two_title);
                    session.listing.blank();
                    write_two_sample_table(session, t.stat, t.mean, t.sd, t.z, t.p2);
                    session.listing.blank();
                }
                let ow = score_one_way(&sa);
                centered(session, one_title);
                session.listing.blank();
                write_one_way_table(session, ow.chisq, ow.df, ow.p);
                session.listing.blank();

                match kind {
                    ScoreKind::Median => out_row.med = Some((two.clone(), ow)),
                    ScoreKind::Savage => out_row.sav = Some((two.clone(), ow)),
                    ScoreKind::Normal => out_row.vw = Some((two.clone(), ow)),
                    ScoreKind::Wilcoxon => {}
                }
            }

            out_rows.push(out_row);
        }
    }

    // Log NOTE (plural-invariant phrasing).
    session.log.note(&format!(
        "There were {} observations read from the data set {}.{}.",
        n_obs, in_libref, in_table
    ));

    // OUT= dataset.
    if let Some(target) = &ast.data_options.output {
        write_out_dataset(session, target, &by_names, &out_rows)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
