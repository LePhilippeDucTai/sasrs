//! Shared linear-combination engine for PROC GLM (and future MIXED/GENMOD…).
//!
//! M37.1 — Phase G infrastructure. This module factors the linear-algebra core
//! that PROC GLM's *multiway* path uses for parameter estimates, LS-means and
//! linear-combination tests into a single reusable engine:
//!
//! ```text
//! LinCombEngine { beta, cov, coding, df, mse }
//!   estimate(l, c) -> LinEstimate   // L·β − c with se / t / Pr>|t|
//!   contrast(l, c) -> LinContrast   // single-row F test of L·β − c
//!   lsmeans(effect)  -> Vec<LsMean> // LS-means of a main-effect factor
//! ```
//!
//! ## Byte-identity contract
//!
//! GLM's listing must remain strictly byte-identical. The numeric methods here
//! reproduce the EXACT floating-point operation order of the code they were
//! extracted from (`execute_multiway` / `lsmean_coef_vector` in `glm.rs`):
//!
//! - `cov` is kept as the raw `(XᵀX)⁻¹`; `mse` is carried separately and applied
//!   as `(mse * q).sqrt()`. Pre-folding `mse` into `cov` would change the last
//!   bit (`mse·Σ l·inv·l` ≠ `Σ l·(mse·inv)·l`).
//! - `lsmeans` keeps the `if l[a]==0.0 { continue }` skip and the
//!   `q += l[a]*inv[a][b]*l[b]` accumulation order verbatim.
//! - `estimate` uses the same quadratic form (no skip), matching the
//!   parameter-table SE when called with a unit selector vector (the zero-skip
//!   and the unit vector collapse to `inv[col][col]`).
//!
//! The `coding` describes the reference-cell dummy layout of the fitted design
//! so that LS-means estimable functions can be rebuilt the same way GLM did.

use crate::stat::{f_cdf, student_t_cdf};
use crate::value::Value;

/// Reference-cell coding layout of a fitted multiway GLM design.
///
/// This is exactly the information `lsmean_coef_vector` needed: the factor level
/// sets (reference = last), the per-term parent-factor indices, the per-term
/// column specs (each column = product of parent `(factor_idx, dummy_idx)`
/// pairs) and the total design column count (intercept + term columns).
#[derive(Debug, Clone)]
pub struct Coding {
    /// Per factor: `(name, levels)` — distinct non-missing levels in `sas_cmp`
    /// order; reference cell = LAST.
    pub factors: Vec<(String, Vec<Value>)>,
    /// Per effect term: indices into `factors` of the parent factors.
    pub term_factor_idxs: Vec<Vec<usize>>,
    /// Per effect term: list of column specs; each spec is the list of parent
    /// `(factor_idx, dummy_idx)` pairs whose product forms that design column.
    pub col_specs: Vec<Vec<Vec<(usize, usize)>>>,
    /// Total number of design columns (intercept + all term columns).
    pub ncols: usize,
}

impl Coding {
    /// Number of dummy columns a factor contributes (levels − 1, last dropped).
    fn n_dummies(&self, fi: usize) -> usize {
        self.factors[fi].1.len().saturating_sub(1)
    }
}

/// Result of an `estimate` call: L·β − c with inference.
#[derive(Debug, Clone)]
pub struct LinEstimate {
    pub estimate: f64,
    pub se: f64,
    pub t: f64,
    /// Two-sided Pr > |t| (None when t is NaN).
    pub p: Option<f64>,
}

/// Result of a `contrast` call: single-row F test of L·β − c.
#[derive(Debug, Clone)]
pub struct LinContrast {
    pub f: f64,
    /// Pr > F (None when F is NaN).
    pub p: Option<f64>,
    pub df1: f64,
    pub df2: f64,
}

/// One LS-mean row for a main-effect factor level.
#[derive(Debug, Clone)]
pub struct LsMean {
    /// Display label of the level (matches the one-way path scheme).
    pub level_label: String,
    pub estimate: f64,
    pub se: f64,
    pub t: f64,
    pub p: Option<f64>,
}

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
    fn quad_form(&self, l: &[f64]) -> f64 {
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
    fn dot(&self, l: &[f64]) -> f64 {
        l.iter().zip(self.beta.iter()).map(|(c, b)| c * b).sum()
    }

    /// Build the estimable LS-mean coefficient vector for level `li` of factor
    /// `fi`. Thin wrapper over [`lsmean_coef`].
    fn lsmean_coef_vector(&self, target_fi: usize, target_li: usize) -> Vec<f64> {
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
fn row_dummies(coding: &Coding, cell_levels: &[usize]) -> Vec<Vec<f64>> {
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
fn level_label_value(v: &Value) -> String {
    match v {
        Value::Char(s) => s.trim_end().to_string(),
        Value::Num(f) => format!("{f}"),
        Value::Missing(k) => k.display(),
    }
}

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Fit a tiny one-way reference-cell model y = μ + α (2 groups A,B) by the
    /// same path GLM uses, and wrap it in a LinCombEngine.
    ///
    /// Data: A=[1,2,3], B=[10,11,12]. sas_cmp order → ref = B (last).
    /// Design cols: [intercept(=B mean), A-dummy].
    fn engine_ab() -> LinCombEngine {
        // y, with usable rows in input order.
        let y = vec![1.0, 2.0, 3.0, 10.0, 11.0, 12.0];
        // reference-cell design: intercept + A-dummy (A=1 for first 3 rows).
        let design = vec![
            vec![1.0, 1.0],
            vec![1.0, 1.0],
            vec![1.0, 1.0],
            vec![1.0, 0.0],
            vec![1.0, 0.0],
            vec![1.0, 0.0],
        ];
        let beta = crate::stat::linalg::least_squares(&design, &y).unwrap();
        let xt = crate::stat::linalg::transpose(&design);
        let xtx = crate::stat::linalg::matrix_mult(&xt, &design);
        let cov = crate::stat::linalg::invert_matrix(&xtx).unwrap();
        // SSE / df / MSE.
        let mut sse = 0.0;
        for (i, row) in design.iter().enumerate() {
            let fitted: f64 = row.iter().zip(beta.iter()).map(|(a, b)| a * b).sum();
            sse += (y[i] - fitted).powi(2);
        }
        let n = y.len();
        let ncols = 2;
        let df_error = (n - ncols) as f64;
        let mse = sse / df_error;
        let coding = Coding {
            factors: vec![(
                "sex".into(),
                vec![Value::Char("A".into()), Value::Char("B".into())],
            )],
            term_factor_idxs: vec![vec![0]],
            // term sex → one column = A-dummy = (factor 0, dummy 0).
            col_specs: vec![vec![vec![(0usize, 0usize)]]],
            ncols,
        };
        LinCombEngine::new(beta, cov, coding, df_error, mse)
    }

    // ── estimate: oracle from glm.rs test_execute_estimate_correct ──────────
    // ESTIMATE 'A vs B' = 1*ȳ_A + (-1)*ȳ_B in *cell-mean* space, but in
    // parameter space (intercept=B mean, A-dummy=α_A) the same contrast is
    // l = [0, 1] (β_A = ȳ_A − ȳ_B = −9), c = 0.  est = −9.
    #[test]
    fn test_estimate_minus_nine() {
        let eng = engine_ab();
        let r = eng.estimate(&[0.0, 1.0], 0.0);
        assert!((r.estimate - (-9.0)).abs() < 1e-9, "est={}", r.estimate);
        // SE = sqrt(MSE * cov[1][1]); MSE=1, t ≈ -11.02.
        assert!(r.t < -10.0 && r.t > -12.0, "t={}", r.t);
        assert!(r.se > 0.0);
    }

    // ── contrast: F == t² of the same estimate (glm test 7 oracle) ──────────
    #[test]
    fn test_contrast_f_eq_t_squared() {
        let eng = engine_ab();
        let est = eng.estimate(&[0.0, 1.0], 0.0);
        let con = eng.contrast(&[0.0, 1.0], 0.0);
        assert!((con.f - est.t * est.t).abs() < 1e-6, "f={} t^2={}", con.f, est.t * est.t);
        assert!((con.f - 121.5).abs() < 0.5, "f={}", con.f);
        assert_eq!(con.df1, 1.0);
        assert_eq!(con.df2, 4.0);
    }

    // ── lsmeans: ȳ_A = 2, ȳ_B = 11 (glm test_execute_lsmeans oracle) ────────
    #[test]
    fn test_lsmeans_group_means() {
        let eng = engine_ab();
        let lsm = eng.lsmeans("sex");
        assert_eq!(lsm.len(), 2);
        // sas_cmp order: A then B.
        assert_eq!(lsm[0].level_label, "A");
        assert_eq!(lsm[1].level_label, "B");
        assert!((lsm[0].estimate - 2.0).abs() < 1e-9, "A={}", lsm[0].estimate);
        assert!((lsm[1].estimate - 11.0).abs() < 1e-9, "B={}", lsm[1].estimate);
        assert!(lsm[0].se > 0.0 && lsm[1].se > 0.0);
    }

    // ── unknown effect → empty ──────────────────────────────────────────────
    #[test]
    fn test_lsmeans_unknown_effect() {
        let eng = engine_ab();
        assert!(eng.lsmeans("nope").is_empty());
    }
}
