use super::*;

// ───────────────────────── clustering core ─────────────────────────

/// One recorded merge step.
#[derive(Debug, Clone)]
pub struct MergeStep {
    pub ncl: usize,
    pub joined_a: String,
    pub joined_b: String,
    pub freq: usize,
    pub sprsq: f64,
    pub rsq: f64,
}

/// An active cluster during agglomeration.
pub(super) struct ClusterNode {
    pub(super) members: Vec<usize>,
    pub(super) centroid: Vec<f64>,
    /// Display label: "OB<i>" for a singleton, "CL<ncl>" for a composite.
    pub(super) label: String,
}

/// Run agglomerative clustering on `coords` (one vector per observation),
/// returning the merge history (NCl from n-1 down to 1).
///
/// `labels` provides the singleton display labels (e.g. ID values or "OB1").
pub fn agglomerate(coords: &[Vec<f64>], method: LinkMethod, labels: &[String]) -> Vec<MergeStep> {
    let n = coords.len();
    let p = if n > 0 { coords[0].len() } else { 0 };

    // Total sum of squared deviations from the global mean (all vars).
    let mut gmean = vec![0.0_f64; p];
    for row in coords {
        for j in 0..p {
            gmean[j] += row[j];
        }
    }
    for m in &mut gmean {
        *m /= n as f64;
    }
    let mut ss_total = 0.0_f64;
    for row in coords {
        for j in 0..p {
            let d = row[j] - gmean[j];
            ss_total += d * d;
        }
    }

    // Initialize clusters (singletons).
    let mut clusters: Vec<Option<ClusterNode>> = coords
        .iter()
        .enumerate()
        .map(|(i, c)| {
            Some(ClusterNode {
                members: vec![i],
                centroid: c.clone(),
                label: labels[i].clone(),
            })
        })
        .collect();

    // Pairwise dissimilarities between active clusters. dmat[i][j] is the
    // linkage-criterion value (for Ward = the merge cost = ΔSS).
    let mut dmat = vec![vec![0.0_f64; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let v = pair_criterion(
                method,
                clusters[i].as_ref().unwrap(),
                clusters[j].as_ref().unwrap(),
                coords,
            );
            dmat[i][j] = v;
            dmat[j][i] = v;
        }
    }

    let mut active: Vec<usize> = (0..n).collect();
    let mut history: Vec<MergeStep> = Vec::new();
    let mut ss_within = 0.0_f64;
    let denom = if ss_total != 0.0 { ss_total } else { 1.0 };

    // n-1 merges. After each merge, NCl = number of remaining clusters.
    for step in 0..n.saturating_sub(1) {
        // Find the closest pair, scanning ascending (i<j); strict-less replace.
        let mut best: Option<(usize, usize, f64)> = None;
        for ai in 0..active.len() {
            for bj in (ai + 1)..active.len() {
                let i = active[ai];
                let j = active[bj];
                let (lo, hi) = if i < j { (i, j) } else { (j, i) };
                let v = dmat[lo][hi];
                match best {
                    None => best = Some((lo, hi, v)),
                    Some((_, _, bv)) if v < bv => best = Some((lo, hi, v)),
                    _ => {}
                }
            }
        }
        let (i, j, _crit) = best.expect("at least one pair while active>1");

        // ΔSS from this merge (Ward formula = exact within-SS increase).
        let ci = clusters[i].as_ref().unwrap();
        let cj = clusters[j].as_ref().unwrap();
        let ni = ci.members.len() as f64;
        let nj = cj.members.len() as f64;
        let d2 = squared_centroid_distance(&ci.centroid, &cj.centroid);
        let delta_ss = (ni * nj) / (ni + nj) * d2;
        ss_within += delta_ss;

        let ncl_after = active.len() - 1;

        // New merged cluster.
        let mut members = ci.members.clone();
        members.extend_from_slice(&cj.members);
        let new_n = members.len() as f64;
        let centroid: Vec<f64> = ci
            .centroid
            .iter()
            .zip(cj.centroid.iter())
            .take(p)
            .map(|(a, b)| (ni * a + nj * b) / new_n)
            .collect();
        let label = if ncl_after == 0 {
            "CL1".to_string()
        } else {
            format!("CL{}", ncl_after)
        };

        let joined_a = ci.label.clone();
        let joined_b = cj.label.clone();

        history.push(MergeStep {
            ncl: ncl_after,
            joined_a,
            joined_b,
            freq: members.len(),
            sprsq: delta_ss / denom,
            rsq: 1.0 - ss_within / denom,
        });

        // Merge j into i; remove j.
        clusters[i] = Some(ClusterNode {
            members,
            centroid,
            label,
        });
        clusters[j] = None;
        active.retain(|&x| x != j);

        // Recompute distances from the new cluster i to all other active.
        for &k in &active {
            if k == i {
                continue;
            }
            let v = pair_criterion(
                method,
                clusters[i].as_ref().unwrap(),
                clusters[k].as_ref().unwrap(),
                coords,
            );
            let (lo, hi) = if i < k { (i, k) } else { (k, i) };
            dmat[lo][hi] = v;
            dmat[hi][lo] = v;
        }
        let _ = step;
    }

    history
}

/// The merge criterion between two clusters for the given linkage method.
///
/// Ward uses the centroid-based ΔSS. Single/Complete/Average are computed
/// exactly from the raw inter-observation Euclidean distances (this is the
/// definition; equivalent to the Lance-Williams recurrences).
pub(super) fn pair_criterion(
    method: LinkMethod,
    a: &ClusterNode,
    b: &ClusterNode,
    coords: &[Vec<f64>],
) -> f64 {
    match method {
        LinkMethod::Ward => {
            let na = a.members.len() as f64;
            let nb = b.members.len() as f64;
            (na * nb) / (na + nb) * squared_centroid_distance(&a.centroid, &b.centroid)
        }
        LinkMethod::Single | LinkMethod::Complete | LinkMethod::Average => {
            let mut acc = match method {
                LinkMethod::Single => f64::INFINITY,
                LinkMethod::Complete => f64::NEG_INFINITY,
                _ => 0.0,
            };
            for &ia in &a.members {
                for &ib in &b.members {
                    let d = euclid(&coords[ia], &coords[ib]);
                    match method {
                        LinkMethod::Single => acc = acc.min(d),
                        LinkMethod::Complete => acc = acc.max(d),
                        _ => acc += d,
                    }
                }
            }
            if method == LinkMethod::Average {
                acc /= (a.members.len() * b.members.len()) as f64;
            }
            acc
        }
    }
}

pub(super) fn squared_centroid_distance(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum()
}

pub(super) fn euclid(a: &[f64], b: &[f64]) -> f64 {
    squared_centroid_distance(a, b).sqrt()
}
