// MQ7.2c — `needless_range_loop` assumé dans ce module : l'indice EST le
// langage du domaine (`a[i][j] * b[j][k]`, parcours colonne-major, triangle
// d'une matrice symétrique). La forme itérateur y coûte plus en lisibilité
// qu'elle n'en rend, et la revue a préféré garder les indices explicites.
#![allow(clippy::needless_range_loop)]

use super::*;

/// Resolve VAR columns (user order preserved), validating existence + type.
pub(super) fn resolve_var_columns(
    ds: &crate::dataset::SasDataset,
    ast: &PrincompAst,
    display: &str,
) -> Result<Vec<usize>> {
    let mut cols: Vec<usize> = Vec::with_capacity(ast.var.len());
    for nm in &ast.var {
        match ds.vars.iter().position(|m| m.name.eq_ignore_ascii_case(nm)) {
            Some(i) => {
                if ds.vars[i].ty != VarType::Num {
                    return Err(SasError::runtime(format!(
                        "Variable '{}' not found in dataset '{}'.",
                        nm, display
                    )));
                }
                cols.push(i);
            }
            None => {
                return Err(SasError::runtime(format!(
                    "Variable '{}' not found in dataset '{}'.",
                    nm, display
                )));
            }
        }
    }
    Ok(cols)
}

/// Means, sample stds (n-1) and the analysis matrix — covariance if `cov`,
/// else correlation — symmetrized exactly before the Jacobi eigen-solver.
pub(super) fn compute_analysis_matrix(
    data_rows: &[Vec<f64>],
    p: usize,
    cov: bool,
) -> (Vec<f64>, Vec<f64>, Vec<Vec<f64>>) {
    let n = data_rows.len();
    // Means and sample std (n-1).
    let nf = n as f64;
    let mut means = vec![0.0_f64; p];
    for row in data_rows {
        for j in 0..p {
            means[j] += row[j];
        }
    }
    for m in &mut means {
        *m /= nf;
    }
    // Sum of squares of deviations per variable; sample std = sqrt(SS/(n-1)).
    let mut ss = vec![0.0_f64; p];
    for row in data_rows {
        for j in 0..p {
            let d = row[j] - means[j];
            ss[j] += d * d;
        }
    }
    let denom = if n > 1 { nf - 1.0 } else { 1.0 };
    let stds: Vec<f64> = ss.iter().map(|s| (s / denom).sqrt()).collect();

    // Covariance matrix (n-1).
    let mut covm = vec![vec![0.0_f64; p]; p];
    for row in data_rows {
        for i in 0..p {
            let di = row[i] - means[i];
            for j in 0..p {
                let dj = row[j] - means[j];
                covm[i][j] += di * dj;
            }
        }
    }
    for i in 0..p {
        for j in 0..p {
            covm[i][j] /= denom;
        }
    }

    // Analysis matrix: covariance if COV, else correlation.
    let mut amat = vec![vec![0.0_f64; p]; p];
    if cov {
        amat = covm.clone();
    } else {
        for i in 0..p {
            for j in 0..p {
                let denom_ij = stds[i] * stds[j];
                amat[i][j] = if denom_ij > 0.0 {
                    (covm[i][j] / denom_ij).clamp(-1.0, 1.0)
                } else {
                    0.0
                };
            }
        }
        // Force exact diagonal 1.0 (clean display, valid correlation matrix).
        for i in 0..p {
            amat[i][i] = 1.0;
        }
    }
    // Enforce exact symmetry before Jacobi (rounding can break it).
    for i in 0..p {
        for j in (i + 1)..p {
            let avg = 0.5 * (amat[i][j] + amat[j][i]);
            amat[i][j] = avg;
            amat[j][i] = avg;
        }
    }
    (means, stds, amat)
}
