use super::*;

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
    let inv = if dims_ok {
        invert_matrix(info).ok()
    } else {
        None
    };
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
    ScoreTest { chi_square, df, p }
}
