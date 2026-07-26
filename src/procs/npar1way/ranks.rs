use super::*;

// ───────────────────────── numeric core ─────────────────────────

/// Wilcoxon two-sample rank-sum statistics (computed only for k=2).
#[derive(Debug, Clone)]
pub(super) struct WilcoxonResult {
    /// Rank sum of group 0 (the first group in sas_cmp order).
    pub(super) w: f64,
    /// Expected value of `w` under H0.
    pub(super) ew: f64,
    /// Variance of `w` (tie-corrected).
    pub(super) var_w: f64,
    /// Standardized statistic `(w - ew) / sqrt(var_w)`.
    pub(super) z: f64,
    /// Two-sided normal-approximation p-value.
    pub(super) p: f64,
}

/// Kruskal-Wallis statistics (always computed for k≥2).
#[derive(Debug, Clone)]
pub(super) struct KruskalResult {
    /// Tie-corrected H statistic.
    pub(super) h: f64,
    /// Degrees of freedom (k-1).
    pub(super) df: usize,
    /// Upper-tail chi-square p-value.
    pub(super) p: f64,
}

/// Combined non-parametric analysis of one VAR across the CLASS groups.
#[derive(Debug, Clone)]
pub(super) struct NparResult {
    /// Total non-missing observations.
    pub(super) n: usize,
    /// Tie-correction factor `1 - Σ(t³-t)/(n³-n)`.
    pub(super) tie_factor: f64,
    /// Wilcoxon result (only when `k == 2`).
    pub(super) wilcoxon: Option<WilcoxonResult>,
    /// Kruskal-Wallis result.
    pub(super) kruskal: KruskalResult,
}

/// Assign mid-ranks (1-based) to a slice of values, averaging ties.
///
/// Returns a vector `ranks` aligned with `values` (same order), and the list of
/// tie-group sizes (for the tie correction).
pub(super) fn midranks(values: &[f64]) -> (Vec<f64>, Vec<usize>) {
    let n = values.len();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| {
        values[a]
            .partial_cmp(&values[b])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut ranks = vec![0.0_f64; n];
    let mut tie_sizes: Vec<usize> = Vec::new();
    let mut i = 0usize;
    while i < n {
        let mut j = i + 1;
        while j < n && values[idx[j]] == values[idx[i]] {
            j += 1;
        }
        // Positions i..j (0-based) share the same value; ranks are i+1..=j.
        let group = j - i;
        // Average of ranks (i+1)..=j = (i+1 + j) / 2.
        let midrank = ((i + 1) + j) as f64 / 2.0;
        for &k in &idx[i..j] {
            ranks[k] = midrank;
        }
        if group > 1 {
            tie_sizes.push(group);
        }
        i = j;
    }
    (ranks, tie_sizes)
}

/// Core numeric routine. `groups[i]` holds the missing-excluded numeric values
/// of CLASS level `i`, in sas_cmp order. Pools all observations, ranks them with
/// mid-ranks, then computes the Kruskal-Wallis statistic (always) and the
/// Wilcoxon rank-sum statistic (when there are exactly two groups).
pub(super) fn analyze(groups: &[Vec<f64>]) -> NparResult {
    let k = groups.len();
    // Flatten, keeping track of which group each pooled value belongs to.
    let mut pooled: Vec<f64> = Vec::new();
    let mut owner: Vec<usize> = Vec::new();
    for (gi, g) in groups.iter().enumerate() {
        for &v in g {
            pooled.push(v);
            owner.push(gi);
        }
    }
    let n = pooled.len();

    let (ranks, tie_sizes) = midranks(&pooled);

    // Tie correction: 1 - Σ(t³ - t) / (n³ - n).
    let nf = n as f64;
    let denom = nf * nf * nf - nf;
    let tie_factor = if denom > 0.0 {
        let s: f64 = tie_sizes
            .iter()
            .map(|&t| {
                let tf = t as f64;
                tf * tf * tf - tf
            })
            .sum();
        1.0 - s / denom
    } else {
        1.0
    };

    // Rank sums and sizes per group.
    let mut rank_sum = vec![0.0_f64; k];
    let mut n_i = vec![0usize; k];
    for r in 0..n {
        rank_sum[owner[r]] += ranks[r];
        n_i[owner[r]] += 1;
    }

    // Kruskal-Wallis: H = [12/(n(n+1))] Σ R_i²/n_i - 3(n+1), corrected by /tie_factor.
    let kruskal = {
        let h_raw = if n >= 1 {
            let mut s = 0.0_f64;
            for i in 0..k {
                if n_i[i] > 0 {
                    s += rank_sum[i] * rank_sum[i] / n_i[i] as f64;
                }
            }
            12.0 / (nf * (nf + 1.0)) * s - 3.0 * (nf + 1.0)
        } else {
            f64::NAN
        };
        let h = if tie_factor > 0.0 {
            h_raw / tie_factor
        } else {
            h_raw
        };
        let df = k.saturating_sub(1);
        let p = if df >= 1 && h.is_finite() {
            (1.0 - chisq_cdf(h, df as f64)).clamp(0.0, 1.0)
        } else {
            f64::NAN
        };
        KruskalResult { h, df, p }
    };

    // Wilcoxon two-sample (only for k == 2).
    let wilcoxon = if k == 2 && n_i[0] > 0 && n_i[1] > 0 {
        let na = n_i[0] as f64;
        let nb = n_i[1] as f64;
        let w = rank_sum[0];
        let ew = na * (nf + 1.0) / 2.0;
        // Var(W) = n_A n_B (n+1) / 12, tie-corrected.
        let var_w = na * nb * (nf + 1.0) / 12.0 * tie_factor;
        let (z, p) = if var_w > 0.0 {
            // SAS applies a 0.5 continuity correction by default (CORRECT=YES).
            let diff = w - ew;
            let z = (diff.abs() - 0.5) / var_w.sqrt() * diff.signum();
            let cdf = probnorm(z);
            let p = (2.0 * cdf.min(1.0 - cdf)).clamp(0.0, 1.0);
            (z, p)
        } else {
            (f64::NAN, f64::NAN)
        };
        Some(WilcoxonResult { w, ew, var_w, z, p })
    } else {
        None
    };

    NparResult {
        n,
        tie_factor,
        wilcoxon,
        kruskal,
    }
}
