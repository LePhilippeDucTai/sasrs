// MQ7.2c — `needless_range_loop` assumé dans ce module : l'indice EST le
// langage du domaine (`a[i][j] * b[j][k]`, parcours colonne-major, triangle
// d'une matrice symétrique). La forme itérateur y coûte plus en lisibilité
// qu'elle n'en rend, et la revue a préféré garder les indices explicites.
#![allow(clippy::needless_range_loop)]

use super::*;

/// Resolve VAR columns (user order preserved), validating existence + type.
pub(super) fn resolve_var_columns(
    ds: &crate::dataset::SasDataset,
    ast: &FactorAst,
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

/// `_TYPE_` values marking the matrix rows of a TYPE=CORR, COV, UCORR, UCOV
/// or SSCP data set (SAS 9.4, Appendix A « Special SAS Data Sets »).
const MATRIX_TYPE_ROWS: &[&str] = &["CORR", "COV", "UCORR", "UCOV", "SSCP"];

/// J02-P4 — a TYPE=CORR/COV data set (character `_TYPE_` and `_NAME_`
/// variables, rows MEAN, STD, N, CORR…) used to be analyzed as raw
/// observations. SAS/STAT 9.4, PROC FACTOR statement, DATA= : such a data
/// set is read as a matrix, which is not implemented. ERROR naming the unit
/// of the manifest (roadmap-avancee J09-P5).
pub(super) fn reject_type_corr_layout(
    ds: &crate::dataset::SasDataset,
    display: &str,
) -> Result<()> {
    let char_var = |name: &str| {
        ds.vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(name) && m.ty == VarType::Char)
    };
    let (Some(type_idx), Some(_)) = (char_var("_TYPE_"), char_var("_NAME_")) else {
        return Ok(());
    };
    let matrix_rows = decode_column(ds, type_idx)?.iter().any(|v| {
        matches!(v, Value::Char(s)
            if MATRIX_TYPE_ROWS.contains(&s.trim().to_ascii_uppercase().as_str()))
    });
    if matrix_rows {
        return Err(SasError::runtime(format!(
            "A TYPE=CORR/COV input data set ({display} has _TYPE_ and _NAME_ variables) is \
             not supported in PROC FACTOR; it can affect results and cannot be ignored \
             (planned: roadmap-avancee J09-P5)."
        )));
    }
    Ok(())
}

/// `(means, sample stds, analysis matrix)` of [`compute_analysis_matrix`].
type AnalysisMatrix = (Vec<f64>, Vec<f64>, Vec<Vec<f64>>);

/// Means, sample stds (n-1) and the analysis matrix — covariance if `cov`,
/// else correlation — symmetrized exactly before the Jacobi eigen-solver.
///
/// J02-P4 — with the correlation matrix, a zero-variance variable used to get
/// its correlations forced to 0 (diagonal 1): an undefined matrix was
/// analyzed silently. ERROR naming the variable (roadmap-avancee J09-P5
/// aligns it on the documented behavior).
pub(super) fn compute_analysis_matrix(
    data_rows: &[Vec<f64>],
    names: &[String],
    cov: bool,
) -> Result<AnalysisMatrix> {
    let p = names.len();
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
    let mut ss = vec![0.0_f64; p];
    for row in data_rows {
        for j in 0..p {
            let d = row[j] - means[j];
            ss[j] += d * d;
        }
    }
    let denom = if n > 1 { nf - 1.0 } else { 1.0 };
    let stds: Vec<f64> = ss.iter().map(|s| (s / denom).sqrt()).collect();

    if !cov
        && let Some(j) = (0..p)
            .find(|&j| stds[j] == 0.0 || data_rows.iter().all(|row| row[j] == data_rows[0][j]))
    {
        return Err(SasError::runtime(format!(
            "The VAR variable {} has zero variance (its correlations are undefined), which \
             is not supported in PROC FACTOR; it can affect results and cannot be ignored \
             (planned: roadmap-avancee J09-P5).",
            names[j].to_uppercase()
        )));
    }

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

    // Analysis matrix: covariance if cov, else correlation.
    let mut amat = vec![vec![0.0_f64; p]; p];
    if cov {
        amat = covm.clone();
    } else {
        // Every std is > 0 here (zero variance rejected above); a non-finite
        // std propagates NaN, which the Jacobi solver rejects.
        for i in 0..p {
            for j in 0..p {
                amat[i][j] = (covm[i][j] / (stds[i] * stds[j])).clamp(-1.0, 1.0);
            }
        }
        for i in 0..p {
            amat[i][i] = 1.0;
        }
    }
    // Enforce exact symmetry before Jacobi.
    for i in 0..p {
        for j in (i + 1)..p {
            let avg = 0.5 * (amat[i][j] + amat[j][i]);
            amat[i][j] = avg;
            amat[j][i] = avg;
        }
    }
    Ok((means, stds, amat))
}
