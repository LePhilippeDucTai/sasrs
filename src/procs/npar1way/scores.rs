use super::*;

// ─────────────────── generic linear-rank score framework ───────────────────

/// Raw score `s(p)` for a 1-based integer rank position `p` in a pooled sample
/// of size `n`, for the requested score method (before tie-averaging).
pub(super) fn raw_score(kind: ScoreKind, p: usize, n: usize) -> f64 {
    let pf = p as f64;
    let nf = n as f64;
    match kind {
        ScoreKind::Wilcoxon => pf,
        // 1.0 above the median position, 0.0 at/below it. The exact middle of an
        // odd-n sample (p == (n+1)/2) gets 0.0.
        ScoreKind::Median => {
            if pf > (nf + 1.0) / 2.0 {
                1.0
            } else {
                0.0
            }
        }
        // Savage: (Σ_{j=1}^{p} 1/(n-j+1)) - 1.
        ScoreKind::Savage => {
            let mut acc = 0.0;
            for j in 1..=p {
                acc += 1.0 / (nf - j as f64 + 1.0);
            }
            acc - 1.0
        }
        // Normal / van der Waerden: Φ⁻¹(p / (n+1)).
        ScoreKind::Normal => phi_inv(pf / (nf + 1.0)),
    }
}

/// Tie-averaged per-observation scores aligned with `pooled`, for `kind`.
/// For each tie group spanning integer positions [lo..=hi], every tied
/// observation receives the average of `raw_score(p)` over that span. For
/// Wilcoxon this reproduces the mid-ranks exactly.
pub(super) fn tie_averaged_scores(pooled: &[f64], kind: ScoreKind) -> Vec<f64> {
    let n = pooled.len();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| {
        pooled[a]
            .partial_cmp(&pooled[b])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut scores = vec![0.0_f64; n];
    let mut i = 0usize;
    while i < n {
        let mut j = i + 1;
        while j < n && pooled[idx[j]] == pooled[idx[i]] {
            j += 1;
        }
        // Positions i..j (0-based) tie; integer rank positions are (i+1)..=j.
        let mut sum = 0.0;
        for p in (i + 1)..=j {
            sum += raw_score(kind, p, n);
        }
        let avg = sum / (j - i) as f64;
        for &o in &idx[i..j] {
            scores[o] = avg;
        }
        i = j;
    }
    scores
}

/// A generic linear-rank score analysis for one VAR across k groups.
#[derive(Debug, Clone)]
pub(super) struct ScoreAnalysis {
    /// Total non-missing observations.
    pub(super) n: usize,
    /// Number of groups.
    pub(super) k: usize,
    /// Per-group score sum `S_j` (in sas_cmp group order).
    pub(super) s: Vec<f64>,
    /// Per-group size `n_j`.
    pub(super) n_j: Vec<usize>,
    /// Mean score `ā`.
    pub(super) abar: f64,
    /// `SS = Σ(a_i − ā)²`.
    pub(super) ss: f64,
}

/// Compute the generic linear-rank score sums for `kind` over `groups`.
pub(super) fn score_analysis(groups: &[Vec<f64>], kind: ScoreKind) -> ScoreAnalysis {
    let k = groups.len();
    let mut pooled: Vec<f64> = Vec::new();
    let mut owner: Vec<usize> = Vec::new();
    for (gi, g) in groups.iter().enumerate() {
        for &v in g {
            pooled.push(v);
            owner.push(gi);
        }
    }
    let n = pooled.len();
    let scores = tie_averaged_scores(&pooled, kind);

    let abar = if n > 0 {
        scores.iter().sum::<f64>() / n as f64
    } else {
        0.0
    };
    let ss: f64 = scores.iter().map(|a| (a - abar) * (a - abar)).sum();

    let mut s = vec![0.0_f64; k];
    let mut n_j = vec![0usize; k];
    for r in 0..n {
        s[owner[r]] += scores[r];
        n_j[owner[r]] += 1;
    }

    ScoreAnalysis {
        n,
        k,
        s,
        n_j,
        abar,
        ss,
    }
}

/// 2-sample linear-rank statistic (k == 2), shaped like the Wilcoxon table.
#[derive(Debug, Clone)]
pub(super) struct ScoreTwoSample {
    /// Score sum of the first group (`S_0`).
    pub(super) stat: f64,
    /// Mean under H0 (`n_0·ā`).
    pub(super) mean: f64,
    /// Standard deviation under H0.
    pub(super) sd: f64,
    /// Standardized statistic (with continuity correction).
    pub(super) z: f64,
    /// Two-sided normal-approximation p-value.
    pub(super) p2: f64,
}

/// k-sample one-way χ² statistic from a generic score analysis.
#[derive(Debug, Clone)]
pub(super) struct ScoreOneWay {
    pub(super) chisq: f64,
    pub(super) df: usize,
    pub(super) p: f64,
}

/// 2-sample statistic for a score analysis. Returns None unless k == 2 and the
/// score variance `SS` is positive.
pub(super) fn score_two_sample(a: &ScoreAnalysis) -> Option<ScoreTwoSample> {
    if a.k != 2 || a.n_j[0] == 0 || a.n_j[1] == 0 || a.ss <= 0.0 {
        return None;
    }
    let n = a.n as f64;
    let n0 = a.n_j[0] as f64;
    let n1 = a.n_j[1] as f64;
    let stat = a.s[0];
    let mean = n0 * a.abar;
    let var = (n0 * n1) / (n * (n - 1.0)) * a.ss;
    if var <= 0.0 {
        return None;
    }
    let sd = var.sqrt();
    // SAS default continuity correction: shift |diff| by 0.5 toward the mean.
    let diff = stat - mean;
    let z = (diff.abs() - 0.5) / sd * diff.signum();
    let cdf = probnorm(z);
    let p2 = (2.0 * cdf.min(1.0 - cdf)).clamp(0.0, 1.0);
    Some(ScoreTwoSample {
        stat,
        mean,
        sd,
        z,
        p2,
    })
}

/// k-sample one-way χ² statistic for a score analysis.
pub(super) fn score_one_way(a: &ScoreAnalysis) -> ScoreOneWay {
    let n = a.n as f64;
    let df = a.k.saturating_sub(1);
    let chisq = if a.ss > 0.0 && a.n >= 2 {
        let mut acc = 0.0;
        for j in 0..a.k {
            if a.n_j[j] > 0 {
                let d = a.s[j] - a.n_j[j] as f64 * a.abar;
                acc += d * d / a.n_j[j] as f64;
            }
        }
        (n - 1.0) / a.ss * acc
    } else {
        f64::NAN
    };
    let p = if df >= 1 && chisq.is_finite() {
        (1.0 - chisq_cdf(chisq, df as f64)).clamp(0.0, 1.0)
    } else {
        f64::NAN
    };
    ScoreOneWay { chisq, df, p }
}
