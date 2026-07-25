use super::*;

// ───────────────────────── OLS fit helper ─────────────────────────

/// Result of an ordinary-least-squares fit on a fully-numeric design matrix.
pub(crate) struct OlsFit {
    /// Coefficient vector (one per column of X).
    pub(crate) beta: Vec<f64>,
    /// Predicted values ŷ = Xβ.
    pub(crate) y_hat: Vec<f64>,
    /// Residuals y − ŷ.
    pub(crate) resid: Vec<f64>,
    /// Σ resid² (residual / error sum of squares).
    pub(crate) sse: f64,
    /// (XᵀX)⁻¹.
    pub(crate) xtx_inv: Vec<Vec<f64>>,
}

/// Fit OLS for the given design matrix `x` (rows are observations, columns are
/// regressors — the caller decides whether an intercept column is present) and
/// response `y`. Pure: no session / printing side effects.
pub(crate) fn ols_fit(x: &[Vec<f64>], y: &[f64]) -> Result<OlsFit> {
    let beta = linalg::least_squares(x, y)?;
    let y_hat: Vec<f64> = x
        .iter()
        .map(|row| row.iter().zip(beta.iter()).map(|(xi, bi)| xi * bi).sum())
        .collect();
    let resid: Vec<f64> = y
        .iter()
        .zip(y_hat.iter())
        .map(|(yi, yhi)| yi - yhi)
        .collect();
    let sse: f64 = resid.iter().map(|r| r * r).sum();
    let xt = linalg::transpose(x);
    let xtx = linalg::matrix_mult(&xt, x);
    let xtx_inv = linalg::invert_matrix(&xtx)?;
    Ok(OlsFit {
        beta,
        y_hat,
        resid,
        sse,
        xtx_inv,
    })
}

/// M36.7 weighting context for weighted least squares / frequency replication.
pub(crate) struct Weighting {
    /// Effective SS weight w_i·f_i per complete-case row (same order as y).
    pub(crate) wf: Vec<f64>,
    /// Σ f_i — the observation count for n / degrees-of-freedom purposes (FREQ
    /// inflates this; WEIGHT alone leaves it equal to the row count).
    pub(crate) total_n: f64,
}

/// Weighted-least-squares fit. Solves `X'WX β = X'Wy` with W = diag(wf_i) by
/// scaling each design row and `y` by √wf_i and reusing the OLS machinery, then
/// recomputes ŷ / residuals on the ORIGINAL scale and the weighted error sum of
/// squares SSEw = Σ wf_i e_i². The returned `OlsFit.xtx_inv` is `(X'WX)⁻¹`
/// (since the scaled cross-product is exactly X'WX) so all downstream SE /
/// covariance formulas use the weighted normal equations directly.
pub(crate) fn weighted_ols_fit(x_mat: &[Vec<f64>], y: &[f64], wf: &[f64]) -> Result<OlsFit> {
    let n = y.len();
    let p = x_mat[0].len();
    let mut xs: Vec<Vec<f64>> = Vec::with_capacity(n);
    let mut ys: Vec<f64> = Vec::with_capacity(n);
    for i in 0..n {
        let s = wf[i].max(0.0).sqrt();
        let mut row = Vec::with_capacity(p);
        for j in 0..p {
            row.push(x_mat[i][j] * s);
        }
        xs.push(row);
        ys.push(y[i] * s);
    }
    // β and (X'WX)⁻¹ from the scaled normal equations.
    let scaled = ols_fit(&xs, &ys)?;
    let beta = scaled.beta;
    let xtx_inv = scaled.xtx_inv;
    // Original-scale predictions / residuals and the WEIGHTED SSE.
    let y_hat: Vec<f64> = x_mat
        .iter()
        .map(|row| row.iter().zip(beta.iter()).map(|(xi, bi)| xi * bi).sum())
        .collect();
    let resid: Vec<f64> = y
        .iter()
        .zip(y_hat.iter())
        .map(|(yi, yhi)| yi - yhi)
        .collect();
    let sse: f64 = resid
        .iter()
        .zip(wf.iter())
        .map(|(e, &w)| w * e * e)
        .sum();
    Ok(OlsFit {
        beta,
        y_hat,
        resid,
        sse,
        xtx_inv,
    })
}

/// Per-observation leverage h_i = x_iᵀ (X'X)⁻¹ x_i for every design row of
/// `x_mat`, given the already-computed `xtx_inv` (M36.2). Σ_i h_i == p_eff.
pub(crate) fn leverages(x_mat: &[Vec<f64>], xtx_inv: &[Vec<f64>]) -> Vec<f64> {
    let p = xtx_inv.len();
    x_mat
        .iter()
        .map(|row| {
            // h = rowᵀ · (X'X)⁻¹ · row.
            let mut acc = 0.0;
            for a in 0..p {
                let mut inner = 0.0;
                for b in 0..p {
                    inner += xtx_inv[a][b] * row[b];
                }
                acc += row[a] * inner;
            }
            acc
        })
        .collect()
}

/// Compute SSE only for a candidate subset fit (used by SELECTION). Builds the
/// design matrix from `xcols` (columns of regressors, each length n) over the
/// `subset` of column indices, optionally prepending an intercept column.
/// Returns `None` if the fit is rank-deficient / not solvable.
pub(crate) fn subset_sse(xcols: &[Vec<f64>], y: &[f64], subset: &[usize], intercept: bool) -> Option<f64> {
    let n = y.len();
    let mut x: Vec<Vec<f64>> = Vec::with_capacity(n);
    for i in 0..n {
        let mut row = Vec::with_capacity(subset.len() + intercept as usize);
        if intercept {
            row.push(1.0);
        }
        for &c in subset {
            row.push(xcols[c][i]);
        }
        x.push(row);
    }
    if x.is_empty() || x[0].is_empty() {
        return None;
    }
    ols_fit(&x, y).ok().map(|f| f.sse)
}

/// Sample standard deviation (divisor n−1). Returns 0 for fewer than 2 points.
pub(crate) fn sample_sd(v: &[f64]) -> f64 {
    let n = v.len();
    if n < 2 {
        return 0.0;
    }
    let mean = v.iter().sum::<f64>() / n as f64;
    let ss: f64 = v.iter().map(|x| (x - mean) * (x - mean)).sum();
    (ss / (n as f64 - 1.0)).sqrt()
}
