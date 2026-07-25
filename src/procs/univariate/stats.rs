use super::*;

// ─────────────────────────── statistics helpers ───────────────────────────

/// SAS skewness g1 (needs n>=3 and s>0, else None):
/// `g1 = n/((n-1)(n-2)) * Σ((x_i-mean)/s)^3`.
pub(super) fn skewness(xs: &[f64]) -> Option<f64> {
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
pub(super) fn kurtosis(xs: &[f64]) -> Option<f64> {
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

/// SAS WEIGHTED quantile (default QNTLDEF=5 analog) of fraction `p` over the
/// already-sorted (ascending by value) `(value, weight)` pairs. All weights are
/// strictly positive (the caller has dropped weights ≤ 0 via
/// `partition_weighted`). Empty → None.
///
/// Rule (the weighted analog of the unweighted Definition 5 above): let
/// `W = Σ w_i` be the total weight and `W_i = Σ_{j≤i} w_j` the cumulative weight
/// through the i-th smallest value (1-indexed, `W_0 = 0`). For a target
/// `t = p·W`:
///
/// ```text
/// p == 0 → x(1) (min);  p == 1 → x(n) (max)
/// find the smallest i with W_i ≥ t:
///   if W_i == t exactly:  Q = (x(i) + x(i+1)) / 2   // average at the
///                                                     // discontinuity
///   else:                 Q = x(i)
/// ```
///
/// This reduces to the unweighted Definition 5 when every weight is 1: then
/// `W = n`, `t = n·p`, and `W_i == t` exactly iff `n·p` is an integer (the
/// averaging case), matching `quantile_def5`.
pub(super) fn weighted_quantile_def5(sorted_pairs: &[(f64, f64)], p: f64) -> Option<f64> {
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
        // Use a relative tolerance so integer weights hit the exact-average
        // branch deterministically (mirrors the `g == 0.0` test unweighted).
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

/// Mode: smallest most-frequent value, but only if some value repeats
/// (count >= 2). If every value appears once, SAS reports no mode → None.
/// `sorted` must be ascending.
pub(super) fn mode(sorted: &[f64]) -> Option<f64> {
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
pub(super) fn fmt_num(v: f64) -> String {
    format_best(v, 12)
}

/// Format an optional statistic: None → "." (SAS missing).
pub(super) fn fmt_opt(v: Option<f64>) -> String {
    match v {
        Some(f) => fmt_num(f),
        None => ".".to_string(),
    }
}
