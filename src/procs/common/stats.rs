use super::*;

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
/// are therefore excluded, as are char cells. The second return value is the
/// count of excluded rows (NOT the MEANS weighted NMiss — see
/// [`partition_weighted_strict`]).
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

/// SAS VARDEF= divisor choice for weighted variances (J03-P2).
///
/// - [`VarDef::Df`] (SAS default): divisor = Σw − 1.
/// - [`VarDef::Weight`] (alias WGT): divisor = Σw − Σw²/(Σw).
///
/// Reference (formules pondérées MEAN/VAR/STD, « Dictionary of formulas for
/// PROC MEANS statistical keywords ») :
/// https://documentation.sas.com/doc/en/pgmsascdc/9.4_3.2/proc/n1y2f6nudl7zfjn1joclatu2h3zh.htm
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarDef {
    /// VARDEF=DF (default): divisor = Σw − 1.
    Df,
    /// VARDEF=WEIGHT (WGT): divisor = Σw − Σw²/(Σw).
    Weight,
}

/// Weighted mean and corrected sum of squares over the usable
/// `(value, weight)` pairs: `(Σw x / Σw, Σw(x−x̄_w)²)`. `None` when there
/// are no pairs or Σw ≤ 0.
pub fn weighted_mean_css(pairs: &[(f64, f64)]) -> Option<(f64, f64)> {
    let sum_w: f64 = pairs.iter().map(|(_, w)| *w).sum();
    if pairs.is_empty() || sum_w <= 0.0 {
        return None;
    }
    let mean = pairs.iter().map(|(x, w)| w * x).sum::<f64>() / sum_w;
    let css = pairs
        .iter()
        .map(|(x, w)| w * (x - mean) * (x - mean))
        .sum::<f64>();
    Some((mean, css))
}

/// Weighted variance with the SAS VARDEF= divisor (J03-P2).
///
/// VARDEF=DF divides the weighted corrected SS by `Σw − 1`;
/// VARDEF=WEIGHT by `Σw − Σw²/(Σw)`. Following the SAS « statistics
/// computed » table, VAR/STD need at least TWO usable observations — a
/// single observation yields `None` (missing) even when the divisor is
/// positive. `None` likewise when the divisor is ≤ 0.
pub fn weighted_variance(pairs: &[(f64, f64)], vardef: VarDef) -> Option<f64> {
    if pairs.len() < 2 {
        return None;
    }
    let sum_w: f64 = pairs.iter().map(|(_, w)| *w).sum();
    if sum_w <= 0.0 {
        return None;
    }
    let (_, css) = weighted_mean_css(pairs)?;
    let denom = match vardef {
        VarDef::Df => sum_w - 1.0,
        VarDef::Weight => {
            let sum_w2: f64 = pairs.iter().map(|(_, w)| w * w).sum();
            sum_w - sum_w2 / sum_w
        }
    };
    if denom > 0.0 { Some(css / denom) } else { None }
}

/// SAS WEIGHTED quantile (weighted analog of Definition 5, QNTLDEF=5) of
/// fraction `p` over the already-sorted (ascending by value) `(value,
/// weight)` pairs. Weights are expected to be strictly positive (callers
/// drop the others per the SAS WEIGHT rules); empty → None.
///
/// Rule: `W = Σ w_i` total weight, `W_i` cumulative weight through the i-th
/// smallest value (1-indexed, `W_0 = 0`), target `t = p·W`; find the
/// smallest i with `W_i ≥ t`:
///
/// ```text
/// p == 0 → x(1) (min);  p == 1 → x(n) (max)
/// if W_i == t exactly (and a next value exists):  Q = (x(i) + x(i+1)) / 2
/// else:                                            Q = x(i)
/// ```
///
/// This reduces to the unweighted Definition 5 when every weight is 1
/// (then `W = n` and `W_i == t` iff `n·p` is an integer — the averaging
/// case). Moved to `procs::common` (J03-P2) so MEANS/SUMMARY and UNIVARIATE
/// share the IDENTICAL computation; the tolerance on the exact-boundary
/// test uses a relative epsilon so integer weights hit the averaging branch
/// deterministically.
///
/// Reference: https://support.sas.com/documentation/cdl/en/proc/61895/HTML/default/a002473616.htm
pub fn weighted_quantile_def5(sorted_pairs: &[(f64, f64)], p: f64) -> Option<f64> {
    let n = sorted_pairs.len();
    if n == 0 {
        return None;
    }
    let x = |i: usize| sorted_pairs[i - 1].0; // 1-indexed value accessor

    if p <= 0.0 {
        return Some(x(1));
    }
    if p >= 1.0 {
        return Some(x(n));
    }

    let total_w: f64 = sorted_pairs.iter().map(|(_, w)| *w).sum();
    let t = p * total_w;

    let mut cum = 0.0_f64;
    for i in 1..=n {
        cum += sorted_pairs[i - 1].1;
        // Relative tolerance so integer weights hit the exact-average branch
        // deterministically (mirrors the `g == 0.0` test unweighted).
        if (cum - t).abs() <= 1e-9 * total_w.max(1.0) {
            return if i < n {
                Some((x(i) + x(i + 1)) / 2.0)
            } else {
                Some(x(n))
            };
        }
        if cum > t {
            return Some(x(i));
        }
    }
    Some(x(n))
}

/// Partition rows under the STRICT SAS WEIGHT rules shared by PROC
/// MEANS/SUMMARY and PROC UNIVARIATE with EXCLNPWGT (J03-P2): an
/// observation is usable when the analysis value is non-missing AND the
/// weight is non-missing and > 0 (a nonpositive weight excludes the
/// observation from the analysis, as does a missing weight).
///
/// The returned count is the weighted NMiss analogue of PROC MEANS:
/// observations whose analysis value is missing WHILE the weight is valid
/// (non-missing and > 0). An observation excluded by its weight is neither
/// in N nor in NMiss.
///
/// Reference: https://documentation.sas.com/doc/en/pgmsascdc/9.4_3.2/proc/p1ays1la1f3e2tn1m8owq8h9n1df.htm
pub fn partition_weighted_strict(
    value_col: &[Value],
    weight_col: &[Value],
    rows: &[usize],
) -> (Vec<(f64, f64)>, usize) {
    let mut pairs = Vec::with_capacity(rows.len());
    let mut nmiss = 0usize;
    for &r in rows {
        let v = value_to_num(&value_col[r]);
        let w = value_to_num(&weight_col[r]);
        let v_ok = matches!(v, Some(f) if !f.is_nan());
        let w_ok = matches!(w, Some(f) if !f.is_nan() && f > 0.0);
        match (v_ok, w_ok) {
            (true, true) => pairs.push((v.unwrap(), w.unwrap())),
            // x missing with a VALID weight → weighted NMiss.
            (false, true) => nmiss += 1,
            // Weight missing or ≤ 0 → excluded from the analysis entirely.
            _ => {}
        }
    }
    (pairs, nmiss)
}

/// Partition rows under the DEFAULT PROC UNIVARIATE WEIGHT rules (J03-P2,
/// no EXCLNPWGT): every non-missing analysis value contributes a pair, with
/// an effective weight of `max(w, 0)` — a zero or negative weight counts 0
/// in SUMWGT/moments but the observation STAYS in N (SAS: "If the value is
/// zero, the observation is counted in the total number of observations").
/// A missing analysis value is counted in NMiss.
///
/// Reference: https://support.sas.com/documentation/cdl/en/procstat/63104/HTML/default/procstat_univariate_sect021.htm
pub fn partition_weighted_lax(
    value_col: &[Value],
    weight_col: &[Value],
    rows: &[usize],
) -> (Vec<(f64, f64)>, usize) {
    let mut pairs = Vec::with_capacity(rows.len());
    let mut nmiss = 0usize;
    for &r in rows {
        let v = value_to_num(&value_col[r]);
        match v {
            Some(vf) if !vf.is_nan() => {
                let weff = match value_to_num(&weight_col[r]) {
                    Some(wf) if !wf.is_nan() && wf > 0.0 => wf,
                    _ => 0.0,
                };
                pairs.push((vf, weff));
            }
            _ => nmiss += 1,
        }
    }
    (pairs, nmiss)
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
    if upper { q } else { -q }
}

/// Two-sided p-value Pr(|T_df| > |t|) from a t statistic.
pub fn two_sided_p(t: f64, df: f64) -> f64 {
    (2.0 * (1.0 - student_t_cdf(t.abs(), df))).clamp(0.0, 1.0)
}
