// MQ7.2c — `needless_range_loop` assumé dans ce module : l'indice EST le
// langage du domaine (`a[i][j] * b[j][k]`, parcours colonne-major, triangle
// d'une matrice symétrique). La forme itérateur y coûte plus en lisibilité
// qu'elle n'en rend, et la revue a préféré garder les indices explicites.
#![allow(clippy::needless_range_loop)]

use super::*;

// ───────────────────────── Linear algebra ─────────────────────────

pub(super) fn mat_vec(mat: &[Vec<f64>], v: &[f64]) -> Vec<f64> {
    mat.iter()
        .map(|row| row.iter().zip(v).map(|(a, b)| a * b).sum())
        .collect()
}

pub(super) fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

pub(super) fn log_det_spd(a: &[Vec<f64>]) -> Result<f64> {
    let l = crate::stat::cholesky(a)?;
    let mut s = 0.0;
    for (i, row) in l.iter().enumerate() {
        s += row[i].ln();
    }
    Ok(2.0 * s)
}

/// Reconstruct the t×t UN covariance block from packed lower-triangular params
/// in SAS UN order: UN(1,1), UN(2,1), UN(2,2), UN(3,1), ...
pub(super) fn un_block(theta: &[f64], t: usize) -> Vec<Vec<f64>> {
    let mut m = vec![vec![0.0; t]; t];
    let mut k = 0;
    for r in 0..t {
        for c in 0..=r {
            let val = theta[k];
            m[r][c] = val;
            m[c][r] = val;
            k += 1;
        }
    }
    m
}
