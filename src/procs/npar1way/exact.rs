use super::*;

// ─────────────────────── exact Wilcoxon permutation test ───────────────────

/// Maximum pooled sample size for which the exact Wilcoxon permutation
/// distribution is enumerated. Beyond this the DP is skipped (and a NOTE
/// emitted) because C(n, n_0) and the integerized rank-sum range grow large.
pub(super) const EXACT_N_CAP: usize = 30;

/// Exact Wilcoxon two-sample p-values.
#[derive(Debug, Clone)]
pub(super) struct ExactWilcoxon {
    /// One-sided lower probability Pr(S ≤ s_obs).
    pub(super) p_lower: f64,
    /// Two-sided exact probability (|sum2 − mean2| ≥ |obs2 − mean2|).
    pub(super) p_two: f64,
}

/// Compute the exact Wilcoxon permutation distribution of the rank-sum for the
/// first group. `groups` must have k == 2; returns None if beyond `EXACT_N_CAP`.
///
/// Algorithm: integerize the pooled mid-ranks (`w_i = round(2·rank_i)`), then
/// DP `dp[count][sum2]` = number of size-`count` subsets summing to `sum2`.
pub(super) fn exact_wilcoxon(groups: &[Vec<f64>]) -> Option<ExactWilcoxon> {
    if groups.len() != 2 {
        return None;
    }
    let mut pooled: Vec<f64> = Vec::new();
    for g in groups {
        pooled.extend_from_slice(g);
    }
    let n = pooled.len();
    let n0 = groups[0].len();
    if n == 0 || n0 == 0 || n0 == n || n > EXACT_N_CAP {
        return None;
    }

    // Mid-ranks of the pooled sample, integerized to u64 (×2).
    let (ranks, _) = midranks(&pooled);
    let w: Vec<u64> = ranks.iter().map(|&r| (2.0 * r).round() as u64).collect();
    let total_sum2: u64 = w.iter().sum();

    // DP over subset size `count` and integerized sum `sum2`.
    let width = (total_sum2 + 1) as usize;
    // dp[count][sum2] as f64 counts (use f64 to avoid overflow on counts).
    let mut dp = vec![vec![0.0_f64; width]; n0 + 1];
    dp[0][0] = 1.0;
    for &wi in &w {
        let wi_us = wi as usize;
        for count in (1..=n0).rev() {
            // iterate sum2 downward to keep 0/1 knapsack semantics
            for sum2 in (wi_us..width).rev() {
                let add = dp[count - 1][sum2 - wi_us];
                if add != 0.0 {
                    dp[count][sum2] += add;
                }
            }
        }
    }

    let total: f64 = dp[n0].iter().sum();
    if total <= 0.0 {
        return None;
    }

    // Observed integerized rank-sum of group 0. The pooled vector lays group 0
    // first, so its mid-ranks are exactly the first n0 entries of `w`.
    let obs2: u64 = w[..n0].iter().sum();
    // 2·mean rank-sum = n_0·(n+1).
    let mean2 = (n0 as u64) * (n as u64 + 1);
    let dist = if obs2 >= mean2 { obs2 - mean2 } else { mean2 - obs2 };

    let mut lower = 0.0_f64;
    let mut two = 0.0_f64;
    for (sum2, &cnt) in dp[n0].iter().enumerate() {
        if cnt == 0.0 {
            continue;
        }
        let s2 = sum2 as u64;
        if s2 <= obs2 {
            lower += cnt;
        }
        let d = if s2 >= mean2 { s2 - mean2 } else { mean2 - s2 };
        if d >= dist {
            two += cnt;
        }
    }

    Some(ExactWilcoxon {
        p_lower: (lower / total).clamp(0.0, 1.0),
        p_two: (two / total).clamp(0.0, 1.0),
    })
}
