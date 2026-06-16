//! Helpers partagés par plusieurs PROCs.
//!
//! Ce module centralise les fonctions utilitaires communes afin d'éviter la
//! duplication de code entre `means`, `freq`, `univariate`, `sort`,
//! `transpose` et `append`. Chaque fonction est extraite verbatim de son
//! premier site d'apparition ; aucune logique n'est modifiée.

use crate::dataset::SasDataset;
use crate::error::{Result, SasError};
use crate::missing::{num_to_value, value_to_num};
use crate::value::{format_best, Value, VarType};
use std::cmp::Ordering;

/// Decode one column of a SasDataset into a `Vec<Value>` (downcast once;
/// never decode per cell).
pub fn decode_column(ds: &SasDataset, col_idx: usize) -> Result<Vec<Value>> {
    let series = ds.df.get_columns()[col_idx].as_materialized_series();
    let values = match ds.vars[col_idx].ty {
        VarType::Num => series.f64()?.iter().map(num_to_value).collect(),
        VarType::Char => series
            .str()?
            .iter()
            .map(|o| Value::Char(o.unwrap_or("").to_string()))
            .collect(),
    };
    Ok(values)
}

/// Sample standard deviation (divisor n-1). Needs n>=2, else None.
pub fn sample_std(xs: &[f64]) -> Option<f64> {
    let n = xs.len();
    if n < 2 {
        return None;
    }
    let mean = xs.iter().sum::<f64>() / n as f64;
    let ss: f64 = xs.iter().map(|v| (v - mean) * (v - mean)).sum();
    Some((ss / (n as f64 - 1.0)).sqrt())
}

/// Split a column's values for one set of row indices into (non-missing
/// numbers, missing count). Char values are treated as missing for numeric
/// statistics.
pub fn partition_numeric(col: &[Value], rows: &[usize]) -> (Vec<f64>, usize) {
    let mut xs = Vec::with_capacity(rows.len());
    let mut nmiss = 0usize;
    for &r in rows {
        match value_to_num(&col[r]) {
            Some(f) if !f.is_nan() => xs.push(f),
            _ => nmiss += 1,
        }
    }
    (xs, nmiss)
}

/// Split a value column paired with a weight column, for one set of row
/// indices, into the usable (value, weight) pairs and an excluded count.
///
/// SAS WEIGHT exclusion rules: an observation is excluded from the weighted
/// analysis when the analysis value is missing, OR the weight is missing, OR
/// the weight is <= 0 (SAS treats a non-positive weight as 0, dropping the
/// observation). Special missing values decode to NaN via `value_to_num` and
/// are therefore excluded, as are char cells. The excluded count is returned
/// as the weighted "NMiss" analogue.
pub fn partition_weighted(
    value_col: &[Value],
    weight_col: &[Value],
    rows: &[usize],
) -> (Vec<(f64, f64)>, usize) {
    let mut pairs = Vec::with_capacity(rows.len());
    let mut excluded = 0usize;
    for &r in rows {
        let v = value_to_num(&value_col[r]);
        let w = value_to_num(&weight_col[r]);
        match (v, w) {
            (Some(vf), Some(wf)) if !vf.is_nan() && !wf.is_nan() && wf > 0.0 => {
                pairs.push((vf, wf));
            }
            _ => excluded += 1,
        }
    }
    (pairs, excluded)
}

// ───────────────────────── Student-t quantile ─────────────────────────
//
// Self-contained Student-t inverse CDF (quantile), added for confidence-
// interval statistics in PROC MEANS (CLM/LCLM/UCLM). This intentionally
// duplicates the betai / ln_gamma machinery already present privately in
// `corr.rs` rather than refactoring corr's copies: keeping corr untouched
// guarantees its listing output stays byte-identical. The duplication is
// small and documented here. If a future increment wants a single source of
// truth, fold corr's private copies into these `pub(crate)` versions and run
// the corr snapshot/tests to confirm no drift.

/// Lanczos approximation of ln Γ(x) for x > 0. Accuracy ~1e-13.
fn ln_gamma(x: f64) -> f64 {
    const COF: [f64; 6] = [
        76.18009172947146,
        -86.50532032941677,
        24.01409824083091,
        -1.231739572450155,
        0.1208650973866179e-2,
        -0.5395239384953e-5,
    ];
    let mut y = x;
    let tmp = x + 5.5 - (x + 0.5) * (x + 5.5).ln();
    let mut ser = 1.000000000190015;
    for c in COF.iter() {
        y += 1.0;
        ser += c / y;
    }
    -tmp + (2.5066282746310005 * ser / x).ln()
}

/// Continued fraction for the incomplete beta function (Lentz's algorithm).
fn betacf(a: f64, b: f64, x: f64) -> f64 {
    const MAXIT: usize = 300;
    const EPS: f64 = 3.0e-15;
    const FPMIN: f64 = 1.0e-300;

    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < FPMIN {
        d = FPMIN;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=MAXIT {
        let m = m as f64;
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            break;
        }
    }
    h
}

/// Regularized incomplete beta function I_x(a, b), x in [0,1].
fn betai(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let ln_beta = ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b);
    let front = (a * x.ln() + b * (1.0 - x).ln() + ln_beta).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        front * betacf(a, b, x) / a
    } else {
        1.0 - front * betacf(b, a, 1.0 - x) / b
    }
}

/// Cumulative distribution function of Student's t with `df` degrees of
/// freedom evaluated at `t`: P(T_df <= t). Uses the regularized incomplete
/// beta identity. Symmetric around 0.
fn student_t_cdf(t: f64, df: f64) -> f64 {
    // P(T <= t) = 1 - 0.5 * I_{df/(df+t^2)}(df/2, 1/2) for t >= 0, mirrored.
    let x = df / (df + t * t);
    let ib = betai(df / 2.0, 0.5, x);
    if t >= 0.0 {
        1.0 - 0.5 * ib
    } else {
        0.5 * ib
    }
}

/// Student-t quantile (inverse CDF): the value `q` such that
/// `P(T_df <= q) = p`, for `0 < p < 1` and `df >= 1`. Symmetric around 0.
///
/// Solved by bisection on the monotone t-CDF (robust; no derivative needed).
/// Accuracy ~1e-8 on the target probability. Used by PROC MEANS for the
/// half-width of confidence limits for the mean: t_{1-alpha/2, n-1}.
pub fn t_quantile(p: f64, df: f64) -> f64 {
    if !(0.0..=1.0).contains(&p) {
        return f64::NAN;
    }
    if p == 0.5 {
        return 0.0;
    }
    // Exploit symmetry: solve for the upper tail then mirror.
    let upper = p > 0.5;
    let target = if upper { p } else { 1.0 - p };

    // Bracket the root. The t distribution has heavier tails than normal, so
    // start wide and expand until the CDF brackets `target`.
    let mut lo = 0.0_f64;
    let mut hi = 1.0_f64;
    while student_t_cdf(hi, df) < target && hi < 1e12 {
        hi *= 2.0;
    }

    // Bisection on [lo, hi] (CDF is strictly increasing here).
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        let c = student_t_cdf(mid, df);
        if c < target {
            lo = mid;
        } else {
            hi = mid;
        }
        if (hi - lo) <= 1e-12 * (1.0 + hi.abs()) {
            break;
        }
    }
    let q = 0.5 * (lo + hi);
    if upper {
        q
    } else {
        -q
    }
}

// ───────────────────────── chi-square survival ─────────────────────────
//
// Upper-tail (survival) probability of the chi-square distribution, used by
// PROC FREQ for the CHISQ statistics. Implemented via the regularized upper
// incomplete gamma function Q(a, x) = gammq, following Numerical Recipes
// (series `gser` for x < a+1, continued fraction `gcf` otherwise). Reuses the
// `ln_gamma` already defined above. Accuracy ~1e-10 on the useful range.

/// Series representation of the lower regularized incomplete gamma P(a, x),
/// valid (convergent) for x < a + 1.
fn gser(a: f64, x: f64) -> f64 {
    const ITMAX: usize = 300;
    const EPS: f64 = 3.0e-15;
    if x <= 0.0 {
        return 0.0;
    }
    let gln = ln_gamma(a);
    let mut ap = a;
    let mut sum = 1.0 / a;
    let mut del = sum;
    for _ in 0..ITMAX {
        ap += 1.0;
        del *= x / ap;
        sum += del;
        if del.abs() < sum.abs() * EPS {
            break;
        }
    }
    sum * (-x + a * x.ln() - gln).exp()
}

/// Continued-fraction representation of the upper regularized incomplete gamma
/// Q(a, x) (Lentz's algorithm), valid (convergent) for x >= a + 1.
fn gcf(a: f64, x: f64) -> f64 {
    const ITMAX: usize = 300;
    const EPS: f64 = 3.0e-15;
    const FPMIN: f64 = 1.0e-300;
    let gln = ln_gamma(a);
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / FPMIN;
    let mut d = 1.0 / b;
    let mut h = d;
    for i in 1..=ITMAX {
        let an = -(i as f64) * (i as f64 - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = b + an / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            break;
        }
    }
    (-x + a * x.ln() - gln).exp() * h
}

/// Regularized upper incomplete gamma function Q(a, x) = 1 - P(a, x).
fn gammq(a: f64, x: f64) -> f64 {
    if x < 0.0 || a <= 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return 1.0;
    }
    if x < a + 1.0 {
        1.0 - gser(a, x)
    } else {
        gcf(a, x)
    }
}

/// Upper-tail (survival) probability of the chi-square distribution with `df`
/// degrees of freedom evaluated at `x`: P(X²_df > x) = Q(df/2, x/2). Returns
/// 1.0 at x <= 0 and ~0 for large x. Accuracy ~1e-10.
pub(crate) fn chisq_sf(x: f64, df: f64) -> f64 {
    if df <= 0.0 {
        return f64::NAN;
    }
    if x <= 0.0 {
        return 1.0;
    }
    gammq(df / 2.0, x / 2.0)
}

// ───────────────────────── normal CDF / combinatorics ─────────────────────
//
// Helpers used by PROC FREQ's advanced statistics (Fisher exact test via
// hypergeometric probabilities, Cochran-Armitage trend test via the standard
// normal CDF). All numeric, no external crate.

/// Error function erf(x), via the regularized lower incomplete gamma
/// P(1/2, x²). Reuses the `ln_gamma`-based `gser`/`gcf` machinery above.
fn erf(x: f64) -> f64 {
    if x == 0.0 {
        return 0.0;
    }
    // P(1/2, x²) = lower regularized incomplete gamma = 1 - Q(1/2, x²).
    let p = 1.0 - gammq(0.5, x * x);
    if x > 0.0 {
        p
    } else {
        -p
    }
}

/// Standard normal CDF Φ(z) = P(Z <= z), matching SAS PROBNORM. Accuracy
/// ~1e-10 over the useful range.
pub fn probnorm(z: f64) -> f64 {
    0.5 * (1.0 + erf(z / std::f64::consts::SQRT_2))
}

/// Natural log of n! = ln Γ(n+1), for n >= 0.
pub fn ln_factorial(n: u64) -> f64 {
    ln_gamma(n as f64 + 1.0)
}

/// Natural log of the binomial coefficient C(n, k). Returns -inf when
/// k > n (coefficient 0).
pub fn ln_choose(n: u64, k: u64) -> f64 {
    if k > n {
        return f64::NEG_INFINITY;
    }
    ln_factorial(n) - ln_factorial(k) - ln_factorial(n - k)
}

/// A resolved BY variable: dataset column index, declared DESCENDING flag,
/// and the variable name (display, original case).
pub struct ByCol {
    pub col_idx: usize,
    pub descending: bool,
    pub name: String,
}

/// Resolve a BY var list against a dataset, validating each name exists.
/// `by` is the parsed (name, descending) list.
pub fn resolve_by_cols(ds: &SasDataset, by: &[(String, bool)]) -> Result<Vec<ByCol>> {
    let mut cols = Vec::with_capacity(by.len());
    for (vname, descending) in by {
        match ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(vname))
        {
            Some(i) => cols.push(ByCol {
                col_idx: i,
                descending: *descending,
                name: ds.vars[i].name.clone(),
            }),
            None => {
                return Err(SasError::runtime(format!(
                    "Variable {} not found.",
                    vname.to_uppercase()
                )));
            }
        }
    }
    Ok(cols)
}

/// Render a BY-key cell value (for the heading line / error message).
fn by_cell(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Missing(k) => k.display(),
        Value::Char(s) => s.trim_end().to_string(),
    }
}

/// Verify the rows are sorted by the BY key (per each var's direction) using
/// `sas_cmp`, then partition the rows (in their existing order) into
/// contiguous BY groups. Returns one (key values, row indices) pair per group
/// in input order.
///
/// On the first out-of-order adjacent pair, returns the SAS error
/// `Data set <display> is not sorted in ascending sequence. ...`.
pub fn by_groups(
    by_values: &[Vec<Value>],
    descending: &[bool],
    n_obs: usize,
    by_names: &[String],
    display: &str,
) -> Result<Vec<(Vec<Value>, Vec<usize>)>> {
    // Compare the BY key of two rows honoring per-key direction.
    let cmp = |a: usize, b: usize| -> Ordering {
        for (k, col) in by_values.iter().enumerate() {
            let mut c = col[a].sas_cmp(&col[b]);
            if descending[k] {
                c = c.reverse();
            }
            if c != Ordering::Equal {
                return c;
            }
        }
        Ordering::Equal
    };

    let mut groups: Vec<(Vec<Value>, Vec<usize>)> = Vec::new();
    for row in 0..n_obs {
        if row > 0 {
            match cmp(row - 1, row) {
                Ordering::Greater => {
                    // Find the first key var that differs to name it in the error.
                    let prev = row - 1;
                    let (vname, v1, v2) = by_values
                        .iter()
                        .enumerate()
                        .find_map(|(k, col)| {
                            if col[prev].sas_cmp(&col[row]) != Ordering::Equal {
                                Some((
                                    by_names[k].clone(),
                                    by_cell(&col[prev]),
                                    by_cell(&col[row]),
                                ))
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| {
                            (by_names[0].clone(), by_cell(&by_values[0][prev]), by_cell(&by_values[0][row]))
                        });
                    return Err(SasError::runtime(format!(
                        "Data set {display} is not sorted in ascending sequence. \
                         The current BY group has {vname}={v1} and the next BY group has {vname}={v2}."
                    )));
                }
                Ordering::Equal => {
                    // Same group as the previous row.
                    let key: Vec<Value> = by_values.iter().map(|c| c[row].clone()).collect();
                    let _ = key; // group key already recorded; just append.
                    groups.last_mut().unwrap().1.push(row);
                    continue;
                }
                Ordering::Less => {}
            }
        }
        let key: Vec<Value> = by_values.iter().map(|c| c[row].clone()).collect();
        groups.push((key, vec![row]));
    }
    Ok(groups)
}

/// Group all rows by the tuple of the given class columns' values, in
/// `sas_cmp` order. Returns (key tuple, row indices) pairs.
pub fn group_by_keys(
    class_values: &[&Vec<Value>],
    n_obs: usize,
) -> Vec<(Vec<Value>, Vec<usize>)> {
    let mut groups: Vec<(Vec<Value>, Vec<usize>)> = Vec::new();
    for row in 0..n_obs {
        let key: Vec<Value> = class_values.iter().map(|c| c[row].clone()).collect();
        // Find an existing group with an equal key (sas_cmp equality).
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
    // Order groups by the key tuple via sas_cmp.
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
