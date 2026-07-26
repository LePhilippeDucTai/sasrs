// MQ7.2c — `needless_range_loop` assumé dans ce module : l'indice EST le
// langage du domaine (`a[i][j] * b[j][k]`, parcours colonne-major, triangle
// d'une matrice symétrique). La forme itérateur y coûte plus en lisibilité
// qu'elle n'en rend, et la revue a préféré garder les indices explicites.
#![allow(clippy::needless_range_loop)]

use super::*;

// ───────────────────────── VARIMAX rotation ─────────────────────────

/// Apply VARIMAX rotation (Kaiser 1958) to loading matrix L (n_vars × k_factors).
/// Returns (L_rotated, R_rotation_matrix).
/// Precondition: k >= 2, all h²[i] > 0.
pub fn varimax(l: &[Vec<f64>]) -> (Vec<Vec<f64>>, Vec<Vec<f64>>) {
    let n_vars = l.len();
    let k = if n_vars > 0 { l[0].len() } else { 0 };

    if k < 2 || n_vars == 0 {
        // No rotation needed.
        let r: Vec<Vec<f64>> = (0..k)
            .map(|i| (0..k).map(|j| if i == j { 1.0 } else { 0.0 }).collect())
            .collect();
        return (l.to_vec(), r);
    }

    // Initial communalities h²[i] = Σⱼ L[i][j]²
    let h2: Vec<f64> = l
        .iter()
        .map(|row| row.iter().map(|&x| x * x).sum())
        .collect();

    // Kaiser normalisation: divide each row i by sqrt(h²[i]).
    let h_sqrt: Vec<f64> = h2
        .iter()
        .map(|&h| if h > 0.0 { h.sqrt() } else { 1.0 })
        .collect();
    let mut l_norm: Vec<Vec<f64>> = l
        .iter()
        .enumerate()
        .map(|(i, row)| row.iter().map(|&x| x / h_sqrt[i]).collect())
        .collect();

    // Rotation matrix R starts as identity.
    let mut rot: Vec<Vec<f64>> = (0..k)
        .map(|i| (0..k).map(|j| if i == j { 1.0 } else { 0.0 }).collect())
        .collect();

    // Compute current variance (criterion to maximise).
    fn varimax_criterion(l_norm: &[Vec<f64>], k: usize) -> f64 {
        let n = l_norm.len() as f64;
        let mut total = 0.0;
        for j in 0..k {
            let s2: f64 = l_norm.iter().map(|r| r[j].powi(4)).sum::<f64>();
            let s1: f64 = l_norm.iter().map(|r| r[j].powi(2)).sum::<f64>();
            total += n * s2 - s1 * s1;
        }
        total
    }

    let mut prev_var = varimax_criterion(&l_norm, k);

    for _iter in 0..1000 {
        for p in 0..k {
            for q in (p + 1)..k {
                // u[i] = A[i]² - B[i]²,  v[i] = 2*A[i]*B[i]
                let a: Vec<f64> = l_norm.iter().map(|r| r[p]).collect();
                let b: Vec<f64> = l_norm.iter().map(|r| r[q]).collect();
                let u: Vec<f64> = a
                    .iter()
                    .zip(&b)
                    .map(|(&ai, &bi)| ai * ai - bi * bi)
                    .collect();
                let v: Vec<f64> = a.iter().zip(&b).map(|(&ai, &bi)| 2.0 * ai * bi).collect();

                let n = n_vars as f64;
                let a_sum: f64 = u.iter().sum();
                let b_sum: f64 = v.iter().sum();
                let c_val: f64 = u.iter().zip(&v).map(|(&ui, &vi)| ui * ui - vi * vi).sum();
                let d_val: f64 = u.iter().zip(&v).map(|(&ui, &vi)| ui * vi).sum::<f64>() * 2.0;

                let num = d_val - 2.0 * a_sum * b_sum / n;
                let denom = c_val - (a_sum * a_sum - b_sum * b_sum) / n;

                let angle = f64::atan2(num, denom) / 4.0;
                let cos_a = angle.cos();
                let sin_a = angle.sin();

                // Apply rotation to l_norm columns p and q.
                for row in l_norm.iter_mut() {
                    let rp = row[p];
                    let rq = row[q];
                    row[p] = cos_a * rp + sin_a * rq;
                    row[q] = -sin_a * rp + cos_a * rq;
                }
                // Accumulate rotation matrix.
                for row in rot.iter_mut() {
                    let rp = row[p];
                    let rq = row[q];
                    row[p] = cos_a * rp + sin_a * rq;
                    row[q] = -sin_a * rp + cos_a * rq;
                }
            }
        }
        let new_var = varimax_criterion(&l_norm, k);
        if (new_var - prev_var).abs() < 1e-6 {
            break;
        }
        prev_var = new_var;
    }

    // Kaiser denormalization: multiply each row i by sqrt(h²[i]).
    let l_rot: Vec<Vec<f64>> = l_norm
        .iter()
        .enumerate()
        .map(|(i, row)| row.iter().map(|&x| x * h_sqrt[i]).collect())
        .collect();

    (l_rot, rot)
}

// ───────────────────────── PROMAX rotation ─────────────────────────

/// Result of a PROMAX (oblique) rotation.
pub struct PromaxResult {
    /// Oblique factor pattern P (n_vars × k): standardized regression
    /// coefficients of the variables on the (correlated) factors.
    pub pattern: Vec<Vec<f64>>,
    /// Inter-factor correlation matrix Φ (k × k).
    pub phi: Vec<Vec<f64>>,
}

/// Multiply two row-major matrices: (m×n) · (n×p) → (m×p).
pub(super) fn matmul(a: &[Vec<f64>], b: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let m = a.len();
    let n = if m > 0 { a[0].len() } else { 0 };
    let p = if !b.is_empty() { b[0].len() } else { 0 };
    let mut out = vec![vec![0.0_f64; p]; m];
    for i in 0..m {
        for k in 0..n {
            let aik = a[i][k];
            if aik == 0.0 {
                continue;
            }
            for j in 0..p {
                out[i][j] += aik * b[k][j];
            }
        }
    }
    out
}

/// Transpose a row-major matrix.
pub(super) fn transpose(a: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let m = a.len();
    let n = if m > 0 { a[0].len() } else { 0 };
    let mut out = vec![vec![0.0_f64; m]; n];
    for i in 0..m {
        for j in 0..n {
            out[j][i] = a[i][j];
        }
    }
    out
}

/// Apply PROMAX (Hendrickson & White 1964) oblique rotation, starting from the
/// orthogonal VARIMAX loadings `l_varimax` (n_vars × k). Power `power` (k=4 by
/// default in SAS) controls how aggressively the target sharpens the loadings.
///
/// Algorithm:
///   1. Build the target matrix `target[i][j] = |l[i][j]|^(power+1) / l[i][j]`
///      (sign-preserving power), i.e. raise each loading to `power` in
///      magnitude while keeping its sign.
///   2. Least-squares fit a transformation Q minimizing ‖L·Q − target‖:
///      Q = (Lᵀ L)⁻¹ Lᵀ target  (Procrustes / column-wise regression).
///   3. Normalize the columns of Q so that diag((QᵀQ)⁻¹) = 1, giving the
///      oblique pattern P = L · Q and inter-factor correlations
///      Φ = (Qᵀ Q)⁻¹ after the same normalization.
///
/// Returns the oblique pattern and the inter-factor correlation matrix.
pub fn promax(l_varimax: &[Vec<f64>], power: i32) -> Result<PromaxResult> {
    let n_vars = l_varimax.len();
    let k = if n_vars > 0 { l_varimax[0].len() } else { 0 };

    if k < 2 {
        // No oblique rotation possible with a single factor: identity Φ.
        let phi = vec![vec![1.0_f64; k.max(1)]; k.max(1)];
        return Ok(PromaxResult {
            pattern: l_varimax.to_vec(),
            phi,
        });
    }

    // 1. Sign-preserving power target: target = sign(l) * |l|^power.
    let target: Vec<Vec<f64>> = l_varimax
        .iter()
        .map(|row| {
            row.iter()
                .map(|&x| x.signum() * x.abs().powi(power))
                .collect()
        })
        .collect();

    // 2. Q = (Lᵀ L)⁻¹ Lᵀ target.
    let lt = transpose(l_varimax);
    let ltl = matmul(&lt, l_varimax); // k×k
    let ltl_inv = invert_matrix(&ltl)?;
    let lt_target = matmul(&lt, &target); // k×k
    let mut q = matmul(&ltl_inv, &lt_target); // k×k

    // 3. Normalize columns of Q so that the resulting factors have unit
    //    variance: scale column j by 1/sqrt(diag((QᵀQ)⁻¹)[j]).
    let qtq = matmul(&transpose(&q), &q); // k×k
    let qtq_inv = invert_matrix(&qtq)?;
    let scale: Vec<f64> = (0..k)
        .map(|j| {
            let d = qtq_inv[j][j];
            if d > 0.0 { d.sqrt() } else { 1.0 }
        })
        .collect();
    for row in q.iter_mut() {
        for j in 0..k {
            row[j] *= scale[j];
        }
    }

    // Oblique pattern P = L · Q.
    let pattern = matmul(l_varimax, &q);

    // Inter-factor correlations Φ = (Qᵀ Q)⁻¹ for the normalized Q.
    let qtq2 = matmul(&transpose(&q), &q);
    let mut phi = invert_matrix(&qtq2)?;
    // Force exact unit diagonal and symmetry (clean display).
    for i in 0..k {
        for j in (i + 1)..k {
            let avg = 0.5 * (phi[i][j] + phi[j][i]);
            phi[i][j] = avg;
            phi[j][i] = avg;
        }
    }
    for i in 0..k {
        phi[i][i] = 1.0;
    }

    Ok(PromaxResult { pattern, phi })
}
