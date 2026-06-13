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

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{decode_column, sample_std};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{format_best, VarType};
use std::cmp::Ordering;

pub struct UnivariateAst {
    pub data: Option<DatasetRef>,
    pub var: Vec<String>,
}

/// Parse `proc univariate [data=a] [noprint] ; [var v...;] [by ...;] ... run;`.
/// Called AFTER "proc univariate" has been consumed. Consumes through
/// `run;`/`quit;`. Unknown sub-statements (e.g. BY, HISTOGRAM) are skipped
/// leniently to their terminating `;` (BY grouping is out of M5 scope).
pub fn parse(ts: &mut StatementStream) -> Result<UnivariateAst> {
    let mut data: Option<DatasetRef> = None;
    let mut var: Vec<String> = Vec::new();

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
            // Accepted and ignored: UNIVARIATE always has a report to show;
            // NOPRINT is meaningful only with OUTPUT (out of M5 scope).
            ts.next();
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
            ts.expect_semi()?;
        } else {
            // Unknown sub-statement (by, histogram, ...): skip leniently.
            ts.skip_to_semi();
        }
    }

    Ok(UnivariateAst { data, var })
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

    session.listing.page_header();
    centered(session, "The UNIVARIATE Procedure");

    for &ci in &var_cols {
        let col = decode_column(&ds, ci)?;
        // Drop missings into a Vec<f64>, tracking original 1-based obs numbers.
        let mut data: Vec<(f64, usize)> = Vec::with_capacity(col.len());
        let mut n_missing = 0usize;
        for (row, val) in col.iter().enumerate() {
            match value_to_num(val) {
                Some(f) if !f.is_nan() => data.push((f, row + 1)),
                _ => n_missing += 1,
            }
        }

        emit_variable(session, &ds.vars[ci].name, &data, n_missing, n_obs);
    }

    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        n_obs, display_name
    ));

    Ok(())
}

/// Write a centered line within LINESIZE.
fn centered(session: &mut Session, text: &str) {
    let ls = session.listing.ls;
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

    #[test]
    fn parse_data_and_var() {
        let ast = parse_univ("proc univariate data=work.t; var x; run;").unwrap();
        assert_eq!(ast.data.as_ref().unwrap().name, "t");
        assert_eq!(ast.var, vec!["x"]);
    }

    #[test]
    fn parse_by_statement_ignored() {
        let ast =
            parse_univ("proc univariate data=work.t; by g; var x y; run;").unwrap();
        // BY is ignored; VAR is still captured.
        assert_eq!(ast.var, vec!["x", "y"]);
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
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // Both numeric variables analyzed; char skipped.
        assert!(listing.contains("Variable: a"), "listing: {listing}");
        assert!(listing.contains("Variable: b"), "listing: {listing}");
        assert!(!listing.contains("Variable: g"), "listing: {listing}");
    }
}
