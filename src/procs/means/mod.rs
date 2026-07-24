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
    /// PRINTALLTYPES PROC option (M33.3). When false (default), the printed
    /// table shows only the all-CLASS-combined `_TYPE_`; when true, every
    /// generated `_TYPE_` subtable is printed.
    pub printalltypes: bool,
    /// WAYS values (M33.3): each requests the `_TYPE_` rows whose number of
    /// active CLASS variables equals the value. Empty → no WAYS restriction.
    pub ways: Vec<usize>,
    /// TYPES specifications (M33.3): each entry is a set of CLASS variable
    /// names (a specific crossing, e.g. `(a*b)`). Empty → no TYPES restriction.
    pub types: Vec<Vec<String>>,
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
    // Percentile keywords (M33.3) — Definition 5, shared with PROC UNIVARIATE.
    "p1", "p5", "p10", "p20", "p25", "p30", "p40", "p50", "p60", "p70", "p75", "p80", "p90", "p95",
    "p99", "q1", "q3", "qrange",
];

fn is_stat_keyword(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    STAT_KEYWORDS.iter().any(|k| *k == l)
}

/// Map a percentile keyword to its target fraction `p` (None if not a single
/// percentile keyword). `Q1`=`P25`, `Q3`=`P75`, `P50`=`MEDIAN`. `QRANGE` is
/// handled separately (it is a difference of two percentiles).
fn percentile_fraction(stat: &str) -> Option<f64> {
    Some(match stat {
        "p1" => 0.01,
        "p5" => 0.05,
        "p10" => 0.10,
        "p20" => 0.20,
        "p25" | "q1" => 0.25,
        "p30" => 0.30,
        "p40" => 0.40,
        "p50" => 0.50,
        "p60" => 0.60,
        "p70" => 0.70,
        "p75" | "q3" => 0.75,
        "p80" => 0.80,
        "p90" => 0.90,
        "p95" => 0.95,
        "p99" => 0.99,
        _ => return None,
    })
}

/// Parse `proc means [data=a] [noprint] [stat...] ; [class ...;] [var ...;]
/// [output out=b stat(var)=name...;] ... run;`. Called AFTER "proc
/// means"/"proc summary" has been consumed. Consumes through `run;`/`quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<MeansAst> {
    let mut data: Option<DatasetRef> = None;
    let mut noprint = false;
    let mut printalltypes = false;
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
            crate::procs::common::expect_eq(ts, "DATA")?;
            data = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("noprint") {
            ts.next();
            noprint = true;
        } else if ts.peek().is_kw("print") {
            // explicit PRINT — undo a noprint default (e.g. PROC SUMMARY).
            ts.next();
            noprint = false;
        } else if ts.peek().is_kw("printalltypes") {
            // PRINTALLTYPES (M33.3): print every generated _TYPE_ subtable.
            ts.next();
            printalltypes = true;
        } else if ts.peek().is_kw("alpha") {
            crate::procs::common::expect_eq(ts, "ALPHA")?;
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
    let mut ways: Vec<usize> = Vec::new();
    let mut types: Vec<Vec<String>> = Vec::new();
    let mut output: Option<MeansOutput> = None;

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    crate::procs::common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "class" => {
                ts.next();
                class = crate::procs::common::parse_class(ts)?;
                true
            }
            "ways" => {
                ts.next();
                ways = parse_ways(ts)?;
                true
            }
            "types" => {
                ts.next();
                types = parse_types(ts)?;
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
        printalltypes,
        ways,
        types,
        output,
    })
}

/// Parse a WAYS statement body (after `ways` was consumed), through its `;`.
/// `ways 0 1 2;` — a list of non-negative integers (the desired numbers of
/// active CLASS variables). Errors on a non-integer token.
fn parse_ways(ts: &mut StatementStream) -> Result<Vec<usize>> {
    let mut out: Vec<usize> = Vec::new();
    loop {
        match ts.peek().kind {
            TokenKind::Semi => {
                ts.next();
                break;
            }
            TokenKind::Eof => break,
            TokenKind::Num(f) if f >= 0.0 && f.fract() == 0.0 => {
                ts.next();
                out.push(f as usize);
            }
            _ => {
                return Err(SasError::parse(
                    "expected a non-negative integer in the WAYS statement",
                    ts.peek().span,
                ));
            }
        }
    }
    Ok(out)
}

/// Parse a TYPES statement body (after `types` was consumed), through its `;`.
/// `types () (a) (a*b) a*b;` — a space-separated list of CLASS crossings; each
/// crossing is a `*`-joined set of CLASS names, optionally parenthesized. `()`
/// denotes the empty crossing (overall, `_TYPE_`=0). Returns one `Vec<String>`
/// per crossing (the empty crossing → an empty inner vector).
fn parse_types(ts: &mut StatementStream) -> Result<Vec<Vec<String>>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    loop {
        match ts.peek().kind {
            TokenKind::Semi => {
                ts.next();
                break;
            }
            TokenKind::Eof => break,
            TokenKind::LParen => {
                ts.next(); // '('
                let mut crossing: Vec<String> = Vec::new();
                loop {
                    if ts.peek().kind == TokenKind::RParen {
                        ts.next();
                        break;
                    }
                    let name = ts.peek().ident().map(str::to_string).ok_or_else(|| {
                        SasError::parse("expected a CLASS name in TYPES", ts.peek().span)
                    })?;
                    ts.next();
                    crossing.push(name);
                    if ts.peek().kind == TokenKind::Star {
                        ts.next();
                    }
                }
                out.push(crossing);
            }
            _ => {
                // Un-parenthesized crossing: name [* name]*.
                let mut crossing: Vec<String> = Vec::new();
                let name = ts.peek().ident().map(str::to_string).ok_or_else(|| {
                    SasError::parse("expected a CLASS name in TYPES", ts.peek().span)
                })?;
                ts.next();
                crossing.push(name);
                while ts.peek().kind == TokenKind::Star {
                    ts.next();
                    let name = ts.peek().ident().map(str::to_string).ok_or_else(|| {
                        SasError::parse("expected a CLASS name after '*' in TYPES", ts.peek().span)
                    })?;
                    ts.next();
                    crossing.push(name);
                }
                out.push(crossing);
            }
        }
    }
    Ok(out)
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
            crate::procs::common::expect_eq(ts, "OUT")?;
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
    // Percentile keywords (M33.3): Definition 5 via UNIVARIATE's shared
    // `quantile_def5`. `qrange` = P75 − P25. Sort the non-missing values once.
    if percentile_fraction(stat).is_some() || stat == "qrange" {
        if n == 0 {
            return Value::missing();
        }
        let mut sorted: Vec<f64> = xs.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        if stat == "qrange" {
            return match (
                crate::procs::univariate::quantile_def5(&sorted, 0.75),
                crate::procs::univariate::quantile_def5(&sorted, 0.25),
            ) {
                (Some(q3), Some(q1)) => Value::Num(q3 - q1),
                _ => Value::missing(),
            };
        }
        let p = percentile_fraction(stat).unwrap();
        return match crate::procs::univariate::quantile_def5(&sorted, p) {
            Some(q) => Value::Num(q),
            None => Value::missing(),
        };
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
        // Weighted percentiles deferred (like MEDIAN): computed UNWEIGHTED on
        // the usable values via Definition 5. Documented divergence.
        other if percentile_fraction(other).is_some() || other == "qrange" => {
            let xs: Vec<f64> = pairs.iter().map(|(x, _)| *x).collect();
            compute(other, &xs, n_excluded, alpha)
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

/// `_TYPE_` bitmask for a set of ACTIVE class positions `active` (indices into
/// the CLASS list, 0-based, left→right) given `k` CLASS variables. The LSB
/// corresponds to the LAST class variable — identical convention to the OUTPUT
/// path. Empty `active` → 0 (the overall row).
fn type_mask(active: &[usize], k: usize) -> u64 {
    let mut ty: u64 = 0;
    for &i in active {
        ty |= 1u64 << (k - 1 - i);
    }
    ty
}

/// Resolve the WAYS/TYPES restrictions (M33.3) into the SET of `_TYPE_` values
/// to keep. Returns `None` when neither WAYS nor TYPES is given (no
/// restriction — every `_TYPE_` is kept, preserving the default path). `k` is
/// the number of CLASS variables; `class` the CLASS names (for TYPES lookups).
fn allowed_types(ast: &MeansAst, class: &[String], k: usize) -> Result<Option<std::collections::BTreeSet<u64>>> {
    if ast.ways.is_empty() && ast.types.is_empty() {
        return Ok(None);
    }
    let mut set: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();

    // WAYS n: keep every _TYPE_ whose number of active CLASS vars == n. Enumerate
    // all 2^k subsets and select those whose popcount matches a requested way.
    for &w in &ast.ways {
        for mask in 0u32..(1u32 << k) {
            let active: Vec<usize> = (0..k).filter(|&i| (mask >> i) & 1 == 1).collect();
            if active.len() == w {
                set.insert(type_mask(&active, k));
            }
        }
    }

    // TYPES (crossing ...): keep the specific _TYPE_ for each named crossing.
    for crossing in &ast.types {
        let mut active: Vec<usize> = Vec::with_capacity(crossing.len());
        for name in crossing {
            let pos = class
                .iter()
                .position(|c| c.eq_ignore_ascii_case(name))
                .ok_or_else(|| {
                    SasError::runtime(format!(
                        "The variable {} in the TYPES statement is not a CLASS variable.",
                        name.to_uppercase()
                    ))
                })?;
            active.push(pos);
        }
        set.insert(type_mask(&active, k));
    }

    Ok(Some(set))
}

/// Execute PROC MEANS / SUMMARY. Called by `procs::execute_proc`.
pub fn execute(ast: &MeansAst, session: &mut Session) -> Result<()> {
    let (ds, in_libref, in_table) = crate::procs::common::open_input(&ast.data, session)?;

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

    // --- WAYS / TYPES restriction (M33.3): the set of _TYPE_ values to keep,
    // or None for "no restriction" (default path). ---
    let k = class_cols.len();
    let allowed = allowed_types(ast, &ast.class, k)?;

    // Which _TYPE_ subtables the listing prints. SAS default: ONLY the highest
    // _TYPE_ (all CLASS crossed). PRINTALLTYPES (or any WAYS/TYPES request)
    // prints each selected _TYPE_ as its own subtable. Without CLASS there is a
    // single _TYPE_=0 table either way (byte-identical default).
    let print_types: Vec<u64> = if k == 0 {
        vec![0]
    } else if ast.printalltypes || allowed.is_some() {
        // Every selected _TYPE_, ascending. With no WAYS/TYPES but
        // PRINTALLTYPES → all 2^k types.
        let mut v: Vec<u64> = match &allowed {
            Some(set) => set.iter().copied().collect(),
            None => (0u32..(1u32 << k))
                .map(|mask| {
                    let active: Vec<usize> = (0..k).filter(|&i| (mask >> i) & 1 == 1).collect();
                    type_mask(&active, k)
                })
                .collect(),
        };
        v.sort_unstable();
        v.dedup();
        v
    } else {
        // Default: only the highest _TYPE_ (all CLASS active) → byte-identical.
        vec![type_mask(&(0..k).collect::<Vec<_>>(), k)]
    };

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
            // Default single-type path stays byte-identical: when k==0 or the
            // only printed type is the full crossing AND neither PRINTALLTYPES
            // nor WAYS/TYPES is active, use the original combined-table emitter.
            let full_type = type_mask(&(0..k).collect::<Vec<_>>(), k);
            if !ast.printalltypes
                && allowed.is_none()
                && print_types == [full_type]
            {
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
            } else {
                for &ty in &print_types {
                    emit_report_type(
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
                        ty,
                    );
                }
            }
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
            allowed.as_ref(),
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
        // Percentile keywords (M33.3): canonical PNN / QRANGE column names.
        p @ ("p1" | "p5" | "p10" | "p20" | "p25" | "p30" | "p40" | "p50" | "p60" | "p70" | "p75"
        | "p80" | "p90" | "p95" | "p99") => p.to_uppercase(),
        "q1" => "P25".to_string(),
        "q3" => "P75".to_string(),
        "qrange" => "QRange".to_string(),
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

/// Emit one MEANS report subtable for a single `_TYPE_` mask `ty` (M33.3,
/// PRINTALLTYPES / WAYS / TYPES). Only the CLASS variables ACTIVE in `ty` head
/// the table; rows are grouped by those active variables within `group_rows`.
/// `ty`=0 → no CLASS columns (the overall section). The combined default-path
/// table is still produced by `emit_report_group`; this is the per-type path.
#[allow(clippy::too_many_arguments)]
fn emit_report_type(
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
    ty: u64,
) {
    let k = class_cols.len();
    // Active CLASS positions for this _TYPE_: bit (k-1-i) set ⇔ class i active.
    let active: Vec<usize> = (0..k).filter(|&i| (ty >> (k - 1 - i)) & 1 == 1).collect();

    let mut headers: Vec<String> = Vec::new();
    let mut aligns: Vec<Align> = Vec::new();
    for &i in &active {
        let ci = class_cols[i];
        headers.push(ds.vars[ci].name.clone());
        aligns.push(match ds.vars[ci].ty {
            VarType::Num => Align::Right,
            VarType::Char => Align::Left,
        });
    }
    headers.push("Variable".to_string());
    aligns.push(Align::Left);
    for s in report_stats {
        for h in stat_report_headers(s, alpha) {
            headers.push(h);
            aligns.push(Align::Right);
        }
    }

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
    if active.is_empty() {
        // Overall (_TYPE_=0): one row per analysis variable over all group rows.
        for (vi, vname_idx) in var_cols.iter().enumerate() {
            let mut row = vec![ds.vars[*vname_idx].name.clone()];
            push_cells(&mut row, vi, group_rows);
            rows.push(row);
        }
    } else {
        let active_refs: Vec<&Vec<Value>> =
            active.iter().map(|&i| &class_values[i]).collect();
        let groups = group_by_keys_subset(&active_refs, group_rows);
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
        _ => match percentile_header(stat) {
            Some(h) => vec![h],
            None => vec![stat_header(stat).to_string()],
        },
    }
}

/// SAS report header for a percentile keyword (None for non-percentile stats).
/// SAS prints percentiles as "Nth Pctl" (e.g. "25th Pctl"), `MEDIAN`/`P50` as
/// "Median", and `QRANGE` as "Quartile Range". `Q1`/`Q3` alias `P25`/`P75`.
fn percentile_header(stat: &str) -> Option<String> {
    match stat {
        "qrange" => Some("Quartile Range".to_string()),
        "median" | "p50" => Some("Median".to_string()),
        _ => {
            let p = percentile_fraction(stat)?;
            // Whole-number percentile, e.g. 0.25 → 25.
            let pct = (p * 100.0).round() as i64;
            let suffix = match (pct % 10, pct % 100) {
                (1, n) if n != 11 => "st",
                (2, n) if n != 12 => "nd",
                (3, n) if n != 13 => "rd",
                _ => "th",
            };
            Some(format!("{pct}{suffix} Pctl"))
        }
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
    allowed_types: Option<&std::collections::BTreeSet<u64>>,
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

            // WAYS / TYPES restriction (M33.3): skip _TYPE_ rows not requested.
            // None → no restriction (default path, every _TYPE_ emitted).
            if let Some(set) = allowed_types {
                if !set.contains(&ty) {
                    continue;
                }
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
mod tests;
