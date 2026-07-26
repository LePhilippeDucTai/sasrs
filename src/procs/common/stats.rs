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
