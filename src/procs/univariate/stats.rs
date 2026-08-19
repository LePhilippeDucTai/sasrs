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

/// SAS WEIGHTED skewness g1 (VARDEF=DF). `pairs` are the usable
/// `(value, weight)` pairs (all weights strictly positive), `mean_w` the
/// weighted mean `Σw_i x_i / Σw_i` and `s_w` the weighted standard deviation
/// `√(Σw_i(x_i-mean_w)² / (n-1))` — both already computed by the caller, so the
/// Skewness line stays consistent with the Mean / Std Deviation lines of the
/// same Moments block.
///
/// With `z_i = √w_i · (x_i - mean_w) / s_w`:
/// `g1 = n/((n-1)(n-2)) * Σ z_i^3`.
///
/// `n` is the number of usable OBSERVATIONS, not the sum of the weights. At
/// `w_i ≡ 1` every `√w_i` is 1 and this reduces exactly to [`skewness`].
/// Needs `n>=3` and `s_w>0`, else None.
pub(super) fn weighted_skewness(pairs: &[(f64, f64)], mean_w: f64, s_w: f64) -> Option<f64> {
    let n = pairs.len();
    if n < 3 || s_w <= 0.0 {
        return None;
    }
    let nf = n as f64;
    let sum3: f64 = pairs
        .iter()
        .map(|(x, w)| (w.sqrt() * (x - mean_w) / s_w).powi(3))
        .sum();
    Some(nf / ((nf - 1.0) * (nf - 2.0)) * sum3)
}

/// SAS WEIGHTED excess kurtosis g2 (VARDEF=DF). Same conventions as
/// [`weighted_skewness`]; with `z_i = √w_i · (x_i - mean_w) / s_w`:
///
/// `g2 = [ n(n+1)/((n-1)(n-2)(n-3)) ] * Σ z_i^4 - 3(n-1)^2 / ((n-2)(n-3))`.
///
/// At `w_i ≡ 1` this reduces exactly to [`kurtosis`]. Needs `n>=4` and
/// `s_w>0`, else None.
pub(super) fn weighted_kurtosis(pairs: &[(f64, f64)], mean_w: f64, s_w: f64) -> Option<f64> {
    let n = pairs.len();
    if n < 4 || s_w <= 0.0 {
        return None;
    }
    let nf = n as f64;
    let sum4: f64 = pairs
        .iter()
        .map(|(x, w)| (w.sqrt() * (x - mean_w) / s_w).powi(4))
        .sum();
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
    if best_cnt >= 2 { Some(best_val) } else { None }
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

/// M45.2 — paramètres (μ̂, σ̂) de la loi normale ajustée à une variable
/// d'analyse sur un groupe BY, pour la table « Fitted Normal Distribution ».
///
/// Ce sont EXACTEMENT la Mean et la Std Deviation du bloc Moments de la même
/// variable : non pondérées quand `weights` est `None`, pondérées (VARDEF=DF,
/// mêmes exclusions via `partition_weighted`) sinon. Les recalculer ici plutôt
/// que de les faire remonter par `emit_variable` garde les deux chemins
/// d'émission indépendants ; les formules sont celles des deux `emit_variable*`.
///
/// `None` quand moins de 2 observations utilisables (σ̂ indéfini).
pub(super) fn fitted_normal_params(
    values: &[Value],
    weights: Option<&[Value]>,
    rows: &[usize],
) -> Option<(f64, f64)> {
    match weights {
        Some(wv) => {
            let (pairs, _) = partition_weighted(values, wv, rows);
            let n = pairs.len();
            if n < 2 {
                return None;
            }
            let sum_w: f64 = pairs.iter().map(|(_, w)| *w).sum();
            if sum_w <= 0.0 {
                return None;
            }
            let mean = pairs.iter().map(|(x, w)| w * x).sum::<f64>() / sum_w;
            let css: f64 = pairs.iter().map(|(x, w)| w * (x - mean) * (x - mean)).sum();
            Some((mean, (css / (n as f64 - 1.0)).sqrt()))
        }
        None => {
            let xs: Vec<f64> = rows
                .iter()
                .filter_map(|&r| value_to_num(&values[r]))
                .filter(|f| !f.is_nan())
                .collect();
            if xs.len() < 2 {
                return None;
            }
            let mean = xs.iter().sum::<f64>() / xs.len() as f64;
            Some((mean, sample_std(&xs)?))
        }
    }
}
