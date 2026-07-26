use super::*;

/// White's specification test (M36.4). Regress e² on the original regressors,
/// their squares, and pairwise cross-products (with intercept). The statistic is
/// `W = n·R²_aux`, χ² with df = number of auxiliary regressors (excluding the
/// intercept). `reg_cols[j]` are the model regressors over complete-case rows
/// (no intercept). Returns `(W, df, p_value)` or `None` if the auxiliary
/// regression is degenerate / has no usable columns.
pub(crate) fn white_spec_test(reg_cols: &[Vec<f64>], resid: &[f64]) -> Option<(f64, usize, f64)> {
    let p = reg_cols.len();
    let n = resid.len();
    if p == 0 || n == 0 {
        return None;
    }
    // Auxiliary response = squared residuals.
    let e2: Vec<f64> = resid.iter().map(|r| r * r).collect();

    // Build the auxiliary regressor set per row: each x_j, each x_j², and each
    // cross-product x_j·x_k (j<k). De-duplicate constant columns later via the
    // rank-robust ols_fit (QR). We keep an intercept column at position 0.
    let n_aux = p + p + p * (p.saturating_sub(1)) / 2; // linear + square + cross
    let mut xaux: Vec<Vec<f64>> = Vec::with_capacity(n);
    for i in 0..n {
        let mut row = Vec::with_capacity(1 + n_aux);
        row.push(1.0);
        for col in reg_cols.iter() {
            row.push(col[i]);
        }
        for col in reg_cols.iter() {
            row.push(col[i] * col[i]);
        }
        for a in 0..p {
            for b in (a + 1)..p {
                row.push(reg_cols[a][i] * reg_cols[b][i]);
            }
        }
        xaux.push(row);
    }

    let fit = ols_fit(&xaux, &e2).ok()?;
    let ybar = e2.iter().sum::<f64>() / n as f64;
    let sst: f64 = e2.iter().map(|v| (v - ybar) * (v - ybar)).sum();
    let r2 = if sst > 0.0 {
        (1.0 - fit.sse / sst).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let df = n_aux; // auxiliary regressors excluding the intercept
    if df == 0 {
        return None;
    }
    let w = n as f64 * r2;
    let p_value = (1.0 - crate::stat::chisq_cdf(w, df as f64)).clamp(0.0, 1.0);
    Some((w, df, p_value))
}

/// Print White's "Test of First and Second Moment Specification" (M36.4).
pub(crate) fn print_spec_test(reg_cols: &[Vec<f64>], resid: &[f64], session: &mut Session) {
    session.listing.blank();
    session.listing.blank();
    centered(session, "Test of First and Second Moment Specification");
    session.listing.blank();
    match white_spec_test(reg_cols, resid) {
        Some((w, df, pv)) => {
            let headers: Vec<String> = vec!["DF".into(), "Chi-Square".into(), "Pr > ChiSq".into()];
            let aligns = vec![Align::Right, Align::Right, Align::Right];
            let rows = vec![vec![format!("{}", df), fmt2(w), fmt_p(Some(pv))]];
            session.listing.write_table(&headers, &aligns, &rows);
        }
        None => {
            centered(
                session,
                "Specification test could not be computed (degenerate auxiliary regression).",
            );
        }
    }
}

/// White HC0 heteroscedasticity-consistent covariance of the estimates (M36.4):
/// `(X'X)⁻¹ (Σ_i e_i² x_i x_iᵀ) (X'X)⁻¹` (p_eff×p_eff, symmetric).
pub(crate) fn acov_hc0(x_mat: &[Vec<f64>], resid: &[f64], xtx_inv: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let n = x_mat.len();
    let p = xtx_inv.len();
    // Meat = Σ_i e_i² x_i x_iᵀ  (p×p).
    let mut meat = vec![vec![0.0; p]; p];
    for i in 0..n {
        let w = resid[i] * resid[i];
        let xi = &x_mat[i];
        for a in 0..p {
            let wa = w * xi[a];
            for b in 0..p {
                meat[a][b] += wa * xi[b];
            }
        }
    }
    // (X'X)⁻¹ · meat · (X'X)⁻¹.
    let tmp = linalg::matrix_mult(xtx_inv, &meat); // p×p
    linalg::matrix_mult(&tmp, xtx_inv)
}

/// Print the "Consistent Covariance of Estimates" matrix and a small table of
/// heteroscedasticity-consistent standard errors / t / Pr>|t| (M36.4).
///
/// Layout: a labeled p_eff×p_eff matrix (one row/column per parameter, Intercept
/// first when present), followed by a "Heteroscedasticity Consistent" parameter
/// table with HC Std Error / t Value / Pr > |t|. The OLS parameter table printed
/// earlier is left intact (SAS adds rather than replaces).
pub(crate) fn print_acov(
    cov: &[Vec<f64>],
    beta: &[f64],
    reg_names: &[String],
    intercept: bool,
    df_e: f64,
    session: &mut Session,
) {
    let p_eff = cov.len();
    let label = |j: usize| -> String {
        if intercept {
            if j == 0 {
                "Intercept".to_string()
            } else {
                reg_names[j - 1].clone()
            }
        } else {
            reg_names[j].clone()
        }
    };

    session.listing.blank();
    session.listing.blank();
    centered(session, "Consistent Covariance of Estimates");
    session.listing.blank();
    let mut headers: Vec<String> = vec!["".into()];
    let mut aligns = vec![Align::Left];
    for j in 0..p_eff {
        headers.push(label(j));
        aligns.push(Align::Right);
    }
    let rows: Vec<Vec<String>> = (0..p_eff)
        .map(|i| {
            let mut row = vec![label(i)];
            for j in 0..p_eff {
                row.push(fmt5(cov[i][j]));
            }
            row
        })
        .collect();
    session.listing.write_table(&headers, &aligns, &rows);

    // HC standard errors / t / p table.
    session.listing.blank();
    session.listing.blank();
    centered(
        session,
        "Parameter Estimates with Heteroscedasticity Consistent Standard Errors",
    );
    session.listing.blank();
    let hh: Vec<String> = vec![
        "Variable".into(),
        "Estimate".into(),
        "HC Std Error".into(),
        "t Value".into(),
        "Pr > |t|".into(),
    ];
    let ha = vec![
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    let rows2: Vec<Vec<String>> = (0..p_eff)
        .map(|j| {
            let se = cov[j][j].max(0.0).sqrt();
            let t = if se > 0.0 { beta[j] / se } else { f64::NAN };
            let pv = if se > 0.0 {
                Some(two_sided_p(t, df_e))
            } else {
                None
            };
            vec![label(j), fmt5(beta[j]), fmt5(se), fmt2(t), fmt_p(pv)]
        })
        .collect();
    session.listing.write_table(&hh, &ha, &rows2);
}
