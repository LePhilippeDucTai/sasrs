//! Linear algebra (maison, déterministe).
//!
//! ## Plan d'implémentation (M24.1)
//!
//! Module d'algèbre linéaire pour modèles linéaires généralisés (M25: REG/ANOVA/GLM),
//! analyse multivariée (M27: PCA, cluster), modèles mixtes (M28).
//!
//! Tous les algorithmes réutilisent les primitives de base (matrix multiply,
//! norm) sans dépendre de crates externes (determinism + stability).
//!
//! ### Décomposition Cholesky
//! - `cholesky(a: &[Vec<f64>]) -> Result<Vec<Vec<f64>>>`
//!   Décomposition A = L·L^T où L est triangulaire inférieure.
//!   Prérequis : A symétrique, définie positive (sinon ERROR).
//!   Algoriithme : décomposition standard, temps O(n³/3).
//!   Utilisé par : résolution systèmes (x = L^{-1}(L^{-T} b)), inversion,
//!   PROC REG (normal equations).
//!   Tests : 2×2, 3×3, 5×5 matrices de référence ; rejet matrice
//!   semi-définie ; conditions numériques (Hilbert matrix).
//!
//! ### Décomposition QR via Householder
//! - `qr_decomposition(a: &[Vec<f64>]) -> Result<(Vec<Vec<f64>>, Vec<Vec<f64>>)>`
//!   Décomposition A = Q·R où Q orthonormale (m×n), R triangulaire sup (n×n).
//!   Algoriithme : réflecteurs Householder, temps O(mn²).
//!   Retour : (Q, R) tous deux matrices.
//!   Utilisé par : régression linéaire (β = R^{-1}(Q^T y)), inversion,
//!   PROC REG via SVD (alternative future), diagnostics rank-déficient.
//!   Validation : A = Q @ R à precision ~1e-12 ; Q^T Q = I.
//!   Tests : matrices pleins rang, rank-déficientes (warning) ; m=n, m>n.
//!
//! ### Moindres carrés
//! - `least_squares(x: &[Vec<f64>], y: &[f64]) -> Result<Vec<f64>>`
//!   Résout arg_β min ‖X·β - y‖² via QR : β = R^{-1}(Q^T y).
//!   Entrée : X matrice m×n, y vecteur m. Retour : β vecteur n.
//!   Gestion rank-déficiente : ERROR ou solution minimale norm (TSVD).
//!   Utilisé par : PROC REG (model simple, via QR).
//!   Tests : solutions exactes (y ∈ range(X)), overdetermined (m>n),
//!   underdetermined (m<n, rank < min(m,n)).
//!
//! ### Inversion matricielle
//! - `invert_matrix(a: &[Vec<f64>]) -> Result<Vec<Vec<f64>>>`
//!   Calcule A^{-1} pour matrice carrée inversible.
//!   Algoriithme : via QR (det. = prod. R diag.) ou Cholesky si définie
//!   positive (plus rapide).
//!   Retour : A^{-1}.
//!   Tests : 2×2, 3×3, 5×5 ; condition number divers ; rejet singulière.
//!
//! ### Valeurs propres via Jacobi
//! - `eigenvalues_jacobi(a: &[Vec<f64>]) -> Result<Vec<f64>>`
//!   Calcule valeurs propres de matrice symétrique.
//!   Algoriithme : méthode Jacobi (rotations planes), convergence quadratique.
//!   Retour : vecteur valeurs propres en ordre DÉCROISSANT.
//!   Convergence : ~O(n³) multiplications, tol. ~1e-14.
//!   Utilisé par : PCA (M27.1), analyse factorielle.
//!   Tests : matrices diagonales (triviales), 3×3 sym. matrices,
//!   Hilbert (ill-conditioned), validation trace(A) = Σλ, det(A) = Πλ.
//!
//! - `eigenvectors_jacobi(a: &[Vec<f64>]) -> Result<(Vec<Vec<f64>>, Vec<f64>)>`
//!   Calcule valeurs propres ET vecteurs propres d'une matrice symétrique.
//!   Algoriithme : Jacobi avec accumulation des rotations.
//!   Retour : (V, λ) où V = colonnes = vecteurs propres (orthonormés),
//!            λ = valeurs propres en ordre DÉCROISSANT.
//!   Validation : A @ V = V @ diag(λ) (dans les limites numériques).
//!   Tests : comme eigenvalues_jacobi + assertion orthonormalité V.
//!
//! ## Précision et stabilité
//!
//! Tous les algoriithmes ciblent la précision relative ~1e-12 en double précision.
//! Opérations basiques (matrix-vector multiply, dot product, norms) implémentées
//! directement (pas de Polars, déterminisme garanti).
//!
//! Gestion des erreurs :
//! - Matrice singulière / rank-déficiente → `SasError::Numerical(...)`
//! - Matrice non-symétrique (pour Jacobi) → erreur ou auto-symmétrisation
//! - Dimensions incompatibles → `SasError::InvalidInput(...)`
//!
//! ## Tests
//!
//! Chaque fonction inclut ≥3 tests :
//! - Cas trivial (identité, diagonale)
//! - Matrice 3×3 de référence
//! - Validation de propriétés (A = QR, det via diag R, etc.)
//! - Edge cases (small values, large values, mixed signs)

use crate::error::{Result, SasError};

/// Validate that `a` is a non-empty square matrix; return its dimension.
fn require_square(a: &[Vec<f64>]) -> Result<usize> {
    let n = a.len();
    if n == 0 {
        return Err(SasError::InvalidInput("empty matrix".into()));
    }
    if a.iter().any(|row| row.len() != n) {
        return Err(SasError::InvalidInput("matrix is not square".into()));
    }
    Ok(n)
}

/// Cholesky decomposition: A = L·L^T for symmetric positive-definite A.
/// Returns L (lower triangular). Errors if A is not square or not SPD.
pub fn cholesky(a: &[Vec<f64>]) -> Result<Vec<Vec<f64>>> {
    let n = require_square(a)?;
    let mut l = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..=i {
            let mut sum = a[i][j];
            for k in 0..j {
                sum -= l[i][k] * l[j][k];
            }
            if i == j {
                if sum <= 0.0 {
                    return Err(SasError::Numerical(
                        "matrix is not positive definite (Cholesky)".into(),
                    ));
                }
                l[i][j] = sum.sqrt();
            } else {
                l[i][j] = sum / l[j][j];
            }
        }
    }
    Ok(l)
}

/// QR decomposition via Householder reflections.
/// Returns (Q, R) where A = Q·R, Q orthonormal (m×n), R upper triangular (n×n).
/// Requires m >= n (at least as many rows as columns).
pub fn qr_decomposition(a: &[Vec<f64>]) -> Result<(Vec<Vec<f64>>, Vec<Vec<f64>>)> {
    let m = a.len();
    if m == 0 {
        return Err(SasError::InvalidInput("empty matrix".into()));
    }
    let n = a[0].len();
    if n == 0 || a.iter().any(|row| row.len() != n) {
        return Err(SasError::InvalidInput("ragged or empty matrix".into()));
    }
    if m < n {
        return Err(SasError::InvalidInput(
            "QR requires rows >= columns (m >= n)".into(),
        ));
    }

    // R starts as a copy of A; Q starts as the m×m identity.
    let mut r = a.to_vec();
    let mut q = vec![vec![0.0; m]; m];
    for (i, row) in q.iter_mut().enumerate() {
        row[i] = 1.0;
    }

    for k in 0..n {
        // Compute the norm of the sub-column R[k..m][k].
        let mut norm = 0.0;
        for i in k..m {
            norm += r[i][k] * r[i][k];
        }
        let norm = norm.sqrt();
        if norm < 1e-300 {
            continue;
        }
        // Householder vector v: sign chosen to avoid cancellation.
        let alpha = if r[k][k] >= 0.0 { -norm } else { norm };
        let mut v = vec![0.0; m];
        v[k] = r[k][k] - alpha;
        for i in (k + 1)..m {
            v[i] = r[i][k];
        }
        let vnorm_sq: f64 = v[k..m].iter().map(|&x| x * x).sum();
        if vnorm_sq < 1e-300 {
            continue;
        }

        // Apply H = I - 2 v v^T / (v^T v) to R (columns k..n).
        for j in k..n {
            let mut dot = 0.0;
            for i in k..m {
                dot += v[i] * r[i][j];
            }
            let factor = 2.0 * dot / vnorm_sq;
            for i in k..m {
                r[i][j] -= factor * v[i];
            }
        }
        // Accumulate Q := Q · H (apply H from the right to Q's columns).
        for row in q.iter_mut() {
            let mut dot = 0.0;
            for i in k..m {
                dot += v[i] * row[i];
            }
            let factor = 2.0 * dot / vnorm_sq;
            for i in k..m {
                row[i] -= factor * v[i];
            }
        }
    }

    // Reduce Q to m×n (first n columns) and R to n×n (first n rows).
    let q_reduced: Vec<Vec<f64>> = q
        .iter()
        .map(|row| row[..n].to_vec())
        .collect();
    let r_reduced: Vec<Vec<f64>> = r[..n].iter().map(|row| row.clone()).collect();
    Ok((q_reduced, r_reduced))
}

/// Least squares solution: argmin_β ‖X·β - y‖² via QR decomposition.
/// Input: X (m×n matrix), y (m-vector).
/// Output: β (n-vector). Solves R·β = Q^T·y.
pub fn least_squares(x: &[Vec<f64>], y: &[f64]) -> Result<Vec<f64>> {
    let m = x.len();
    if m == 0 {
        return Err(SasError::InvalidInput("empty design matrix".into()));
    }
    if y.len() != m {
        return Err(SasError::InvalidInput(
            "y length does not match number of rows in X".into(),
        ));
    }
    let n = x[0].len();
    let (q, r) = qr_decomposition(x)?;
    // qty = Q^T y  (n-vector): qty[j] = Σ_i Q[i][j] * y[i].
    let mut qty = vec![0.0; n];
    for (j, qj) in qty.iter_mut().enumerate() {
        let mut sum = 0.0;
        for i in 0..m {
            sum += q[i][j] * y[i];
        }
        *qj = sum;
    }
    solve_upper_triangular(&r, &qty)
}

/// Matrix inversion. Solves A·X = I column by column via QR decomposition.
pub fn invert_matrix(a: &[Vec<f64>]) -> Result<Vec<Vec<f64>>> {
    let n = require_square(a)?;
    let (q, r) = qr_decomposition(a)?;
    // For each unit column e_k, solve A x = e_k = Q R x  →  R x = Q^T e_k.
    let mut inv = vec![vec![0.0; n]; n];
    for k in 0..n {
        // Q^T e_k is the k-th row of Q^T = the k-th column of Q.
        let qte: Vec<f64> = (0..n).map(|j| q[k][j]).collect();
        let col = solve_upper_triangular(&r, &qte)?;
        for (i, &val) in col.iter().enumerate() {
            inv[i][k] = val;
        }
    }
    Ok(inv)
}

/// Run the Jacobi eigenvalue iteration on a symmetric matrix, returning the
/// (eigenvalue, eigenvector-matrix) pair before sorting. V columns are the
/// eigenvectors; the diagonal of the rotated matrix holds the eigenvalues.
fn jacobi(a: &[Vec<f64>]) -> Result<(Vec<f64>, Vec<Vec<f64>>)> {
    let n = require_square(a)?;
    // Verify (approximate) symmetry.
    for i in 0..n {
        for j in (i + 1)..n {
            if (a[i][j] - a[j][i]).abs() > 1e-9 * (1.0 + a[i][j].abs()) {
                return Err(SasError::InvalidInput(
                    "matrix is not symmetric (Jacobi)".into(),
                ));
            }
        }
    }

    let mut m = a.to_vec();
    // V accumulates the rotations; starts as identity.
    let mut v = vec![vec![0.0; n]; n];
    for (i, row) in v.iter_mut().enumerate() {
        row[i] = 1.0;
    }

    const MAX_SWEEPS: usize = 100;
    for _ in 0..MAX_SWEEPS {
        // Off-diagonal Frobenius magnitude.
        let mut off = 0.0;
        for i in 0..n {
            for j in (i + 1)..n {
                off += m[i][j] * m[i][j];
            }
        }
        if off.sqrt() < 1e-15 {
            break;
        }
        for p in 0..n {
            for q in (p + 1)..n {
                if m[p][q].abs() < 1e-300 {
                    continue;
                }
                // Compute the Jacobi rotation angle.
                let theta = (m[q][q] - m[p][p]) / (2.0 * m[p][q]);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;

                // Apply rotation to M: rows/cols p and q.
                for k in 0..n {
                    let mkp = m[k][p];
                    let mkq = m[k][q];
                    m[k][p] = c * mkp - s * mkq;
                    m[k][q] = s * mkp + c * mkq;
                }
                for k in 0..n {
                    let mpk = m[p][k];
                    let mqk = m[q][k];
                    m[p][k] = c * mpk - s * mqk;
                    m[q][k] = s * mpk + c * mqk;
                }
                // Accumulate the rotation into V.
                for row in v.iter_mut() {
                    let vp = row[p];
                    let vq = row[q];
                    row[p] = c * vp - s * vq;
                    row[q] = s * vp + c * vq;
                }
            }
        }
    }

    let eigvals: Vec<f64> = (0..n).map(|i| m[i][i]).collect();
    Ok((eigvals, v))
}

/// Eigenvalues of symmetric matrix via Jacobi method.
/// Returns eigenvalues in descending order.
pub fn eigenvalues_jacobi(a: &[Vec<f64>]) -> Result<Vec<f64>> {
    let (mut eigvals, _) = jacobi(a)?;
    eigvals.sort_by(|x, y| y.partial_cmp(x).unwrap_or(std::cmp::Ordering::Equal));
    Ok(eigvals)
}

/// Eigenvalues and eigenvectors of symmetric matrix via Jacobi method.
/// Returns (V, λ) where V columns are orthonormal eigenvectors,
/// λ are eigenvalues in descending order.
/// Satisfies: A @ V = V @ diag(λ) (approximately).
pub fn eigenvectors_jacobi(a: &[Vec<f64>]) -> Result<(Vec<Vec<f64>>, Vec<f64>)> {
    let (eigvals, v) = jacobi(a)?;
    let n = eigvals.len();
    // Sort indices by eigenvalue descending.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| {
        eigvals[j]
            .partial_cmp(&eigvals[i])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let sorted_vals: Vec<f64> = order.iter().map(|&i| eigvals[i]).collect();
    // Reorder columns of V to match.
    let mut sorted_v = vec![vec![0.0; n]; n];
    for (new_col, &old_col) in order.iter().enumerate() {
        for row in 0..n {
            sorted_v[row][new_col] = v[row][old_col];
        }
    }
    Ok((sorted_v, sorted_vals))
}

// ───────────────────────── Helper functions ───────────────────────────

/// Transpose a matrix.
fn transpose(a: &[Vec<f64>]) -> Vec<Vec<f64>> {
    if a.is_empty() {
        return vec![];
    }
    let n_cols = a[0].len();
    let mut result = vec![vec![0.0; a.len()]; n_cols];
    for (i, row) in a.iter().enumerate() {
        for (j, &val) in row.iter().enumerate() {
            result[j][i] = val;
        }
    }
    result
}

/// Matrix-vector multiplication: y = A @ x.
fn matrix_vec_mult(a: &[Vec<f64>], x: &[f64]) -> Vec<f64> {
    let mut y = vec![0.0; a.len()];
    for (i, row) in a.iter().enumerate() {
        for (j, &val) in row.iter().enumerate() {
            y[i] += val * x[j];
        }
    }
    y
}

/// Matrix-matrix multiplication: C = A @ B.
fn matrix_mult(a: &[Vec<f64>], b: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let m = a.len();
    let n = b[0].len();
    let k = b.len();
    let mut c = vec![vec![0.0; n]; m];
    for i in 0..m {
        for j in 0..n {
            for p in 0..k {
                c[i][j] += a[i][p] * b[p][j];
            }
        }
    }
    c
}

/// Frobenius norm: ‖A‖_F = sqrt(Σ a_ij²).
fn frobenius_norm(a: &[Vec<f64>]) -> f64 {
    a.iter().flat_map(|row| row.iter()).map(|&x| x * x).sum::<f64>().sqrt()
}

/// Solve upper triangular system R @ x = y.
fn solve_upper_triangular(r: &[Vec<f64>], y: &[f64]) -> Result<Vec<f64>> {
    let n = r.len();
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let mut sum = y[i];
        for j in (i + 1)..n {
            sum -= r[i][j] * x[j];
        }
        if r[i][i].abs() < 1e-14 {
            return Err(SasError::Numerical("Singular matrix in back-substitution".into()));
        }
        x[i] = sum / r[i][i];
    }
    Ok(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * (1.0 + b.abs())
    }

    fn mat_approx(a: &[Vec<f64>], b: &[Vec<f64>], tol: f64) -> bool {
        a.len() == b.len()
            && a.iter().zip(b).all(|(ra, rb)| {
                ra.len() == rb.len() && ra.iter().zip(rb).all(|(&x, &y)| approx(x, y, tol))
            })
    }

    // ───────────────────────── helper coverage ─────────────────────────

    #[test]
    fn test_helpers() {
        let a = vec![vec![1.0, 2.0], vec![3.0, 4.0]];
        let t = transpose(&a);
        assert_eq!(t, vec![vec![1.0, 3.0], vec![2.0, 4.0]]);

        let y = matrix_vec_mult(&a, &[1.0, 1.0]);
        assert_eq!(y, vec![3.0, 7.0]);

        let c = matrix_mult(&a, &transpose(&a));
        assert_eq!(c, vec![vec![5.0, 11.0], vec![11.0, 25.0]]);

        let fn_norm = frobenius_norm(&vec![vec![3.0, 4.0]]);
        assert!(approx(fn_norm, 5.0, 1e-12));

        let r = vec![vec![2.0, 1.0], vec![0.0, 3.0]];
        let x = solve_upper_triangular(&r, &[5.0, 9.0]).unwrap();
        // 3x1=9 → x1=3; 2x0+1*3=5 → x0=1.
        assert!(approx(x[0], 1.0, 1e-12) && approx(x[1], 3.0, 1e-12));
    }

    // ───────────────────────── Cholesky ─────────────────────────

    #[test]
    fn test_cholesky_2x2() {
        let a = vec![vec![4.0, 2.0], vec![2.0, 3.0]];
        let l = cholesky(&a).unwrap();
        // Reconstruct L·L^T == A.
        let recon = matrix_mult(&l, &transpose(&l));
        assert!(mat_approx(&recon, &a, 1e-12));
        // L lower triangular.
        assert!(approx(l[0][1], 0.0, 1e-15));
    }

    #[test]
    fn test_cholesky_3x3() {
        let a = vec![
            vec![25.0, 15.0, -5.0],
            vec![15.0, 18.0, 0.0],
            vec![-5.0, 0.0, 11.0],
        ];
        let l = cholesky(&a).unwrap();
        let recon = matrix_mult(&l, &transpose(&l));
        assert!(mat_approx(&recon, &a, 1e-10));
    }

    #[test]
    fn test_cholesky_not_spd() {
        // Negative definite / indefinite → error.
        let a = vec![vec![1.0, 2.0], vec![2.0, 1.0]];
        assert!(cholesky(&a).is_err());
        // Non-square → error.
        let b = vec![vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0]];
        assert!(cholesky(&b).is_err());
    }

    // ───────────────────────── QR ─────────────────────────

    #[test]
    fn test_qr_reconstruction() {
        let a = vec![
            vec![12.0, -51.0, 4.0],
            vec![6.0, 167.0, -68.0],
            vec![-4.0, 24.0, -41.0],
        ];
        let (q, r) = qr_decomposition(&a).unwrap();
        let recon = matrix_mult(&q, &r);
        assert!(mat_approx(&recon, &a, 1e-9));
    }

    #[test]
    fn test_qr_orthonormal() {
        let a = vec![
            vec![1.0, 1.0],
            vec![1.0, 0.0],
            vec![0.0, 1.0],
        ];
        let (q, _) = qr_decomposition(&a).unwrap();
        // Q^T Q = I (n×n).
        let qtq = matrix_mult(&transpose(&q), &q);
        let id = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        assert!(mat_approx(&qtq, &id, 1e-10));
    }

    #[test]
    fn test_qr_tall_and_errors() {
        // m < n must error.
        let wide = vec![vec![1.0, 2.0, 3.0]];
        assert!(qr_decomposition(&wide).is_err());
        // Identity QR.
        let id = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let (_, r) = qr_decomposition(&id).unwrap();
        assert!(approx(r[0][0].abs(), 1.0, 1e-12));
    }

    // ───────────────────────── least squares ─────────────────────────

    #[test]
    fn test_least_squares_exact() {
        // y exactly in range(X): solve perfectly. X has intercept + slope.
        let x = vec![
            vec![1.0, 0.0],
            vec![1.0, 1.0],
            vec![1.0, 2.0],
        ];
        // y = 2 + 3*t.
        let y = vec![2.0, 5.0, 8.0];
        let beta = least_squares(&x, &y).unwrap();
        assert!(approx(beta[0], 2.0, 1e-10) && approx(beta[1], 3.0, 1e-10));
    }

    #[test]
    fn test_least_squares_overdetermined() {
        // Noisy y, fit slope/intercept (known SAS-style result).
        let x = vec![
            vec![1.0, 1.0],
            vec![1.0, 2.0],
            vec![1.0, 3.0],
            vec![1.0, 4.0],
        ];
        let y = vec![6.0, 5.0, 7.0, 10.0];
        let beta = least_squares(&x, &y).unwrap();
        // Normal-equations result: slope=1.4, intercept=3.5.
        assert!(approx(beta[0], 3.5, 1e-8));
        assert!(approx(beta[1], 1.4, 1e-8));
    }

    #[test]
    fn test_least_squares_errors() {
        let x = vec![vec![1.0], vec![1.0]];
        let y = vec![1.0]; // wrong length
        assert!(least_squares(&x, &y).is_err());
    }

    // ───────────────────────── inversion ─────────────────────────

    #[test]
    fn test_invert_2x2() {
        let a = vec![vec![4.0, 7.0], vec![2.0, 6.0]];
        let inv = invert_matrix(&a).unwrap();
        let prod = matrix_mult(&a, &inv);
        let id = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        assert!(mat_approx(&prod, &id, 1e-10));
    }

    #[test]
    fn test_invert_3x3() {
        let a = vec![
            vec![2.0, -1.0, 0.0],
            vec![-1.0, 2.0, -1.0],
            vec![0.0, -1.0, 2.0],
        ];
        let inv = invert_matrix(&a).unwrap();
        let prod = matrix_mult(&a, &inv);
        let id = vec![
            vec![1.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![0.0, 0.0, 1.0],
        ];
        assert!(mat_approx(&prod, &id, 1e-9));
    }

    #[test]
    fn test_invert_singular() {
        let a = vec![vec![1.0, 2.0], vec![2.0, 4.0]];
        assert!(invert_matrix(&a).is_err());
    }

    // ───────────────────────── eigenvalues ─────────────────────────

    #[test]
    fn test_eigenvalues_diagonal() {
        let a = vec![
            vec![3.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![0.0, 0.0, 2.0],
        ];
        let ev = eigenvalues_jacobi(&a).unwrap();
        // Descending order.
        assert!(approx(ev[0], 3.0, 1e-12));
        assert!(approx(ev[1], 2.0, 1e-12));
        assert!(approx(ev[2], 1.0, 1e-12));
    }

    #[test]
    fn test_eigenvalues_symmetric() {
        let a = vec![vec![2.0, 1.0], vec![1.0, 2.0]];
        let ev = eigenvalues_jacobi(&a).unwrap();
        // Eigenvalues 3 and 1.
        assert!(approx(ev[0], 3.0, 1e-10));
        assert!(approx(ev[1], 1.0, 1e-10));
        // trace = Σλ.
        assert!(approx(ev.iter().sum::<f64>(), 4.0, 1e-10));
    }

    #[test]
    fn test_eigenvalues_not_symmetric() {
        let a = vec![vec![1.0, 2.0], vec![0.0, 1.0]];
        assert!(eigenvalues_jacobi(&a).is_err());
    }

    // ───────────────────────── eigenvectors ─────────────────────────

    #[test]
    fn test_eigenvectors_reconstruction() {
        let a = vec![vec![2.0, 1.0], vec![1.0, 2.0]];
        let (v, lam) = eigenvectors_jacobi(&a).unwrap();
        // A @ V should equal V @ diag(lam).
        let av = matrix_mult(&a, &v);
        let mut vd = vec![vec![0.0; 2]; 2];
        for i in 0..2 {
            for j in 0..2 {
                vd[i][j] = v[i][j] * lam[j];
            }
        }
        assert!(mat_approx(&av, &vd, 1e-9));
    }

    #[test]
    fn test_eigenvectors_orthonormal() {
        let a = vec![
            vec![4.0, 1.0, 0.0],
            vec![1.0, 3.0, 1.0],
            vec![0.0, 1.0, 2.0],
        ];
        let (v, _) = eigenvectors_jacobi(&a).unwrap();
        // V^T V = I.
        let vtv = matrix_mult(&transpose(&v), &v);
        let id = vec![
            vec![1.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![0.0, 0.0, 1.0],
        ];
        assert!(mat_approx(&vtv, &id, 1e-9));
    }

    #[test]
    fn test_eigenvectors_descending() {
        let a = vec![vec![1.0, 0.0], vec![0.0, 5.0]];
        let (_, lam) = eigenvectors_jacobi(&a).unwrap();
        assert!(lam[0] >= lam[1]);
        assert!(approx(lam[0], 5.0, 1e-12));
    }
}
