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
