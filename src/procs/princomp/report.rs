use super::*;

/// "Simple Statistics" table: rows Mean / StdDev, columns = variables.
pub(super) fn print_simple_statistics(session: &mut Session, names: &[String], means: &[f64], stds: &[f64]) {
    let p = names.len();
    centered(session, "Simple Statistics");
    session.listing.blank();
    let mut headers: Vec<String> = vec![String::new()];
    let mut aligns: Vec<Align> = vec![Align::Left];
    for nm in names {
        headers.push(nm.clone());
        aligns.push(Align::Right);
    }
    let mut mean_row = vec!["Mean".to_string()];
    for j in 0..p {
        mean_row.push(format!("{:.4}", means[j]));
    }
    let mut std_row = vec!["StdDev".to_string()];
    for j in 0..p {
        std_row.push(format!("{:.4}", stds[j]));
    }
    session
        .listing
        .write_table(&headers, &aligns, &[mean_row, std_row]);
    session.listing.blank();
}

/// Correlation / Covariance Matrix table.
pub(super) fn print_analysis_matrix(session: &mut Session, cov: bool, names: &[String], amat: &[Vec<f64>]) {
    let p = names.len();
    let matrix_title = if cov {
        "Covariance Matrix"
    } else {
        "Correlation Matrix"
    };
    centered(session, matrix_title);
    session.listing.blank();
    let mut headers: Vec<String> = vec![String::new()];
    let mut aligns: Vec<Align> = vec![Align::Left];
    for nm in names {
        headers.push(nm.clone());
        aligns.push(Align::Right);
    }
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(p);
    for i in 0..p {
        let mut row = vec![names[i].clone()];
        for j in 0..p {
            row.push(format!("{:.4}", amat[i][j]));
        }
        rows.push(row);
    }
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
}

/// Eigenvalues table (first k of p, PRIN1..PRINk rows).
pub(super) fn print_eigenvalue_table(
    session: &mut Session,
    cov: bool,
    lambda: &[f64],
    trace: f64,
    p: usize,
    k: usize,
) {
    let eig_title = if cov {
        "Eigenvalues of the Covariance Matrix"
    } else {
        "Eigenvalues of the Correlation Matrix"
    };
    centered(session, eig_title);
    session.listing.blank();
    let headers: Vec<String> = vec![
        String::new(),
        "Eigenvalue".into(),
        "Difference".into(),
        "Proportion".into(),
        "Cumulative".into(),
    ];
    let aligns = vec![
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(k);
    let mut cumulative = 0.0_f64;
    for i in 0..k {
        cumulative += lambda[i];
        let diff = if i + 1 < p {
            format!("{:.4}", lambda[i] - lambda[i + 1])
        } else {
            ".".to_string()
        };
        let proportion = if trace != 0.0 { lambda[i] / trace } else { 0.0 };
        let cumul = if trace != 0.0 { cumulative / trace } else { 0.0 };
        rows.push(vec![
            format!("PRIN{}", i + 1),
            format!("{:.4}", lambda[i]),
            diff,
            format!("{:.4}", proportion),
            format!("{:.4}", cumul),
        ]);
    }
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
}

/// Eigenvectors table (6 decimals, first k columns).
pub(super) fn print_eigenvectors(session: &mut Session, names: &[String], v: &[Vec<f64>], k: usize) {
    let p = names.len();
    centered(session, "Eigenvectors");
    session.listing.blank();
    let mut headers: Vec<String> = vec![String::new()];
    let mut aligns: Vec<Align> = vec![Align::Left];
    for i in 0..k {
        headers.push(format!("PRIN{}", i + 1));
        aligns.push(Align::Right);
    }
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(p);
    for row in 0..p {
        let mut r = vec![names[row].clone()];
        for col in 0..k {
            r.push(format!("{:.6}", v[row][col]));
        }
        rows.push(r);
    }
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
}

/// Build and write the PRINCOMP OUT= dataset: every input column plus
/// `Prin1..Prink` component scores. `decoded` holds the analysis columns in
/// original-row order (NaN for missing); `means`/`stds` are the sample
/// statistics; `v` the eigenvectors (columns), already sign-fixed.
#[allow(clippy::too_many_arguments)]
pub(super) fn write_out_dataset(
    session: &mut Session,
    ds: &crate::dataset::SasDataset,
    decoded: &[Vec<f64>],
    means: &[f64],
    stds: &[f64],
    v: &[Vec<f64>],
    cov: bool,
    p: usize,
    k: usize,
    out_ref: &DatasetRef,
) -> Result<()> {
    use crate::dataset::{SasDataset, VarMeta};
    use polars::prelude::*;

    let n_read = ds.n_obs();

    // Per-component score columns (Option<f64>: None => SAS missing).
    let mut score_cols: Vec<Vec<Option<f64>>> = vec![Vec::with_capacity(n_read); k];
    for r in 0..n_read {
        let row: Vec<f64> = decoded.iter().map(|col| col[r]).collect();
        if row.iter().all(|x| x.is_finite()) {
            // Standardize (or center for COV).
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
            for comp in 0..k {
                let score: f64 = (0..p).map(|j| z[j] * v[j][comp]).sum();
                score_cols[comp].push(Some(score));
            }
        } else {
            for comp in 0..k {
                score_cols[comp].push(None);
            }
        }
    }

    let mut out_df = ds.df.clone();
    for comp in 0..k {
        let name = format!("Prin{}", comp + 1);
        out_df
            .with_column(Series::new(name.into(), score_cols[comp].clone()))
            .map_err(|e| SasError::runtime(format!("PRINCOMP OUT= build failed: {e}")))?;
    }

    let mut vars = ds.vars.clone();
    for comp in 0..k {
        vars.push(VarMeta {
            name: format!("Prin{}", comp + 1),
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
