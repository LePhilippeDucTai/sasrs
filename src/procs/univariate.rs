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
//! - Quantiles / Extreme Observations pondérés : DIFFÉRÉ — choix (a) : ces
//!   sections sont OMISES lorsque WEIGHT est présent (une note centrée
//!   "Quantiles and Extreme Observations are not computed with a WEIGHT
//!   variable." est affichée à la place). On ne présente JAMAIS des
//!   quantiles non pondérés comme s'ils étaient pondérés.

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{
    by_groups, decode_column, partition_weighted, phi_inv, probnorm, resolve_by_cols, sample_std,
};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{format_best, Value, VarType};
use polars::prelude::*;
use std::cmp::Ordering;

pub struct UnivariateAst {
    pub data: Option<DatasetRef>,
    pub var: Vec<String>,
    /// BY variables (var, descending). Input must be sorted by the BY key.
    pub by: Vec<(String, bool)>,
    /// WEIGHT variable (single numeric var). When `Some`, the Moments and
    /// Basic Measures mean/std/variance use the weighted formulas; Quantiles
    /// and Extreme Observations are omitted (see file header).
    pub weight: Option<String>,
    pub output: Option<UnivariateOutput>,
    /// Tests for Normality requested (PROC option `normal` or `var x / normal`).
    /// When true and no WEIGHT is in effect, the "Tests for Normality" block is
    /// emitted after the Quantiles section. Default false → report is
    /// byte-identical to the pre-M21.3 output.
    pub normal: bool,
    /// Number of graphical statements (HISTOGRAM/QQPLOT/PROBPLOT/CDFPLOT) seen.
    /// Image rendering is deferred to ODS GRAPHICS (M29); for M21.3 these are
    /// parsed and a single NOTE is emitted, never a panic.
    pub graphics_deferred: usize,
}

/// OUTPUT OUT= specification: target dataset + (statistic keyword, output
/// variable names) pairs. Output names are paired positionally with the VAR
/// list.
pub struct UnivariateOutput {
    pub out: DatasetRef,
    /// (stat keyword lowercased, output var names in VAR-list order)
    pub specs: Vec<(String, Vec<String>)>,
}

/// Parse `proc univariate [data=a] [noprint] ; [var v...;] [by ...;] ... run;`.
/// Called AFTER "proc univariate" has been consumed. Consumes through
/// `run;`/`quit;`. Unknown sub-statements (e.g. BY, HISTOGRAM) are skipped
/// leniently to their terminating `;` (BY grouping is out of M5 scope).
pub fn parse(ts: &mut StatementStream) -> Result<UnivariateAst> {
    let mut data: Option<DatasetRef> = None;
    let mut var: Vec<String> = Vec::new();
    let mut normal = false;
    let mut graphics_deferred = 0usize;

    // --- PROC UNIVARIATE statement options, until `;` ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next(); // consume `;`
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            ts.next();
            expect_eq(ts, "DATA")?;
            data = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("noprint") {
            // Accepted and ignored for rendering: UNIVARIATE always shows its
            // report here. (NOPRINT only matters paired with OUTPUT in SAS.)
            ts.next();
        } else if ts.peek().is_kw("normal") || ts.peek().is_kw("normaltest") {
            // PROC-level request for the Tests for Normality block.
            ts.next();
            normal = true;
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            // Unknown header option: skip its token (and a possible `=value`)
            // leniently rather than error, to stay synchronized.
            ts.next();
            if ts.peek().kind == TokenKind::Eq {
                ts.next();
                if ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    ts.next();
                }
            }
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC UNIVARIATE statement.",
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; ---
    let mut by: Vec<(String, bool)> = Vec::new();
    let mut weight: Option<String> = None;
    let mut output: Option<UnivariateOutput> = None;

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
            var = ts.parse_name_list()?;
            // Optional `/ option…` clause on VAR (e.g. `var x / normal;`).
            if ts.peek().kind == TokenKind::Slash {
                ts.next();
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    if ts.peek().is_kw("normal") || ts.peek().is_kw("normaltest") {
                        normal = true;
                    }
                    ts.next();
                }
            }
            ts.expect_semi()?;
        } else if ts.peek().is_kw("by") {
            ts.next();
            by = crate::procs::means::parse_by_list(ts)?;
        } else if ts.peek().is_kw("weight") {
            ts.next();
            weight = Some(crate::procs::means::parse_single_var(ts, "WEIGHT")?);
        } else if ts.peek().is_kw("output") {
            ts.next();
            output = Some(parse_output(ts)?);
        } else if ts.peek().is_kw("histogram")
            || ts.peek().is_kw("qqplot")
            || ts.peek().is_kw("probplot")
            || ts.peek().is_kw("cdfplot")
            || ts.peek().is_kw("ppplot")
        {
            // Graphical statement: parse (skip its body to `;`); image rendering
            // is deferred to ODS GRAPHICS (M29). Never error or panic.
            ts.next();
            ts.skip_to_semi();
            graphics_deferred += 1;
        } else {
            // Unknown sub-statement (id, class, ...): skip leniently.
            ts.skip_to_semi();
        }
    }

    Ok(UnivariateAst {
        data,
        var,
        by,
        weight,
        output,
        normal,
        graphics_deferred,
    })
}

/// Recognized OUTPUT statistic keywords (paired positionally with VAR list).
fn is_output_stat(s: &str) -> bool {
    matches!(
        s,
        "mean"
            | "std"
            | "stddev"
            | "min"
            | "max"
            | "median"
            | "n"
            | "nmiss"
            | "sum"
            | "q1"
            | "q3"
            | "p25"
            | "p75"
            | "p50"
            | "p1"
            | "p5"
            | "p10"
            | "p90"
            | "p95"
            | "p99"
            | "range"
            | "qrange"
            | "var"
    )
}

/// Parse the OUTPUT statement body (after "output" consumed), through `;`.
/// `output out=lib.t [stat=name [name...]] ... ;` — each statistic keyword is
/// followed by one or more output variable names, paired positionally with the
/// VAR list. `var=` is accepted as the VARIANCE keyword.
fn parse_output(ts: &mut StatementStream) -> Result<UnivariateOutput> {
    let mut out: Option<DatasetRef> = None;
    let mut specs: Vec<(String, Vec<String>)> = Vec::new();

    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("out") {
            ts.next();
            expect_eq(ts, "OUT")?;
            out = Some(ts.parse_dataset_ref()?);
        } else if let Some(kw) = ts.peek().ident().map(str::to_string) {
            let stat = kw.to_ascii_lowercase();
            if !is_output_stat(&stat) {
                return Err(SasError::parse(
                    format!("Unsupported statistic '{}' in OUTPUT statement.", kw.to_uppercase()),
                    ts.peek().span,
                ));
            }
            ts.next(); // stat keyword
            expect_eq(ts, "OUTPUT statistic")?;
            // Collect one or more output names until the next stat keyword,
            // `out`, or `;`.
            let mut names: Vec<String> = Vec::new();
            while let Some(n) = ts.peek().ident().map(str::to_string) {
                let nl = n.to_ascii_lowercase();
                // Stop if this ident is actually the next keyword followed
                // by '=' (e.g. `mean=mx n=nx`).
                if (is_output_stat(&nl) || nl == "out") && ts.peek2().kind == TokenKind::Eq {
                    break;
                }
                ts.next();
                names.push(n);
            }
            if names.is_empty() {
                return Err(SasError::parse(
                    format!("expected an output variable name after {}=", stat),
                    ts.peek().span,
                ));
            }
            specs.push((stat, names));
        } else {
            return Err(SasError::parse(
                "unexpected token in OUTPUT statement",
                ts.peek().span,
            ));
        }
    }

    let out = out.ok_or_else(|| {
        SasError::runtime("The OUTPUT statement requires the OUT= option in PROC UNIVARIATE.")
    })?;
    Ok(UnivariateOutput { out, specs })
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

/// Resolve `data=` or `_LAST_` into a concrete DatasetRef.
fn resolve_input(ast: &UnivariateAst, session: &Session) -> Result<DatasetRef> {
    match &ast.data {
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

// ─────────────────────────── statistics helpers ───────────────────────────

/// SAS skewness g1 (needs n>=3 and s>0, else None):
/// `g1 = n/((n-1)(n-2)) * Σ((x_i-mean)/s)^3`.
fn skewness(xs: &[f64]) -> Option<f64> {
    let n = xs.len();
    if n < 3 {
        return None;
    }
    let s = sample_std(xs)?;
    if s == 0.0 {
        return None;
    }
    let nf = n as f64;
    let mean = xs.iter().sum::<f64>() / nf;
    let sum3: f64 = xs.iter().map(|x| ((x - mean) / s).powi(3)).sum();
    Some(nf / ((nf - 1.0) * (nf - 2.0)) * sum3)
}

/// SAS excess kurtosis g2 (needs n>=4 and s>0, else None):
/// `g2 = [ n(n+1)/((n-1)(n-2)(n-3)) ] * Σ((x_i-mean)/s)^4
///       - 3(n-1)^2 / ((n-2)(n-3))`.
fn kurtosis(xs: &[f64]) -> Option<f64> {
    let n = xs.len();
    if n < 4 {
        return None;
    }
    let s = sample_std(xs)?;
    if s == 0.0 {
        return None;
    }
    let nf = n as f64;
    let mean = xs.iter().sum::<f64>() / nf;
    let sum4: f64 = xs.iter().map(|x| ((x - mean) / s).powi(4)).sum();
    let term1 = nf * (nf + 1.0) / ((nf - 1.0) * (nf - 2.0) * (nf - 3.0)) * sum4;
    let term2 = 3.0 * (nf - 1.0).powi(2) / ((nf - 2.0) * (nf - 3.0));
    Some(term1 - term2)
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
fn quantile_def5(sorted: &[f64], p: f64) -> Option<f64> {
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

/// Mode: smallest most-frequent value, but only if some value repeats
/// (count >= 2). If every value appears once, SAS reports no mode → None.
/// `sorted` must be ascending.
fn mode(sorted: &[f64]) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let mut best_val = sorted[0];
    let mut best_cnt = 1usize;
    let mut cur_val = sorted[0];
    let mut cur_cnt = 1usize;
    for &v in &sorted[1..] {
        if v == cur_val {
            cur_cnt += 1;
        } else {
            if cur_cnt > best_cnt {
                best_cnt = cur_cnt;
                best_val = cur_val;
            }
            cur_val = v;
            cur_cnt = 1;
        }
    }
    if cur_cnt > best_cnt {
        best_cnt = cur_cnt;
        best_val = cur_val;
    }
    if best_cnt >= 2 {
        Some(best_val)
    } else {
        None
    }
}

/// Format a numeric statistic value (BEST-style, width 12).
fn fmt_num(v: f64) -> String {
    format_best(v, 12)
}

/// Format an optional statistic: None → "." (SAS missing).
fn fmt_opt(v: Option<f64>) -> String {
    match v {
        Some(f) => fmt_num(f),
        None => ".".to_string(),
    }
}

// ─────────────────────────────── execute ──────────────────────────────────

pub fn execute(ast: &UnivariateAst, session: &mut Session) -> Result<()> {
    let in_ref = resolve_input(ast, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();
    let display_name = format!("{in_libref}.{in_table}");

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }

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

    session.listing.page_header();
    centered(session, "The UNIVARIATE Procedure");

    for (by_key, grp_rows) in &by_groups_list {
        if !by_names.is_empty() {
            emit_by_heading(session, &by_names, by_key);
        }
        for (vi, &ci) in var_cols.iter().enumerate() {
            match &weight_values {
                Some(wv) => {
                    // Weighted path: usable (value, weight) pairs + excluded count.
                    let (pairs, n_missing) = partition_weighted(&var_values[vi], wv, grp_rows);
                    emit_variable_weighted(
                        session,
                        &ds.vars[ci].name,
                        &pairs,
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
                    );
                }
            }
        }
    }

    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        n_obs, display_name
    ));

    if ast.graphics_deferred > 0 {
        session.log.note(
            "HISTOGRAM/QQPLOT: graphical output deferred to ODS GRAPHICS (M29).",
        );
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

/// Emit the SAS BY heading line into the listing: `var1=val1 var2=val2`.
fn emit_by_heading(session: &mut Session, by_names: &[String], by_key: &[Value]) {
    let parts: Vec<String> = by_names
        .iter()
        .zip(by_key)
        .map(|(name, v)| {
            let cell = match v {
                Value::Num(f) => format_best(*f, 12),
                Value::Missing(k) => k.display(),
                Value::Char(s) => s.trim_end().to_string(),
            };
            format!("{}={}", name, cell)
        })
        .collect();
    session.listing.write_line(&parts.join(" "));
    session.listing.blank();
}

/// Compute one OUTPUT statistic for a single variable over the group's
/// non-missing values `xs` (sorted in `sorted`), the missing count, and the
/// total row count. Returns `None` (→ SAS missing) when undefined.
fn output_stat(
    stat: &str,
    xs: &[f64],
    sorted: &[f64],
    n_missing: usize,
) -> Option<f64> {
    let n = xs.len();
    let mean = if n > 0 {
        Some(xs.iter().sum::<f64>() / n as f64)
    } else {
        None
    };
    match stat {
        "n" => Some(n as f64),
        "nmiss" => Some(n_missing as f64),
        "sum" => Some(xs.iter().sum()),
        "mean" => mean,
        "std" | "stddev" => sample_std(xs),
        "var" => sample_std(xs).map(|s| s * s),
        "min" | "p0" => sorted.first().copied(),
        "max" | "p100" => sorted.last().copied(),
        "median" | "p50" => quantile_def5(sorted, 0.50),
        "q1" | "p25" => quantile_def5(sorted, 0.25),
        "q3" | "p75" => quantile_def5(sorted, 0.75),
        "p1" => quantile_def5(sorted, 0.01),
        "p5" => quantile_def5(sorted, 0.05),
        "p10" => quantile_def5(sorted, 0.10),
        "p90" => quantile_def5(sorted, 0.90),
        "p95" => quantile_def5(sorted, 0.95),
        "p99" => quantile_def5(sorted, 0.99),
        "range" => {
            if n > 0 {
                Some(sorted[n - 1] - sorted[0])
            } else {
                None
            }
        }
        "qrange" => match (quantile_def5(sorted, 0.75), quantile_def5(sorted, 0.25)) {
            (Some(a), Some(b)) => Some(a - b),
            _ => None,
        },
        _ => None,
    }
}

/// Build and write the OUTPUT OUT= dataset: one row per BY group (one overall
/// when no BY), with BY variables followed by the requested statistic columns
/// (each statistic keyword paired positionally with the VAR list).
#[allow(clippy::too_many_arguments)]
fn write_output(
    session: &mut Session,
    ds: &SasDataset,
    var_cols: &[usize],
    var_values: &[Vec<Value>],
    out: &UnivariateOutput,
    by_cols: &[crate::procs::common::ByCol],
    by_groups_list: &[(Vec<Value>, Vec<usize>)],
) -> Result<()> {
    // Validate: each spec must not request more output names than there are
    // analysis variables (positional pairing with VAR list).
    for (stat, names) in &out.specs {
        if names.len() > var_cols.len() {
            return Err(SasError::runtime(format!(
                "The OUTPUT statement requests {} names for statistic {} but only {} \
                 analysis variable(s) are available.",
                names.len(),
                stat.to_uppercase(),
                var_cols.len()
            )));
        }
    }

    let n_rows = by_groups_list.len();
    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();

    // BY columns first (one value per BY group).
    for (bi, bc) in by_cols.iter().enumerate() {
        let meta = &ds.vars[bc.col_idx];
        let series = match meta.ty {
            VarType::Num => {
                let vals: Vec<Option<f64>> = by_groups_list
                    .iter()
                    .map(|(key, _)| value_to_num(&key[bi]))
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
            VarType::Char => {
                let vals: Vec<Option<String>> = by_groups_list
                    .iter()
                    .map(|(key, _)| match &key[bi] {
                        Value::Char(s) if s.is_empty() => None,
                        Value::Char(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
        };
        columns.push(series.into());
        vars.push(meta.clone());
    }

    // Precompute, per BY group, the (xs, sorted, n_missing) per analysis var.
    struct VarStats {
        xs: Vec<f64>,
        sorted: Vec<f64>,
        n_missing: usize,
    }
    let mut per_group: Vec<Vec<VarStats>> = Vec::with_capacity(by_groups_list.len());
    for (_key, grp_rows) in by_groups_list {
        let mut per_var: Vec<VarStats> = Vec::with_capacity(var_cols.len());
        for vv in var_values.iter() {
            let mut xs: Vec<f64> = Vec::with_capacity(grp_rows.len());
            let mut n_missing = 0usize;
            for &row in grp_rows {
                match value_to_num(&vv[row]) {
                    Some(f) if !f.is_nan() => xs.push(f),
                    _ => n_missing += 1,
                }
            }
            let mut sorted = xs.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            per_var.push(VarStats { xs, sorted, n_missing });
        }
        per_group.push(per_var);
    }

    // One statistic column per (spec, paired analysis variable).
    for (stat, names) in &out.specs {
        for (vi, outname) in names.iter().enumerate() {
            let vals: Vec<Option<f64>> = per_group
                .iter()
                .map(|pv| {
                    let vs = &pv[vi];
                    output_stat(stat, &vs.xs, &vs.sorted, vs.n_missing)
                })
                .collect();
            columns.push(Series::new(outname.as_str().into(), vals).into());
            vars.push(num_var_meta(outname));
        }
    }

    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars };

    let out_libref = out.out.libref_or_work();
    let out_table = out.out.name.to_uppercase();
    let display = format!("{out_libref}.{out_table}");
    let n_vars = out_ds.vars.len();

    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(display.clone());

    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_rows, n_vars
    ));

    Ok(())
}

/// VarMeta for a numeric output column.
fn num_var_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
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

/// Emit the full report for a single analysis variable. `data` holds the
/// non-missing (value, obs_number) pairs in original observation order.
fn emit_variable(
    session: &mut Session,
    name: &str,
    data: &[(f64, usize)],
    n_missing: usize,
    n_total: usize,
    normal: bool,
) {
    session.listing.blank();
    centered(session, &format!("Variable: {name}"));
    session.listing.blank();

    let n = data.len();
    // Plain non-missing values.
    let xs: Vec<f64> = data.iter().map(|(v, _)| *v).collect();
    // Sorted values (for quantiles / mode / median / extremes).
    let mut sorted: Vec<f64> = xs.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

    let nf = n as f64;
    let sum: f64 = xs.iter().sum();
    let mean = if n > 0 { Some(sum / nf) } else { None };
    let s = sample_std(&xs);
    let variance = s.map(|v| v * v);
    let uss: f64 = xs.iter().map(|x| x * x).sum();
    let css: f64 = match mean {
        Some(m) => xs.iter().map(|x| (x - m) * (x - m)).sum(),
        None => 0.0,
    };
    let cv = match (mean, s) {
        (Some(m), Some(sd)) if m != 0.0 => Some(100.0 * sd / m),
        _ => None,
    };
    let std_err = match s {
        Some(sd) if n >= 1 => Some(sd / nf.sqrt()),
        _ => None,
    };
    let skew = skewness(&xs);
    let kurt = kurtosis(&xs);

    // ── Moments ──
    centered(session, "Moments");
    session.listing.blank();
    let moments: Vec<(&str, String, &str, String)> = vec![
        (
            "N",
            format!("{n}"),
            "Sum Weights",
            format!("{n}"),
        ),
        (
            "Mean",
            fmt_opt(mean),
            "Sum Observations",
            fmt_num(sum),
        ),
        (
            "Std Deviation",
            fmt_opt(s),
            "Variance",
            fmt_opt(variance),
        ),
        (
            "Skewness",
            fmt_opt(skew),
            "Kurtosis",
            fmt_opt(kurt),
        ),
        (
            "Uncorrected SS",
            fmt_num(uss),
            "Corrected SS",
            fmt_num(css),
        ),
        (
            "Coeff Variation",
            fmt_opt(cv),
            "Std Error Mean",
            fmt_opt(std_err),
        ),
    ];
    let m_rows: Vec<Vec<String>> = moments
        .into_iter()
        .map(|(la, va, lb, vb)| vec![la.to_string(), va, lb.to_string(), vb])
        .collect();
    session.listing.write_table(
        &[
            "Label1".into(),
            "Value1".into(),
            "Label2".into(),
            "Value2".into(),
        ],
        &[Align::Left, Align::Right, Align::Left, Align::Right],
        &m_rows,
    );

    // ── Basic Statistical Measures ──
    session.listing.blank();
    centered(session, "Basic Statistical Measures");
    session.listing.blank();
    let median = quantile_def5(&sorted, 0.5);
    let mode_v = mode(&sorted);
    let range = if n > 0 {
        Some(sorted[n - 1] - sorted[0])
    } else {
        None
    };
    let q3 = quantile_def5(&sorted, 0.75);
    let q1 = quantile_def5(&sorted, 0.25);
    let iqr = match (q3, q1) {
        (Some(a), Some(b)) => Some(a - b),
        _ => None,
    };
    let basic_rows: Vec<Vec<String>> = vec![
        vec![
            "Mean".into(),
            fmt_opt(mean),
            "Std Deviation".into(),
            fmt_opt(s),
        ],
        vec![
            "Median".into(),
            fmt_opt(median),
            "Variance".into(),
            fmt_opt(variance),
        ],
        vec![
            "Mode".into(),
            fmt_opt(mode_v),
            "Range".into(),
            fmt_opt(range),
        ],
        vec![
            "".into(),
            "".into(),
            "Interquartile Range".into(),
            fmt_opt(iqr),
        ],
    ];
    session.listing.write_table(
        &[
            "LocLabel".into(),
            "LocValue".into(),
            "VarLabel".into(),
            "VarValue".into(),
        ],
        &[Align::Left, Align::Right, Align::Left, Align::Right],
        &basic_rows,
    );

    // ── Tests for Normality (only when requested via NORMAL) ──
    if normal {
        emit_normality_tests(session, &sorted, mean, s, n);
    }

    // ── Quantiles (Definition 5) ──
    session.listing.blank();
    centered(session, "Quantiles (Definition 5)");
    session.listing.blank();
    let levels: &[(&str, f64)] = &[
        ("100% Max", 1.0),
        ("99%", 0.99),
        ("95%", 0.95),
        ("90%", 0.90),
        ("75% Q3", 0.75),
        ("50% Median", 0.50),
        ("25% Q1", 0.25),
        ("10%", 0.10),
        ("5%", 0.05),
        ("1%", 0.01),
        ("0% Min", 0.0),
    ];
    let q_rows: Vec<Vec<String>> = levels
        .iter()
        .map(|(label, p)| vec![label.to_string(), fmt_opt(quantile_def5(&sorted, *p))])
        .collect();
    session.listing.write_table(
        &["Quantile".into(), "Estimate".into()],
        &[Align::Left, Align::Right],
        &q_rows,
    );

    // ── Extreme Observations ──
    session.listing.blank();
    centered(session, "Extreme Observations");
    session.listing.blank();
    // Order data by value, then by obs number (stable for ties).
    let mut by_val: Vec<(f64, usize)> = data.to_vec();
    by_val.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });
    let k = by_val.len().min(5);
    let lowest = &by_val[..k];
    let highest = &by_val[by_val.len().saturating_sub(5)..];
    // Pair them up row-by-row (both columns show up to 5 entries).
    let mut ext_rows: Vec<Vec<String>> = Vec::new();
    for i in 0..5 {
        let (lv, lo) = match lowest.get(i) {
            Some((v, o)) => (fmt_num(*v), format!("{o}")),
            None => (String::new(), String::new()),
        };
        // Highest displayed ascending too (SAS shows the top 5 in ascending
        // order within the Highest column).
        let (hv, ho) = match highest.get(i) {
            Some((v, o)) => (fmt_num(*v), format!("{o}")),
            None => (String::new(), String::new()),
        };
        ext_rows.push(vec![lv, lo, hv, ho]);
    }
    session.listing.write_table(
        &[
            "Lowest Value".into(),
            "Lowest Obs".into(),
            "Highest Value".into(),
            "Highest Obs".into(),
        ],
        &[Align::Right, Align::Right, Align::Right, Align::Right],
        &ext_rows,
    );

    // ── Missing Values ──
    if n_missing > 0 {
        session.listing.blank();
        centered(session, "Missing Values");
        session.listing.blank();
        let pct = if n_total > 0 {
            100.0 * n_missing as f64 / n_total as f64
        } else {
            0.0
        };
        session.listing.write_table(
            &[
                "Missing Value".into(),
                "Count".into(),
                "Percent Of All Obs".into(),
            ],
            &[Align::Left, Align::Right, Align::Right],
            &[vec![".".into(), format!("{n_missing}"), fmt_num(pct)]],
        );
    }
}

// ───────────────────────────── normality tests ─────────────────────────────
//
// All four statistics are computed from the ascending-sorted non-missing
// values, the sample mean, and the sample standard deviation (VARDEF=DF, the
// same `sample_std` used elsewhere). p-values follow published approximations
// (Royston for Shapiro-Wilk; Stephens for Anderson-Darling / Cramér-von Mises;
// Lilliefors/Dallal-Wilkinson for the EDF Kolmogorov-Smirnov D). They are NOT
// bit-for-bit identical to SAS 9.4, but reproduce the documented reference
// values; this is noted in PROGRESS.md.

/// A computed normality test: name, statistic label, statistic value, and an
/// optional p-value (None → not computable, shown as ".").
struct NormalityTest {
    name: &'static str,
    stat_label: &'static str,
    stat: f64,
    p: Option<f64>,
}

/// Emit the "Tests for Normality" block for one variable. Requires the sorted
/// non-missing values plus the sample mean/std already computed by the caller.
/// Degenerate inputs (n < 3, zero variance, …) print a centered NOTE instead
/// of the table — never a panic.
fn emit_normality_tests(
    session: &mut Session,
    sorted: &[f64],
    mean: Option<f64>,
    std: Option<f64>,
    n: usize,
) {
    session.listing.blank();
    centered(session, "Tests for Normality");
    session.listing.blank();

    let (mean, std) = match (mean, std) {
        (Some(m), Some(s)) if s > 0.0 && n >= 3 => (m, s),
        _ => {
            centered(
                session,
                "Tests for Normality require at least 3 nonmissing values with positive variance.",
            );
            return;
        }
    };

    let tests = compute_normality_tests(sorted, mean, std, n);

    // Columns: Test | Statistic-label | Statistic-value | "p Value"-label |
    // p-value. SAS renders the p as `Pr < W`, `Pr > D`, etc.
    let rows: Vec<Vec<String>> = tests
        .iter()
        .map(|t| {
            let pcell = match t.p {
                Some(p) => fmt_num(p),
                None => ".".to_string(),
            };
            let plabel = match t.name {
                "Shapiro-Wilk" => "Pr < W",
                "Kolmogorov-Smirnov" => "Pr > D",
                "Cramer-von Mises" => "Pr > W-Sq",
                "Anderson-Darling" => "Pr > A-Sq",
                _ => "Pr",
            };
            vec![
                t.name.to_string(),
                t.stat_label.to_string(),
                fmt_num(t.stat),
                plabel.to_string(),
                pcell,
            ]
        })
        .collect();

    session.listing.write_table(
        &[
            "Test".into(),
            "StatLabel".into(),
            "StatValue".into(),
            "PLabel".into(),
            "PValue".into(),
        ],
        &[Align::Left, Align::Left, Align::Right, Align::Left, Align::Right],
        &rows,
    );
}

/// Compute the four normality statistics + p-values. `sorted` ascending,
/// `mean`/`std` the sample moments (std > 0), `n == sorted.len() >= 3`.
fn compute_normality_tests(sorted: &[f64], mean: f64, std: f64, n: usize) -> Vec<NormalityTest> {
    let mut out = Vec::with_capacity(4);

    // Shapiro-Wilk (only defined for 3 <= n <= 2000).
    let (sw_w, sw_p) = shapiro_wilk(sorted);
    out.push(NormalityTest {
        name: "Shapiro-Wilk",
        stat_label: "W",
        stat: sw_w.unwrap_or(f64::NAN),
        p: sw_p,
    });

    // Standardized, sorted z_i = (x_(i) - mean) / std.
    let z: Vec<f64> = sorted.iter().map(|&x| (x - mean) / std).collect();

    // Kolmogorov-Smirnov D (Lilliefors, estimated parameters).
    let (ks_d, ks_p) = kolmogorov_smirnov(&z, n);
    out.push(NormalityTest {
        name: "Kolmogorov-Smirnov",
        stat_label: "D",
        stat: ks_d,
        p: ks_p,
    });

    // Cramér-von Mises W².
    let (cvm, cvm_p) = cramer_von_mises(&z, n);
    out.push(NormalityTest {
        name: "Cramer-von Mises",
        stat_label: "W-Sq",
        stat: cvm,
        p: cvm_p,
    });

    // Anderson-Darling A².
    let (ad, ad_p) = anderson_darling(&z, n);
    out.push(NormalityTest {
        name: "Anderson-Darling",
        stat_label: "A-Sq",
        stat: ad,
        p: ad_p,
    });

    out
}

/// Shapiro-Wilk W and its p-value (Royston 1992 algorithm AS R94).
/// Valid for 3 <= n <= 2000. Returns `(Some(W), Some(p))`, or `(None, None)`
/// when n is out of range. `sorted` must be ascending with positive variance.
fn shapiro_wilk(sorted: &[f64]) -> (Option<f64>, Option<f64>) {
    let n = sorted.len();
    if n < 3 || n > 2000 {
        return (None, None);
    }
    let nf = n as f64;
    let mean = sorted.iter().sum::<f64>() / nf;
    let ss: f64 = sorted.iter().map(|x| (x - mean) * (x - mean)).sum();
    if ss <= 0.0 {
        return (None, None);
    }

    // Expected values of standard normal order statistics, m_i = Φ⁻¹((i-3/8)/(n+1/4)).
    let m: Vec<f64> = (1..=n)
        .map(|i| phi_inv((i as f64 - 0.375) / (nf + 0.25)))
        .collect();
    let m_sq_sum: f64 = m.iter().map(|v| v * v).sum();
    let rsn = 1.0 / nf.sqrt();

    // Royston polynomial corrections for a_n and a_{n-1}.
    let poly = |c: &[f64], x: f64| -> f64 {
        // Horner with c[0] the constant term.
        c.iter().rev().fold(0.0, |acc, &ci| acc * x + ci)
    };
    const C1: [f64; 6] = [0.0, 0.221157, -0.147981, -2.071190, 4.434685, -2.706056];
    const C2: [f64; 6] = [0.0, 0.042981, -0.293762, -1.752461, 5.682633, -3.582633];

    let mut a = vec![0.0_f64; n];
    let a_n = m[n - 1] / m_sq_sum.sqrt() + poly(&C1, rsn);
    let (i1, fac);
    if n > 5 {
        let a_n1 = m[n - 2] / m_sq_sum.sqrt() + poly(&C2, rsn);
        a[n - 1] = a_n;
        a[n - 2] = a_n1;
        a[0] = -a_n;
        a[1] = -a_n1;
        // Rescale the interior coefficients.
        let phi = (m_sq_sum - 2.0 * m[n - 1] * m[n - 1] - 2.0 * m[n - 2] * m[n - 2])
            / (1.0 - 2.0 * a_n * a_n - 2.0 * a_n1 * a_n1);
        fac = phi.sqrt();
        i1 = 2;
    } else {
        a[n - 1] = a_n;
        a[0] = -a_n;
        let phi = (m_sq_sum - 2.0 * m[n - 1] * m[n - 1]) / (1.0 - 2.0 * a_n * a_n);
        fac = phi.sqrt();
        i1 = 1;
    }
    for i in i1..(n - i1) {
        a[i] = m[i] / fac;
    }

    // W = (Σ a_i x_(i))² / Σ(x_i - x̄)².
    let num: f64 = a.iter().zip(sorted.iter()).map(|(&ai, &xi)| ai * xi).sum();
    let w = (num * num) / ss;
    let w = w.min(1.0);

    // p-value via Royston's normalizing transform.
    let p = shapiro_wilk_pvalue(w, n);
    (Some(w), Some(p))
}

/// Royston (1992) p-value for Shapiro-Wilk W, n >= 3.
fn shapiro_wilk_pvalue(w: f64, n: usize) -> f64 {
    let nf = n as f64;
    if n == 3 {
        // Exact small-sample formula (Royston): p = 6/π · (asin(√W) − asin(√(3/4))).
        let pi = std::f64::consts::PI;
        let p = 6.0 / pi * ((w.sqrt()).asin() - (0.75_f64.sqrt()).asin());
        return (1.0 - p).clamp(0.0, 1.0);
    }
    let ln_n = nf.ln();
    let (mu, sigma, z);
    if n <= 11 {
        // Small-sample branch: γ-transform of (1 - W).
        const G: [f64; 2] = [-2.273, 0.459];
        const M: [f64; 4] = [0.5440, -0.39978, 0.025054, -6.714e-4];
        const S: [f64; 4] = [1.3822, -0.77857, 0.062767, -0.0020322];
        let gamma = G[0] + G[1] * nf;
        mu = M[0] + M[1] * nf + M[2] * nf * nf + M[3] * nf * nf * nf;
        let ln_sigma = S[0] + S[1] * nf + S[2] * nf * nf + S[3] * nf * nf * nf;
        sigma = ln_sigma.exp();
        let y = -(gamma - (1.0 - w).ln()).ln();
        z = (y - mu) / sigma;
    } else {
        // Large-sample branch (n >= 12): ln(1 - W) normalized in ln(n).
        const M: [f64; 4] = [-1.5861, -0.31082, -0.083751, 0.0038915];
        const S: [f64; 3] = [-0.4803, -0.082676, 0.0030302];
        mu = M[0] + M[1] * ln_n + M[2] * ln_n * ln_n + M[3] * ln_n * ln_n * ln_n;
        let ln_sigma = S[0] + S[1] * ln_n + S[2] * ln_n * ln_n;
        sigma = ln_sigma.exp();
        let y = (1.0 - w).ln();
        z = (y - mu) / sigma;
    }
    // p = P(Z > z) = upper tail of standard normal.
    1.0 - probnorm(z)
}

/// Kolmogorov-Smirnov D (Lilliefors test, parameters estimated from the data)
/// and an approximate p-value. `z` are the standardized sorted values; `n` is
/// the sample size.
fn kolmogorov_smirnov(z: &[f64], n: usize) -> (f64, Option<f64>) {
    let nf = n as f64;
    let mut d = 0.0_f64;
    for (i, &zi) in z.iter().enumerate() {
        let f = probnorm(zi);
        let d_plus = (i as f64 + 1.0) / nf - f; // F_n(x_i) - F(x_i)
        let d_minus = f - (i as f64) / nf; // F(x_i) - F_n(x_i⁻)
        d = d.max(d_plus).max(d_minus);
    }
    let p = lilliefors_pvalue(d, n);
    (d, Some(p))
}

/// Approximate Lilliefors p-value for the KS D statistic with estimated
/// parameters, via the Dallal & Wilkinson (1986) analytic approximation.
///
/// This single-exponential form is the published upper-tail probability and is
/// accurate for the significant region p ≤ 0.10; for larger D the exponent
/// becomes < 1 (often > 1 before clamping), so values are clamped to 1.0 and
/// interpreted as "p > 0.10" (non-significant). Documented approximation — not
/// bit-identical to SAS's internal Lilliefors table.
fn lilliefors_pvalue(d: f64, n: usize) -> f64 {
    if d <= 0.0 {
        return 1.0;
    }
    // For n > 100, scale D and cap the effective sample size at 100
    // (Dallal-Wilkinson extension).
    let (d_eff, n_eff) = if n > 100 {
        (d * (n as f64 / 100.0).powf(0.49), 100.0_f64)
    } else {
        (d, n as f64)
    };
    let expo = -7.01256 * d_eff * d_eff * (n_eff + 2.78019)
        + 2.99587 * d_eff * (n_eff + 2.78019).sqrt()
        - 0.122119
        + 0.974598 / n_eff.sqrt()
        + 1.67997 / n_eff;
    let pval = expo.exp();
    if pval.is_finite() {
        pval.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// Cramér-von Mises W² (estimated parameters) and Stephens p-value.
fn cramer_von_mises(z: &[f64], n: usize) -> (f64, Option<f64>) {
    let nf = n as f64;
    let mut w2 = 1.0 / (12.0 * nf);
    for (i, &zi) in z.iter().enumerate() {
        let f = probnorm(zi);
        let term = f - (2.0 * (i as f64 + 1.0) - 1.0) / (2.0 * nf);
        w2 += term * term;
    }
    // Modification for estimated parameters.
    let w2_star = w2 * (1.0 + 0.5 / nf);
    let p = cvm_pvalue(w2_star);
    (w2, Some(p))
}

/// Stephens (1974) p-value regions for the (modified) Cramér-von Mises W²*.
fn cvm_pvalue(w: f64) -> f64 {
    // Piecewise upper-tail approximation (Stephens / D'Agostino & Stephens).
    if w < 0.0275 {
        1.0
    } else if w < 0.051 {
        1.0 - (-13.953 + 775.5 * w - 12542.61 * w * w).exp()
    } else if w < 0.092 {
        1.0 - (-5.903 + 179.546 * w - 1515.29 * w * w).exp()
    } else if w < 1.1 {
        (0.886 - 31.62 * w + 10.897 * w * w).exp()
    } else {
        0.0
    }
    .clamp(0.0, 1.0)
}

/// Anderson-Darling A² (estimated parameters) and Stephens p-value.
fn anderson_darling(z: &[f64], n: usize) -> (f64, Option<f64>) {
    let nf = n as f64;
    let mut s = 0.0_f64;
    for i in 0..n {
        let fi = probnorm(z[i]); // Φ(z_(i))
        let fr = probnorm(z[n - 1 - i]); // Φ(z_(n+1-i)) with 0-based index
        // Guard the logs against 0/1 (degenerate tails).
        let a = fi.clamp(1e-300, 1.0 - 1e-16);
        let b = (1.0 - fr).clamp(1e-300, 1.0);
        s += (2.0 * (i as f64 + 1.0) - 1.0) * (a.ln() + b.ln());
    }
    let a2 = -nf - s / nf;
    let a2_star = a2 * (1.0 + 0.75 / nf + 2.25 / (nf * nf));
    let p = ad_pvalue(a2_star);
    (a2, Some(p))
}

/// Stephens (1974) p-value regions for the (modified) Anderson-Darling A²*.
fn ad_pvalue(a: f64) -> f64 {
    if a < 0.2 {
        1.0 - (-13.436 + 101.14 * a - 223.73 * a * a).exp()
    } else if a < 0.34 {
        1.0 - (-8.318 + 42.796 * a - 59.938 * a * a).exp()
    } else if a < 0.6 {
        (0.9177 - 4.279 * a - 1.38 * a * a).exp()
    } else if a < 13.0 {
        (1.2937 - 5.709 * a + 0.0186 * a * a).exp()
    } else {
        0.0
    }
    .clamp(0.0, 1.0)
}

/// Emit the report for a single analysis variable with a WEIGHT variable in
/// effect. `pairs` are the usable (value, weight) pairs (excluding missing
/// values, missing weights, and weights ≤ 0); `n_missing` is the excluded
/// count, `n_total` the group's total row count.
///
/// Moments and Basic Measures mean/std/variance use the weighted formulas
/// (see file header). Skewness/Kurtosis are computed on the UNWEIGHTED values
/// (documented divergence). Quantiles and Extreme Observations are OMITTED.
fn emit_variable_weighted(
    session: &mut Session,
    name: &str,
    pairs: &[(f64, f64)],
    n_missing: usize,
    n_total: usize,
) {
    session.listing.blank();
    centered(session, &format!("Variable: {name}"));
    session.listing.blank();

    let n = pairs.len();
    let nf = n as f64;
    let xs: Vec<f64> = pairs.iter().map(|(x, _)| *x).collect();

    let sum_w: f64 = pairs.iter().map(|(_, w)| *w).sum();
    let sum_wx: f64 = pairs.iter().map(|(x, w)| w * x).sum();
    let mean_w = if sum_w != 0.0 {
        Some(sum_wx / sum_w)
    } else {
        None
    };
    // Weighted corrected / uncorrected sums of squares.
    let css_w: f64 = match mean_w {
        Some(m) => pairs.iter().map(|(x, w)| w * (x - m) * (x - m)).sum(),
        None => 0.0,
    };
    let uss_w: f64 = pairs.iter().map(|(x, w)| w * x * x).sum();
    let variance = if n >= 2 {
        Some(css_w / (nf - 1.0))
    } else {
        None
    };
    let std = variance.map(|v| v.sqrt());
    let cv = match (mean_w, std) {
        (Some(m), Some(sd)) if m != 0.0 => Some(100.0 * sd / m),
        _ => None,
    };
    // SAS weighted std error of the mean: Std / sqrt(Σ w_i).
    let std_err = match std {
        Some(sd) if sum_w > 0.0 => Some(sd / sum_w.sqrt()),
        _ => None,
    };
    // Skewness / kurtosis deferred → computed on UNWEIGHTED values.
    let skew = skewness(&xs);
    let kurt = kurtosis(&xs);

    // ── Moments ──
    centered(session, "Moments");
    session.listing.blank();
    let moments: Vec<(&str, String, &str, String)> = vec![
        ("N", format!("{n}"), "Sum Weights", fmt_num(sum_w)),
        ("Mean", fmt_opt(mean_w), "Sum Observations", fmt_num(sum_wx)),
        ("Std Deviation", fmt_opt(std), "Variance", fmt_opt(variance)),
        ("Skewness", fmt_opt(skew), "Kurtosis", fmt_opt(kurt)),
        ("Uncorrected SS", fmt_num(uss_w), "Corrected SS", fmt_num(css_w)),
        ("Coeff Variation", fmt_opt(cv), "Std Error Mean", fmt_opt(std_err)),
    ];
    let m_rows: Vec<Vec<String>> = moments
        .into_iter()
        .map(|(la, va, lb, vb)| vec![la.to_string(), va, lb.to_string(), vb])
        .collect();
    session.listing.write_table(
        &[
            "Label1".into(),
            "Value1".into(),
            "Label2".into(),
            "Value2".into(),
        ],
        &[Align::Left, Align::Right, Align::Left, Align::Right],
        &m_rows,
    );

    // ── Basic Statistical Measures ── (weighted mean/std/variance; mode,
    // median, range, IQR depend on quantiles → deferred, shown as missing).
    session.listing.blank();
    centered(session, "Basic Statistical Measures");
    session.listing.blank();
    let basic_rows: Vec<Vec<String>> = vec![
        vec![
            "Mean".into(),
            fmt_opt(mean_w),
            "Std Deviation".into(),
            fmt_opt(std),
        ],
        vec![
            "Median".into(),
            ".".into(),
            "Variance".into(),
            fmt_opt(variance),
        ],
        vec!["Mode".into(), ".".into(), "Range".into(), ".".into()],
        vec![
            "".into(),
            "".into(),
            "Interquartile Range".into(),
            ".".into(),
        ],
    ];
    session.listing.write_table(
        &[
            "LocLabel".into(),
            "LocValue".into(),
            "VarLabel".into(),
            "VarValue".into(),
        ],
        &[Align::Left, Align::Right, Align::Left, Align::Right],
        &basic_rows,
    );

    // ── Quantiles / Extreme Observations: deferred with WEIGHT (choice (a)). ──
    session.listing.blank();
    centered(
        session,
        "Quantiles and Extreme Observations are not computed with a WEIGHT variable.",
    );

    // ── Missing Values ──
    if n_missing > 0 {
        session.listing.blank();
        centered(session, "Missing Values");
        session.listing.blank();
        let pct = if n_total > 0 {
            100.0 * n_missing as f64 / n_total as f64
        } else {
            0.0
        };
        session.listing.write_table(
            &[
                "Missing Value".into(),
                "Count".into(),
                "Percent Of All Obs".into(),
            ],
            &[Align::Left, Align::Right, Align::Right],
            &[vec![".".into(), format!("{n_missing}"), fmt_num(pct)]],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::session::Session;
    use crate::source::SourceFile;
    use crate::value::VarType;
    use polars::df;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn parse_univ(src: &str) -> Result<UnivariateAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "univariate"
        parse(&mut ts)
    }

    fn num_meta(name: &str) -> VarMeta {
        VarMeta {
            name: name.to_string(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        }
    }

    // ───────────────────────────── parse tests ─────────────────────────────

    // ─────────────────────────── normality tests ──────────────────────────

    fn sorted_of(xs: &[f64]) -> Vec<f64> {
        let mut s = xs.to_vec();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        s
    }

    fn moments(xs: &[f64]) -> (f64, f64) {
        let n = xs.len() as f64;
        let mean = xs.iter().sum::<f64>() / n;
        let s = sample_std(xs).unwrap();
        (mean, s)
    }

    #[test]
    fn phi_inv_known_quantiles() {
        // Standard probit reference values.
        assert!((phi_inv(0.5)).abs() < 1e-12, "phi_inv(0.5)");
        assert!((phi_inv(0.975) - 1.959963985).abs() < 1e-7, "phi_inv(0.975)");
        assert!((phi_inv(0.95) - 1.644853627).abs() < 1e-7, "phi_inv(0.95)");
        assert!((phi_inv(0.025) + 1.959963985).abs() < 1e-7, "phi_inv(0.025)");
        // Round-trip with probnorm.
        for &p in &[0.01, 0.1, 0.3, 0.6, 0.9, 0.99] {
            let z = phi_inv(p);
            assert!((probnorm(z) - p).abs() < 1e-9, "roundtrip p={p}");
        }
    }

    #[test]
    fn shapiro_wilk_w_near_one_for_normalish() {
        // Symmetric, roughly normal sample → W close to 1, large p (not reject).
        let xs = sorted_of(&[-2.0, -1.0, 0.0, 1.0, 2.0]);
        let (w, p) = shapiro_wilk(&xs);
        let w = w.unwrap();
        assert!(w > 0.9 && w <= 1.0, "W={w}");
        let p = p.unwrap();
        assert!((0.0..=1.0).contains(&p), "p={p}");
        assert!(p > 0.3, "p should be large for ~normal data, got {p}");
    }

    #[test]
    fn shapiro_wilk_low_w_for_outlier() {
        // A strong outlier makes the data non-normal → smaller W, smaller p.
        let normalish = sorted_of(&[-2.0, -1.0, 0.0, 1.0, 2.0]);
        let skewed = sorted_of(&[1.0, 2.0, 3.0, 4.0, 100.0]);
        let (wn, _) = shapiro_wilk(&normalish);
        let (ws, ps) = shapiro_wilk(&skewed);
        assert!(ws.unwrap() < wn.unwrap(), "outlier W should be smaller");
        assert!(ps.unwrap() < 0.2, "p for skewed sample should be small: {:?}", ps);
    }

    #[test]
    fn shapiro_wilk_out_of_range() {
        assert_eq!(shapiro_wilk(&[1.0, 2.0]), (None, None)); // n<3
    }

    #[test]
    fn anderson_darling_known_sample() {
        // Sample {1,2,3,4,5}: mean=3, s=sqrt(2.5)=1.5811388.
        // z = (x-3)/s = {-1.264911,-0.632456,0,0.632456,1.264911}.
        // Compute A² directly from the definition and compare.
        let xs = sorted_of(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let (mean, s) = moments(&xs);
        let z: Vec<f64> = xs.iter().map(|&x| (x - mean) / s).collect();
        let (a2, p) = anderson_darling(&z, 5);
        // Hand-computed value for this z-vector (verified in Python against the
        // exact A² definition) = 0.1435942.
        assert!((a2 - 0.1435942).abs() < 1e-4, "A²={a2}");
        let p = p.unwrap();
        assert!((0.0..=1.0).contains(&p), "p={p}");
        assert!(p > 0.5, "near-normal → large p, got {p}");
    }

    #[test]
    fn cramer_von_mises_known_sample() {
        let xs = sorted_of(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let (mean, s) = moments(&xs);
        let z: Vec<f64> = xs.iter().map(|&x| (x - mean) / s).collect();
        let (w2, p) = cramer_von_mises(&z, 5);
        // Hand-computed W² for this z-vector (verified in Python against the
        // exact W² definition) = 0.0193421.
        assert!((w2 - 0.0193421).abs() < 1e-5, "W²={w2}");
        let p = p.unwrap();
        assert!((0.0..=1.0).contains(&p), "p={p}");
    }

    #[test]
    fn kolmogorov_smirnov_known_sample() {
        let xs = sorted_of(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let (mean, s) = moments(&xs);
        let z: Vec<f64> = xs.iter().map(|&x| (x - mean) / s).collect();
        let (d, p) = kolmogorov_smirnov(&z, 5);
        // D = max over i of |F_n − Φ|. For this symmetric z-vector,
        // Φ(z) = {0.10282,0.26354,0.5,0.73646,0.89718}; the largest gap is at
        // the first point: |0.2 − 0.10282| = 0.09718, vs |0.10282 − 0| etc.
        // Computed reference D ≈ 0.13646.
        assert!((d - 0.13646).abs() < 1e-3, "D={d}");
        assert!(p.unwrap() > 0.1, "near-normal → not significant");
    }

    #[test]
    fn anderson_darling_pvalue_monotone() {
        // Larger A² → smaller p (upper-tail).
        assert!(ad_pvalue(0.3) > ad_pvalue(1.0));
        assert!(ad_pvalue(1.0) > ad_pvalue(3.0));
        assert!((0.0..=1.0).contains(&ad_pvalue(0.1)));
        assert!((0.0..=1.0).contains(&ad_pvalue(5.0)));
    }

    #[test]
    fn cvm_pvalue_monotone() {
        assert!(cvm_pvalue(0.05) > cvm_pvalue(0.2));
        assert!(cvm_pvalue(0.2) > cvm_pvalue(0.8));
    }

    #[test]
    fn normality_block_emitted_only_with_normal() {
        let mut session = make_session();
        let df = df!["x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);
        let ast = UnivariateAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            var: vec!["x".into()],
            by: vec![],
            weight: None,
            output: None,
            normal: true,
            graphics_deferred: 0,
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("Tests for Normality"), "listing: {listing}");
        assert!(listing.contains("Shapiro-Wilk"), "listing: {listing}");
        assert!(listing.contains("Anderson-Darling"), "listing: {listing}");
        assert!(listing.contains("Cramer-von Mises"), "listing: {listing}");
        assert!(listing.contains("Kolmogorov-Smirnov"), "listing: {listing}");
    }

    #[test]
    fn normality_degenerate_note_no_panic() {
        let mut session = make_session();
        // Constant column → zero variance → NOTE, no panic, no table.
        let df = df!["x" => [5.0_f64, 5.0, 5.0, 5.0]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);
        let ast = UnivariateAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            var: vec!["x".into()],
            by: vec![],
            weight: None,
            output: None,
            normal: true,
            graphics_deferred: 0,
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("Tests for Normality"), "listing: {listing}");
        assert!(listing.contains("at least 3 nonmissing"), "listing: {listing}");
    }

    #[test]
    fn parse_normal_option_on_proc() {
        let ast = parse_univ("proc univariate data=a normal; var x; run;").unwrap();
        assert!(ast.normal);
    }

    #[test]
    fn parse_normal_option_on_var() {
        let ast = parse_univ("proc univariate data=a; var x / normal; run;").unwrap();
        assert!(ast.normal);
        assert_eq!(ast.var, vec!["x"]);
    }

    #[test]
    fn parse_graphics_statements_skipped() {
        let ast = parse_univ(
            "proc univariate data=a; var x; histogram x / normal; qqplot x; run;",
        )
        .unwrap();
        assert_eq!(ast.graphics_deferred, 2);
        assert_eq!(ast.var, vec!["x"]);
    }

    #[test]
    fn execute_graphics_emits_deferred_note() {
        let mut session = make_session();
        let df = df!["x" => [1.0_f64, 2.0, 3.0]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);
        let ast = UnivariateAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            var: vec!["x".into()],
            by: vec![],
            weight: None,
            output: None,
            normal: false,
            graphics_deferred: 1,
        };
        execute(&ast, &mut session).unwrap();
        let log = session.log.into_string();
        assert!(
            log.contains("graphical output deferred to ODS GRAPHICS"),
            "log: {log}"
        );
    }

    #[test]
    fn parse_data_and_var() {
        let ast = parse_univ("proc univariate data=work.t; var x; run;").unwrap();
        assert_eq!(ast.data.as_ref().unwrap().name, "t");
        assert_eq!(ast.var, vec!["x"]);
    }

    #[test]
    fn parse_by_statement_captured() {
        let ast =
            parse_univ("proc univariate data=work.t; by g descending h; var x y; run;").unwrap();
        assert_eq!(ast.var, vec!["x", "y"]);
        assert_eq!(
            ast.by,
            vec![("g".to_string(), false), ("h".to_string(), true)]
        );
    }

    #[test]
    fn parse_noprint_and_default_var() {
        let ast = parse_univ("proc univariate data=a noprint; run;").unwrap();
        assert!(ast.var.is_empty());
        assert_eq!(ast.data.as_ref().unwrap().name, "a");
    }

    // ─────────────────────────── quantile def-5 tests ──────────────────────

    fn q(xs: &[f64], p: f64) -> f64 {
        let mut s = xs.to_vec();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        quantile_def5(&s, p).unwrap()
    }

    #[test]
    fn quantile_def5_pinned_odd() {
        // [1,2,3,4,5]
        let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
        // median p=.5: np=2.5, j=2, g=.5 -> x[3]=3
        assert_eq!(q(&xs, 0.5), 3.0);
        // Q1 p=.25: np=1.25, j=1, g=.25 -> x[2]=2
        assert_eq!(q(&xs, 0.25), 2.0);
        // Q3 p=.75: np=3.75, j=3, g=.75 -> x[4]=4
        assert_eq!(q(&xs, 0.75), 4.0);
        // edges
        assert_eq!(q(&xs, 1.0), 5.0);
        assert_eq!(q(&xs, 0.0), 1.0);
    }

    #[test]
    fn quantile_def5_pinned_even_discontinuity() {
        // [1,2,3,4]: median np=2, g=0 -> (x[2]+x[3])/2 = (2+3)/2 = 2.5
        let xs = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(q(&xs, 0.5), 2.5);
    }

    // ───────────────────────── skewness / kurtosis tests ───────────────────

    #[test]
    fn skewness_symmetric_is_zero() {
        let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
        let g1 = skewness(&xs).unwrap();
        assert!(g1.abs() < 1e-12, "g1 = {g1}");
    }

    #[test]
    fn skewness_known_skewed_sample() {
        // [1,2,3,4,10] : computed with the SAS formula.
        // mean=4, s=sqrt(Σ(x-mean)^2/4)=sqrt((9+4+1+0+36)/4)=sqrt(12.5)
        // g1 = 5/((4)(3)) * Σ z^3, z=(x-4)/s.
        let xs = [1.0, 2.0, 3.0, 4.0, 10.0];
        let g1 = skewness(&xs).unwrap();
        // Reference value (SAS g1 formula): ~1.6970563
        assert!((g1 - 1.6970563).abs() < 1e-4, "g1 = {g1}");
    }

    #[test]
    fn skewness_needs_n_ge_3() {
        assert!(skewness(&[1.0, 2.0]).is_none());
        assert!(skewness(&[1.0]).is_none());
    }

    #[test]
    fn kurtosis_needs_n_ge_4() {
        assert!(kurtosis(&[1.0, 2.0, 3.0]).is_none());
        assert!(kurtosis(&[1.0, 2.0]).is_none());
    }

    #[test]
    fn kurtosis_known_sample() {
        // [1,2,3,4,5] excess kurtosis (SAS) reference ~ -1.2
        let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
        let g2 = kurtosis(&xs).unwrap();
        assert!((g2 - (-1.2)).abs() < 1e-6, "g2 = {g2}");
    }

    #[test]
    fn mode_smallest_repeat_or_none() {
        let mut a = [1.0, 1.0, 2.0, 2.0, 3.0]; // 1 and 2 both twice -> smallest = 1
        a.sort_by(|x, y| x.partial_cmp(y).unwrap());
        assert_eq!(mode(&a), Some(1.0));

        let mut b = [1.0, 2.0, 3.0]; // all unique -> no mode
        b.sort_by(|x, y| x.partial_cmp(y).unwrap());
        assert_eq!(mode(&b), None);
    }

    // ───────────────────────────── execute tests ───────────────────────────

    fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
        session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
        session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
    }

    #[test]
    fn execute_report_contains_sections_and_median() {
        let mut session = make_session();
        // [1,2,3,4,5] plus a missing -> median 3, n_missing 1.
        let df = df![
            "x" => [Some(1.0_f64), Some(2.0), Some(3.0), Some(4.0), Some(5.0), None]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = UnivariateAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            var: vec!["x".into()],
            by: vec![],
            weight: None,
            output: None,
            normal: false,
            graphics_deferred: 0,
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(
            listing.contains("The UNIVARIATE Procedure"),
            "listing: {listing}"
        );
        assert!(listing.contains("Variable: x"), "listing: {listing}");
        assert!(listing.contains("Median"), "listing: {listing}");
        // median of [1..5] is 3
        assert!(listing.contains("Median"), "listing: {listing}");
        assert!(listing.contains("Missing Values"), "listing: {listing}");
        // moments header
        assert!(listing.contains("Moments"), "listing: {listing}");
        assert!(listing.contains("Quantiles"), "listing: {listing}");
    }

    #[test]
    fn execute_default_all_numeric_vars() {
        let mut session = make_session();
        let df = df![
            "a" => [1.0_f64, 2.0, 3.0],
            "g" => ["x", "y", "z"],
            "b" => [4.0_f64, 5.0, 6.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![
                num_meta("a"),
                VarMeta {
                    name: "g".into(),
                    ty: VarType::Char,
                    length: 1,
                    format: None,
                    label: None,
                },
                num_meta("b"),
            ],
        };
        write_dataset(&mut session, "T", ds);

        let ast = UnivariateAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            var: vec![],
            by: vec![],
            weight: None,
            output: None,
            normal: false,
            graphics_deferred: 0,
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // Both numeric variables analyzed; char skipped.
        assert!(listing.contains("Variable: a"), "listing: {listing}");
        assert!(listing.contains("Variable: b"), "listing: {listing}");
        assert!(!listing.contains("Variable: g"), "listing: {listing}");
    }

    // ─────────────────────────── BY / OUTPUT tests ─────────────────────────

    fn char_meta(name: &str) -> VarMeta {
        VarMeta {
            name: name.to_string(),
            ty: VarType::Char,
            length: 4,
            format: None,
            label: None,
        }
    }

    fn read_num_col(session: &Session, table: &str, col: &str) -> Vec<Value> {
        let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
        let idx = ds.vars.iter().position(|m| m.name == col).unwrap();
        decode_column(&ds, idx).unwrap()
    }

    #[test]
    fn execute_by_per_group_sections() {
        let mut session = make_session();
        // Sorted by g: a,a,b,b.
        let df = df![
            "g" => ["a", "a", "b", "b"],
            "x" => [1.0_f64, 3.0, 10.0, 20.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = UnivariateAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            var: vec!["x".into()],
            by: vec![("g".into(), false)],
            weight: None,
            output: None,
            normal: false,
            graphics_deferred: 0,
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("The UNIVARIATE Procedure"), "listing: {listing}");
        assert!(listing.contains("g=a"), "listing: {listing}");
        assert!(listing.contains("g=b"), "listing: {listing}");
        // One Variable: x section per group (2 total).
        assert_eq!(listing.matches("Variable: x").count(), 2, "listing: {listing}");
    }

    #[test]
    fn execute_by_unsorted_errors() {
        let mut session = make_session();
        // NOT sorted by g: a,b,a.
        let df = df![
            "g" => ["a", "b", "a"],
            "x" => [1.0_f64, 2.0, 3.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = UnivariateAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            var: vec!["x".into()],
            by: vec![("g".into(), false)],
            weight: None,
            output: None,
            normal: false,
            graphics_deferred: 0,
        };
        let r = execute(&ast, &mut session);
        assert!(r.is_err());
        let msg = r.err().unwrap().to_string();
        assert!(
            msg.contains("not sorted in ascending sequence"),
            "msg: {msg}"
        );
    }

    #[test]
    fn execute_output_no_by() {
        let mut session = make_session();
        // [1,2,3,4,5] -> mean 3, n 5, min 1, max 5, median 3.
        let df = df!["x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = UnivariateAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            var: vec!["x".into()],
            by: vec![],
            weight: None,
            output: Some(UnivariateOutput {
                out: DatasetRef { libref: Some("WORK".into()), name: "O".into() },
                specs: vec![
                    ("mean".into(), vec!["m".into()]),
                    ("n".into(), vec!["cnt".into()]),
                    ("min".into(), vec!["lo".into()]),
                    ("max".into(), vec!["hi".into()]),
                    ("median".into(), vec!["med".into()]),
                ],
            }),
            normal: false,
            graphics_deferred: 0,
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        assert_eq!(out.n_obs(), 1);
        assert_eq!(read_num_col(&session, "O", "m"), vec![Value::Num(3.0)]);
        assert_eq!(read_num_col(&session, "O", "cnt"), vec![Value::Num(5.0)]);
        assert_eq!(read_num_col(&session, "O", "lo"), vec![Value::Num(1.0)]);
        assert_eq!(read_num_col(&session, "O", "hi"), vec![Value::Num(5.0)]);
        assert_eq!(read_num_col(&session, "O", "med"), vec![Value::Num(3.0)]);
        assert_eq!(session.last_dataset.as_deref(), Some("WORK.O"));
    }

    #[test]
    fn execute_output_with_by() {
        let mut session = make_session();
        // Sorted by g: a(1,3) b(10,20).
        let df = df![
            "g" => ["a", "a", "b", "b"],
            "x" => [1.0_f64, 3.0, 10.0, 20.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = UnivariateAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            var: vec!["x".into()],
            by: vec![("g".into(), false)],
            weight: None,
            output: Some(UnivariateOutput {
                out: DatasetRef { libref: Some("WORK".into()), name: "O".into() },
                specs: vec![("mean".into(), vec!["mx".into()])],
            }),
            normal: false,
            graphics_deferred: 0,
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        assert_eq!(out.n_obs(), 2);
        let g = read_num_col(&session, "O", "g"); // char decoded
        let mx = read_num_col(&session, "O", "mx");
        assert_eq!(g[0], Value::Char("a".into()));
        assert_eq!(g[1], Value::Char("b".into()));
        assert_eq!(mx[0], Value::Num(2.0));
        assert_eq!(mx[1], Value::Num(15.0));
        // BY column precedes the statistic column.
        let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["g", "mx"]);
    }

    #[test]
    fn parse_output_statement() {
        let ast = parse_univ(
            "proc univariate data=a noprint; var x; output out=o mean=mx n=nx q1=q1x; run;",
        )
        .unwrap();
        let out = ast.output.as_ref().unwrap();
        assert_eq!(out.out.name, "o");
        assert_eq!(
            out.specs,
            vec![
                ("mean".to_string(), vec!["mx".to_string()]),
                ("n".to_string(), vec!["nx".to_string()]),
                ("q1".to_string(), vec!["q1x".to_string()]),
            ]
        );
    }

    // ───────────────────────────── WEIGHT tests ────────────────────────────

    #[test]
    fn parse_weight_statement() {
        let ast = parse_univ("proc univariate data=a; var x; weight w; run;").unwrap();
        assert_eq!(ast.weight.as_deref(), Some("w"));
        assert_eq!(ast.var, vec!["x"]);
    }

    #[test]
    fn execute_weighted_moments() {
        let mut session = make_session();
        // values [1,2,3] weights [1,2,3] + an excluded row (w<=0).
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0, 99.0],
            "w" => [1.0_f64, 2.0, 3.0, 0.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("w")] };
        write_dataset(&mut session, "T", ds);

        let ast = UnivariateAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            var: vec!["x".into()],
            by: vec![],
            weight: Some("w".into()),
            output: None,
            normal: false,
            graphics_deferred: 0,
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("The UNIVARIATE Procedure"), "listing: {listing}");
        assert!(listing.contains("Variable: x"), "listing: {listing}");
        assert!(listing.contains("Moments"), "listing: {listing}");
        // Weighted: Sum Weights = 6, Sum Observations = 14.
        assert!(listing.contains("Sum Weights"), "listing: {listing}");
        // Quantiles section omitted; replacement note shown instead.
        assert!(
            listing.contains("not computed with a WEIGHT variable"),
            "listing: {listing}"
        );
        assert!(!listing.contains("Quantiles (Definition 5)"), "listing: {listing}");
        // The excluded (w<=0) row counts as a missing value.
        assert!(listing.contains("Missing Values"), "listing: {listing}");
    }

    #[test]
    fn execute_weighted_no_quantiles_section() {
        let mut session = make_session();
        let df = df![
            "x" => [10.0_f64, 20.0, 30.0],
            "w" => [1.0_f64, 1.0, 1.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("w")] };
        write_dataset(&mut session, "T", ds);

        let ast = UnivariateAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            var: vec!["x".into()],
            by: vec![],
            weight: Some("w".into()),
            output: None,
            normal: false,
            graphics_deferred: 0,
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // With equal weights the weighted mean equals the plain mean (20).
        assert!(listing.contains("Mean"), "listing: {listing}");
        // The Quantiles (Definition 5) table is not emitted.
        assert!(!listing.contains("Quantiles (Definition 5)"), "listing: {listing}");
        // The Extreme Observations table header is not emitted (only the
        // replacement note mentions the phrase).
        assert!(!listing.contains("Lowest Value"), "listing: {listing}");
    }
}
