use super::*;

/// Build and write the FACTOR OUT= dataset: every input column plus
/// `Factor1..Factorm` regression factor scores. Scores = Z · (R⁻¹ · pattern),
/// where Z is the standardized (or, for COV, centered) data and R = `amat` the
/// analysis matrix. Incomplete observations receive missing scores; rows are
/// kept in input order (mirroring SAS).
#[allow(clippy::too_many_arguments)]
pub(super) fn write_out_dataset(
    session: &mut Session,
    ds: &crate::dataset::SasDataset,
    decoded: &[Vec<f64>],
    means: &[f64],
    stds: &[f64],
    amat: &[Vec<f64>],
    pattern: &[Vec<f64>],
    cov: bool,
    p: usize,
    k: usize,
    out_ref: &DatasetRef,
) -> Result<()> {
    use crate::dataset::{SasDataset, VarMeta};
    use polars::prelude::*;

    // Scoring coefficients B = R⁻¹ · pattern  (p × k).
    let r_inv = invert_matrix(amat)?;
    let coef = matmul(&r_inv, pattern);

    let n_read = ds.n_obs();
    let mut score_cols: Vec<Vec<Option<f64>>> = vec![Vec::with_capacity(n_read); k];
    for row_idx in 0..n_read {
        let row: Vec<f64> = decoded.iter().map(|col| col[row_idx]).collect();
        if row.iter().all(|x| x.is_finite()) {
            let z: Vec<f64> = (0..p)
                .map(|j| {
                    let centered = row[j] - means[j];
                    if cov {
                        centered
                    } else if stds[j] > 0.0 {
                        centered / stds[j]
                    } else {
                        0.0
                    }
                })
                .collect();
            for f in 0..k {
                let score: f64 = (0..p).map(|j| z[j] * coef[j][f]).sum();
                score_cols[f].push(Some(score));
            }
        } else {
            for f in 0..k {
                score_cols[f].push(None);
            }
        }
    }

    let mut out_df = ds.df.clone();
    for f in 0..k {
        let name = format!("Factor{}", f + 1);
        out_df
            .with_column(Series::new(name.into(), score_cols[f].clone()))
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
