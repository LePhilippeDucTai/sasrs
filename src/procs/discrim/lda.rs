use super::*;

// ───────────────────────── Linear algebra helpers ─────────────────────────

/// Multiply matrix (m×k) by vector (k) → vector (m).
pub(super) fn mat_vec(mat: &[Vec<f64>], vec: &[f64]) -> Vec<f64> {
    mat.iter()
        .map(|row| row.iter().zip(vec.iter()).map(|(a, b)| a * b).sum())
        .collect()
}

/// Inner product of two vectors.
pub(super) fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

// ───────────────────────── Core LDA computation ─────────────────────────

/// Result of fitting the LDA model. All vectors/matrices indexed by class
/// order in `classes`.
pub(super) struct LdaModel {
    pub(super) classes: Vec<Value>,
    pub(super) class_labels: Vec<String>,
    pub(super) counts: Vec<usize>,
    pub(super) priors: Vec<f64>,
    pub(super) means: Vec<Vec<f64>>, // means[k] = centroid of class k (length p)
    pub(super) within_cov: Vec<Vec<Vec<f64>>>, // within_cov[k] = S_k (p×p)
    pub(super) pooled_inv: Vec<Vec<f64>>, // Σ_pooled⁻¹ (p×p)
    pub(super) pooled: Vec<Vec<f64>>, // Σ_pooled (p×p)
    pub(super) coefs: Vec<Vec<f64>>, // coefs[k] = Σ⁻¹ μ_k (length p)
    pub(super) constants: Vec<f64>,  // constants[k]
    pub(super) n_total: usize,
    pub(super) n_groups: usize,
    pub(super) p: usize,
}

impl LdaModel {
    /// Discriminant score for class k at point x.
    pub(super) fn score(&self, k: usize, x: &[f64]) -> f64 {
        dot(x, &self.coefs[k]) + self.constants[k]
    }

    /// Posterior probabilities (softmax over scores) for point x.
    pub(super) fn posteriors(&self, x: &[f64]) -> Vec<f64> {
        let scores: Vec<f64> = (0..self.n_groups).map(|k| self.score(k, x)).collect();
        let max = scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let exps: Vec<f64> = scores.iter().map(|&s| (s - max).exp()).collect();
        let sum: f64 = exps.iter().sum();
        exps.iter().map(|&e| e / sum).collect()
    }

    /// argmax class index by score.
    pub(super) fn classify(&self, x: &[f64]) -> usize {
        let mut best = 0usize;
        let mut best_score = f64::NEG_INFINITY;
        for k in 0..self.n_groups {
            let s = self.score(k, x);
            if s > best_score {
                best_score = s;
                best = k;
            }
        }
        best
    }

    /// Mahalanobis² distance between group i and group j centroids.
    pub(super) fn group_distance(&self, i: usize, j: usize) -> f64 {
        let diff: Vec<f64> = (0..self.p)
            .map(|d| self.means[i][d] - self.means[j][d])
            .collect();
        let tmp = mat_vec(&self.pooled_inv, &diff);
        dot(&diff, &tmp)
    }
}

/// Sample covariance matrix (denominator n-1) for the rows in `data`.
pub(super) fn sample_cov(data: &[Vec<f64>], mean: &[f64]) -> Vec<Vec<f64>> {
    let n = data.len();
    let p = mean.len();
    let mut cov = vec![vec![0.0; p]; p];
    if n < 2 {
        return cov;
    }
    for row in data {
        for a in 0..p {
            for b in 0..p {
                cov[a][b] += (row[a] - mean[a]) * (row[b] - mean[b]);
            }
        }
    }
    let denom = (n - 1) as f64;
    for a in 0..p {
        for b in 0..p {
            cov[a][b] /= denom;
        }
    }
    cov
}

/// Fit the LDA model from class-labeled observations.
/// `obs` : one (class_value, predictor-vector) per complete observation.
pub(super) fn fit_lda(
    classes: Vec<Value>,
    class_obs: &[Vec<Vec<f64>>], // class_obs[k] = rows for class k
    priors_mode: &Priors,
    p: usize,
) -> Result<LdaModel> {
    let n_groups = classes.len();
    let counts: Vec<usize> = class_obs.iter().map(|c| c.len()).collect();
    let n_total: usize = counts.iter().sum();

    // Means per class.
    let means: Vec<Vec<f64>> = class_obs
        .iter()
        .map(|rows| {
            let n = rows.len() as f64;
            let mut m = vec![0.0; p];
            for row in rows {
                for d in 0..p {
                    m[d] += row[d];
                }
            }
            for d in 0..p {
                m[d] /= n;
            }
            m
        })
        .collect();

    // Within-class covariance per class (n_k - 1 denominator).
    let within_cov: Vec<Vec<Vec<f64>>> = class_obs
        .iter()
        .zip(means.iter())
        .map(|(rows, m)| sample_cov(rows, m))
        .collect();

    // Pooled covariance = Σ (n_k - 1) S_k / (N - G).
    let df_within = (n_total as i64 - n_groups as i64).max(1) as f64;
    let mut pooled = vec![vec![0.0; p]; p];
    for (k, sk) in within_cov.iter().enumerate() {
        let w = (counts[k] as i64 - 1).max(0) as f64;
        for a in 0..p {
            for b in 0..p {
                pooled[a][b] += w * sk[a][b];
            }
        }
    }
    for a in 0..p {
        for b in 0..p {
            pooled[a][b] /= df_within;
        }
    }

    let pooled_inv = invert_matrix(&pooled)?;

    // Priors.
    let priors: Vec<f64> = match priors_mode {
        Priors::Equal => vec![1.0 / n_groups as f64; n_groups],
        Priors::Proportional => counts.iter().map(|&c| c as f64 / n_total as f64).collect(),
    };

    // Coefficients and constants.
    let mut coefs: Vec<Vec<f64>> = Vec::with_capacity(n_groups);
    let mut constants: Vec<f64> = Vec::with_capacity(n_groups);
    for k in 0..n_groups {
        let coef_k = mat_vec(&pooled_inv, &means[k]);
        let const_k = -0.5 * dot(&means[k], &coef_k) + priors[k].ln();
        coefs.push(coef_k);
        constants.push(const_k);
    }

    let class_labels = classes.iter().map(value_label).collect();

    Ok(LdaModel {
        classes,
        class_labels,
        counts,
        priors,
        means,
        within_cov,
        pooled_inv,
        pooled,
        coefs,
        constants,
        n_total,
        n_groups,
        p,
    })
}

// ───────────────────────── Execute ─────────────────────────

/// One kept (complete) observation: original row, class value, x-vector.
pub(super) struct Obs {
    pub(super) orig_row: usize,
    pub(super) class: Value,
    pub(super) x: Vec<f64>,
}

/// Position of a class value in the class list (sas_cmp equality).
pub(super) fn class_index_of(cls: &[Value], v: &Value) -> usize {
    cls.iter()
        .position(|c| c.sas_cmp(v) == std::cmp::Ordering::Equal)
        .unwrap()
}
