use super::*;

// ───────────────────────── k-means core ─────────────────────────

/// Result of running k-means.
pub struct KMeansResult {
    /// Cluster assignment (1..=k) per observation.
    pub assign: Vec<usize>,
    /// Final centroids.
    pub centroids: Vec<Vec<f64>>,
}

/// Pick k initial seeds by farthest-first: seed 1 = first observation, each
/// subsequent seed = observation maximizing its minimum distance to the
/// already-chosen seeds. Returns observation indices.
pub(super) fn farthest_first_seeds(coords: &[Vec<f64>], k: usize) -> Vec<usize> {
    let n = coords.len();
    let mut seeds = vec![0usize];
    while seeds.len() < k && seeds.len() < n {
        let mut best_idx = 0usize;
        let mut best_d = -1.0_f64;
        for i in 0..n {
            if seeds.contains(&i) {
                continue;
            }
            let mind = seeds
                .iter()
                .map(|&s| distance(DistMethod::Euclid, &coords[i], &coords[s]))
                .fold(f64::INFINITY, f64::min);
            if mind > best_d {
                best_d = mind;
                best_idx = i;
            }
        }
        seeds.push(best_idx);
    }
    seeds
}

/// Run k-means with farthest-first seeding.
pub fn kmeans(coords: &[Vec<f64>], k: usize, maxiter: usize, converge: f64) -> KMeansResult {
    let n = coords.len();
    let p = if n > 0 { coords[0].len() } else { 0 };
    let k = k.min(n).max(1);

    let seeds = farthest_first_seeds(coords, k);
    let mut centroids: Vec<Vec<f64>> = seeds.iter().map(|&s| coords[s].clone()).collect();
    let mut assign = vec![0usize; n];

    // Overall RMS std scale for the convergence threshold.
    let rms_scale = overall_rms_std(coords).max(1e-12);

    let iters = maxiter.max(1);
    for _ in 0..iters {
        // Assign.
        for i in 0..n {
            let mut best = 0usize;
            let mut bestd = f64::INFINITY;
            for (c, cent) in centroids.iter().enumerate() {
                let d = distance(DistMethod::Euclid, &coords[i], cent);
                if d < bestd {
                    bestd = d;
                    best = c;
                }
            }
            assign[i] = best;
        }

        // Recompute centroids.
        let mut sums = vec![vec![0.0_f64; p]; k];
        let mut counts = vec![0usize; k];
        for i in 0..n {
            let c = assign[i];
            counts[c] += 1;
            for j in 0..p {
                sums[c][j] += coords[i][j];
            }
        }
        let mut max_move = 0.0_f64;
        for c in 0..k {
            if counts[c] == 0 {
                continue;
            }
            let mut newc = vec![0.0_f64; p];
            for j in 0..p {
                newc[j] = sums[c][j] / counts[c] as f64;
            }
            let move_d = distance(DistMethod::Euclid, &newc, &centroids[c]);
            max_move = max_move.max(move_d);
            centroids[c] = newc;
        }

        if max_move < converge * rms_scale {
            // Re-assign once more against the updated centroids to settle.
            for i in 0..n {
                let mut best = 0usize;
                let mut bestd = f64::INFINITY;
                for (c, cent) in centroids.iter().enumerate() {
                    let d = distance(DistMethod::Euclid, &coords[i], cent);
                    if d < bestd {
                        bestd = d;
                        best = c;
                    }
                }
                assign[i] = best;
            }
            break;
        }
    }

    KMeansResult {
        assign: assign.iter().map(|&c| c + 1).collect(),
        centroids,
    }
}

/// Overall pooled RMS std across all variables (n-1 per variable, averaged).
pub(super) fn overall_rms_std(coords: &[Vec<f64>]) -> f64 {
    let n = coords.len();
    if n < 2 {
        return 0.0;
    }
    let p = coords[0].len();
    let mut ss_sum = 0.0_f64;
    for j in 0..p {
        let mean = coords.iter().map(|r| r[j]).sum::<f64>() / n as f64;
        let ss: f64 = coords.iter().map(|r| (r[j] - mean).powi(2)).sum();
        ss_sum += ss;
    }
    (ss_sum / ((n - 1) as f64 * p as f64)).sqrt()
}

pub(super) fn ratio(rsq: f64) -> f64 {
    if (1.0 - rsq).abs() < 1e-12 {
        f64::INFINITY
    } else {
        rsq / (1.0 - rsq)
    }
}

/// Pooled RMS std within one cluster (denominator = freq, across all vars).
pub(super) fn cluster_rms_std(coords: &[Vec<f64>], assign: &[usize], cluster: usize, centroid: &[f64]) -> f64 {
    let p = centroid.len();
    let mut ss = 0.0_f64;
    let mut cnt = 0usize;
    for (i, &c) in assign.iter().enumerate() {
        if c == cluster {
            cnt += 1;
            for j in 0..p {
                ss += (coords[i][j] - centroid[j]).powi(2);
            }
        }
    }
    if cnt == 0 {
        0.0
    } else {
        (ss / (cnt * p) as f64).sqrt()
    }
}

/// Per-variable Total STD (n-1), Within STD (pooled), R-Square, and overall.
pub(super) fn variable_rsquare(
    coords: &[Vec<f64>],
    assign: &[usize],
    centroids: &[Vec<f64>],
    p: usize,
) -> (Vec<f64>, f64, Vec<f64>, Vec<f64>) {
    let n = coords.len();
    let mut total_ss = vec![0.0_f64; p];
    let mut within_ss = vec![0.0_f64; p];
    for j in 0..p {
        let mean = coords.iter().map(|r| r[j]).sum::<f64>() / n as f64;
        for (i, &c) in assign.iter().enumerate() {
            total_ss[j] += (coords[i][j] - mean).powi(2);
            within_ss[j] += (coords[i][j] - centroids[c - 1][j]).powi(2);
        }
    }
    let total_std: Vec<f64> = total_ss
        .iter()
        .map(|s| if n > 1 { (s / (n - 1) as f64).sqrt() } else { 0.0 })
        .collect();
    let k = centroids.len();
    let within_df = (n.saturating_sub(k)).max(1) as f64;
    let within_std: Vec<f64> = within_ss.iter().map(|s| (s / within_df).sqrt()).collect();
    let var_rsq: Vec<f64> = (0..p)
        .map(|j| {
            if total_ss[j] > 0.0 {
                1.0 - within_ss[j] / total_ss[j]
            } else {
                0.0
            }
        })
        .collect();
    let tot_sum: f64 = total_ss.iter().sum();
    let wit_sum: f64 = within_ss.iter().sum();
    let overall = if tot_sum > 0.0 {
        1.0 - wit_sum / tot_sum
    } else {
        0.0
    };
    (var_rsq, overall, total_std, within_std)
}
