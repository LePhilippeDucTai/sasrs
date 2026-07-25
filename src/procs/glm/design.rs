use super::*;

// ───────────────────────── Multiway engine (M34.5) ─────────────────────────

/// One CLASS factor resolved against the usable rows of a dependent variable.
pub(super) struct Factor {
    pub(super) name: String,
    /// Distinct non-missing levels in `sas_cmp` order. Reference cell = LAST.
    pub(super) levels: Vec<Value>,
}

impl Factor {
    /// Index of the level for value `v` (must exist).
    pub(super) fn level_of(&self, v: &Value) -> usize {
        self.levels
            .iter()
            .position(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
            .unwrap()
    }
    /// Number of dummy columns this factor contributes (levels − 1, last dropped).
    pub(super) fn n_dummies(&self) -> usize {
        self.levels.len().saturating_sub(1)
    }
}

/// Human-readable level label, matching the one-way path's scheme.
pub(super) fn level_label_value(v: &Value) -> String {
    match v {
        Value::Char(s) => s.trim_end().to_string(),
        Value::Num(f) => format!("{f}"),
        Value::Missing(k) => k.display(),
    }
}

/// Compute residual sum of squares for a design matrix `x` (rows × cols) and `y`.
/// Returns SSE = ‖y − Xβ̂‖². On a rank-deficient / singular fit returns NaN.
pub(super) fn sse_of(x: &[Vec<f64>], y: &[f64]) -> f64 {
    if x.is_empty() || x[0].is_empty() {
        // No predictors at all → SSE around 0 (degenerate); treat as total.
        let ybar = y.iter().sum::<f64>() / y.len().max(1) as f64;
        return y.iter().map(|&v| (v - ybar).powi(2)).sum();
    }
    let beta = match crate::stat::linalg::least_squares(x, y) {
        Ok(b) => b,
        Err(_) => return f64::NAN,
    };
    let mut sse = 0.0;
    for (i, row) in x.iter().enumerate() {
        let fitted: f64 = row.iter().zip(beta.iter()).map(|(a, b)| a * b).sum();
        sse += (y[i] - fitted).powi(2);
    }
    sse
}

/// Build the reference-cell dummy values for a single row, per factor.
/// `dummies[f]` is a Vec of length `factors[f].n_dummies()`; entry j = 1 if the
/// row is at level j of factor f (j < levels−1), else 0. (Reference level → all 0.)
pub(super) fn row_dummies(factors: &[Factor], row_levels: &[usize]) -> Vec<Vec<f64>> {
    factors
        .iter()
        .zip(row_levels.iter())
        .map(|(f, &li)| {
            let nd = f.n_dummies();
            let mut d = vec![0.0; nd];
            if li < nd {
                d[li] = 1.0;
            }
            d
        })
        .collect()
}

/// Build the sum-to-zero (effect / deviation) coded values for a single row,
/// per factor. Each factor with levels 1..L (sas_cmp order) contributes L−1
/// columns; column j (0-based) = +1 if the row is at level j, −1 if the row is
/// at the LAST level L−1, else 0.
///
/// This full-rank effect coding spans the same column space as the reference-cell
/// coding, but interaction columns built from these centered contrasts make the
/// per-term partial SS coincide with the SAS Type III estimable-function SS.
pub(super) fn row_effects(factors: &[Factor], row_levels: &[usize]) -> Vec<Vec<f64>> {
    factors
        .iter()
        .zip(row_levels.iter())
        .map(|(f, &li)| {
            let nd = f.n_dummies();
            let last = f.levels.len().saturating_sub(1);
            let mut d = vec![0.0; nd];
            if li == last {
                for v in d.iter_mut() {
                    *v = -1.0;
                }
            } else if li < nd {
                d[li] = 1.0;
            }
            d
        })
        .collect()
}

/// Build the full design matrix column layout for a set of terms.
/// Returns, per term, the list of (factor_index, dummy_index) pairs identifying
/// the parent dummies whose elementwise product forms each interaction column.
/// For a main effect each "column spec" is a single pair.
pub(super) fn term_column_specs(
    terms: &[Vec<usize>],
    factors: &[Factor],
) -> Vec<Vec<Vec<(usize, usize)>>> {
    terms
        .iter()
        .map(|term_factor_idxs| {
            // Cartesian product of each parent factor's dummy indices.
            let mut combos: Vec<Vec<(usize, usize)>> = vec![vec![]];
            for &fi in term_factor_idxs {
                let nd = factors[fi].n_dummies();
                let mut next = Vec::new();
                for prefix in &combos {
                    for j in 0..nd {
                        let mut c = prefix.clone();
                        c.push((fi, j));
                        next.push(c);
                    }
                }
                combos = next;
            }
            combos
        })
        .collect()
}
