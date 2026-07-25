use super::*;

/// Shared linear-combination engine over a fitted linear model.
///
/// `beta` are the fitted coefficients, `cov` is the raw `(XᵀX)⁻¹`, `mse` the
/// error mean square and `df` the error degrees of freedom. `coding` describes
/// the design layout for LS-means estimable functions.
#[derive(Debug, Clone)]
pub struct LinCombEngine {
    beta: Vec<f64>,
    cov: Vec<Vec<f64>>,
    coding: Coding,
    df: f64,
    mse: f64,
}

impl LinCombEngine {
    /// Build an engine from a fitted multiway GLM.
    pub fn new(beta: Vec<f64>, cov: Vec<Vec<f64>>, coding: Coding, df: f64, mse: f64) -> Self {
        LinCombEngine {
            beta,
            cov,
            coding,
            df,
            mse,
        }
    }

    /// Fitted coefficients β (intercept first, then term columns).
    pub fn beta(&self) -> &[f64] {
        &self.beta
    }

    /// Raw covariance `(XᵀX)⁻¹` (NOT scaled by MSE).
    pub fn cov(&self) -> &[Vec<f64>] {
        &self.cov
    }

    /// Error degrees of freedom.
    pub fn df(&self) -> f64 {
        self.df
    }

    /// Error mean square.
    pub fn mse(&self) -> f64 {
        self.mse
    }

    /// Design column coding layout.
    pub fn coding(&self) -> &Coding {
        &self.coding
    }

    /// Quadratic form `lᵀ (XᵀX)⁻¹ l` over the full column set.
    ///
    /// Mirrors the LS-means SE accumulation verbatim: skip zero coefficients,
    /// accumulate `q += l[a]·inv[a][b]·l[b]`. Used for the estimable-function
    /// variance of both LS-means and user estimates.
    pub(super) fn quad_form(&self, l: &[f64]) -> f64 {
        let n = self.coding.ncols;
        let mut q = 0.0;
        for a in 0..n {
            if l[a] == 0.0 {
                continue;
            }
            for (b, invb) in self.cov[a].iter().enumerate().take(n) {
                q += l[a] * invb * l[b];
            }
        }
        q
    }

    /// Estimate the linear combination `L·β − c` with standard error, t value
    /// and two-sided Pr > |t|.
    ///
    /// `l` is in parameter (design-column) space, length `coding.ncols`.
    pub fn estimate(&self, l: &[f64], c: f64) -> LinEstimate {
        let est = self.dot(l) - c;
        let q = self.quad_form(l);
        let se = if !self.mse.is_nan() && q >= 0.0 {
            (self.mse * q).sqrt()
        } else {
            f64::NAN
        };
        let t = if se > 0.0 { est / se } else { f64::NAN };
        let p = if t.is_nan() {
            None
        } else {
            Some(2.0 * (1.0 - student_t_cdf(t.abs(), self.df)))
        };
        LinEstimate {
            estimate: est,
            se,
            t,
            p,
        }
    }

    /// Single-row F test of the linear combination `L·β − c`.
    ///
    /// `F = (L·β − c)² / (MSE · lᵀ(XᵀX)⁻¹l)` on (1, df) degrees of freedom —
    /// numerically the square of the `estimate` t value.
    pub fn contrast(&self, l: &[f64], c: f64) -> LinContrast {
        let est = self.dot(l) - c;
        let q = self.quad_form(l);
        let denom = self.mse * q;
        let f = if denom > 0.0 && !denom.is_nan() {
            est * est / denom
        } else {
            f64::NAN
        };
        let df1 = 1.0;
        let df2 = self.df;
        let p = if f.is_nan() {
            None
        } else {
            Some((1.0 - f_cdf(f, df1, df2)).clamp(0.0, 1.0))
        };
        LinContrast { f, p, df1, df2 }
    }

    /// LS-means of a main-effect factor (one row per level).
    ///
    /// Each LS-mean is the estimable function obtained by averaging the predicted
    /// cell mean uniformly over all OTHER factors' levels. Returns one [`LsMean`]
    /// per level of `effect`; an empty vector if `effect` is not a factor.
    pub fn lsmeans(&self, effect: &str) -> Vec<LsMean> {
        let fi = match self
            .coding
            .factors
            .iter()
            .position(|(n, _)| n.eq_ignore_ascii_case(effect))
        {
            Some(i) => i,
            None => return Vec::new(),
        };
        let nlevels = self.coding.factors[fi].1.len();
        let mut out = Vec::with_capacity(nlevels);
        for li in 0..nlevels {
            let lvec = self.lsmean_coef_vector(fi, li);
            let est = self.dot(&lvec);
            let q = self.quad_form(&lvec);
            let se = if !self.mse.is_nan() && q >= 0.0 {
                (self.mse * q).sqrt()
            } else {
                f64::NAN
            };
            let t = if se > 0.0 { est / se } else { f64::NAN };
            let p = if t.is_nan() {
                None
            } else {
                Some(2.0 * (1.0 - student_t_cdf(t.abs(), self.df)))
            };
            out.push(LsMean {
                level_label: level_label_value(&self.coding.factors[fi].1[li]),
                estimate: est,
                se,
                t,
                p,
            });
        }
        out
    }

    /// Dot product `l · β` in the canonical column order.
    pub(super) fn dot(&self, l: &[f64]) -> f64 {
        l.iter().zip(self.beta.iter()).map(|(c, b)| c * b).sum()
    }

    /// Build the estimable LS-mean coefficient vector for level `li` of factor
    /// `fi`. Thin wrapper over [`lsmean_coef`].
    pub(super) fn lsmean_coef_vector(&self, target_fi: usize, target_li: usize) -> Vec<f64> {
        lsmean_coef(&self.coding, target_fi, target_li)
    }
}

/// Build the estimable LS-mean coefficient vector (length `coding.ncols`) for
/// level `target_li` of factor `target_fi`, averaging uniformly over all OTHER
/// factors' levels. Same column order as the full design (intercept first).
///
/// This is a pure function of the design [`Coding`] (it needs neither β nor the
/// covariance), so callers that have β but no covariance can still rebuild the
/// estimable function. Extracted verbatim from GLM's `lsmean_coef_vector`.
pub fn lsmean_coef(coding: &Coding, target_fi: usize, target_li: usize) -> Vec<f64> {
    // Enumerate the balanced grid of all factors' levels, fixing target=li.
    let dims: Vec<usize> = coding.factors.iter().map(|(_, lv)| lv.len()).collect();
    let mut grid_levels: Vec<Vec<usize>> = vec![vec![]];
    for (fi, &dim) in dims.iter().enumerate() {
        let mut next = Vec::new();
        for prefix in &grid_levels {
            if fi == target_fi {
                let mut c = prefix.clone();
                c.push(target_li);
                next.push(c);
            } else {
                for l in 0..dim {
                    let mut c = prefix.clone();
                    c.push(l);
                    next.push(c);
                }
            }
        }
        grid_levels = next;
    }
    let ncells = grid_levels.len().max(1) as f64;

    // For each cell, build its design row (intercept + term cols), then average.
    let mut acc = vec![0.0; coding.ncols];
    for cell in &grid_levels {
        let dummies = row_dummies(coding, cell);
        let mut row = vec![1.0];
        for specs in coding.col_specs.iter() {
            for spec in specs {
                let mut prod = 1.0;
                for &(fi, dj) in spec {
                    prod *= dummies[fi][dj];
                }
                row.push(prod);
            }
        }
        for (a, &v) in row.iter().enumerate() {
            acc[a] += v / ncells;
        }
    }
    acc
}

/// Build the reference-cell dummy values for a single cell, per factor.
/// `out[f]` has length `n_dummies(f)`; entry j = 1 if the cell is at level j of
/// factor f (j < levels−1), else 0. (Reference level → all 0.)
pub(super) fn row_dummies(coding: &Coding, cell_levels: &[usize]) -> Vec<Vec<f64>> {
    coding
        .factors
        .iter()
        .enumerate()
        .zip(cell_levels.iter())
        .map(|((fi, _), &li)| {
            let nd = coding.n_dummies(fi);
            let mut d = vec![0.0; nd];
            if li < nd {
                d[li] = 1.0;
            }
            d
        })
        .collect()
}

/// Human-readable level label, matching the GLM one-way path's scheme.
pub(super) fn level_label_value(v: &Value) -> String {
    match v {
        Value::Char(s) => s.trim_end().to_string(),
        Value::Num(f) => format!("{f}"),
        Value::Missing(k) => k.display(),
    }
}
