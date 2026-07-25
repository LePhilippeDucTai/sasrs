use super::*;

// ───────────────────────── Collinearity / spec diagnostics (M36.4) ─────────────────────────

/// Per-regressor VIF and tolerance (M36.4). `reg_cols[j]` is the j-th regressor
/// over the complete-case rows (length n); these are the regressors actually in
/// the fitted model (NOT the intercept). For each j we regress x_j on all the
/// OTHER regressors WITH an intercept; `R²_j` is that fit's R², from which
/// `TOL_j = 1 − R²_j` and `VIF_j = 1/TOL_j`. Returns `(tol, vif)` vectors,
/// length = `reg_cols.len()`. A regressor that is perfectly collinear with the
/// others (TOL ≈ 0) reports VIF = +inf; a single regressor reports TOL=1, VIF=1.
pub(crate) fn vif_tol(reg_cols: &[Vec<f64>]) -> (Vec<f64>, Vec<f64>) {
    let p = reg_cols.len();
    let n = if p > 0 { reg_cols[0].len() } else { 0 };
    let mut tol = vec![1.0; p];
    let mut vif = vec![1.0; p];
    if p <= 1 {
        return (tol, vif);
    }
    for j in 0..p {
        // Response = x_j; predictors = all other regressors + intercept.
        let yj = &reg_cols[j];
        let mut xaux: Vec<Vec<f64>> = Vec::with_capacity(n);
        for i in 0..n {
            let mut row = Vec::with_capacity(p); // intercept + (p-1) others
            row.push(1.0);
            for (k, col) in reg_cols.iter().enumerate() {
                if k != j {
                    row.push(col[i]);
                }
            }
            xaux.push(row);
        }
        // R²_j from the auxiliary regression (corrected total, intercept present).
        let r2j = match ols_fit(&xaux, yj) {
            Ok(f) => {
                let ybar = yj.iter().sum::<f64>() / n as f64;
                let sst: f64 = yj.iter().map(|v| (v - ybar) * (v - ybar)).sum();
                if sst > 0.0 {
                    (1.0 - f.sse / sst).clamp(0.0, 1.0)
                } else {
                    0.0
                }
            }
            // Rank-deficient auxiliary fit ⇒ treat as no explanatory power.
            Err(_) => 0.0,
        };
        let t = 1.0 - r2j;
        tol[j] = t;
        vif[j] = if t > 0.0 { 1.0 / t } else { f64::INFINITY };
    }
    (tol, vif)
}

/// Collinearity-diagnostics output (M36.4): eigenvalues, condition indices and
/// variance-decomposition proportions of the scaled-X cross-product matrix.
pub(crate) struct Collin {
    /// Eigenvalues, sorted descending.
    pub(crate) eigenvalues: Vec<f64>,
    /// Condition index_k = √(λ_max / λ_k), same order as `eigenvalues`.
    pub(crate) condition_index: Vec<f64>,
    /// `proportions[k][j]` = variance proportion of regressor column j on the
    /// k-th eigenvalue row. Each column j sums to 1 across k (±1e-9).
    pub(crate) proportions: Vec<Vec<f64>>,
    /// Column labels (in analysis order): "Intercept" first when included.
    pub(crate) col_labels: Vec<String>,
}

/// Compute the collinearity diagnostics from the design matrix. `x_mat` columns
/// are ordered [intercept?] then the regressors. When `oint` (COLLINOINT) and an
/// intercept column is present, the intercept column is dropped from the
/// analysis (no centering — SAS's COLLINOINT simply excludes the intercept).
/// `reg_names` are the regressor names (no intercept); `intercept` indicates
/// whether column 0 of `x_mat` is the intercept.
pub(crate) fn compute_collin(
    x_mat: &[Vec<f64>],
    reg_names: &[String],
    intercept: bool,
    oint: bool,
) -> Result<Collin> {
    let n = x_mat.len();
    let full_p = x_mat[0].len();
    // Choose the columns to analyse.
    let drop_intercept = oint && intercept;
    let cols: Vec<usize> = if drop_intercept {
        (1..full_p).collect()
    } else {
        (0..full_p).collect()
    };
    let m = cols.len();
    let mut col_labels = Vec::with_capacity(m);
    for &c in &cols {
        let lbl = if intercept {
            if c == 0 {
                "Intercept".to_string()
            } else {
                reg_names[c - 1].clone()
            }
        } else {
            reg_names[c].clone()
        };
        col_labels.push(lbl);
    }

    // Scale each analysed column to unit (2-norm) length.
    let norms: Vec<f64> = cols
        .iter()
        .map(|&c| (0..n).map(|i| x_mat[i][c] * x_mat[i][c]).sum::<f64>().sqrt())
        .collect();
    // Scaled cross-product A = ZᵀZ (m×m) where Z column c is x[:,c]/‖x[:,c]‖.
    let mut a = vec![vec![0.0; m]; m];
    for (p, &cp) in cols.iter().enumerate() {
        for (q, &cq) in cols.iter().enumerate() {
            let mut s = 0.0;
            for i in 0..n {
                s += x_mat[i][cp] * x_mat[i][cq];
            }
            let denom = norms[p] * norms[q];
            a[p][q] = if denom > 0.0 { s / denom } else { 0.0 };
        }
    }

    // Eigen-decomposition (descending eigenvalues, eigenvector columns).
    let (vecs, eigvals) = linalg::eigenvectors_jacobi(&a)?;
    // Guard tiny negatives from round-off.
    let eigenvalues: Vec<f64> = eigvals.iter().map(|&l| l.max(0.0)).collect();
    let lmax = eigenvalues.iter().cloned().fold(0.0_f64, f64::max);
    let condition_index: Vec<f64> = eigenvalues
        .iter()
        .map(|&l| if l > 0.0 { (lmax / l).sqrt() } else { f64::INFINITY })
        .collect();

    // Variance proportions. φ_{kj} = v_{jk}² / λ_k ; π_{jk} = φ_{kj}/Σ_k φ_{kj}.
    // vecs[row][col] : column k is the k-th eigenvector, row j the j-th variable.
    let mut phi = vec![vec![0.0; m]; m]; // phi[k][j]
    for k in 0..m {
        let lk = eigenvalues[k];
        for j in 0..m {
            let vjk = vecs[j][k];
            phi[k][j] = if lk > 0.0 { vjk * vjk / lk } else { 0.0 };
        }
    }
    // Column sums Σ_k φ_{kj}.
    let mut colsum = vec![0.0; m];
    for j in 0..m {
        for k in 0..m {
            colsum[j] += phi[k][j];
        }
    }
    let mut proportions = vec![vec![0.0; m]; m];
    for k in 0..m {
        for j in 0..m {
            proportions[k][j] = if colsum[j] > 0.0 {
                phi[k][j] / colsum[j]
            } else {
                0.0
            };
        }
    }

    Ok(Collin {
        eigenvalues,
        condition_index,
        proportions,
        col_labels,
    })
}

/// Print the "Collinearity Diagnostics" table (M36.4).
pub(crate) fn print_collin(c: &Collin, oint: bool, session: &mut Session) {
    let m = c.eigenvalues.len();
    let mut headers: Vec<String> = vec![
        "Number".into(),
        "Eigenvalue".into(),
        "Condition Index".into(),
    ];
    let mut aligns = vec![Align::Right, Align::Right, Align::Right];
    for lbl in &c.col_labels {
        headers.push(format!("Proportion of Variation {}", lbl));
        aligns.push(Align::Right);
    }
    let rows: Vec<Vec<String>> = (0..m)
        .map(|k| {
            let mut row = vec![
                format!("{}", k + 1),
                fmt_collin(c.eigenvalues[k]),
                fmt5(c.condition_index[k]),
            ];
            for j in 0..m {
                row.push(fmt5(c.proportions[k][j]));
            }
            row
        })
        .collect();

    session.listing.blank();
    session.listing.blank();
    let title = if oint {
        "Collinearity Diagnostics (intercept adjusted)"
    } else {
        "Collinearity Diagnostics"
    };
    centered(session, title);
    session.listing.blank();
    session.listing.write_table(&headers, &aligns, &rows);
}

/// Eigenvalues print with more precision than fmt5 in SAS; use 8 decimals but
/// trim is not needed (insta locks bytes). SAS uses a varying g-format; we fix
/// at 5 decimals like the rest of the table for determinism.
pub(crate) fn fmt_collin(v: f64) -> String {
    format!("{v:.5}")
}
