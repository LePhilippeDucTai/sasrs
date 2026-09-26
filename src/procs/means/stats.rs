use super::*;

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
        // SAS unweighted SumWgt = N (chaque observation pèse 1).
        "sumwgt" => Value::Num(n as f64),
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

/// Weighted analogue of `compute` (J03-P2). `pairs` holds the usable
/// (value, weight) pairs of a group (from `common::partition_weighted_strict`);
/// `n_missing` is the weighted NMiss (x manquants à poids valide). `vardef`
/// selects the variance divisor: DF → Σw − 1 (défaut SAS), WEIGHT →
/// Σw − Σw²/Σw (`common::weighted_variance`).
///
/// Formulas: see the file header. MEDIAN / percentiles / QRANGE use the
/// shared weighted Definition 5 (`common::weighted_quantile_def5`).
pub fn compute_weighted(
    stat: &str,
    pairs: &[(f64, f64)],
    n_missing: usize,
    vardef: VarDef,
    alpha: f64,
) -> Value {
    let n = pairs.len();
    let sum_w: f64 = pairs.iter().map(|(_, w)| *w).sum();
    let sum_wx: f64 = pairs.iter().map(|(x, w)| w * x).sum();
    let mean_w = if sum_w != 0.0 {
        Some(sum_wx / sum_w)
    } else {
        None
    };
    // J03-P2 — VARDEF= divisor: DF → W−1, WEIGHT → W−Σw²/W (n ≥ 2 requis).
    let variance = weighted_variance(pairs, vardef);
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
        // SumWgt = Σw_i (SAS PROC MEANS keyword SUMWGT).
        "sumwgt" => Value::Num(sum_w),
        "nmiss" => Value::Num(n_missing as f64),
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
        // J03-P2 — weighted Definition-5 median (shared with UNIVARIATE).
        "median" => {
            let mut sorted: Vec<(f64, f64)> = pairs.to_vec();
            sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
            match weighted_quantile_def5(&sorted, 0.5) {
                Some(m) => Value::Num(m),
                None => Value::missing(),
            }
        }
        // J03-P2 — weighted percentiles / QRANGE via the shared weighted
        // Definition 5 (position by cumulative weight).
        other if percentile_fraction(other).is_some() || other == "qrange" => {
            let mut sorted: Vec<(f64, f64)> = pairs.to_vec();
            sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
            if other == "qrange" {
                return match (
                    weighted_quantile_def5(&sorted, 0.75),
                    weighted_quantile_def5(&sorted, 0.25),
                ) {
                    (Some(q3), Some(q1)) => Value::Num(q3 - q1),
                    _ => Value::missing(),
                };
            }
            let p = percentile_fraction(other).unwrap();
            match weighted_quantile_def5(&sorted, p) {
                Some(q) => Value::Num(q),
                None => Value::missing(),
            }
        }
        _ => Value::missing(),
    }
}

/// Confidence-limit half-width h = t_{1-alpha/2, n-1} * stderr, and the
/// requested single bound. `clm` has no single-value meaning (it is a pair of
/// columns in the listing) → missing here; only `lclm`/`uclm` resolve.
pub(super) fn clm_value(stat: &str, mean: f64, stderr: f64, n: usize, alpha: f64) -> Value {
    let h = clm_halfwidth(stderr, n, alpha);
    match stat {
        "lclm" => Value::Num(mean - h),
        "uclm" => Value::Num(mean + h),
        _ => Value::missing(),
    }
}

/// Half-width of the confidence interval for the mean: t_{1-alpha/2, n-1} *
/// stderr. Requires n>=2 (caller guarantees a finite stderr).
pub(super) fn clm_halfwidth(stderr: f64, n: usize, alpha: f64) -> f64 {
    let df = (n - 1) as f64;
    let t = t_quantile(1.0 - alpha / 2.0, df);
    t * stderr
}

/// Median of the non-missing values (None when empty).
pub(super) fn median(xs: &[f64]) -> Option<f64> {
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
