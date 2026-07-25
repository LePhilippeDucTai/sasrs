use super::*;

pub(super) fn transpose(m: &Matrix) -> Matrix {
    let (nr, nc) = dims(m);
    let mut out = vec![vec![0.0; nr]; nc];
    for i in 0..nr {
        for j in 0..nc {
            out[j][i] = m[i][j];
        }
    }
    out
}

pub(super) fn elementwise(l: &Matrix, r: &Matrix, f: impl Fn(f64, f64) -> f64) -> Result<Matrix> {
    let (lr, lc) = dims(l);
    let (rr, rc) = dims(r);
    // Diffusion scalaire.
    if rr == 1 && rc == 1 {
        let s = r[0][0];
        return Ok(l.iter().map(|row| row.iter().map(|v| f(*v, s)).collect()).collect());
    }
    if lr == 1 && lc == 1 {
        let s = l[0][0];
        return Ok(r.iter().map(|row| row.iter().map(|v| f(s, *v)).collect()).collect());
    }
    if lr != rr || lc != rc {
        return Err(SasError::runtime(format!(
            "IML: matrices do not conform ({lr}x{lc} and {rr}x{rc})."
        )));
    }
    let mut out = vec![vec![0.0; lc]; lr];
    for i in 0..lr {
        for j in 0..lc {
            out[i][j] = f(l[i][j], r[i][j]);
        }
    }
    Ok(out)
}

pub(super) fn kronecker(l: &Matrix, r: &Matrix) -> Matrix {
    let (lr, lc) = dims(l);
    let (rr, rc) = dims(r);
    let mut out = vec![vec![0.0; lc * rc]; lr * rr];
    for i in 0..lr {
        for j in 0..lc {
            for p in 0..rr {
                for q in 0..rc {
                    out[i * rr + p][j * rc + q] = l[i][j] * r[p][q];
                }
            }
        }
    }
    out
}

pub(super) fn all_elems(m: &Matrix) -> Vec<f64> {
    m.iter().flat_map(|r| r.iter().cloned()).collect()
}

pub(super) fn map_elems(m: &Matrix, f: impl Fn(f64) -> f64) -> Matrix {
    m.iter().map(|r| r.iter().map(|v| f(*v)).collect()).collect()
}

// ───────────────────────── M28a.3 : algèbre linéaire ─────────────────────────

/// Vérifie qu'une matrice est carrée ; renvoie sa dimension.
pub(super) fn require_square(m: &Matrix, fname: &str) -> Result<usize> {
    let (nr, nc) = dims(m);
    if nr == 0 || nr != nc {
        return Err(SasError::runtime(format!(
            "IML: {fname} requires a square matrix (got {nr}x{nc})."
        )));
    }
    Ok(nr)
}

/// `INV(A)` → A⁻¹ via `invert_matrix`.
pub(super) fn iml_inv(a: &Matrix) -> Result<Matrix> {
    require_square(a, "INV")?;
    crate::stat::linalg::invert_matrix(a)
}

/// `SOLVE(A, b)` → x tel que A*x = b, colonne par colonne via `least_squares`.
pub(super) fn iml_solve(a: &Matrix, b: &Matrix) -> Result<Matrix> {
    let (an, _) = dims(a);
    let (bn, bc) = dims(b);
    if an != bn {
        return Err(SasError::runtime(format!(
            "IML: SOLVE dimensions do not conform (A has {an} rows, b has {bn} rows)."
        )));
    }
    let ncols_x = a[0].len();
    // Résoudre chaque colonne de b ; assembler la solution (n×bc).
    let mut sols: Vec<Vec<f64>> = Vec::with_capacity(bc);
    for j in 0..bc {
        let col: Vec<f64> = (0..bn).map(|i| b[i][j]).collect();
        sols.push(crate::stat::linalg::least_squares(a, &col)?);
    }
    let mut out = vec![vec![0.0; bc]; ncols_x];
    for (j, sol) in sols.iter().enumerate() {
        for (i, &v) in sol.iter().enumerate() {
            out[i][j] = v;
        }
    }
    Ok(out)
}

/// `EIGVAL(A)` → vecteur colonne des valeurs propres (ordre décroissant).
/// SAS n'accepte que les matrices symétriques.
pub(super) fn iml_eigval(a: &Matrix) -> Result<Matrix> {
    let n = require_square(a, "EIGVAL")?;
    for i in 0..n {
        for j in (i + 1)..n {
            if (a[i][j] - a[j][i]).abs() > 1e-10 {
                return Err(SasError::runtime(
                    "ERROR: The argument to the EIGVAL function must be a symmetric matrix.",
                ));
            }
        }
    }
    let vals = crate::stat::linalg::eigenvalues_jacobi(a)?;
    Ok(vals.into_iter().map(|v| vec![v]).collect())
}

/// `CHOL(A)` → U upper triangular telle que U'*U = A (SAS convention).
/// `cholesky` renvoie L (lower) avec L*L'=A ; on transpose.
pub(super) fn iml_chol(a: &Matrix) -> Result<Matrix> {
    require_square(a, "CHOL")?;
    let l = crate::stat::linalg::cholesky(a)?;
    Ok(transpose(&l))
}

/// `SHAPE(x, nrow [, ncol])` → reshape row-major, recycling elements (SAS).
/// `nrow = 0` infers the number of rows from the element count and `ncol`.
/// `ncol = 0` (or omitted) infers the number of columns from the count and
/// `nrow`. When the target has more cells than the source, the source elements
/// are recycled (repeated) in row-major order; when fewer, they are truncated.
pub(super) fn iml_shape(src: &Matrix, nrow: i64, ncol: i64) -> Result<Matrix> {
    let elems = all_elems(src);
    let cnt = elems.len();
    if nrow < 0 || ncol < 0 {
        return Err(SasError::runtime("IML: SHAPE dimensions must be non-negative."));
    }
    let (nr, nc) = match (nrow, ncol) {
        (0, 0) => {
            return Err(SasError::runtime(
                "IML: SHAPE requires at least the number of rows.",
            ));
        }
        (r, 0) => {
            let r = r as usize;
            if r == 0 {
                return Err(SasError::runtime("IML: SHAPE row count cannot be zero here."));
            }
            // ncol inferred: ceil(cnt / r) so all elements fit.
            let c = cnt.div_ceil(r).max(1);
            (r, c)
        }
        (0, c) => {
            let c = c as usize;
            let r = cnt.div_ceil(c).max(1);
            (r, c)
        }
        (r, c) => (r as usize, c as usize),
    };
    if cnt == 0 {
        return Err(SasError::runtime("IML: SHAPE of an empty matrix."));
    }
    let mut out = vec![vec![0.0; nc]; nr];
    let mut k = 0usize;
    for i in 0..nr {
        for j in 0..nc {
            out[i][j] = elems[k % cnt];
            k += 1;
        }
    }
    Ok(out)
}

/// `DET(A)` → determinant via LU decomposition with partial pivoting.
pub(super) fn iml_det(a: &Matrix) -> Result<f64> {
    let n = require_square(a, "DET")?;
    // Work on a mutable copy.
    let mut m: Vec<Vec<f64>> = a.to_vec();
    let mut det = 1.0_f64;
    for col in 0..n {
        // Partial pivot: find the largest magnitude entry in this column.
        let mut pivot = col;
        let mut best = m[col][col].abs();
        for r in (col + 1)..n {
            if m[r][col].abs() > best {
                best = m[r][col].abs();
                pivot = r;
            }
        }
        if best < 1e-300 {
            return Ok(0.0);
        }
        if pivot != col {
            m.swap(pivot, col);
            det = -det;
        }
        det *= m[col][col];
        let inv = 1.0 / m[col][col];
        for r in (col + 1)..n {
            let factor = m[r][col] * inv;
            if factor != 0.0 {
                for c in col..n {
                    m[r][c] -= factor * m[col][c];
                }
            }
        }
    }
    Ok(det)
}

/// `EIGVEC(A)` → matrix of eigenvectors of a symmetric matrix (columns are the
/// orthonormal eigenvectors, ordered by descending eigenvalue, matching EIGVAL).
pub(super) fn iml_eigvec(a: &Matrix) -> Result<Matrix> {
    let (vecs, _vals) = symmetric_eigen(a, "EIGVEC")?;
    Ok(vecs)
}

/// Shared symmetric eigendecomposition for EIGVEC and CALL EIGEN. Validates the
/// matrix is square and symmetric, then returns (V, λ) with columns of V being
/// orthonormal eigenvectors and λ in descending order.
pub(super) fn symmetric_eigen(a: &Matrix, fname: &str) -> Result<(Matrix, Vec<f64>)> {
    let n = require_square(a, fname)?;
    for i in 0..n {
        for j in (i + 1)..n {
            if (a[i][j] - a[j][i]).abs() > 1e-10 {
                return Err(SasError::runtime(format!(
                    "ERROR: The argument to the {fname} function must be a symmetric matrix."
                )));
            }
        }
    }
    crate::stat::linalg::eigenvectors_jacobi(a)
}

/// `CALL SVDCD(U, D, V, A)` : A = U*diag(D)*V', via la méthode ATA-Jacobi.
/// Renvoie (U, D, V) où D est un vecteur colonne des valeurs singulières.
pub(super) fn iml_svdcd(a: &Matrix) -> Result<(Matrix, Matrix, Matrix)> {
    let (m, n) = dims(a);
    if m == 0 || n == 0 {
        return Err(SasError::runtime("IML: SVDCD requires a non-empty matrix."));
    }
    if m < n {
        return Err(SasError::runtime(
            "IML: SVDCD currently requires rows >= columns (m >= n).",
        ));
    }
    // S = A'*A (n×n, symétrique).
    let at = transpose(a);
    let s = crate::stat::linalg::matrix_mult(&at, a);
    // Eigendécomposition : S = V diag(λ) V', λ décroissants.
    let (v, lambda) = crate::stat::linalg::eigenvectors_jacobi(&s)?;
    // σᵢ = sqrt(max(λᵢ, 0)).
    let sigma: Vec<f64> = lambda.iter().map(|&l| l.max(0.0).sqrt()).collect();
    // U[:,i] = A * V[:,i] / σᵢ.
    let mut u = vec![vec![0.0; n]; m];
    for i in 0..n {
        if sigma[i] > 1e-12 {
            // colonne i de V.
            let vi: Vec<f64> = (0..n).map(|r| v[r][i]).collect();
            let avi = crate::stat::linalg::matrix_vec_mult(a, &vi);
            for r in 0..m {
                u[r][i] = avi[r] / sigma[i];
            }
        } else {
            return Err(SasError::runtime(
                "IML: SVDCD: rank-deficient matrix; orthonormal completion not yet implemented.",
            ));
        }
    }
    let d: Matrix = sigma.into_iter().map(|s| vec![s]).collect();
    Ok((u, d, v))
}
