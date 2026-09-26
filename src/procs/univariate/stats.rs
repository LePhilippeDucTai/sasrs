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
/// `(value, weight)` pairs (weights ≥ 0, Σw > 0), `mean_w` the weighted
/// mean `Σw_i x_i / Σw_i` and `s_w` the weighted standard deviation
/// `√(Σw_i(x_i-mean_w)² / (Σw_i − 1))` (J03-P2 : diviseur W−1, comme la
/// Variance du bloc Moments) — both already computed by the caller, so the
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
            // J03-P2 — σ̂ suit la Std Deviation pondérée VARDEF=DF du bloc
            // Moments : diviseur Σw − 1 (oracle weighted_stats).
            let mean = weighted_mean_css(&pairs)?.0;
            let std = weighted_variance(&pairs, VarDef::Df)?.sqrt();
            Some((mean, std))
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
