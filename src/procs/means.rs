//! PROC MEANS / SUMMARY (jalon M5).
//!
//! # Plan du fichier — voir PLAN.md  (difficulté : ÉLEVÉE — _TYPE_)
//!
//! `proc means data=a [noprint] [stats...] ; class c1 c2 ; var v1 v2 ;
//! output out=b [stat(var)=name...] ; run ;`  — SUMMARY = MEANS noprint
//! par défaut.
//!
//! ## Sémantique à répliquer
//! - Stats défaut du rapport : N, Mean, Std Dev, Minimum, Maximum.
//!   Stats demandables : n nmiss mean std min max sum range stderr cv
//!   median.  SAS EXCLUT les missings : chaque stat est calculée sur les
//!   valeurs numériques NON missing du groupe (helper `compute`).
//! - CLASS sans OUTPUT : rapport par combinaison de classes.
//! - OUTPUT OUT= avec CLASS : produit TOUTES les combinaisons de
//!   sous-ensembles de classes — `_TYPE_` = masque binaire (bit le plus
//!   à droite = dernière variable CLASS), `_FREQ_` = effectif. Ordre :
//!   _TYPE_ croissant puis valeurs de classes. Lignes des classes non
//!   actives → missing.
//! - VAR absent : toutes les numériques hors CLASS/BY.
//! - Rapport listing : table par variable x stat, en-tête style SAS
//!   ("The MEANS Procedure").
//!
//! ## Choix de rendu (documenté pour l'orchestrateur)
//! - Titre centré "The MEANS Procedure" via `page_header()` puis une ligne
//!   centrée.
//! - Sans CLASS : une table, colonne `Variable` puis une colonne par stat
//!   demandée (défaut : N, Mean, Std Dev, Minimum, Maximum). Une ligne par
//!   variable analysée.
//! - Avec CLASS : une table COMBINÉE — colonnes de tête = chaque variable
//!   CLASS, puis colonne `Variable`, puis une colonne par stat. Une ligne
//!   par (combinaison de classes × variable). Les combinaisons de classes
//!   sont ordonnées par `sas_cmp`.
//!
//! ## WEIGHT statement (jalon WEIGHT)
//! `weight <var>;` — une seule variable numérique. Quand elle est présente,
//! toutes les stats passent par `compute_weighted` (analogue pondéré de
//! `compute`). Le chemin non-pondéré reste BYTE-IDENTIQUE : `compute_weighted`
//! n'est appelé que si `ast.weight.is_some()`. Fonctionne avec CLASS et BY
//! (poids partitionnés par groupe), et OUTPUT OUT= utilise les stats pondérées.
//!
//! Formules pondérées (VARDEF=DF) — n = nb d'obs utilisables, w_i poids, x_i :
//!   SumWgt = Σw_i ; Sum = Σw_i x_i ; Mean = Σw_i x_i / Σw_i ;
//!   CSS_w = Σw_i(x_i−x̄_w)² ; Variance = CSS_w/(n−1) ; Std = √Variance ;
//!   StdErr = Std/√(Σw_i) (SAS pondère l'erreur-type par √ΣW) ;
//!   CV = 100·Std/x̄_w ; Min/Max = min/max NON pondérés de x_i ; N = n ;
//!   NMiss = nb d'obs exclues (valeur missing, poids missing, ou poids ≤ 0).
//! Exclusions : voir `common::partition_weighted`.
//!
//! ## Simplifications SAS documentées (WEIGHT)
//! - MEDIAN avec WEIGHT : la vraie médiane pondérée de SAS est complexe ;
//!   DIFFÉRÉ. Ici MEDIAN est calculée NON pondérée (médiane simple des x_i
//!   utilisables) — divergence assumée et documentée.

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{
    by_groups, decode_column, partition_numeric, partition_weighted, resolve_by_cols, sample_std,
    t_quantile,
};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{format_best, Value, VarType};
use polars::prelude::*;
use std::cmp::Ordering;

pub struct MeansAst {
    pub data: Option<DatasetRef>,
    pub summary: bool,
    pub noprint: bool,
    pub stats: Vec<String>,
    pub class: Vec<String>,
    pub var: Vec<String>,
    /// BY variables (var, descending). Outer grouping; input must be sorted.
    pub by: Vec<(String, bool)>,
    /// WEIGHT variable (single numeric var). When `Some`, all statistics are
    /// computed through the weighted code path (see `compute_weighted`).
    pub weight: Option<String>,
    /// Confidence level alpha for CLM/LCLM/UCLM (SAS default 0.05). Only the
    /// CI statistics consult it; it never affects the default output.
    pub alpha: f64,
    pub output: Option<MeansOutput>,
}

pub struct MeansOutput {
    pub out: DatasetRef,
    /// (stat, var source, nom de sortie)
    pub specs: Vec<(String, String, String)>,
}

/// Recognized statistic keywords accepted on the PROC MEANS statement.
const STAT_KEYWORDS: &[&str] = &[
    "n", "nmiss", "mean", "std", "stddev", "min", "max", "sum", "range", "stderr", "cv", "median",
    "clm", "lclm", "uclm",
];

fn is_stat_keyword(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    STAT_KEYWORDS.iter().any(|k| *k == l)
}

/// Parse `proc means [data=a] [noprint] [stat...] ; [class ...;] [var ...;]
/// [output out=b stat(var)=name...;] ... run;`. Called AFTER "proc
/// means"/"proc summary" has been consumed. Consumes through `run;`/`quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<MeansAst> {
    let mut data: Option<DatasetRef> = None;
    let mut noprint = false;
    let mut stats: Vec<String> = Vec::new();
    // SAS default confidence level. Stays 0.05 unless ALPHA= is given; only
    // the CI statistics read it, so the default path is unaffected.
    let mut alpha: f64 = 0.05;

    // --- PROC MEANS statement options, until `;` ---
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
            ts.next();
            noprint = true;
        } else if ts.peek().is_kw("print") {
            // explicit PRINT — undo a noprint default (e.g. PROC SUMMARY).
            ts.next();
            noprint = false;
        } else if ts.peek().is_kw("alpha") {
            ts.next();
            expect_eq(ts, "ALPHA")?;
            let tok = ts.peek().clone();
            let val = match tok.kind {
                TokenKind::Num(f) => f,
                _ => {
                    return Err(SasError::parse(
                        "expected a number after ALPHA=",
                        tok.span,
                    ));
                }
            };
            ts.next();
            if !(val > 0.0 && val < 1.0) {
                return Err(SasError::runtime(format!(
                    "The ALPHA= value {val} must be between 0 and 1."
                )));
            }
            alpha = val;
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            if is_stat_keyword(&name) {
                ts.next();
                stats.push(name.to_ascii_lowercase());
            } else {
                let span = ts.peek().span;
                return Err(SasError::parse(
                    format!(
                        "Unexpected option '{}' on PROC MEANS statement.",
                        name.to_uppercase()
                    ),
                    span,
                ));
            }
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC MEANS statement.",
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; ---
    let mut class: Vec<String> = Vec::new();
    let mut var: Vec<String> = Vec::new();
    let mut by: Vec<(String, bool)> = Vec::new();
    let mut weight: Option<String> = None;
    let mut output: Option<MeansOutput> = None;

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    crate::procs::common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "class" => {
                ts.next();
                class = crate::procs::common::parse_class(ts)?;
                true
            }
            "var" => {
                ts.next();
                var = crate::procs::common::parse_var_list(ts)?;
                true
            }
            "by" => {
                ts.next();
                by = parse_by_list(ts)?;
                true
            }
            "weight" => {
                ts.next();
                weight = Some(crate::procs::common::parse_weight(ts)?);
                true
            }
            "output" => {
                ts.next();
                output = Some(parse_output(ts)?);
                true
            }
            _ => false,
        })
    })?;

    Ok(MeansAst {
        data,
        summary: false,
        noprint,
        stats,
        class,
        var,
        by,
        weight,
        alpha,
        output,
    })
}

/// Parse a single-variable statement body (after the keyword was consumed),
/// e.g. `weight <var> ;`. Errors if no variable or extra tokens before `;`.
// `parse_single_var` et `parse_by_list` ont été déplacés vers `procs::common`
// (M31.2). Ré-export `pub(crate)` pour les appelants existants
// (`means.rs` lui-même, `univariate.rs`, `rank.rs` via `means::parse_by_list`).
pub(crate) use crate::procs::common::{parse_by as parse_by_list, parse_single_var};

/// Parse the OUTPUT statement body (after "output" was consumed), through
/// its terminating `;`. `output out=lib.t [stat(var)=name ...] ;`
fn parse_output(ts: &mut StatementStream) -> Result<MeansOutput> {
    let mut out: Option<DatasetRef> = None;
    let mut specs: Vec<(String, String, String)> = Vec::new();

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
        } else if let Some(stat) = ts.peek().ident().map(str::to_string) {
            // Expect `stat(var)=name`.
            ts.next(); // stat
            if ts.peek().kind != TokenKind::LParen {
                return Err(SasError::parse(
                    format!("expected '(' after statistic '{}' in OUTPUT", stat),
                    ts.peek().span,
                ));
            }
            ts.next(); // '('
            let var = match ts.peek().ident().map(str::to_string) {
                Some(v) => {
                    ts.next();
                    v
                }
                None => {
                    return Err(SasError::parse(
                        "expected a variable name inside OUTPUT statistic spec",
                        ts.peek().span,
                    ));
                }
            };
            if ts.peek().kind != TokenKind::RParen {
                return Err(SasError::parse(
                    "expected ')' in OUTPUT statistic spec",
                    ts.peek().span,
                ));
            }
            ts.next(); // ')'
            expect_eq(ts, "OUTPUT statistic")?;
            let name = match ts.peek().ident().map(str::to_string) {
                Some(n) => {
                    ts.next();
                    n
                }
                None => {
                    return Err(SasError::parse(
                        "expected an output variable name in OUTPUT statistic spec",
                        ts.peek().span,
                    ));
                }
            };
            specs.push((stat.to_ascii_lowercase(), var, name));
        } else {
            return Err(SasError::parse(
                "unexpected token in OUTPUT statement",
                ts.peek().span,
            ));
        }
    }

    let out = out.ok_or_else(|| {
        SasError::runtime("The OUTPUT statement requires the OUT= option in PROC MEANS.")
    })?;
    Ok(MeansOutput { out, specs })
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

/// Compute one statistic over the NON-MISSING numeric values `xs` of a
/// group. `n`/`nmiss` are passed the group's non-missing/missing counts
/// separately because they depend on the missing tally, not on `xs`.
/// Returns a `Value` (`Value::missing()` when undefined for the group).
pub fn compute(stat: &str, xs: &[f64], n_missing: usize, alpha: f64) -> Value {
    let n = xs.len();
    // Confidence limits for the mean (CLM/LCLM/UCLM). Require n>=2 (need a
    // valid std error). half-width h = t_{1-alpha/2, n-1} * stderr.
    if matches!(stat, "lclm" | "uclm" | "clm") {
        let mean = if n == 0 {
            return Value::missing();
        } else {
            xs.iter().sum::<f64>() / n as f64
        };
        let stderr = match sample_std(xs) {
            Some(s) if n >= 2 => s / (n as f64).sqrt(),
            _ => return Value::missing(),
        };
        return clm_value(stat, mean, stderr, n, alpha);
    }
    match stat {
        "n" => Value::Num(n as f64),
        "nmiss" => Value::Num(n_missing as f64),
        "min" => {
            if n == 0 {
                Value::missing()
            } else {
                Value::Num(xs.iter().cloned().fold(f64::INFINITY, f64::min))
            }
        }
        "max" => {
            if n == 0 {
                Value::missing()
            } else {
                Value::Num(xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max))
            }
        }
        "range" => {
            if n == 0 {
                Value::missing()
            } else {
                let mn = xs.iter().cloned().fold(f64::INFINITY, f64::min);
                let mx = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                Value::Num(mx - mn)
            }
        }
        "sum" => Value::Num(xs.iter().sum()),
        "mean" => {
            if n == 0 {
                Value::missing()
            } else {
                Value::Num(xs.iter().sum::<f64>() / n as f64)
            }
        }
        "std" | "stddev" => match sample_std(xs) {
            Some(s) => Value::Num(s),
            None => Value::missing(),
        },
        "stderr" => match sample_std(xs) {
            Some(s) if n >= 1 => Value::Num(s / (n as f64).sqrt()),
            _ => Value::missing(),
        },
        "cv" => {
            let mean = if n == 0 {
                return Value::missing();
            } else {
                xs.iter().sum::<f64>() / n as f64
            };
            match sample_std(xs) {
                Some(s) if mean != 0.0 => Value::Num(100.0 * s / mean),
                _ => Value::missing(),
            }
        }
        "median" => match median(xs) {
            Some(m) => Value::Num(m),
            None => Value::missing(),
        },
        _ => Value::missing(),
    }
}

/// Weighted analogue of `compute`. `pairs` holds the usable (value, weight)
/// pairs of a group (from `common::partition_weighted`); `n_excluded` is the
/// count of observations dropped by the WEIGHT exclusion rules. VARDEF=DF.
///
/// See the file header for the formulas. MEDIAN is computed UNWEIGHTED here
/// (weighted median deferred — documented divergence).
pub fn compute_weighted(stat: &str, pairs: &[(f64, f64)], n_excluded: usize, alpha: f64) -> Value {
    let n = pairs.len();
    let sum_w: f64 = pairs.iter().map(|(_, w)| *w).sum();
    let sum_wx: f64 = pairs.iter().map(|(x, w)| w * x).sum();
    let mean_w = if sum_w != 0.0 {
        Some(sum_wx / sum_w)
    } else {
        None
    };
    // Weighted corrected sum of squares: Σ w_i (x_i − x̄_w)^2.
    let css_w = match mean_w {
        Some(m) => pairs.iter().map(|(x, w)| w * (x - m) * (x - m)).sum::<f64>(),
        None => 0.0,
    };
    // Variance = CSS_w / (n − 1) using the COUNT of usable obs.
    let variance = if n >= 2 {
        Some(css_w / (n as f64 - 1.0))
    } else {
        None
    };
    let std = variance.map(|v| v.sqrt());

    // Weighted confidence limits for the mean. Reuse the SAME weighted std
    // error MEANS displays (Std/sqrt(Σw)) so the CI is consistent with the
    // reported StdErr; df = n-1 over the usable-obs COUNT. Documented choice.
    if matches!(stat, "lclm" | "uclm" | "clm") {
        match (mean_w, std) {
            (Some(m), Some(s)) if n >= 2 && sum_w > 0.0 => {
                let stderr = s / sum_w.sqrt();
                return clm_value(stat, m, stderr, n, alpha);
            }
            _ => return Value::missing(),
        }
    }

    match stat {
        "n" => Value::Num(n as f64),
        "nmiss" => Value::Num(n_excluded as f64),
        "min" => {
            if n == 0 {
                Value::missing()
            } else {
                Value::Num(pairs.iter().map(|(x, _)| *x).fold(f64::INFINITY, f64::min))
            }
        }
        "max" => {
            if n == 0 {
                Value::missing()
            } else {
                Value::Num(
                    pairs
                        .iter()
                        .map(|(x, _)| *x)
                        .fold(f64::NEG_INFINITY, f64::max),
                )
            }
        }
        "range" => {
            if n == 0 {
                Value::missing()
            } else {
                let mn = pairs.iter().map(|(x, _)| *x).fold(f64::INFINITY, f64::min);
                let mx = pairs
                    .iter()
                    .map(|(x, _)| *x)
                    .fold(f64::NEG_INFINITY, f64::max);
                Value::Num(mx - mn)
            }
        }
        // Weighted SUM = Σ w_i x_i (matches SAS PROC MEANS with WEIGHT).
        "sum" => Value::Num(sum_wx),
        "mean" => match mean_w {
            Some(m) => Value::Num(m),
            None => Value::missing(),
        },
        "std" | "stddev" => match std {
            Some(s) => Value::Num(s),
            None => Value::missing(),
        },
        // SAS weighted std error divides Std by sqrt(Σ w_i).
        "stderr" => match std {
            Some(s) if sum_w > 0.0 => Value::Num(s / sum_w.sqrt()),
            _ => Value::missing(),
        },
        "cv" => match (mean_w, std) {
            (Some(m), Some(s)) if m != 0.0 => Value::Num(100.0 * s / m),
            _ => Value::missing(),
        },
        // Weighted median deferred → unweighted median of the usable values.
        "median" => {
            let xs: Vec<f64> = pairs.iter().map(|(x, _)| *x).collect();
            match median(&xs) {
                Some(m) => Value::Num(m),
                None => Value::missing(),
            }
        }
        _ => Value::missing(),
    }
}

/// Confidence-limit half-width h = t_{1-alpha/2, n-1} * stderr, and the
/// requested single bound. `clm` has no single-value meaning (it is a pair of
/// columns in the listing) → missing here; only `lclm`/`uclm` resolve.
fn clm_value(stat: &str, mean: f64, stderr: f64, n: usize, alpha: f64) -> Value {
    let h = clm_halfwidth(stderr, n, alpha);
    match stat {
        "lclm" => Value::Num(mean - h),
        "uclm" => Value::Num(mean + h),
        _ => Value::missing(),
    }
}

/// Half-width of the confidence interval for the mean: t_{1-alpha/2, n-1} *
/// stderr. Requires n>=2 (caller guarantees a finite stderr).
fn clm_halfwidth(stderr: f64, n: usize, alpha: f64) -> f64 {
    let df = (n - 1) as f64;
    let t = t_quantile(1.0 - alpha / 2.0, df);
    t * stderr
}

/// Median of the non-missing values (None when empty).
fn median(xs: &[f64]) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    let mut v: Vec<f64> = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        Some(v[n / 2])
    } else {
        Some((v[n / 2 - 1] + v[n / 2]) / 2.0)
    }
}

/// Header text used both as report header and a stat label in the listing.
pub fn stat_header(stat: &str) -> &'static str {
    match stat {
        "n" => "N",
        "nmiss" => "NMiss",
        "mean" => "Mean",
        "std" | "stddev" => "Std Dev",
        "min" => "Minimum",
        "max" => "Maximum",
        "sum" => "Sum",
        "range" => "Range",
        "stderr" => "Std Error",
        "cv" => "CV",
        "median" => "Median",
        // CI stats: alpha-dependent labels are produced by
        // `stat_report_headers`; these are generic fallbacks.
        "lclm" => "Lower CL for Mean",
        "uclm" => "Upper CL for Mean",
        "clm" => "CL for Mean",
        _ => "Stat",
    }
}

/// Render a single computed stat value into a listing cell.
fn fmt_stat_cell(stat: &str, v: &Value) -> String {
    match v {
        Value::Num(f) => {
            if stat == "n" || stat == "nmiss" {
                format!("{}", *f as i64)
            } else {
                format_best(*f, 12)
            }
        }
        Value::Missing(k) => k.display(),
        Value::Char(s) => s.clone(),
    }
}

/// Execute PROC MEANS / SUMMARY. Called by `procs::execute_proc`.
pub fn execute(ast: &MeansAst, session: &mut Session) -> Result<()> {
    let in_ref = crate::procs::common::resolve_last_dataset(&ast.data, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }

    let n_obs = ds.n_obs();

    // Resolve CLASS column indices (validate existence).
    let mut class_cols: Vec<usize> = Vec::with_capacity(ast.class.len());
    for cname in &ast.class {
        match ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(cname))
        {
            Some(i) => class_cols.push(i),
            None => {
                return Err(SasError::runtime(format!(
                    "Variable {} not found.",
                    cname.to_uppercase()
                )));
            }
        }
    }

    // Determine the VAR list: explicit `var`, else all NUMERIC variables not
    // in CLASS.
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
            .filter(|&i| ds.vars[i].ty == VarType::Num && !class_cols.contains(&i))
            .collect()
    };

    // Decode CLASS columns and VAR columns once each.
    let class_values: Vec<Vec<Value>> = class_cols
        .iter()
        .map(|&ci| decode_column(&ds, ci))
        .collect::<Result<_>>()?;
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

    // Default report stats when none requested.
    let report_stats: Vec<String> = if ast.stats.is_empty() {
        vec![
            "n".into(),
            "mean".into(),
            "std".into(),
            "min".into(),
            "max".into(),
        ]
    } else {
        ast.stats.clone()
    };

    // --- BY processing: resolve, verify sortedness, partition into groups. ---
    // No BY → a single group spanning all rows (output byte-identical).
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
        let in_display = format!("{in_libref}.{in_table}");
        by_groups(&by_values, &descending, n_obs, &by_names, &in_display)?
    };
    let by_names: Vec<String> = by_cols.iter().map(|c| c.name.clone()).collect();

    // --- Report ---
    if !ast.noprint {
        // Title printed once per proc invocation.
        session.listing.page_header();
        let title = "The MEANS Procedure";
        let ls = session.listing.ls();
        let pad = ls.saturating_sub(title.len()) / 2;
        session
            .listing
            .write_line(&format!("{}{}", " ".repeat(pad), title));
        session.listing.blank();

        for (by_key, grp_rows) in &by_groups_list {
            if !by_names.is_empty() {
                emit_by_heading(session, &by_names, by_key);
            }
            emit_report_group(
                session,
                &ds,
                &class_cols,
                &class_values,
                &var_cols,
                &var_values,
                weight_values.as_deref(),
                &report_stats,
                ast.alpha,
                grp_rows,
            );
        }
    }

    // --- OUTPUT OUT= ---
    if let Some(out) = &ast.output {
        write_output(
            session,
            &ds,
            &class_cols,
            &class_values,
            &var_values,
            &var_cols,
            weight_values.as_deref(),
            out,
            &by_cols,
            &by_groups_list,
            ast.alpha,
        )?;
    }

    // --- ODS OUTPUT Summary= (M22.3) ---
    // Capture la table ODS "Summary" comme dataset si `ODS OUTPUT Summary=...`
    // est actif. Inactif par défaut (registre vide) → aucun effet, listing
    // byte-identique. La table Summary = une ligne par variable de VAR, avec
    // colonnes Variable + une par statistique du rapport.
    if let Some(target) = session.ods_output_target("Summary") {
        write_ods_summary(
            session,
            &ds,
            &var_cols,
            &var_values,
            weight_values.as_deref(),
            &report_stats,
            ast.alpha,
            &target,
        )?;
    }

    Ok(())
}

/// M22.3 — écrit la table ODS "Summary" de PROC MEANS comme dataset SAS.
///
/// Structure (périmètre v1) : une observation par variable analysée (VAR),
/// colonne caractère `Variable` (nom de la variable) puis une colonne numérique
/// par statistique du rapport (N, Mean, StdDev, Min, Max par défaut). Les stats
/// sont calculées sur l'ensemble des lignes (pas de partition CLASS/BY en v1 :
/// si CLASS/BY sont présents, on agrège globalement et une NOTE le documente).
#[allow(clippy::too_many_arguments)]
fn write_ods_summary(
    session: &mut Session,
    ds: &SasDataset,
    var_cols: &[usize],
    var_values: &[Vec<Value>],
    weight_values: Option<&[Value]>,
    report_stats: &[String],
    alpha: f64,
    target: &DatasetRef,
) -> Result<()> {
    let n_obs = ds.n_obs();
    let all_rows: Vec<usize> = (0..n_obs).collect();

    // Colonne caractère "Variable" : un nom de variable par ligne.
    let var_names: Vec<Option<String>> = var_cols
        .iter()
        .map(|&ci| Some(ds.vars[ci].name.clone()))
        .collect();
    let name_len = var_cols
        .iter()
        .map(|&ci| ds.vars[ci].name.len())
        .max()
        .unwrap_or(8)
        .max(8);

    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();

    columns.push(Series::new("Variable".into(), var_names).into());
    vars.push(VarMeta {
        name: "Variable".to_string(),
        ty: VarType::Char,
        length: name_len,
        format: None,
        label: None,
    });

    // Une colonne numérique par statistique demandée.
    for stat in report_stats {
        let colname = ods_summary_stat_colname(stat);
        let vals: Vec<Option<f64>> = (0..var_cols.len())
            .map(|vi| {
                let v = match weight_values {
                    Some(wv) => {
                        let (pairs, nmiss) = partition_weighted(&var_values[vi], wv, &all_rows);
                        compute_weighted(stat, &pairs, nmiss, alpha)
                    }
                    None => {
                        let (xs, nmiss) = partition_numeric(&var_values[vi], &all_rows);
                        compute(stat, &xs, nmiss, alpha)
                    }
                };
                value_to_num(&v)
            })
            .collect();
        columns.push(Series::new(colname.as_str().into(), vals).into());
        vars.push(num_var_meta(&colname));
    }

    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars };

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

/// Nom de colonne du dataset Summary pour une statistique du rapport.
/// (StdDev pour `std`/`stddev` ; libellé capitalisé pour les autres.)
fn ods_summary_stat_colname(stat: &str) -> String {
    match stat.to_ascii_lowercase().as_str() {
        "n" => "N".to_string(),
        "nmiss" => "NMiss".to_string(),
        "mean" => "Mean".to_string(),
        "std" | "stddev" => "StdDev".to_string(),
        "min" => "Min".to_string(),
        "max" => "Max".to_string(),
        "sum" => "Sum".to_string(),
        "range" => "Range".to_string(),
        "stderr" => "StdErr".to_string(),
        "cv" => "CV".to_string(),
        "median" => "Median".to_string(),
        "clm" => "CLM".to_string(),
        "lclm" => "LowerCLMean".to_string(),
        "uclm" => "UpperCLMean".to_string(),
        other => {
            let mut c = other.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        }
    }
}

/// Emit the SAS BY heading line into the listing: `var1=val1 var2=val2`.
fn emit_by_heading(session: &mut Session, by_names: &[String], by_key: &[Value]) {
    let parts: Vec<String> = by_names
        .iter()
        .zip(by_key)
        .map(|(name, v)| format!("{}={}", name, class_cell(v)))
        .collect();
    session.listing.write_line(&parts.join(" "));
    session.listing.blank();
}

/// Emit one MEANS report table for the rows in `group_rows` (the full row set
/// when no BY is active). Does NOT emit the procedure title (caller does that
/// once). CLASS grouping is applied within `group_rows` only.
#[allow(clippy::too_many_arguments)]
fn emit_report_group(
    session: &mut Session,
    ds: &SasDataset,
    class_cols: &[usize],
    class_values: &[Vec<Value>],
    var_cols: &[usize],
    var_values: &[Vec<Value>],
    weight_values: Option<&[Value]>,
    report_stats: &[String],
    alpha: f64,
    group_rows: &[usize],
) {
    let mut headers: Vec<String> = Vec::new();
    let mut aligns: Vec<Align> = Vec::new();

    // Leading CLASS columns (only when CLASS present).
    for &ci in class_cols {
        headers.push(ds.vars[ci].name.clone());
        aligns.push(match ds.vars[ci].ty {
            VarType::Num => Align::Right,
            VarType::Char => Align::Left,
        });
    }
    headers.push("Variable".to_string());
    aligns.push(Align::Left);
    // CLM expands to two columns; LCLM/UCLM to one CL column each; all others
    // to a single column. Header text reflects the confidence level.
    for s in report_stats {
        for h in stat_report_headers(s, alpha) {
            headers.push(h);
            aligns.push(Align::Right);
        }
    }

    // Append the per-stat cells for one analysis variable to `row`, choosing
    // the weighted or unweighted path. CLM yields two cells (lower, upper).
    let push_cells = |row: &mut Vec<String>, vi: usize, grp_rows: &[usize]| match weight_values {
        Some(wv) => {
            let (pairs, nmiss) = partition_weighted(&var_values[vi], wv, grp_rows);
            for s in report_stats {
                for cell in stat_report_cells(s, &|st| compute_weighted(st, &pairs, nmiss, alpha)) {
                    row.push(cell);
                }
            }
        }
        None => {
            let (xs, nmiss) = partition_numeric(&var_values[vi], grp_rows);
            for s in report_stats {
                for cell in stat_report_cells(s, &|st| compute(st, &xs, nmiss, alpha)) {
                    row.push(cell);
                }
            }
        }
    };

    let mut rows: Vec<Vec<String>> = Vec::new();

    if class_cols.is_empty() {
        // One section over the group's rows: one row per analysis variable.
        for (vi, vname_idx) in var_cols.iter().enumerate() {
            let mut row = vec![ds.vars[*vname_idx].name.clone()];
            push_cells(&mut row, vi, group_rows);
            rows.push(row);
        }
    } else {
        // CLASS grouping restricted to this BY group's rows.
        let cv_refs: Vec<&Vec<Value>> = class_values.iter().collect();
        let groups = group_by_keys_subset(&cv_refs, group_rows);
        for (key, grp_rows) in &groups {
            for (vi, vname_idx) in var_cols.iter().enumerate() {
                let mut row: Vec<String> = Vec::new();
                for kv in key {
                    row.push(class_cell(kv));
                }
                row.push(ds.vars[*vname_idx].name.clone());
                push_cells(&mut row, vi, grp_rows);
                rows.push(row);
            }
        }
    }

    session.listing.write_table(&headers, &aligns, &rows);
}

/// Report column header(s) for a stat. Most stats map to one header; the
/// confidence-limit stats produce alpha-dependent labels and CLM produces two.
fn stat_report_headers(stat: &str, alpha: f64) -> Vec<String> {
    let pct = cl_percent_label(alpha);
    match stat {
        "lclm" => vec![format!("Lower {pct}% CL for Mean")],
        "uclm" => vec![format!("Upper {pct}% CL for Mean")],
        "clm" => vec![
            format!("Lower {pct}% CL for Mean"),
            format!("Upper {pct}% CL for Mean"),
        ],
        _ => vec![stat_header(stat).to_string()],
    }
}

/// Report cell(s) for a stat, computing values via `f` (the unweighted or
/// weighted `compute*` closure). CLM emits two cells (LCLM then UCLM).
fn stat_report_cells(stat: &str, f: &dyn Fn(&str) -> Value) -> Vec<String> {
    match stat {
        "clm" => vec![
            fmt_stat_cell("lclm", &f("lclm")),
            fmt_stat_cell("uclm", &f("uclm")),
        ],
        _ => vec![fmt_stat_cell(stat, &f(stat))],
    }
}

/// Format the confidence percentage for a CL header from alpha, e.g.
/// 0.05 → "95", 0.10 → "90", 0.01 → "99". Whole percents print without a
/// decimal; otherwise the trailing zeros are trimmed (matches SAS labels).
fn cl_percent_label(alpha: f64) -> String {
    let pct = 100.0 * (1.0 - alpha);
    // Round to a sensible precision to avoid FP noise like 94.99999999.
    let rounded = (pct * 1e6).round() / 1e6;
    if (rounded - rounded.round()).abs() < 1e-9 {
        format!("{}", rounded.round() as i64)
    } else {
        format!("{rounded}")
    }
}

/// Like `group_by_keys`, but only considers `rows` (a subset of all rows),
/// grouping by the class-value tuple in `sas_cmp` order. Used so CLASS
/// grouping happens *within* a BY group.
fn group_by_keys_subset(
    class_values: &[&Vec<Value>],
    rows: &[usize],
) -> Vec<(Vec<Value>, Vec<usize>)> {
    let mut groups: Vec<(Vec<Value>, Vec<usize>)> = Vec::new();
    for &row in rows {
        let key: Vec<Value> = class_values.iter().map(|c| c[row].clone()).collect();
        let pos = groups.iter().position(|(k, _)| {
            k.len() == key.len()
                && k.iter()
                    .zip(&key)
                    .all(|(a, b)| a.sas_cmp(b) == Ordering::Equal)
        });
        match pos {
            Some(p) => groups[p].1.push(row),
            None => groups.push((key, vec![row])),
        }
    }
    groups.sort_by(|(a, _), (b, _)| {
        for (x, y) in a.iter().zip(b) {
            let c = x.sas_cmp(y);
            if c != Ordering::Equal {
                return c;
            }
        }
        Ordering::Equal
    });
    groups
}

/// Render a class-value cell in the listing.
fn class_cell(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Missing(k) => k.display(),
        Value::Char(s) => s.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn write_output(
    session: &mut Session,
    ds: &SasDataset,
    class_cols: &[usize],
    class_values: &[Vec<Value>],
    var_values: &[Vec<Value>],
    var_cols: &[usize],
    weight_values: Option<&[Value]>,
    out: &MeansOutput,
    by_cols: &[crate::procs::common::ByCol],
    by_groups_list: &[(Vec<Value>, Vec<usize>)],
    alpha: f64,
) -> Result<()> {
    let k = class_cols.len();

    // Resolve each output spec's source VAR to an index into var_cols /
    // var_values (the source column must be a VAR — decode it on demand if
    // not already in the VAR list).
    // Build a name->decoded-column map for the spec sources.
    struct Spec {
        stat: String,
        outname: String,
        col: Vec<Value>,
    }
    let mut specs: Vec<Spec> = Vec::with_capacity(out.specs.len());
    for (stat, srcvar, outname) in &out.specs {
        // Find the source column in the dataset.
        let col_idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(srcvar))
            .ok_or_else(|| {
                SasError::runtime(format!("Variable {} not found.", srcvar.to_uppercase()))
            })?;
        // Reuse already-decoded VAR column if available, else decode.
        let col = match var_cols.iter().position(|&c| c == col_idx) {
            Some(p) => var_values[p].clone(),
            None => decode_column(ds, col_idx)?,
        };
        specs.push(Spec {
            stat: stat.clone(),
            outname: outname.clone(),
            col,
        });
    }

    // Output rows accumulate as: (BY-group index, type,
    // per-class-cell-values, sort key, freq, stat-values).
    struct OutRow {
        by_idx: usize,
        ty: f64,
        // class cell value per class var (active = group key value;
        // inactive = missing of right type).
        class_cells: Vec<Value>,
        // sort key: only the active classes' values (in class order).
        sort_key: Vec<Value>,
        freq: f64,
        stats: Vec<Value>,
    }
    let mut out_rows: Vec<OutRow> = Vec::new();

    let class_refs: Vec<&Vec<Value>> = class_values.iter().collect();

    // One block of CLASS-subset rows per BY group (one group overall if no BY),
    // restricting the analysis to that BY group's rows.
    for (by_idx, (_by_key, by_rows)) in by_groups_list.iter().enumerate() {
        // Enumerate all 2^k CLASS subsets within this BY group.
        for mask in 0u32..(1u32 << k) {
            let active: Vec<usize> = (0..k).filter(|&i| (mask >> i) & 1 == 1).collect();

            // _TYPE_ : LSB corresponds to the LAST class variable.
            let mut ty: u64 = 0;
            for &i in &active {
                ty |= 1u64 << (k - 1 - i);
            }

            // Group this BY group's rows by the active class variables.
            let active_refs: Vec<&Vec<Value>> = active.iter().map(|&i| class_refs[i]).collect();
            let groups = group_by_keys_subset(&active_refs, by_rows);

            for (active_key, grp_rows) in &groups {
                let mut class_cells: Vec<Value> = Vec::with_capacity(k);
                let mut ai = 0usize;
                for (i, &col_idx) in class_cols.iter().enumerate() {
                    if active.contains(&i) {
                        class_cells.push(active_key[ai].clone());
                        ai += 1;
                    } else {
                        match ds.vars[col_idx].ty {
                            VarType::Num => class_cells.push(Value::missing()),
                            VarType::Char => class_cells.push(Value::Char(String::new())),
                        }
                    }
                }

                let freq = grp_rows.len() as f64;

                let mut stat_vals: Vec<Value> = Vec::with_capacity(specs.len());
                for sp in &specs {
                    match weight_values {
                        Some(wv) => {
                            let (pairs, nmiss) = partition_weighted(&sp.col, wv, grp_rows);
                            stat_vals.push(compute_weighted(&sp.stat, &pairs, nmiss, alpha));
                        }
                        None => {
                            let (xs, nmiss) = partition_numeric(&sp.col, grp_rows);
                            stat_vals.push(compute(&sp.stat, &xs, nmiss, alpha));
                        }
                    }
                }

                out_rows.push(OutRow {
                    by_idx,
                    ty: ty as f64,
                    class_cells,
                    sort_key: active_key.clone(),
                    freq,
                    stats: stat_vals,
                });
            }
        }
    }

    // Order rows: BY group order (outer, preserved), then _TYPE_ ascending,
    // then active class-value tuple via sas_cmp.
    out_rows.sort_by(|a, b| {
        match a.by_idx.cmp(&b.by_idx) {
            Ordering::Equal => {}
            other => return other,
        }
        match a.ty.partial_cmp(&b.ty).unwrap_or(Ordering::Equal) {
            Ordering::Equal => {}
            other => return other,
        }
        for (x, y) in a.sort_key.iter().zip(&b.sort_key) {
            let c = x.sas_cmp(y);
            if c != Ordering::Equal {
                return c;
            }
        }
        Ordering::Equal
    });

    // Build the output DataFrame column-by-column.
    let n_rows = out_rows.len();
    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();

    // BY columns first (copy input VarMeta; values from the BY-group key).
    for (bi, bc) in by_cols.iter().enumerate() {
        let meta = &ds.vars[bc.col_idx];
        let series = match meta.ty {
            VarType::Num => {
                let vals: Vec<Option<f64>> = out_rows
                    .iter()
                    .map(|r| value_to_num(&by_groups_list[r.by_idx].0[bi]))
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
            VarType::Char => {
                let vals: Vec<Option<String>> = out_rows
                    .iter()
                    .map(|r| match &by_groups_list[r.by_idx].0[bi] {
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

    // CLASS columns (copy input VarMeta; encode per-row values).
    for (ci, &col_idx) in class_cols.iter().enumerate() {
        let meta = &ds.vars[col_idx];
        let series = match meta.ty {
            VarType::Num => {
                let vals: Vec<Option<f64>> = out_rows
                    .iter()
                    .map(|r| value_to_num(&r.class_cells[ci]))
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
            VarType::Char => {
                let vals: Vec<Option<String>> = out_rows
                    .iter()
                    .map(|r| match &r.class_cells[ci] {
                        Value::Char(s) if s.is_empty() => None,
                        Value::Char(s) => Some(s.clone()),
                        Value::Missing(_) => None,
                        Value::Num(_) => None,
                    })
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
        };
        columns.push(series.into());
        vars.push(meta.clone());
    }

    // _TYPE_
    let type_vals: Vec<Option<f64>> = out_rows.iter().map(|r| Some(r.ty)).collect();
    columns.push(Series::new("_TYPE_".into(), type_vals).into());
    vars.push(num_var_meta("_TYPE_"));

    // _FREQ_
    let freq_vals: Vec<Option<f64>> = out_rows.iter().map(|r| Some(r.freq)).collect();
    columns.push(Series::new("_FREQ_".into(), freq_vals).into());
    vars.push(num_var_meta("_FREQ_"));

    // One column per output spec.
    for (si, sp) in specs.iter().enumerate() {
        let vals: Vec<Option<f64>> = out_rows
            .iter()
            .map(|r| value_to_num(&r.stats[si]))
            .collect();
        columns.push(Series::new(sp.outname.as_str().into(), vals).into());
        vars.push(num_var_meta(&sp.outname));
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

fn num_var_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
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

    fn parse_means(src: &str) -> Result<MeansAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "means"
        parse(&mut ts)
    }

    // ───────────────────────────── parse tests ─────────────────────────────

    #[test]
    fn parse_header_stats() {
        let ast = parse_means("proc means data=a n mean std min max; run;").unwrap();
        assert_eq!(ast.data.as_ref().unwrap().name, "a");
        assert!(!ast.noprint);
        assert_eq!(ast.stats, vec!["n", "mean", "std", "min", "max"]);
    }

    #[test]
    fn parse_noprint() {
        let ast = parse_means("proc means data=a noprint; run;").unwrap();
        assert!(ast.noprint);
    }

    #[test]
    fn parse_class_and_var() {
        let ast =
            parse_means("proc means data=a; class g h; var x y; run;").unwrap();
        assert_eq!(ast.class, vec!["g", "h"]);
        assert_eq!(ast.var, vec!["x", "y"]);
    }

    #[test]
    fn parse_output_specs() {
        let ast = parse_means(
            "proc means data=a; var height; output out=b mean(height)=avg_h n(height)=n_h; run;",
        )
        .unwrap();
        let out = ast.output.as_ref().unwrap();
        assert_eq!(out.out.name, "b");
        assert_eq!(
            out.specs,
            vec![
                ("mean".to_string(), "height".to_string(), "avg_h".to_string()),
                ("n".to_string(), "height".to_string(), "n_h".to_string()),
            ]
        );
    }

    #[test]
    fn parse_unknown_option_errors() {
        let r = parse_means("proc means data=a bogus; run;");
        assert!(r.is_err());
        let msg = r.err().unwrap().to_string();
        assert!(msg.contains("BOGUS"), "msg: {msg}");
    }

    // ───────────────────────────── compute tests ───────────────────────────

    #[test]
    fn compute_basic_stats_with_a_missing() {
        // values: 2, 4, 6, missing -> non-missing [2,4,6], nmiss=1
        let xs = vec![2.0, 4.0, 6.0];
        assert_eq!(compute("n", &xs, 1, 0.05), Value::Num(3.0));
        assert_eq!(compute("nmiss", &xs, 1, 0.05), Value::Num(1.0));
        assert_eq!(compute("mean", &xs, 1, 0.05), Value::Num(4.0));
        assert_eq!(compute("min", &xs, 1, 0.05), Value::Num(2.0));
        assert_eq!(compute("max", &xs, 1, 0.05), Value::Num(6.0));
        assert_eq!(compute("sum", &xs, 1, 0.05), Value::Num(12.0));
        assert_eq!(compute("range", &xs, 1, 0.05), Value::Num(4.0));
        assert_eq!(compute("median", &xs, 1, 0.05), Value::Num(4.0));
        // std of [2,4,6]: variance = ((2-4)^2+(4-4)^2+(6-4)^2)/2 = 8/2 = 4 -> std 2
        assert_eq!(compute("std", &xs, 1, 0.05), Value::Num(2.0));
    }

    #[test]
    fn compute_median_even() {
        let xs = vec![1.0, 2.0, 3.0, 4.0];
        assert_eq!(compute("median", &xs, 0, 0.05), Value::Num(2.5));
    }

    #[test]
    fn compute_edge_n0_and_n1() {
        let empty: Vec<f64> = vec![];
        assert_eq!(compute("n", &empty, 0, 0.05), Value::Num(0.0));
        assert!(compute("mean", &empty, 0, 0.05).is_missing());
        assert!(compute("std", &empty, 0, 0.05).is_missing());
        assert!(compute("min", &empty, 0, 0.05).is_missing());
        assert!(compute("range", &empty, 0, 0.05).is_missing());
        assert_eq!(compute("sum", &empty, 0, 0.05), Value::Num(0.0));

        let one = vec![5.0];
        assert_eq!(compute("n", &one, 0, 0.05), Value::Num(1.0));
        assert_eq!(compute("mean", &one, 0, 0.05), Value::Num(5.0));
        // std needs n>=2.
        assert!(compute("std", &one, 0, 0.05).is_missing());
        assert!(compute("stderr", &one, 0, 0.05).is_missing());
        assert_eq!(compute("min", &one, 0, 0.05), Value::Num(5.0));
    }

    #[test]
    fn compute_cv_and_stderr() {
        // [2,4,6]: mean 4, std 2 -> cv = 100*2/4 = 50; stderr = 2/sqrt(3)
        let xs = vec![2.0, 4.0, 6.0];
        assert_eq!(compute("cv", &xs, 0, 0.05), Value::Num(50.0));
        if let Value::Num(se) = compute("stderr", &xs, 0, 0.05) {
            assert!((se - 2.0 / 3.0_f64.sqrt()).abs() < 1e-12);
        } else {
            panic!("stderr should be numeric");
        }
    }

    // ──────────────────────── confidence-interval tests ────────────────────

    #[test]
    fn t_quantile_known_values() {
        // t_{0.975, 1} ≈ 12.7062
        assert!((t_quantile(0.975, 1.0) - 12.7062).abs() < 1e-3);
        // t_{0.975, 10} ≈ 2.2281
        assert!((t_quantile(0.975, 10.0) - 2.2281).abs() < 1e-3);
        // t_{0.975, large} → z_{0.975} ≈ 1.95996
        assert!((t_quantile(0.975, 100000.0) - 1.95996).abs() < 1e-3);
        // Symmetry and median.
        assert_eq!(t_quantile(0.5, 7.0), 0.0);
        assert!((t_quantile(0.025, 10.0) + t_quantile(0.975, 10.0)).abs() < 1e-6);
    }

    #[test]
    fn compute_clm_hand_computed() {
        // values [2,4,4,4,5,5,7,9]: mean 5, n=8. SAS uses the SAMPLE std
        // (VARDEF=DF): var = 32/7 → std = 2.13809, stderr = std/sqrt(8) =
        // 0.75593, t_{0.975,7} = 2.36462 → h = 1.78749. (The task brief's
        // 3.3278/6.6722 assumed std=2, which is not the sample std of this
        // data; SAS reports the values below.)
        let xs = vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let lo = match compute("lclm", &xs, 0, 0.05) {
            Value::Num(f) => f,
            _ => panic!("lclm numeric"),
        };
        let hi = match compute("uclm", &xs, 0, 0.05) {
            Value::Num(f) => f,
            _ => panic!("uclm numeric"),
        };
        assert!((lo - 3.21251).abs() < 1e-3, "lclm={lo}");
        assert!((hi - 6.78749).abs() < 1e-3, "uclm={hi}");
    }

    #[test]
    fn compute_clm_alpha_widens_interval() {
        let xs = vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let lo05 = match compute("lclm", &xs, 0, 0.05) {
            Value::Num(f) => f,
            _ => unreachable!(),
        };
        let lo01 = match compute("lclm", &xs, 0, 0.01) {
            Value::Num(f) => f,
            _ => unreachable!(),
        };
        // Smaller alpha → wider interval → lower lower-limit.
        assert!(lo01 < lo05, "lo01={lo01} lo05={lo05}");
    }

    #[test]
    fn compute_clm_requires_n2() {
        let one = vec![5.0];
        assert!(compute("lclm", &one, 0, 0.05).is_missing());
        assert!(compute("uclm", &one, 0, 0.05).is_missing());
        let empty: Vec<f64> = vec![];
        assert!(compute("lclm", &empty, 0, 0.05).is_missing());
        // "clm" has no single-value meaning.
        assert!(compute("clm", &one, 0, 0.05).is_missing());
    }

    #[test]
    fn parse_alpha_option() {
        let ast = parse_means("proc means data=a alpha=0.1 clm; var x; run;").unwrap();
        assert!((ast.alpha - 0.1).abs() < 1e-12);
        assert!(ast.stats.contains(&"clm".to_string()));
    }

    #[test]
    fn parse_alpha_default() {
        let ast = parse_means("proc means data=a; var x; run;").unwrap();
        assert!((ast.alpha - 0.05).abs() < 1e-12);
    }

    #[test]
    fn parse_alpha_invalid_errors() {
        for bad in ["alpha=0", "alpha=1", "alpha=1.5"] {
            let r = parse_means(&format!("proc means data=a {bad}; run;"));
            assert!(r.is_err(), "{bad} should error");
            assert!(
                r.err().unwrap().to_string().contains("between 0 and 1"),
                "{bad}"
            );
        }
    }

    #[test]
    fn cl_percent_label_values() {
        assert_eq!(cl_percent_label(0.05), "95");
        assert_eq!(cl_percent_label(0.10), "90");
        assert_eq!(cl_percent_label(0.01), "99");
    }

    // ───────────────────────────── execute tests ───────────────────────────

    fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
        session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
        session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
    }

    fn read_num_col(session: &Session, table: &str, col: &str) -> Vec<Value> {
        let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
        let idx = ds.vars.iter().position(|m| m.name == col).unwrap();
        decode_column(&ds, idx).unwrap()
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

    fn char_meta(name: &str) -> VarMeta {
        VarMeta {
            name: name.to_string(),
            ty: VarType::Char,
            length: 4,
            format: None,
            label: None,
        }
    }

    #[test]
    fn execute_output_k0_no_class() {
        let mut session = make_session();
        let df = df!["x" => [Some(2.0_f64), Some(4.0), Some(6.0), None]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = MeansAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            summary: false,
            noprint: true,
            stats: vec![],
            class: vec![],
            var: vec!["x".into()],
            by: vec![],
            weight: None,
            alpha: 0.05,
            output: Some(MeansOutput {
                out: DatasetRef { libref: Some("WORK".into()), name: "O".into() },
                specs: vec![
                    ("mean".into(), "x".into(), "m".into()),
                    ("n".into(), "x".into(), "cnt".into()),
                    ("nmiss".into(), "x".into(), "nm".into()),
                ],
            }),
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        assert_eq!(out.n_obs(), 1);
        let ty = read_num_col(&session, "O", "_TYPE_");
        let freq = read_num_col(&session, "O", "_FREQ_");
        let m = read_num_col(&session, "O", "m");
        let cnt = read_num_col(&session, "O", "cnt");
        let nm = read_num_col(&session, "O", "nm");
        assert_eq!(ty, vec![Value::Num(0.0)]);
        assert_eq!(freq, vec![Value::Num(4.0)]); // all rows incl. missing
        assert_eq!(m, vec![Value::Num(4.0)]);
        assert_eq!(cnt, vec![Value::Num(3.0)]);
        assert_eq!(nm, vec![Value::Num(1.0)]);
    }

    #[test]
    fn execute_output_k1() {
        let mut session = make_session();
        // group g: a(1,3) b(10)  -> means a=2, b=10
        let df = df![
            "g" => ["a", "a", "b"],
            "x" => [1.0_f64, 3.0, 10.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = MeansAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            summary: false,
            noprint: true,
            stats: vec![],
            class: vec!["g".into()],
            var: vec!["x".into()],
            by: vec![],
            weight: None,
            alpha: 0.05,
            output: Some(MeansOutput {
                out: DatasetRef { libref: Some("WORK".into()), name: "O".into() },
                specs: vec![
                    ("mean".into(), "x".into(), "mx".into()),
                ],
            }),
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        // _TYPE_ 0 (overall) + 2 levels = 3 rows.
        assert_eq!(out.n_obs(), 3);

        let ty = read_num_col(&session, "O", "_TYPE_");
        let freq = read_num_col(&session, "O", "_FREQ_");
        let mx = read_num_col(&session, "O", "mx");
        let g = read_num_col(&session, "O", "g"); // char col decoded as Value::Char

        // Row 0 = overall (_TYPE_=0): freq 3, mean (1+3+10)/3.
        assert_eq!(ty[0], Value::Num(0.0));
        assert_eq!(freq[0], Value::Num(3.0));
        assert_eq!(mx[0], Value::Num((1.0 + 3.0 + 10.0) / 3.0));
        // The overall class cell is blank (inactive char -> empty/null).
        assert_eq!(g[0], Value::Char(String::new()));

        // Rows 1,2 = per-level (_TYPE_=1), ordered a then b.
        assert_eq!(ty[1], Value::Num(1.0));
        assert_eq!(ty[2], Value::Num(1.0));
        assert_eq!(g[1], Value::Char("a".into()));
        assert_eq!(g[2], Value::Char("b".into()));
        assert_eq!(freq[1], Value::Num(2.0));
        assert_eq!(freq[2], Value::Num(1.0));
        assert_eq!(mx[1], Value::Num(2.0));
        assert_eq!(mx[2], Value::Num(10.0));
    }

    #[test]
    fn execute_output_k2_type_set_and_rowcount() {
        let mut session = make_session();
        // c0 (g) has 2 levels {a,b}; c1 (h) has 2 levels {1,2}.
        // combos present: (a,1),(a,2),(b,1) -> 3 combos.
        let df = df![
            "g" => ["a", "a", "b"],
            "h" => [1.0_f64, 2.0, 1.0],
            "x" => [5.0_f64, 7.0, 9.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("g"), num_meta("h"), num_meta("x")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = MeansAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            summary: false,
            noprint: true,
            stats: vec![],
            class: vec!["g".into(), "h".into()],
            var: vec!["x".into()],
            by: vec![],
            weight: None,
            alpha: 0.05,
            output: Some(MeansOutput {
                out: DatasetRef { libref: Some("WORK".into()), name: "O".into() },
                specs: vec![("sum".into(), "x".into(), "sx".into())],
            }),
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        // Expected rows:
        //   _TYPE_=0 : 1 (overall)
        //   _TYPE_=1 : levels of LAST class (h) = {1,2} -> 2
        //   _TYPE_=2 : levels of FIRST class (g) = {a,b} -> 2
        //   _TYPE_=3 : combos = 3
        // total = 1 + 2 + 2 + 3 = 8
        assert_eq!(out.n_obs(), 8);

        let ty = read_num_col(&session, "O", "_TYPE_");
        let type_set: std::collections::BTreeSet<i64> = ty
            .iter()
            .map(|v| match v {
                Value::Num(f) => *f as i64,
                _ => panic!("type must be numeric"),
            })
            .collect();
        assert_eq!(
            type_set,
            [0i64, 1, 2, 3].iter().cloned().collect()
        );

        // _TYPE_ is ascending.
        let tys: Vec<f64> = ty.iter().map(|v| match v {
            Value::Num(f) => *f,
            _ => unreachable!(),
        }).collect();
        let mut sorted = tys.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(tys, sorted);

        // _TYPE_=3 sum check: combo (a,1) sum=5, (a,2)=7, (b,1)=9; overall freq.
        let freq = read_num_col(&session, "O", "_FREQ_");
        assert_eq!(freq[0], Value::Num(3.0)); // overall _TYPE_=0
    }

    #[test]
    fn execute_report_contains_title_and_var() {
        let mut session = make_session();
        let df = df!["height" => [60.0_f64, 62.0, 64.0]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("height")] };
        write_dataset(&mut session, "T", ds);

        let ast = MeansAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            summary: false,
            noprint: false,
            stats: vec![],
            class: vec![],
            var: vec![],
            by: vec![],
            weight: None,
            alpha: 0.05,
            output: None,
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("The MEANS Procedure"), "listing: {listing}");
        assert!(listing.contains("height"), "listing: {listing}");
        // default stats headers
        assert!(listing.contains("Mean"), "listing: {listing}");
        assert!(listing.contains("Minimum"), "listing: {listing}");
    }

    #[test]
    fn execute_noprint_writes_nothing_to_listing() {
        let mut session = make_session();
        let df = df!["x" => [1.0_f64, 2.0]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = MeansAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            summary: true,
            noprint: true,
            stats: vec![],
            class: vec![],
            var: vec![],
            by: vec![],
            weight: None,
            alpha: 0.05,
            output: None,
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(
            !listing.contains("The MEANS Procedure"),
            "noprint should not emit a report: {listing}"
        );
    }

    #[test]
    fn execute_clm_output_readback() {
        let mut session = make_session();
        // [2,4,4,4,5,5,7,9]: mean 5, lclm≈3.21251, uclm≈6.78749 (alpha 0.05,
        // SAS sample-std CI).
        let df = df!["x" => [2.0_f64, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = MeansAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            summary: false,
            noprint: true,
            stats: vec![],
            class: vec![],
            var: vec!["x".into()],
            by: vec![],
            weight: None,
            alpha: 0.05,
            output: Some(MeansOutput {
                out: DatasetRef { libref: Some("WORK".into()), name: "O".into() },
                specs: vec![
                    ("lclm".into(), "x".into(), "lo".into()),
                    ("uclm".into(), "x".into(), "hi".into()),
                ],
            }),
        };
        execute(&ast, &mut session).unwrap();

        let lo = read_num_col(&session, "O", "lo");
        let hi = read_num_col(&session, "O", "hi");
        if let (Value::Num(l), Value::Num(h)) = (&lo[0], &hi[0]) {
            assert!((l - 3.21251).abs() < 1e-3, "lo={l}");
            assert!((h - 6.78749).abs() < 1e-3, "hi={h}");
        } else {
            panic!("lclm/uclm numeric");
        }
    }

    #[test]
    fn execute_clm_report_headers() {
        let mut session = make_session();
        let df = df!["x" => [2.0_f64, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = MeansAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            summary: false,
            noprint: false,
            stats: vec!["mean".into(), "clm".into()],
            class: vec![],
            var: vec!["x".into()],
            by: vec![],
            weight: None,
            alpha: 0.05,
            output: None,
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(
            listing.contains("Lower 95% CL for Mean"),
            "listing: {listing}"
        );
        assert!(
            listing.contains("Upper 95% CL for Mean"),
            "listing: {listing}"
        );
    }

    // ───────────────────────────── BY tests ────────────────────────────────

    #[test]
    fn execute_by_per_group_report_and_headings() {
        let mut session = make_session();
        // Sorted by sex: F,F,M,M.
        let df = df![
            "sex" => ["F", "F", "M", "M"],
            "x" => [2.0_f64, 4.0, 10.0, 20.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("sex"), num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = MeansAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            summary: false,
            noprint: false,
            stats: vec!["mean".into()],
            class: vec![],
            var: vec!["x".into()],
            by: vec![("sex".into(), false)],
            weight: None,
            alpha: 0.05,
            output: None,
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // Title once, BY headings for each group.
        assert!(listing.contains("The MEANS Procedure"), "listing: {listing}");
        assert!(listing.contains("sex=F"), "listing: {listing}");
        assert!(listing.contains("sex=M"), "listing: {listing}");
        // The F group mean is 3, the M group mean is 15.
        assert!(listing.contains("15"), "listing: {listing}");
    }

    #[test]
    fn execute_by_unsorted_errors() {
        let mut session = make_session();
        // NOT sorted by sex: F,M,F.
        let df = df![
            "sex" => ["F", "M", "F"],
            "x" => [1.0_f64, 2.0, 3.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("sex"), num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = MeansAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            summary: false,
            noprint: false,
            stats: vec![],
            class: vec![],
            var: vec!["x".into()],
            by: vec![("sex".into(), false)],
            weight: None,
            alpha: 0.05,
            output: None,
        };
        let r = execute(&ast, &mut session);
        assert!(r.is_err());
        let msg = r.err().unwrap().to_string();
        assert!(
            msg.contains("not sorted in ascending sequence")
                && msg.contains("sex=M")
                && msg.contains("sex=F"),
            "msg: {msg}"
        );
    }

    #[test]
    fn execute_by_output_dataset_rows() {
        let mut session = make_session();
        // Sorted by sex: F,F,M.
        let df = df![
            "sex" => ["F", "F", "M"],
            "x" => [2.0_f64, 4.0, 10.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("sex"), num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = MeansAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            summary: false,
            noprint: true,
            stats: vec![],
            class: vec![],
            var: vec!["x".into()],
            by: vec![("sex".into(), false)],
            weight: None,
            alpha: 0.05,
            output: Some(MeansOutput {
                out: DatasetRef { libref: Some("WORK".into()), name: "O".into() },
                specs: vec![("mean".into(), "x".into(), "mx".into())],
            }),
        };
        execute(&ast, &mut session).unwrap();

        // No CLASS → one row per BY group (k=0, _TYPE_=0).
        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        assert_eq!(out.n_obs(), 2);
        let sex = read_num_col(&session, "O", "sex"); // char decoded
        let mx = read_num_col(&session, "O", "mx");
        assert_eq!(sex[0], Value::Char("F".into()));
        assert_eq!(sex[1], Value::Char("M".into()));
        assert_eq!(mx[0], Value::Num(3.0));
        assert_eq!(mx[1], Value::Num(10.0));
        // BY column comes before _TYPE_.
        let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names[0], "sex");
        assert!(names.contains(&"_TYPE_"));
    }

    // ───────────────────────────── WEIGHT tests ────────────────────────────

    #[test]
    fn compute_weighted_hand_values() {
        // values [1,2,3] weights [1,2,3]:
        //   SumWgt=6, Sum=14, mean=14/6=2.33333...
        //   CSS_w = 1*(1-m)^2 + 2*(2-m)^2 + 3*(3-m)^2 = 3.33333...
        //   Variance = CSS_w/(n-1) = 3.33333/2 = 1.66667
        //   Std = sqrt(1.66667) = 1.2909944
        //   StdErr = Std/sqrt(6) = 0.5270463
        //   CV = 100*Std/mean = 55.3283
        //   USS_w = 1*1 + 2*4 + 3*9 = 36
        let pairs = vec![(1.0, 1.0), (2.0, 2.0), (3.0, 3.0)];
        assert_eq!(compute_weighted("n", &pairs, 0, 0.05), Value::Num(3.0));
        assert_eq!(compute_weighted("nmiss", &pairs, 0, 0.05), Value::Num(0.0));
        assert_eq!(compute_weighted("sum", &pairs, 0, 0.05), Value::Num(14.0));
        assert_eq!(compute_weighted("min", &pairs, 0, 0.05), Value::Num(1.0));
        assert_eq!(compute_weighted("max", &pairs, 0, 0.05), Value::Num(3.0));

        let m = match compute_weighted("mean", &pairs, 0, 0.05) {
            Value::Num(f) => f,
            _ => panic!("mean numeric"),
        };
        assert!((m - 14.0 / 6.0).abs() < 1e-12, "mean = {m}");

        let std = match compute_weighted("std", &pairs, 0, 0.05) {
            Value::Num(f) => f,
            _ => panic!("std numeric"),
        };
        assert!((std - (5.0_f64 / 3.0).sqrt()).abs() < 1e-12, "std = {std}");

        let se = match compute_weighted("stderr", &pairs, 0, 0.05) {
            Value::Num(f) => f,
            _ => panic!("stderr numeric"),
        };
        assert!(
            (se - (5.0_f64 / 3.0).sqrt() / 6.0_f64.sqrt()).abs() < 1e-12,
            "stderr = {se}"
        );

        let cv = match compute_weighted("cv", &pairs, 0, 0.05) {
            Value::Num(f) => f,
            _ => panic!("cv numeric"),
        };
        let expected_cv = 100.0 * (5.0_f64 / 3.0).sqrt() / (14.0 / 6.0);
        assert!((cv - expected_cv).abs() < 1e-9, "cv = {cv}");
    }

    #[test]
    fn compute_weighted_n1_std_missing() {
        let pairs = vec![(5.0, 2.0)];
        assert_eq!(compute_weighted("n", &pairs, 0, 0.05), Value::Num(1.0));
        assert_eq!(compute_weighted("mean", &pairs, 0, 0.05), Value::Num(5.0));
        assert!(compute_weighted("std", &pairs, 0, 0.05).is_missing());
        assert!(compute_weighted("stderr", &pairs, 0, 0.05).is_missing());
    }

    #[test]
    fn execute_weight_report_and_exclusions() {
        let mut session = make_session();
        // x: 1,2,3, bad(w<=0), bad(missing w), bad(missing x)
        // weights: 1,2,3, 5, ., 4  -> only first three usable.
        let df = df![
            "x" => [Some(1.0_f64), Some(2.0), Some(3.0), Some(9.0), Some(7.0), None],
            "w" => [Some(1.0_f64), Some(2.0), Some(3.0), Some(0.0), None, Some(4.0)]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("w")] };
        write_dataset(&mut session, "T", ds);

        let ast = MeansAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            summary: false,
            noprint: true,
            stats: vec!["n".into(), "nmiss".into(), "mean".into(), "sum".into()],
            class: vec![],
            var: vec!["x".into()],
            by: vec![],
            weight: Some("w".into()),
            alpha: 0.05,
            output: Some(MeansOutput {
                out: DatasetRef { libref: Some("WORK".into()), name: "O".into() },
                specs: vec![
                    ("n".into(), "x".into(), "nx".into()),
                    ("nmiss".into(), "x".into(), "nmx".into()),
                    ("mean".into(), "x".into(), "mx".into()),
                    ("sum".into(), "x".into(), "sx".into()),
                ],
            }),
        };
        execute(&ast, &mut session).unwrap();

        let nx = read_num_col(&session, "O", "nx");
        let nmx = read_num_col(&session, "O", "nmx");
        let mx = read_num_col(&session, "O", "mx");
        let sx = read_num_col(&session, "O", "sx");
        assert_eq!(nx, vec![Value::Num(3.0)]);
        assert_eq!(nmx, vec![Value::Num(3.0)]); // w<=0, missing w, missing x
        assert_eq!(sx, vec![Value::Num(14.0)]); // weighted sum Σw_i x_i
        if let Value::Num(m) = mx[0] {
            assert!((m - 14.0 / 6.0).abs() < 1e-12, "mean = {m}");
        } else {
            panic!("mean numeric");
        }
    }

    #[test]
    fn execute_weight_with_by() {
        let mut session = make_session();
        // Sorted by g: a(values 1,2,3 weights 1,2,3) b(values 10,20 weights 1,1)
        let df = df![
            "g" => ["a", "a", "a", "b", "b"],
            "x" => [1.0_f64, 2.0, 3.0, 10.0, 20.0],
            "w" => [1.0_f64, 2.0, 3.0, 1.0, 1.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x"), num_meta("w")] };
        write_dataset(&mut session, "T", ds);

        let ast = MeansAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            summary: false,
            noprint: true,
            stats: vec![],
            class: vec![],
            var: vec!["x".into()],
            by: vec![("g".into(), false)],
            weight: Some("w".into()),
            alpha: 0.05,
            output: Some(MeansOutput {
                out: DatasetRef { libref: Some("WORK".into()), name: "O".into() },
                specs: vec![("mean".into(), "x".into(), "mx".into())],
            }),
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        assert_eq!(out.n_obs(), 2);
        let mx = read_num_col(&session, "O", "mx");
        // a: 14/6 = 2.33333 ; b: (10+20)/2 = 15.
        if let Value::Num(m) = mx[0] {
            assert!((m - 14.0 / 6.0).abs() < 1e-12, "a mean = {m}");
        } else {
            panic!("numeric");
        }
        assert_eq!(mx[1], Value::Num(15.0));
    }

    #[test]
    fn execute_weight_with_class() {
        let mut session = make_session();
        // class g: a(1,2,3 w 1,2,3) b(10 w 5)
        let df = df![
            "g" => ["a", "a", "a", "b"],
            "x" => [1.0_f64, 2.0, 3.0, 10.0],
            "w" => [1.0_f64, 2.0, 3.0, 5.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x"), num_meta("w")] };
        write_dataset(&mut session, "T", ds);

        let ast = MeansAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            summary: false,
            noprint: true,
            stats: vec![],
            class: vec!["g".into()],
            var: vec!["x".into()],
            by: vec![],
            weight: Some("w".into()),
            alpha: 0.05,
            output: Some(MeansOutput {
                out: DatasetRef { libref: Some("WORK".into()), name: "O".into() },
                specs: vec![("mean".into(), "x".into(), "mx".into())],
            }),
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        // _TYPE_ 0 (overall) + 2 levels = 3 rows.
        assert_eq!(out.n_obs(), 3);
        let ty = read_num_col(&session, "O", "_TYPE_");
        let mx = read_num_col(&session, "O", "mx");
        // overall: Σwx/Σw = (1+4+9+50)/(1+2+3+5) = 64/11 = 5.81818...
        assert_eq!(ty[0], Value::Num(0.0));
        if let Value::Num(m) = mx[0] {
            assert!((m - 64.0 / 11.0).abs() < 1e-12, "overall mean = {m}");
        } else {
            panic!("numeric");
        }
        // level a (_TYPE_=1): 14/6 ; level b: 10.
        if let Value::Num(m) = mx[1] {
            assert!((m - 14.0 / 6.0).abs() < 1e-12, "a mean = {m}");
        } else {
            panic!("numeric");
        }
        assert_eq!(mx[2], Value::Num(10.0));
    }

    #[test]
    fn parse_weight_statement() {
        let ast = parse_means("proc means data=a; var x; weight w; run;").unwrap();
        assert_eq!(ast.weight.as_deref(), Some("w"));
        assert_eq!(ast.var, vec!["x"]);
    }

    // ─────────────────────── ODS OUTPUT Summary= (M22.3) ────────────────────

    fn means_ast_var_x() -> MeansAst {
        MeansAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            summary: false,
            noprint: false,
            stats: vec![],
            class: vec![],
            var: vec!["x".into()],
            by: vec![],
            weight: None,
            alpha: 0.05,
            output: None,
        }
    }

    #[test]
    fn ods_output_summary_captures_dataset() {
        let mut session = make_session();
        let df = df!["x" => [Some(2.0_f64), Some(4.0), Some(6.0)]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        // Activate ODS OUTPUT Summary=means_out.
        session.set_ods_output(&[(
            "summary".into(),
            DatasetRef { libref: None, name: "means_out".into() },
        )]);

        let ast = means_ast_var_x();
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("MEANS_OUT").unwrap();
        assert_eq!(out.n_obs(), 1, "one row per VAR variable");
        // Columns: Variable, N, Mean, StdDev, Min, Max.
        let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["Variable", "N", "Mean", "StdDev", "Min", "Max"]);

        // Variable name column is char "x".
        let var_idx = out.vars.iter().position(|v| v.name == "Variable").unwrap();
        assert_eq!(out.vars[var_idx].ty, VarType::Char);

        assert_eq!(read_num_col(&session, "MEANS_OUT", "N"), vec![Value::Num(3.0)]);
        assert_eq!(read_num_col(&session, "MEANS_OUT", "Mean"), vec![Value::Num(4.0)]);
        // std of [2,4,6] = 2.
        assert_eq!(read_num_col(&session, "MEANS_OUT", "StdDev"), vec![Value::Num(2.0)]);
        assert_eq!(read_num_col(&session, "MEANS_OUT", "Min"), vec![Value::Num(2.0)]);
        assert_eq!(read_num_col(&session, "MEANS_OUT", "Max"), vec![Value::Num(6.0)]);

        // last_dataset points at the captured dataset.
        assert_eq!(session.last_dataset.as_deref(), Some("WORK.MEANS_OUT"));
    }

    #[test]
    fn ods_output_summary_case_insensitive_table_name() {
        let mut session = make_session();
        let df = df!["x" => [Some(1.0_f64), Some(3.0)]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        // Registered under a different casing; matching must be case-insensitive.
        session.set_ods_output(&[(
            "SuMMaRy".into(),
            DatasetRef { libref: None, name: "o".into() },
        )]);

        execute(&means_ast_var_x(), &mut session).unwrap();
        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        assert_eq!(out.n_obs(), 1);
        assert_eq!(read_num_col(&session, "O", "Mean"), vec![Value::Num(2.0)]);
    }

    #[test]
    fn ods_output_inactive_writes_no_dataset() {
        // Invariant: with an empty ods_output_map, no capture dataset is written.
        let mut session = make_session();
        let df = df!["x" => [Some(2.0_f64), Some(4.0)]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        execute(&means_ast_var_x(), &mut session).unwrap();

        // No "SUMMARY" dataset, and last_dataset unchanged (still the input T).
        assert!(session.libs.get("WORK").unwrap().read("SUMMARY").is_err());
        assert_eq!(session.last_dataset.as_deref(), Some("WORK.T"));
    }

    #[test]
    fn ods_output_summary_multiple_vars_one_row_each() {
        let mut session = make_session();
        let df = df![
            "x" => [Some(1.0_f64), Some(3.0)],
            "y" => [Some(10.0_f64), Some(20.0)],
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("y")] };
        write_dataset(&mut session, "T", ds);

        session.set_ods_output(&[(
            "summary".into(),
            DatasetRef { libref: None, name: "o".into() },
        )]);

        let mut ast = means_ast_var_x();
        ast.var = vec!["x".into(), "y".into()];
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        assert_eq!(out.n_obs(), 2, "one row per VAR variable");
        assert_eq!(
            read_num_col(&session, "O", "Mean"),
            vec![Value::Num(2.0), Value::Num(15.0)]
        );
    }
}
