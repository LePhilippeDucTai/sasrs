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

use crate::stat::{chisq_cdf, f_cdf, student_t_cdf};
use crate::stat::linalg::invert_matrix;
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

// ───────────────────────── Rao score (Lagrange-multiplier) test ─────────────────────────

/// Result of a Rao score (Lagrange-multiplier) test.
#[derive(Debug, Clone)]
pub struct ScoreTest {
    /// Score statistic χ² = Uᵀ I⁻¹ U.
    pub chi_square: f64,
    /// Degrees of freedom = length of the score vector U.
    pub df: f64,
    /// Pr > χ² (None when the information matrix is singular / χ² is NaN).
    pub p: Option<f64>,
}

/// Rao score (Lagrange-multiplier) test statistic `χ² = Uᵀ I⁻¹ U`.
///
/// `u` is the score (gradient of the log-likelihood) evaluated under the null,
/// `info` the corresponding (expected or observed) Fisher information matrix.
/// Degrees of freedom equal the length of `u`. The χ² tail probability is
/// computed from the already-available [`chisq_cdf`].
///
/// The matrix inversion reuses [`invert_matrix`]; when `info` is singular (or
/// dimensions are inconsistent) the test degrades gracefully to
/// `chi_square = NaN`, `p = None`.
pub fn score_test(u: &[f64], info: &[Vec<f64>]) -> ScoreTest {
    let k = u.len();
    let df = k as f64;
    // Dimension sanity: square info matching u.
    let dims_ok = info.len() == k && info.iter().all(|r| r.len() == k);
    let inv = if dims_ok { invert_matrix(info).ok() } else { None };
    let chi_square = match inv {
        Some(inv) => {
            // Quadratic form Uᵀ I⁻¹ U, mirroring quad_form's accumulation shape.
            let mut q = 0.0;
            for a in 0..k {
                if u[a] == 0.0 {
                    continue;
                }
                for (b, invb) in inv[a].iter().enumerate().take(k) {
                    q += u[a] * invb * u[b];
                }
            }
            q
        }
        None => f64::NAN,
    };
    let p = if chi_square.is_nan() || k == 0 {
        None
    } else {
        Some((1.0 - chisq_cdf(chi_square, df)).clamp(0.0, 1.0))
    };
    ScoreTest {
        chi_square,
        df,
        p,
    }
}

// ───────────────────────── CLASS variable coding ─────────────────────────

/// SAS CLASS-variable parameterization (`PARAM=` option).
///
/// Selects how the levels of a CLASS variable are expanded into design
/// (indicator) columns. The reference cell, where applicable, is the **last**
/// level in `sas_cmp` order (SAS default), matching the existing reference-cell
/// coding used by `mixed`/`glimmix`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Param {
    /// `PARAM=REFERENCE` (SAS default for many procs). `L−1` columns of 0/1; the
    /// indicator of level `i` (`i < L−1`); the reference (last) level is all 0.
    Ref,
    /// `PARAM=EFFECT`. `L−1` columns; level `i` (`i < L−1`) → its own 0/1
    /// indicator; the reference (last) level → `−1` in every column, so each
    /// column sums to 0 over the levels.
    Effect,
    /// `PARAM=GLM`. `L` columns (over-parameterized): one 0/1 indicator per
    /// level, no reference dropped. Each level's row has exactly one `1`.
    Glm,
    /// `PARAM=POLY`. `L−1` columns of orthogonal polynomials of degrees `1..=L−1`
    /// evaluated on the equally-spaced integer scores `1, 2, …, L` of the levels
    /// (in `sas_cmp` order). Column 0 is the linear trend; columns are pairwise
    /// orthogonal. (See [`poly_coding`] for the exact normalization.)
    Poly,
}

/// Build the CLASS-variable coding matrix for `levels`, parameterized by `param`.
///
/// **Precondition:** `levels` must already be the distinct levels of the CLASS
/// variable in `sas_cmp` order (deduped + sorted by the caller). This function
/// does NOT reorder — for [`Param::Ref`] / [`Param::Effect`] the reference cell
/// is taken to be the LAST element. Ordering is the caller's responsibility
/// (reuse `Value::sas_cmp`, never raw string compares).
///
/// Returns one row per level (same order as `levels`); each row is that level's
/// coding (indicator/contrast) values. The number of columns depends on `param`:
/// `Ref`/`Effect`/`Poly` → `L−1`; `Glm` → `L` (where `L = levels.len()`). With
/// `L == 0` the result is empty; with `L == 1`, `Ref`/`Effect`/`Poly` yield one
/// empty row and `Glm` yields `[[1.0]]`.
pub fn class_coding(levels: &[Value], param: Param) -> Vec<Vec<f64>> {
    let l = levels.len();
    match param {
        Param::Ref => {
            // L−1 columns; row i = unit vector e_i for i<L−1, reference (last) = 0.
            let ncol = l.saturating_sub(1);
            (0..l)
                .map(|i| {
                    let mut row = vec![0.0; ncol];
                    if i < ncol {
                        row[i] = 1.0;
                    }
                    row
                })
                .collect()
        }
        Param::Effect => {
            // L−1 columns; row i = e_i for i<L−1, reference (last) = all −1.
            let ncol = l.saturating_sub(1);
            (0..l)
                .map(|i| {
                    if i < ncol {
                        let mut row = vec![0.0; ncol];
                        row[i] = 1.0;
                        row
                    } else {
                        vec![-1.0; ncol]
                    }
                })
                .collect()
        }
        Param::Glm => {
            // L columns; row i = unit vector e_i (one indicator per level).
            (0..l)
                .map(|i| {
                    let mut row = vec![0.0; l];
                    row[i] = 1.0;
                    row
                })
                .collect()
        }
        Param::Poly => poly_coding(l),
    }
}

/// Orthogonal-polynomial coding on equally-spaced integer scores `1..=L`.
///
/// Produces `L` rows (one per level, in score order `1, 2, …, L`) of `L−1`
/// columns; column `d−1` holds the degree-`d` orthogonal polynomial
/// (`d = 1..=L−1`) evaluated at the scores, built by Gram–Schmidt of the power
/// basis `{1, x, x², …}` against the constant. Each contrast vector is scaled to
/// unit length (Euclidean norm 1), so columns are orthonormal; column 0 is the
/// monotone linear trend. (`L ≤ 1` → no contrast columns.)
fn poly_coding(l: usize) -> Vec<Vec<f64>> {
    if l == 0 {
        return Vec::new();
    }
    let ncol = l - 1;
    // Scores x_k = k+1 (1-based), as f64.
    let x: Vec<f64> = (0..l).map(|k| (k + 1) as f64).collect();
    // Orthogonal basis vectors, starting with the constant (degree 0), which we
    // keep only to orthogonalize against (it is NOT emitted as a column).
    let mut basis: Vec<Vec<f64>> = Vec::with_capacity(ncol + 1);
    basis.push(vec![1.0; l]); // degree 0 (constant)
    for deg in 1..=ncol {
        // Raw power vector x^deg.
        let mut v: Vec<f64> = x.iter().map(|&xi| xi.powi(deg as i32)).collect();
        // Gram–Schmidt against all previous (orthogonal) basis vectors.
        for u in &basis {
            let uu: f64 = u.iter().map(|&a| a * a).sum();
            if uu == 0.0 {
                continue;
            }
            let uv: f64 = u.iter().zip(v.iter()).map(|(&a, &b)| a * b).sum();
            let coef = uv / uu;
            for (vi, &ui) in v.iter_mut().zip(u.iter()) {
                *vi -= coef * ui;
            }
        }
        // Normalize to unit length.
        let norm: f64 = v.iter().map(|&a| a * a).sum::<f64>().sqrt();
        if norm > 0.0 {
            for vi in v.iter_mut() {
                *vi /= norm;
            }
        }
        basis.push(v);
    }
    // Emit rows: row k = (col_1[k], …, col_{L-1}[k]); skip basis[0] (constant).
    (0..l)
        .map(|k| basis[1..].iter().map(|col| col[k]).collect())
        .collect()
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

    // ─────────────────────── class_coding ───────────────────────

    fn levels(names: &[&str]) -> Vec<Value> {
        names.iter().map(|s| Value::Char((*s).into())).collect()
    }

    /// Ref coding must reproduce the manual reference-cell layout (last = ref).
    #[test]
    fn test_class_coding_ref_matches_manual() {
        let lv = levels(&["A", "B", "C"]); // already sas_cmp order
        let coding = class_coding(&lv, Param::Ref);
        // 3 rows, 2 columns each.
        assert_eq!(coding, vec![
            vec![1.0, 0.0], // A
            vec![0.0, 1.0], // B
            vec![0.0, 0.0], // C = reference
        ]);
        // Manual oracle: column j is the indicator of levels[j] (j < L−1).
        let l = lv.len();
        for (li, _) in lv.iter().enumerate() {
            for j in 0..l - 1 {
                let manual = if li == j { 1.0 } else { 0.0 };
                assert_eq!(coding[li][j], manual, "li={li} j={j}");
            }
        }
    }

    /// Effect coding: reference (last) = −1 everywhere ⇒ each column sums to 0.
    #[test]
    fn test_class_coding_effect_columns_sum_to_zero() {
        let lv = levels(&["A", "B", "C", "D"]);
        let coding = class_coding(&lv, Param::Effect);
        assert_eq!(coding.len(), 4);
        let ncol = lv.len() - 1;
        for row in &coding {
            assert_eq!(row.len(), ncol);
        }
        // Last row all −1.
        assert_eq!(coding[3], vec![-1.0, -1.0, -1.0]);
        // Each column sums to 0.
        for j in 0..ncol {
            let s: f64 = coding.iter().map(|r| r[j]).sum();
            assert!(s.abs() < 1e-12, "col {j} sum = {s}");
        }
    }

    /// Glm coding: L columns, exactly one 1 per row, rest 0.
    #[test]
    fn test_class_coding_glm_one_hot() {
        let lv = levels(&["A", "B", "C"]);
        let coding = class_coding(&lv, Param::Glm);
        assert_eq!(coding.len(), 3);
        for (i, row) in coding.iter().enumerate() {
            assert_eq!(row.len(), 3, "L columns");
            let ones: Vec<usize> = row.iter().enumerate().filter(|&(_, &v)| v == 1.0).map(|(k, _)| k).collect();
            assert_eq!(ones, vec![i], "row {i} must be one-hot at i");
            let sum: f64 = row.iter().sum();
            assert_eq!(sum, 1.0);
        }
    }

    /// Poly coding: L−1 columns, pairwise orthogonal, col 0 = linear trend.
    #[test]
    fn test_class_coding_poly_orthogonal() {
        let lv = levels(&["A", "B", "C", "D", "E"]); // L = 5
        let coding = class_coding(&lv, Param::Poly);
        let l = lv.len();
        let ncol = l - 1;
        assert_eq!(coding.len(), l);
        for row in &coding {
            assert_eq!(row.len(), ncol);
        }
        // Pairwise orthogonality (and orthogonality to the constant).
        for a in 0..ncol {
            let dot_const: f64 = (0..l).map(|k| coding[k][a]).sum();
            assert!(dot_const.abs() < 1e-9, "col {a} not orthogonal to constant: {dot_const}");
            for b in (a + 1)..ncol {
                let dot: f64 = (0..l).map(|k| coding[k][a] * coding[k][b]).sum();
                assert!(dot.abs() < 1e-9, "cols {a},{b} not orthogonal: {dot}");
            }
            // Unit norm (orthonormal).
            let nn: f64 = (0..l).map(|k| coding[k][a] * coding[k][a]).sum();
            assert!((nn - 1.0).abs() < 1e-9, "col {a} not unit norm: {nn}");
        }
        // Column 0 is the linear trend: strictly monotone with equal spacing.
        let col0: Vec<f64> = (0..l).map(|k| coding[k][0]).collect();
        let step = col0[1] - col0[0];
        assert!(step > 0.0, "linear trend must increase: step={step}");
        for k in 1..l {
            assert!((col0[k] - col0[k - 1] - step).abs() < 1e-9, "col0 not equally spaced at {k}");
        }
    }

    /// Degenerate sizes.
    #[test]
    fn test_class_coding_edge_sizes() {
        assert!(class_coding(&[], Param::Ref).is_empty());
        assert!(class_coding(&[], Param::Glm).is_empty());
        let one = levels(&["X"]);
        assert_eq!(class_coding(&one, Param::Ref), vec![vec![] as Vec<f64>]);
        assert_eq!(class_coding(&one, Param::Effect), vec![vec![] as Vec<f64>]);
        assert_eq!(class_coding(&one, Param::Poly), vec![vec![] as Vec<f64>]);
        assert_eq!(class_coding(&one, Param::Glm), vec![vec![1.0]]);
    }

    // ─────────────────────── score_test (Rao) ───────────────────────

    fn identity(k: usize) -> Vec<Vec<f64>> {
        (0..k)
            .map(|i| (0..k).map(|j| if i == j { 1.0 } else { 0.0 }).collect())
            .collect()
    }

    /// I = identity ⇒ χ² = Σ uᵢ², df = k.
    #[test]
    fn test_score_identity() {
        let u = vec![1.0, -2.0, 3.0];
        let st = score_test(&u, &identity(3));
        assert!((st.chi_square - (1.0 + 4.0 + 9.0)).abs() < 1e-12, "chi2={}", st.chi_square);
        assert_eq!(st.df, 3.0);
        let p = st.p.unwrap();
        assert!((0.0..=1.0).contains(&p));
        // χ²=14 on 3 df → p ≈ 0.0029.
        assert!((p - 0.0029074).abs() < 1e-4, "p={p}");
    }

    /// Hand-checked 2×2: I = [[2,1],[1,2]] (det=3), I⁻¹ = (1/3)[[2,-1],[-1,2]].
    /// U = [1, 0] ⇒ χ² = Uᵀ I⁻¹ U = inv[0][0] = 2/3.
    #[test]
    fn test_score_2x2() {
        let u = vec![1.0, 0.0];
        let info = vec![vec![2.0, 1.0], vec![1.0, 2.0]];
        let st = score_test(&u, &info);
        assert!((st.chi_square - 2.0 / 3.0).abs() < 1e-10, "chi2={}", st.chi_square);
        assert_eq!(st.df, 2.0);
        // Full U = [1, 2]: Uᵀ I⁻¹ U = (1/3)(2·1 −1·2 −2·1 +2·4) = (1/3)(2−2−2+8)=2.
        let st2 = score_test(&[1.0, 2.0], &info);
        assert!((st2.chi_square - 2.0).abs() < 1e-10, "chi2={}", st2.chi_square);
    }

    /// Singular information ⇒ NaN statistic, p = None.
    #[test]
    fn test_score_singular() {
        let u = vec![1.0, 1.0];
        let info = vec![vec![1.0, 1.0], vec![1.0, 1.0]]; // rank 1
        let st = score_test(&u, &info);
        assert!(st.chi_square.is_nan());
        assert!(st.p.is_none());
        assert_eq!(st.df, 2.0);
    }
}
