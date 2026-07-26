//! Impression de matrices (X'X, inverse, COVB/CORRB) et SIMPLE / CORR.

use super::*;

// ───────────────────────── Printed matrices (M36.8) ─────────────────────────

/// Design-column label: "Intercept" for column 0 when an intercept is present,
/// otherwise the corresponding regressor name (M36.8).
pub(super) fn design_label(j: usize, reg_names: &[String], intercept: bool) -> String {
    if intercept {
        if j == 0 {
            "Intercept".to_string()
        } else {
            reg_names[j - 1].clone()
        }
    } else {
        reg_names[j].clone()
    }
}

/// Build the X'X crossproducts matrix augmented with X'Y and Y'Y (M36.8).
/// Returns a (p_eff+1)×(p_eff+1) matrix: the leading p_eff×p_eff block is X'X,
/// the last column is X'Y (and the last row is its transpose Y'X), and the
/// bottom-right corner is Y'Y. Column/row order matches the design matrix
/// (intercept first when present), with the dependent appended last.
pub(super) fn build_xpx(x_mat: &[Vec<f64>], y: &[f64]) -> Vec<Vec<f64>> {
    let p = x_mat[0].len();
    let n = x_mat.len();
    let m = p + 1;
    let mut a = vec![vec![0.0; m]; m];
    for r in 0..p {
        for c in 0..p {
            let mut s = 0.0;
            for i in 0..n {
                s += x_mat[i][r] * x_mat[i][c];
            }
            a[r][c] = s;
        }
    }
    // X'Y column / Y'X row.
    for r in 0..p {
        let mut s = 0.0;
        for i in 0..n {
            s += x_mat[i][r] * y[i];
        }
        a[r][p] = s;
        a[p][r] = s;
    }
    // Y'Y.
    a[p][p] = y.iter().map(|v| v * v).sum();
    a
}

/// Print a labeled square matrix (M36.8). `title` is centered; the first column
/// carries the row labels under a blank header, then one numeric column per
/// `col_labels` entry. Values use the 8-decimal `fmt8` rendering SAS uses for
/// these crossproduct/inverse displays.
fn print_labeled_matrix(
    title: &str,
    row_labels: &[String],
    col_labels: &[String],
    data: &[Vec<f64>],
    session: &mut Session,
) {
    session.listing.blank();
    session.listing.blank();
    centered(session, "The REG Procedure");
    centered(session, title);
    session.listing.blank();
    let mut headers: Vec<String> = vec!["".into()];
    let mut aligns = vec![Align::Left];
    for lbl in col_labels {
        headers.push(lbl.clone());
        aligns.push(Align::Right);
    }
    let rows: Vec<Vec<String>> = row_labels
        .iter()
        .enumerate()
        .map(|(i, lbl)| {
            let mut row = vec![lbl.clone()];
            for j in 0..col_labels.len() {
                row.push(fmt8(data[i][j]));
            }
            row
        })
        .collect();
    session.listing.write_table(&headers, &aligns, &rows);
}

/// 8-decimal fixed formatting for the crossproduct / inverse matrix cells.
fn fmt8(v: f64) -> String {
    format!("{v:.8}")
}

/// Print the XPX crossproducts matrix block (M36.8). Layout: rows/cols are
/// Intercept (if present), the regressors, then the dependent variable; the cell
/// values are X'X augmented with X'Y / Y'Y.
pub(super) fn print_xpx(
    xpx: &[Vec<f64>],
    reg_names: &[String],
    dep_name: &str,
    intercept: bool,
    session: &mut Session,
) {
    let p = xpx.len() - 1;
    let mut labels: Vec<String> = (0..p)
        .map(|j| design_label(j, reg_names, intercept))
        .collect();
    labels.push(dep_name.to_string());
    print_labeled_matrix(
        "Model Crossproducts X'X X'Y Y'Y",
        &labels,
        &labels,
        xpx,
        session,
    );
}

/// Print the (X'X)⁻¹ block augmented with the parameter estimates and SSE
/// (M36.8). The matrix is (p_eff+1)×(p_eff+1): the leading block is (X'X)⁻¹;
/// the last column holds the parameter estimates β (and the last row their
/// transpose); the bottom-right corner holds the error sum of squares (SSE).
/// This mirrors SAS's "X'X Inverse, Parameter Estimates, and SSE" display.
pub(super) fn print_inverse(
    xtx_inv: &[Vec<f64>],
    beta: &[f64],
    sse: f64,
    reg_names: &[String],
    dep_name: &str,
    intercept: bool,
    session: &mut Session,
) {
    let p = xtx_inv.len();
    let m = p + 1;
    let mut aug = vec![vec![0.0; m]; m];
    for r in 0..p {
        for c in 0..p {
            aug[r][c] = xtx_inv[r][c];
        }
        aug[r][p] = beta[r];
        aug[p][r] = beta[r];
    }
    aug[p][p] = sse;
    let mut labels: Vec<String> = (0..p)
        .map(|j| design_label(j, reg_names, intercept))
        .collect();
    labels.push(dep_name.to_string());
    print_labeled_matrix(
        "X'X Inverse, Parameter Estimates, and SSE",
        &labels,
        &labels,
        &aug,
        session,
    );
}

/// Covariance of the estimates = MSE·(X'X)⁻¹ (M36.8).
pub(super) fn covb_matrix(xtx_inv: &[Vec<f64>], mse: f64) -> Vec<Vec<f64>> {
    xtx_inv
        .iter()
        .map(|row| row.iter().map(|&v| mse * v).collect())
        .collect()
}

/// Correlation of the estimates from the covariance matrix (M36.8):
/// corr_ij = covb_ij / √(covb_ii·covb_jj); diagonal = 1.
pub(super) fn corrb_matrix(covb: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let p = covb.len();
    let mut c = vec![vec![0.0; p]; p];
    for i in 0..p {
        for j in 0..p {
            let d = (covb[i][i] * covb[j][j]).sqrt();
            c[i][j] = if d > 0.0 { covb[i][j] / d } else { 0.0 };
        }
    }
    c
}

/// Print the COVB ("Covariance of Estimates") and/or CORRB ("Correlation of
/// Estimates") matrices (M36.8), each labeled with Intercept + regressor names.
pub(super) fn print_estimate_matrices(
    model: &RegModel,
    xtx_inv: &[Vec<f64>],
    mse: f64,
    reg_names: &[String],
    intercept: bool,
    session: &mut Session,
) {
    let p = xtx_inv.len();
    let labels: Vec<String> = (0..p)
        .map(|j| design_label(j, reg_names, intercept))
        .collect();
    let covb = covb_matrix(xtx_inv, mse);
    if model.covb {
        print_labeled_matrix("Covariance of Estimates", &labels, &labels, &covb, session);
    }
    if model.corrb {
        let corrb = corrb_matrix(&covb);
        print_labeled_matrix(
            "Correlation of Estimates",
            &labels,
            &labels,
            &corrb,
            session,
        );
    }
}

// ───────────────────────── SIMPLE / CORR (M36.8) ─────────────────────────

/// Print the PROC-level "Descriptive Statistics" table (SIMPLE option, M36.8).
/// `var_names` are the analysis variables in SAS order (regressors then the
/// dependent); `cols[v]` holds the complete-case values for variable v. Columns
/// printed: Variable, Sum, Mean, Uncorrected SS, Variance, Std Deviation.
pub(super) fn print_simple_stats(var_names: &[String], cols: &[Vec<f64>], session: &mut Session) {
    session.listing.blank();
    session.listing.blank();
    centered(session, "The REG Procedure");
    centered(session, "Descriptive Statistics");
    session.listing.blank();
    let headers: Vec<String> = vec![
        "Variable".into(),
        "Sum".into(),
        "Mean".into(),
        "Uncorrected SS".into(),
        "Variance".into(),
        "Std Deviation".into(),
    ];
    let aligns = vec![
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    let rows: Vec<Vec<String>> = var_names
        .iter()
        .zip(cols.iter())
        .map(|(name, col)| {
            let n = col.len();
            let sum: f64 = col.iter().sum();
            let mean = if n > 0 { sum / n as f64 } else { f64::NAN };
            let uss: f64 = col.iter().map(|v| v * v).sum();
            let var = sample_variance(col);
            let sd = var.sqrt();
            vec![
                name.clone(),
                fmt5(sum),
                fmt5(mean),
                fmt5(uss),
                fmt5(var),
                fmt5(sd),
            ]
        })
        .collect();
    session.listing.write_table(&headers, &aligns, &rows);
}

/// Sample variance (divisor n−1). Returns NaN for fewer than 2 points.
pub(super) fn sample_variance(v: &[f64]) -> f64 {
    let n = v.len();
    if n < 2 {
        return f64::NAN;
    }
    let mean = v.iter().sum::<f64>() / n as f64;
    let ss: f64 = v.iter().map(|x| (x - mean) * (x - mean)).sum();
    ss / (n as f64 - 1.0)
}

/// Pearson correlation between two equal-length vectors. Returns NaN if either
/// has zero variance.
fn pearson_r(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len();
    if n < 2 {
        return f64::NAN;
    }
    let ma = a.iter().sum::<f64>() / n as f64;
    let mb = b.iter().sum::<f64>() / n as f64;
    let mut sab = 0.0;
    let mut saa = 0.0;
    let mut sbb = 0.0;
    for i in 0..n {
        let da = a[i] - ma;
        let db = b[i] - mb;
        sab += da * db;
        saa += da * da;
        sbb += db * db;
    }
    let d = (saa * sbb).sqrt();
    if d > 0.0 { sab / d } else { f64::NAN }
}

/// Print the PROC-level "Correlation" matrix (CORR option, M36.8) among all
/// analysis variables. Symmetric, diagonal 1; uses Pearson r over the
/// complete-case rows.
pub(super) fn print_corr_matrix(var_names: &[String], cols: &[Vec<f64>], session: &mut Session) {
    let k = var_names.len();
    session.listing.blank();
    session.listing.blank();
    centered(session, "The REG Procedure");
    centered(session, "Correlation");
    session.listing.blank();
    let mut headers: Vec<String> = vec!["Variable".into()];
    let mut aligns = vec![Align::Left];
    for name in var_names {
        headers.push(name.clone());
        aligns.push(Align::Right);
    }
    let rows: Vec<Vec<String>> = (0..k)
        .map(|i| {
            let mut row = vec![var_names[i].clone()];
            for j in 0..k {
                let r = if i == j {
                    1.0
                } else {
                    pearson_r(&cols[i], &cols[j])
                };
                row.push(fmt_fit4(r));
            }
            row
        })
        .collect();
    session.listing.write_table(&headers, &aligns, &rows);
}

/// Gather the complete-case numeric columns for the SIMPLE / CORR PROC-level
/// displays (M36.8). Uses the FIRST model's regressors + dependent, applying the
/// same listwise deletion (and WEIGHT/FREQ exclusion of non-positive weights /
/// missing freqs) as the model fit, over the given group rows. Returns the
/// analysis variable names in SAS order (regressors first, dependent last) and
/// the parallel complete-case value columns, or `None` if a variable is missing
/// / non-numeric (mirroring the error the model path would raise).
pub(super) fn gather_simple_corr(
    ds: &SasDataset,
    model: &RegModel,
    rows: &[usize],
    weight_col: Option<&[crate::value::Value]>,
    freq_col: Option<&[crate::value::Value]>,
) -> Option<(Vec<String>, Vec<Vec<f64>>)> {
    let find_col = |nm: &str| -> Option<usize> {
        ds.vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(nm))
            .filter(|&i| ds.vars[i].ty == VarType::Num)
    };
    // Analysis variables: regressors (MODEL order) then the dependent. SAS lists
    // the regressors first and the dependent last in the descriptive table.
    let mut names: Vec<String> = model.regressors.clone();
    names.push(model.dependent().to_string());
    let idxs: Vec<usize> = names.iter().map(|nm| find_col(nm)).collect::<Option<_>>()?;
    let decoded: Vec<Vec<crate::value::Value>> = idxs
        .iter()
        .map(|&i| decode_column(ds, i).ok())
        .collect::<Option<_>>()?;
    let k = names.len();
    let mut cols: Vec<Vec<f64>> = vec![Vec::new(); k];
    for &i in rows {
        // FREQ / WEIGHT exclusion identical to the model path.
        if let Some(col) = freq_col {
            match value_to_num(&col[i]) {
                Some(v) if !v.is_nan() && v.trunc() >= 1.0 => {}
                _ => continue,
            }
        }
        if let Some(col) = weight_col {
            match value_to_num(&col[i]) {
                Some(v) if !v.is_nan() && v > 0.0 => {}
                _ => continue,
            }
        }
        let mut vals = Vec::with_capacity(k);
        let mut ok = true;
        for c in &decoded {
            match value_to_num(&c[i]) {
                Some(v) if !v.is_nan() => vals.push(v),
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            for (c, v) in vals.into_iter().enumerate() {
                cols[c].push(v);
            }
        }
    }
    Some((names, cols))
}
