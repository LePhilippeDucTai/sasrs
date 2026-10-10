use super::*;

/// Validate the OUT= request before any output (J02-P4): the libref must be
/// assigned and the scoring coefficients B = R⁻¹ · pattern (p × k) must be
/// computable. Both failures used to surface only after the listing had been
/// printed. Returns B.
pub(super) fn prepare_out_scoring(
    session: &Session,
    out_ref: &DatasetRef,
    amat: &[Vec<f64>],
    pattern: &[Vec<f64>],
    cov: bool,
) -> Result<Vec<Vec<f64>>> {
    session.libs.get(&out_ref.libref_or_work())?;
    let r_inv = invert_matrix(amat).map_err(|_| {
        SasError::runtime(format!(
            "The {} matrix is singular; PROC FACTOR cannot compute the OUT= factor scores.",
            if cov { "covariance" } else { "correlation" }
        ))
    })?;
    Ok(matmul(&r_inv, pattern))
}

/// Build and write the FACTOR OUT= dataset: every input column plus
/// `Factor1..Factorm` regression factor scores. Scores = Z · B, where Z is the
/// standardized (or, for COV, centered) data and B = R⁻¹ · pattern the
/// scoring coefficients of [`prepare_out_scoring`]. Incomplete observations
/// receive missing scores; rows are kept in input order (mirroring SAS).
#[allow(clippy::too_many_arguments)]
pub(super) fn write_out_dataset(
    session: &mut Session,
    ds: &crate::dataset::SasDataset,
    decoded: &[Vec<f64>],
    means: &[f64],
    stds: &[f64],
    coef: &[Vec<f64>],
    cov: bool,
    p: usize,
    k: usize,
    out_ref: &DatasetRef,
) -> Result<()> {
    use crate::dataset::{SasDataset, VarMeta};
    use polars::prelude::*;

    let n_read = ds.n_obs();
    let mut score_cols: Vec<Vec<Option<f64>>> = vec![Vec::with_capacity(n_read); k];
    for row_idx in 0..n_read {
        let row: Vec<f64> = decoded.iter().map(|col| col[row_idx]).collect();
        if row.iter().all(|x| x.is_finite()) {
            // Correlation analysis rejects a zero-variance variable upstream,
            // so every std is > 0 here.
            let z: Vec<f64> = (0..p)
                .map(|j| {
                    let centered = row[j] - means[j];
                    if cov { centered } else { centered / stds[j] }
                })
                .collect();
            for f in 0..k {
                let score: f64 = (0..p).map(|j| z[j] * coef[j][f]).sum();
                score_cols[f].push(Some(score));
            }
        } else {
            for col in score_cols.iter_mut().take(k) {
                col.push(None);
            }
        }
    }

    let mut out_df = ds.df.clone();
    for (f, col) in score_cols.iter().enumerate().take(k) {
        let name = format!("Factor{}", f + 1);
        out_df
            .with_column(Series::new(name.into(), col.clone()))
            .map_err(|e| SasError::runtime(format!("FACTOR OUT= build failed: {e}")))?;
    }

    let mut vars = ds.vars.clone();
    for f in 0..k {
        vars.push(VarMeta {
            name: format!("Factor{}", f + 1),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
            informat: None,
        });
    }

    let out_ds = SasDataset { df: out_df, vars };
    let out_libref = out_ref.libref_or_work();
    let out_table = out_ref.name.to_uppercase();
    let out_display = format!("{out_libref}.{out_table}");
    let n_rows = out_ds.n_obs();
    let n_vars = out_ds.vars.len();
    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(out_display.clone());
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        out_display, n_rows, n_vars
    ));
    Ok(())
}
