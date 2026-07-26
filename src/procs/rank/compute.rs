use super::*;

// ───────────────────────── ranking core ─────────────────────────

/// Compute the rank/group/score output for one decoded column.
///
/// Returns a vector of `Value` aligned to the input rows: `Value::Num(..)`
/// for non-missing input cells, `Value::missing()` for missing cells.
///
/// `groups` (GROUPS=) takes priority over `method`; otherwise the TIES-adjusted
/// ordinary rank is transformed per `method`.
pub(super) fn rank_column(
    col: &[Value],
    descending: bool,
    ties: Ties,
    groups: Option<usize>,
    method: Method,
) -> Vec<Value> {
    let n = col.len();

    // Indices of non-missing cells (special missings are NaN via value_to_num).
    let mut idx: Vec<usize> = Vec::with_capacity(n);
    for (i, v) in col.iter().enumerate() {
        match value_to_num(v) {
            Some(f) if !f.is_nan() => idx.push(i),
            _ => {}
        }
    }
    let k = idx.len();

    // Stable sort the non-missing indices via sas_cmp (DESCENDING reverses).
    idx.sort_by(|&a, &b| {
        let c = col[a].sas_cmp(&col[b]);
        if descending { c.reverse() } else { c }
    });

    // Output buffer; missing cells stay missing.
    let mut out = vec![Value::missing(); n];
    if k == 0 {
        return out;
    }

    // SAVAGE needs the cumulative reverse-harmonic per ordinal. Precompute
    // s_m = (sum_{j=k-m+1}^{k} 1/j) - 1 for m = 1..=k.
    let savage = matches!(method, Method::Savage);
    let savage_scores: Vec<f64> = if savage {
        let mut acc = 0.0;
        let mut v = Vec::with_capacity(k);
        // m=1 adds 1/k, m=2 adds 1/(k-1), ... m=k adds 1/1.
        for m in 1..=k {
            acc += 1.0 / (k - m + 1) as f64;
            v.push(acc - 1.0);
        }
        v
    } else {
        Vec::new()
    };

    // Walk the sorted order, grouping consecutive equal values (sas_cmp).
    // For each tie group occupying ordinal positions lo..=hi (1-based), assign
    // a group number (GROUPS=) or the (possibly transformed) rank/score.
    let mut pos = 0usize; // 0-based offset into `idx`
    let mut dense_rank = 0usize; // consecutive distinct-value counter (DENSE)
    while pos < k {
        let mut end = pos + 1;
        while end < k && col[idx[end]].sas_cmp(&col[idx[pos]]) == Ordering::Equal {
            end += 1;
        }
        let lo = pos + 1; // 1-based first ordinal of the tie group
        let hi = end; // 1-based last ordinal of the tie group
        dense_rank += 1;

        let value = match groups {
            Some(ng) => {
                // SAS group formula on the LOW ordinal rank (ties share it):
                // group = floor(n_groups * r / (k + 1)), clamped to 0..n-1.
                let r = lo;
                let g = (ng * r) / (k + 1);
                let g = g.min(ng - 1);
                g as f64
            }
            None if savage => {
                // Savage scores: aggregate the per-ordinal scores over the tie
                // group according to TIES (MEAN → average, LOW/HIGH → endpoint
                // ordinal's score, DENSE → LOW ordinal's score).
                match ties {
                    Ties::Mean => {
                        let sum: f64 = savage_scores[pos..end].iter().sum();
                        sum / (end - pos) as f64
                    }
                    Ties::Low | Ties::Dense => savage_scores[lo - 1],
                    Ties::High => savage_scores[hi - 1],
                }
            }
            None => {
                // TIES-adjusted ordinary rank, then the method transform.
                let r = match ties {
                    Ties::Mean => (lo + hi) as f64 / 2.0,
                    Ties::Low => lo as f64,
                    Ties::High => hi as f64,
                    Ties::Dense => dense_rank as f64,
                };
                transform_rank(r, k, method)
            }
        };

        for &orig in &idx[pos..end] {
            out[orig] = Value::Num(value);
        }
        pos = end;
    }

    out
}

/// Transform a TIES-adjusted ordinary rank `r` (over `k` non-missing values)
/// per the ranking `method`. SAVAGE is handled in `rank_column` (it needs the
/// ordinal, not just `r`); this covers RANK/FRACTION/NPLUS1/PERCENT/NORMAL.
pub(super) fn transform_rank(r: f64, k: usize, method: Method) -> f64 {
    let kf = k as f64;
    match method {
        Method::Rank => r,
        Method::Fraction => r / kf,
        Method::NPlus1 => r / (kf + 1.0),
        Method::Percent => 100.0 * r / kf,
        Method::Normal(score) => {
            let y = match score {
                NormalScore::Blom => (r - 0.375) / (kf + 0.25),
                NormalScore::Tukey => (r - 1.0 / 3.0) / (kf + 1.0 / 3.0),
                NormalScore::Vw => r / (kf + 1.0),
            };
            phi_inv(y)
        }
        // SAVAGE never reaches here (handled in rank_column).
        Method::Savage => r,
    }
}

/// Build an f64 Polars series from rank `Value`s (missing → null/NaN-payload
/// via `value_to_num`).
pub(super) fn rank_series(name: &str, values: &[Value], n_obs: usize) -> Series {
    debug_assert_eq!(values.len(), n_obs);
    let data: Vec<Option<f64>> = values.iter().map(value_to_num).collect();
    Series::new(name.into(), data)
}
