//! Diagnostics : VIF/TOL, collinéarité, test de White, Durbin-Watson, ACOV.

use super::*;

// ───────────────────────── Collinearity / spec diagnostics (M36.4) ─────────────────────────

/// Per-regressor VIF and tolerance (M36.4). `reg_cols[j]` is the j-th regressor
/// over the complete-case rows (length n); these are the regressors actually in
/// the fitted model (NOT the intercept). For each j we regress x_j on all the
/// OTHER regressors WITH an intercept; `R²_j` is that fit's R², from which
/// `TOL_j = 1 − R²_j` and `VIF_j = 1/TOL_j`. Returns `(tol, vif)` vectors,
/// length = `reg_cols.len()`. A regressor that is perfectly collinear with the
/// others (TOL ≈ 0) reports VIF = +inf; a single regressor reports TOL=1, VIF=1.
pub(super) fn vif_tol(reg_cols: &[Vec<f64>]) -> (Vec<f64>, Vec<f64>) {
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
pub(super) struct Collin {
    /// Eigenvalues, sorted descending.
    pub(super) eigenvalues: Vec<f64>,
    /// Condition index_k = √(λ_max / λ_k), same order as `eigenvalues`.
    pub(super) condition_index: Vec<f64>,
    /// `proportions[k][j]` = variance proportion of regressor column j on the
    /// k-th eigenvalue row. Each column j sums to 1 across k (±1e-9).
    pub(super) proportions: Vec<Vec<f64>>,
    /// Column labels (in analysis order): "Intercept" first when included.
    pub(super) col_labels: Vec<String>,
}

/// Compute the collinearity diagnostics from the design matrix. `x_mat` columns
/// are ordered [intercept?] then the regressors. When `oint` (COLLINOINT) and an
/// intercept column is present, the intercept column is dropped from the
/// analysis (no centering — SAS's COLLINOINT simply excludes the intercept).
/// `reg_names` are the regressor names (no intercept); `intercept` indicates
/// whether column 0 of `x_mat` is the intercept.
pub(super) fn compute_collin(
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
pub(super) fn print_collin(c: &Collin, oint: bool, session: &mut Session) {
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
fn fmt_collin(v: f64) -> String {
    format!("{v:.5}")
}

/// White's specification test (M36.4). Regress e² on the original regressors,
/// their squares, and pairwise cross-products (with intercept). The statistic is
/// `W = n·R²_aux`, χ² with df = number of auxiliary regressors (excluding the
/// intercept). `reg_cols[j]` are the model regressors over complete-case rows
/// (no intercept). Returns `(W, df, p_value)` or `None` if the auxiliary
/// regression is degenerate / has no usable columns.
pub(super) fn white_spec_test(reg_cols: &[Vec<f64>], resid: &[f64]) -> Option<(f64, usize, f64)> {
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
pub(super) fn print_spec_test(reg_cols: &[Vec<f64>], resid: &[f64], session: &mut Session) {
    session.listing.blank();
    session.listing.blank();
    centered(session, "Test of First and Second Moment Specification");
    session.listing.blank();
    match white_spec_test(reg_cols, resid) {
        Some((w, df, pv)) => {
            let headers: Vec<String> = vec![
                "DF".into(),
                "Chi-Square".into(),
                "Pr > ChiSq".into(),
            ];
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

/// Durbin-Watson statistic and related quantities (M36.4).
pub(super) struct DwResult {
    pub(super) d: f64,
    pub(super) rho: f64,
    n: usize,
    /// Pr < DW (positive autocorrelation) / Pr > DW (negative) — normal
    /// approximation; `None` when not requested.
    pub(super) pr_pos: Option<f64>,
    pub(super) pr_neg: Option<f64>,
}

/// Compute the Durbin-Watson statistic in dataset order. `x_mat` and
/// `xtx_inv` are used only for the (optional) normal-approximation p-values via
/// the trace formulas. `want_prob` controls whether p-values are produced.
pub(super) fn durbin_watson(
    resid: &[f64],
    x_mat: &[Vec<f64>],
    xtx_inv: &[Vec<f64>],
    want_prob: bool,
) -> DwResult {
    let n = resid.len();
    let denom: f64 = resid.iter().map(|e| e * e).sum();
    let mut num = 0.0;
    let mut lag = 0.0;
    for t in 1..n {
        let de = resid[t] - resid[t - 1];
        num += de * de;
        lag += resid[t] * resid[t - 1];
    }
    let d = if denom > 0.0 { num / denom } else { f64::NAN };
    let rho = if denom > 0.0 { lag / denom } else { f64::NAN };

    let (pr_pos, pr_neg) = if want_prob && denom > 0.0 && n > 2 {
        // Normal approximation to the null distribution of d. Under H0 the DW
        // statistic d = e'A e / e'e with A the second-difference operator. Its
        // mean and variance (residual-maker corrected) are
        //   E[d] = (P − trace(A·M·... )) — exactly E[d] = tr(MA)/(n−p),
        //   Var[d] = 2·(tr((MA)²) − (n−p)·E[d]²) / ((n−p)(n−p+2)),
        // with M = I − X(X'X)⁻¹X'. We form MA implicitly column by column.
        // NOTE: this is the standard NORMAL APPROXIMATION (Durbin & Watson
        // 1971 give the exact Imhof/Pan procedure; we deliberately use the
        // moment-matched normal tail for tractability — documented as approx).
        match dw_normal_prob(d, x_mat, xtx_inv) {
            Some((pp, pn)) => (Some(pp), Some(pn)),
            None => (None, None),
        }
    } else {
        (None, None)
    };

    DwResult {
        d,
        rho,
        n,
        pr_pos,
        pr_neg,
    }
}

/// Normal-approximation p-values for the Durbin-Watson statistic.
///
/// Builds A (the tridiagonal second-difference quadratic-form matrix so that
/// e'A e = Σ_{t≥2}(e_t−e_{t-1})²) and M = I − X(X'X)⁻¹X', then matches the first
/// two moments of d = e'A e / e'e under H0 (Gaussian errors) to a normal:
///   E[d] = tr(MA)/(n−p),  Var[d] = 2[tr((MA)²) − (n−p)E[d]²]/[(n−p)(n−p+2)].
/// `Pr < DW` = Φ((d − E)/√Var) (probability of a SMALLER d ⇒ positive
/// autocorrelation evidence), `Pr > DW` = 1 − that. Returns `None` if the
/// variance is non-positive.
fn dw_normal_prob(d: f64, x_mat: &[Vec<f64>], xtx_inv: &[Vec<f64>]) -> Option<(f64, f64)> {
    let n = x_mat.len();
    let p = x_mat[0].len();
    if n <= p {
        return None;
    }
    // Hat matrix H = X (X'X)⁻¹ X'  (n×n). M = I − H.
    // We need tr(MA) and tr((MA)²). Build MA = (I−H)A as an n×n matrix.
    // A is the symmetric tridiagonal second-difference operator:
    //   A[0][0]=1, A[n-1][n-1]=1, A[t][t]=2 (1<t<n-1 interior), off-diagonals −1.
    let mut a = vec![vec![0.0; n]; n];
    for t in 0..n {
        a[t][t] = if t == 0 || t == n - 1 { 1.0 } else { 2.0 };
    }
    for t in 1..n {
        a[t][t - 1] = -1.0;
        a[t - 1][t] = -1.0;
    }
    // H = X·(X'X)⁻¹·X'. Compute B = X·(X'X)⁻¹ (n×p), then H = B·Xᵀ.
    let b = linalg::matrix_mult(x_mat, xtx_inv); // n×p
    let xt = linalg::transpose(x_mat); // p×n
    let h = linalg::matrix_mult(&b, &xt); // n×n
    // MA = A − H·A.
    let ha = linalg::matrix_mult(&h, &a); // n×n
    let mut ma = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..n {
            ma[i][j] = a[i][j] - ha[i][j];
        }
    }
    // tr(MA).
    let tr_ma: f64 = (0..n).map(|i| ma[i][i]).sum();
    // tr((MA)²) = Σ_{i,j} ma[i][j]·ma[j][i].
    let mut tr_ma2 = 0.0;
    for i in 0..n {
        for j in 0..n {
            tr_ma2 += ma[i][j] * ma[j][i];
        }
    }
    let dfree = (n - p) as f64;
    let mean = tr_ma / dfree;
    let var = 2.0 * (tr_ma2 - dfree * mean * mean) / (dfree * (dfree + 2.0));
    if !(var > 0.0) {
        return None;
    }
    let z = (d - mean) / var.sqrt();
    let pr_less = crate::stat::probnorm(z).clamp(0.0, 1.0);
    Some((pr_less, (1.0 - pr_less).clamp(0.0, 1.0)))
}

/// Print the Durbin-Watson block (M36.4).
pub(super) fn print_durbin_watson(dwr: &DwResult, session: &mut Session) {
    session.listing.blank();
    session.listing.blank();
    centered(session, "Durbin-Watson Statistics");
    session.listing.blank();
    session
        .listing
        .write_line(&format!("Durbin-Watson D                {}", fmt5(dwr.d)));
    if let (Some(pp), Some(pn)) = (dwr.pr_pos, dwr.pr_neg) {
        session
            .listing
            .write_line(&format!("Pr < DW                        {}", fmt_p(Some(pp))));
        session
            .listing
            .write_line(&format!("Pr > DW                        {}", fmt_p(Some(pn))));
    }
    session.listing.write_line(&format!(
        "Number of Observations         {}",
        dwr.n
    ));
    session.listing.write_line(&format!(
        "1st Order Autocorrelation      {}",
        fmt5(dwr.rho)
    ));
}

/// White HC0 heteroscedasticity-consistent covariance of the estimates (M36.4):
/// `(X'X)⁻¹ (Σ_i e_i² x_i x_iᵀ) (X'X)⁻¹` (p_eff×p_eff, symmetric).
pub(super) fn acov_hc0(x_mat: &[Vec<f64>], resid: &[f64], xtx_inv: &[Vec<f64>]) -> Vec<Vec<f64>> {
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
pub(super) fn print_acov(
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
            vec![
                label(j),
                fmt5(beta[j]),
                fmt5(se),
                fmt2(t),
                fmt_p(pv),
            ]
        })
        .collect();
    session.listing.write_table(&hh, &ha, &rows2);
}
