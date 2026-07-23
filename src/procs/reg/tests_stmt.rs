//! Statements TEST / RESTRICT (M36.1) et MTEST (M36.10).

use super::*;

// ───────────────────────── TEST / RESTRICT (M36.1) ─────────────────────────

/// Build the L (q×p_eff) matrix and c (q-vector) for a set of linear equations,
/// with columns ordered exactly like `fit.beta`: intercept first (if present),
/// then `reg_names` in order. Returns an error naming the first unknown
/// variable. The intercept keyword `INTERCEPT` maps to column 0 (only valid
/// when an intercept is in the model).
pub(super) fn build_lc(
    equations: &[LinEq],
    reg_names: &[String],
    intercept: bool,
) -> Result<(Vec<Vec<f64>>, Vec<f64>)> {
    let p_eff = reg_names.len() + intercept as usize;
    // Column index for a (already uppercased) variable name.
    let col_of = |name: &str| -> Option<usize> {
        if name == "INTERCEPT" {
            return if intercept { Some(0) } else { None };
        }
        let base = intercept as usize;
        reg_names
            .iter()
            .position(|r| r.eq_ignore_ascii_case(name))
            .map(|k| base + k)
    };
    let mut l = Vec::with_capacity(equations.len());
    let mut c = Vec::with_capacity(equations.len());
    for eq in equations {
        let mut row = vec![0.0; p_eff];
        for (coef, name) in &eq.terms {
            match col_of(name) {
                Some(j) => row[j] += *coef,
                None => {
                    return Err(SasError::runtime(format!(
                        "Variable {} in TEST/RESTRICT not in the model.",
                        name
                    )))
                }
            }
        }
        l.push(row);
        c.push(eq.rhs);
    }
    Ok((l, c))
}

/// Restricted-fit results threaded into `fit_and_print`.
pub(super) struct Restricted {
    /// Restricted coefficient estimates β_r (same column order as `fit.beta`).
    pub(super) beta_r: Vec<f64>,
    /// Restricted error/residual sum of squares.
    pub(super) sse_r: f64,
    /// Restricted error degrees of freedom = (n − p_eff) + qr.
    pub(super) df_r: f64,
    /// Predicted values from β_r.
    pub(super) y_hat_r: Vec<f64>,
    /// Residuals from β_r.
    pub(super) resid_r: Vec<f64>,
    /// SE / t / p for each β_r (column order matches `beta_r`).
    pub(super) se_r: Vec<f64>,
    pub(super) t_r: Vec<f64>,
    pub(super) p_r: Vec<f64>,
    /// One appended RESTRICT row per restriction: (label, λ, SE, t, p).
    pub(super) lambda_rows: Vec<(String, f64, f64, f64, f64)>,
}

/// Compute the constrained least-squares fit under all RESTRICT equations of
/// the model. `x_mat` is the design matrix (column order == `fit.beta`), `y`
/// the response. Returns `None` if there are no restrictions.
pub(super) fn compute_restricted(
    restricts: &[RegRestrict],
    reg_names: &[String],
    intercept: bool,
    x_mat: &[Vec<f64>],
    y: &[f64],
    fit: &OlsFit,
    n: usize,
) -> Result<Option<Restricted>> {
    // Gather every restriction equation (with a label for the table).
    let mut eqs: Vec<LinEq> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    for r in restricts {
        for eq in &r.equations {
            labels.push(restrict_label(eq, reg_names, intercept));
            eqs.push(eq.clone());
        }
    }
    if eqs.is_empty() {
        return Ok(None);
    }
    let (l, c) = build_lc(&eqs, reg_names, intercept)?;
    let qr = l.len();
    let p_eff = x_mat[0].len();
    let h = &fit.xtx_inv; // (X'X)⁻¹
    let beta = &fit.beta;

    // Lβ − c.
    let lb = linalg::matrix_vec_mult(&l, beta);
    let diff: Vec<f64> = lb.iter().zip(c.iter()).map(|(a, b)| a - b).collect();

    // M = L H Lᵀ  (qr×qr); Minv.
    let lt = linalg::transpose(&l);
    let lh = linalg::matrix_mult(&l, h); // qr×p_eff
    let m = linalg::matrix_mult(&lh, &lt); // qr×qr
    let minv = linalg::invert_matrix(&m)?;

    // λ = Minv (Lβ − c).
    let lambda = linalg::matrix_vec_mult(&minv, &diff);
    // β_r = β − H Lᵀ λ.
    let hlt = linalg::matrix_mult(h, &lt); // p_eff×qr
    let correction = linalg::matrix_vec_mult(&hlt, &lambda);
    let beta_r: Vec<f64> = beta
        .iter()
        .zip(correction.iter())
        .map(|(b, d)| b - d)
        .collect();

    // SSE_r = sse + (Lβ−c)ᵀ Minv (Lβ−c).
    let m_diff = linalg::matrix_vec_mult(&minv, &diff);
    let quad: f64 = diff.iter().zip(m_diff.iter()).map(|(a, b)| a * b).sum();
    let sse_r = fit.sse + quad;
    let df_r = (n - p_eff) as f64 + qr as f64;
    let mse_r = sse_r / df_r;

    // Restricted ŷ / residuals.
    let y_hat_r: Vec<f64> = x_mat
        .iter()
        .map(|row| row.iter().zip(beta_r.iter()).map(|(xi, bi)| xi * bi).sum())
        .collect();
    let resid_r: Vec<f64> = y
        .iter()
        .zip(y_hat_r.iter())
        .map(|(yi, yhi)| yi - yhi)
        .collect();

    // Var(β_r) = MSE_r (H − H Lᵀ Minv L H).
    let mlh = linalg::matrix_mult(&minv, &lh); // qr×p_eff
    let hlt_mlh = linalg::matrix_mult(&hlt, &mlh); // p_eff×p_eff
    let mut se_r = vec![0.0; p_eff];
    let mut t_r = vec![0.0; p_eff];
    let mut p_r = vec![0.0; p_eff];
    for j in 0..p_eff {
        let var = mse_r * (h[j][j] - hlt_mlh[j][j]);
        let se = if var > 0.0 { var.sqrt() } else { 0.0 };
        se_r[j] = se;
        t_r[j] = if se > 0.0 { beta_r[j] / se } else { 0.0 };
        p_r[j] = if se > 0.0 {
            two_sided_p(t_r[j], df_r)
        } else {
            f64::NAN
        };
    }

    // Var(λ) = MSE_r Minv → SE(λ_i), t_i = λ_i/SE, p via two_sided_p(·, df_r).
    let mut lambda_rows = Vec::with_capacity(qr);
    for i in 0..qr {
        let var = mse_r * minv[i][i];
        let se = if var > 0.0 { var.sqrt() } else { 0.0 };
        let t = if se > 0.0 { lambda[i] / se } else { 0.0 };
        let pv = if se > 0.0 {
            two_sided_p(t, df_r)
        } else {
            f64::NAN
        };
        lambda_rows.push((labels[i].clone(), lambda[i], se, t, pv));
    }

    Ok(Some(Restricted {
        beta_r,
        sse_r,
        df_r,
        y_hat_r,
        resid_r,
        se_r,
        t_r,
        p_r,
        lambda_rows,
    }))
}

/// Human-readable label for a restriction row, reconstructed from the equation
/// (e.g. `X1 = X2`, `X1 + X2 = 1`). Used in the parameter-estimates Label
/// column for RESTRICT rows.
fn restrict_label(eq: &LinEq, _reg_names: &[String], _intercept: bool) -> String {
    if eq.terms.is_empty() {
        return format!("{}", eq.rhs);
    }
    let mut s = String::new();
    for (i, (coef, name)) in eq.terms.iter().enumerate() {
        let c = *coef;
        if i == 0 {
            if c == 1.0 {
                s.push_str(name);
            } else if c == -1.0 {
                s.push('-');
                s.push_str(name);
            } else {
                s.push_str(&format!("{}*{}", trim_num(c), name));
            }
        } else {
            let mag = c.abs();
            s.push_str(if c < 0.0 { " - " } else { " + " });
            if mag == 1.0 {
                s.push_str(name);
            } else {
                s.push_str(&format!("{}*{}", trim_num(mag), name));
            }
        }
    }
    s.push_str(&format!(" = {}", trim_num(eq.rhs)));
    s
}

/// Format a coefficient/constant without trailing `.0` for integral values.
fn trim_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{}", v)
    }
}

/// Run and print every TEST statement of a model, after the parameter table.
/// `beta`, `xtx_inv`, `sse`, `df_e`, `p_eff` come from the model **as fitted**
/// (restricted if RESTRICT statements are present, else the OLS fit).
#[allow(clippy::too_many_arguments)]
pub(super) fn run_tests(
    tests: &[RegTest],
    reg_names: &[String],
    intercept: bool,
    dep_name: &str,
    beta: &[f64],
    xtx_inv: &[Vec<f64>],
    sse: f64,
    df_e: f64,
    p_eff: usize,
    session: &mut Session,
) -> Result<()> {
    if tests.is_empty() {
        return Ok(());
    }
    let mse = sse / df_e;
    for (ti, test) in tests.iter().enumerate() {
        let (l, c) = build_lc(&test.equations, reg_names, intercept)?;
        let q = l.len();
        // Lβ − c.
        let lb = linalg::matrix_vec_mult(&l, beta);
        let diff: Vec<f64> = lb.iter().zip(c.iter()).map(|(a, b)| a - b).collect();
        // M = L H Lᵀ.
        let lt = linalg::transpose(&l);
        let lh = linalg::matrix_mult(&l, xtx_inv);
        let m = linalg::matrix_mult(&lh, &lt);
        let minv = linalg::invert_matrix(&m)?;
        // SS = diffᵀ Minv diff.
        let md = linalg::matrix_vec_mult(&minv, &diff);
        let ss: f64 = diff.iter().zip(md.iter()).map(|(a, b)| a * b).sum();
        let ms_num = ss / q as f64;
        let f = if mse > 0.0 { ms_num / mse } else { f64::NAN };
        let p_f = (1.0 - f_cdf(f, q as f64, df_e)).clamp(0.0, 1.0);

        let _ = p_eff;
        // SAS heading is "Test <name> Results …"; an unlabeled TEST uses the
        // bare ordinal (→ "Test 1 …"), a labeled one its name (→ "Test peak …").
        let label = test
            .label
            .clone()
            .unwrap_or_else(|| format!("{}", ti + 1));

        session.listing.blank();
        session.listing.blank();
        centered(
            session,
            &format!(
                "Test {} Results for Dependent Variable {}",
                label, dep_name
            ),
        );
        session.listing.blank();
        let headers: Vec<String> = vec![
            "Source".into(),
            "DF".into(),
            "Mean Square".into(),
            "F Value".into(),
            "Pr > F".into(),
        ];
        let aligns = vec![
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        let rows: Vec<Vec<String>> = vec![
            vec![
                "Numerator".into(),
                format!("{}", q),
                fmt5(ms_num),
                fmt2(f),
                fmt_p(Some(p_f)),
            ],
            vec![
                "Denominator".into(),
                format!("{}", df_e as usize),
                fmt5(mse),
                "".into(),
                "".into(),
            ],
        ];
        session.listing.write_table(&headers, &aligns, &rows);
    }
    Ok(())
}

// ───────────────────────── MTEST (M36.10) ─────────────────────────

/// The four multivariate test statistics with their F approximations.
pub(super) struct MtestStat {
    pub(super) name: &'static str,
    pub(super) value: f64,
    pub(super) f: f64,
    df1: f64,
    df2: f64,
}

/// Compute Wilks / Pillai / Hotelling-Lawley / Roy statistics with the standard
/// SAS MANOVA F approximations from the generalized eigenvalues `λ_i` of E⁻¹H.
///
/// Parameters follow the SAS convention: `p` = number of responses, `q` =
/// hypothesis degrees of freedom (rank of L), `v` = error degrees of freedom
/// (N − rank(X)). `s = min(p, q)` is the number of non-zero eigenvalues.
pub(super) fn mtest_statistics(lambda: &[f64], p: f64, q: f64, v: f64) -> Vec<MtestStat> {
    let s = p.min(q);
    // Σ λ/(1+λ), Σ λ, Π 1/(1+λ), max λ.
    let pillai: f64 = lambda.iter().map(|&l| l / (1.0 + l)).sum();
    let hlt: f64 = lambda.iter().sum();
    let wilks: f64 = lambda.iter().map(|&l| 1.0 / (1.0 + l)).product();
    let roy: f64 = lambda.iter().cloned().fold(0.0_f64, f64::max);

    let m_ = ((p - q).abs() - 1.0) / 2.0;
    let n_ = (v - p - 1.0) / 2.0;

    // ── Wilks' Lambda (Rao's F).
    let (w_f, w_df1, w_df2) = {
        let df1 = p * q;
        let denom = p * p + q * q - 5.0;
        let t = if denom > 0.0 {
            ((p * p * q * q - 4.0) / denom).max(0.0).sqrt()
        } else {
            1.0
        };
        let t = if t == 0.0 { 1.0 } else { t };
        let r = v - (p - q + 1.0) / 2.0;
        let u = (p * q - 2.0) / 4.0;
        let df2 = r * t - 2.0 * u;
        let lam_root = wilks.powf(1.0 / t);
        let f = if lam_root > 0.0 && df1 > 0.0 {
            (1.0 - lam_root) / lam_root * (df2 / df1)
        } else {
            f64::NAN
        };
        (f, df1, df2)
    };

    // ── Pillai's Trace.
    let (pi_f, pi_df1, pi_df2) = {
        let df1 = s * (2.0 * m_ + s + 1.0);
        let df2 = s * (2.0 * n_ + s + 1.0);
        let f = if (s - pillai).abs() > 0.0 && df1 > 0.0 {
            (df2 / df1) * (pillai / (s - pillai))
        } else {
            f64::NAN
        };
        (f, df1, df2)
    };

    // ── Hotelling-Lawley Trace.
    let (hl_f, hl_df1, hl_df2) = {
        let df1 = s * (2.0 * m_ + s + 1.0);
        let df2 = 2.0 * (s * n_ + 1.0);
        let f = if df1 > 0.0 {
            df2 * hlt / (s * df1)
        } else {
            f64::NAN
        };
        (f, df1, df2)
    };

    // ── Roy's Greatest Root (upper bound F).
    let (roy_f, roy_df1, roy_df2) = {
        let r2 = p.max(q);
        let df1 = r2;
        let df2 = v - r2 + q;
        let f = if df1 > 0.0 {
            roy * df2 / df1
        } else {
            f64::NAN
        };
        (f, df1, df2)
    };

    vec![
        MtestStat { name: "Wilks' Lambda", value: wilks, f: w_f, df1: w_df1, df2: w_df2 },
        MtestStat { name: "Pillai's Trace", value: pillai, f: pi_f, df1: pi_df1, df2: pi_df2 },
        MtestStat {
            name: "Hotelling-Lawley Trace",
            value: hlt,
            f: hl_f,
            df1: hl_df1,
            df2: hl_df2,
        },
        MtestStat {
            name: "Roy's Greatest Root",
            value: roy,
            f: roy_f,
            df1: roy_df1,
            df2: roy_df2,
        },
    ]
}

/// Generalized eigenvalues λ_i of E⁻¹H for SPD `E` (q×q) and symmetric `H`
/// (q×q), via the symmetric reduction M = L⁻¹ H L⁻ᵀ (E = L Lᵀ, Cholesky), whose
/// ordinary symmetric eigenvalues are exactly the generalized ones. Returns them
/// in descending order. Errors if E is not SPD.
pub(super) fn generalized_eigenvalues(e: &[Vec<f64>], h: &[Vec<f64>]) -> Result<Vec<f64>> {
    let l = linalg::cholesky(e)?; // E = L Lᵀ, L lower-triangular.
    let n = l.len();
    // Solve L W = H for W (forward substitution, column by column): W = L⁻¹ H.
    let mut w = vec![vec![0.0; n]; n];
    for col in 0..n {
        for i in 0..n {
            let mut sum = h[i][col];
            for k in 0..i {
                sum -= l[i][k] * w[k][col];
            }
            w[i][col] = sum / l[i][i];
        }
    }
    // Solve L M = Wᵀ for M (so M = L⁻¹ Wᵀ = L⁻¹ H L⁻ᵀ, symmetric).
    let wt = linalg::transpose(&w);
    let mut m = vec![vec![0.0; n]; n];
    for col in 0..n {
        for i in 0..n {
            let mut sum = wt[i][col];
            for k in 0..i {
                sum -= l[i][k] * m[k][col];
            }
            m[i][col] = sum / l[i][i];
        }
    }
    // Symmetrize against round-off, then symmetric eigensolve.
    for i in 0..n {
        for j in (i + 1)..n {
            let avg = 0.5 * (m[i][j] + m[j][i]);
            m[i][j] = avg;
            m[j][i] = avg;
        }
    }
    let mut eig = linalg::eigenvalues_jacobi(&m)?;
    // Generalized eigenvalues of E⁻¹H are non-negative; clamp tiny negatives.
    for e in &mut eig {
        if *e < 0.0 && *e > -1e-9 {
            *e = 0.0;
        }
    }
    Ok(eig)
}

/// Run and print every MTEST statement of a model. Self-contained: gathers a
/// multivariate response/design matrix with listwise deletion across all
/// responses + the selected regressors, builds E and H, solves the generalized
/// eigenproblem, and prints the four MANOVA statistics. (M36.10)
#[allow(clippy::too_many_arguments)]
pub(super) fn run_mtests(
    mtests: &[RegMtest],
    model: &RegModel,
    ds: &SasDataset,
    rows: &[usize],
    selected: &[usize],
    sel_reg_names: &[String],
    intercept: bool,
    session: &mut Session,
) -> Result<()> {
    let find_col = |nm: &str| -> Result<usize> {
        ds.vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(nm))
            .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", nm.to_uppercase())))
    };

    // Resolve and decode every response column.
    let q_resp = model.dependents.len();
    let mut resp_cols: Vec<Vec<crate::value::Value>> = Vec::with_capacity(q_resp);
    for nm in &model.dependents {
        let idx = find_col(nm)?;
        if ds.vars[idx].ty != VarType::Num {
            return Err(SasError::runtime(format!(
                "Dependent variable {} must be numeric.",
                nm.to_uppercase()
            )));
        }
        resp_cols.push(decode_column(ds, idx)?);
    }
    // Resolve and decode the selected regressor columns (MTEST runs on the
    // selected design, intercept handled separately).
    let mut reg_cols: Vec<Vec<crate::value::Value>> = Vec::with_capacity(selected.len());
    for nm in sel_reg_names {
        let idx = find_col(nm)?;
        reg_cols.push(decode_column(ds, idx)?);
    }

    // Listwise deletion across all responses + selected regressors.
    let p_eff = sel_reg_names.len() + intercept as usize;
    let mut x_mat: Vec<Vec<f64>> = Vec::new();
    let mut y_mat: Vec<Vec<f64>> = Vec::new(); // rows = obs, cols = responses
    for &i in rows {
        let mut yr = Vec::with_capacity(q_resp);
        let mut ok = true;
        for rc in &resp_cols {
            match value_to_num(&rc[i]) {
                Some(v) if !v.is_nan() => yr.push(v),
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let mut xr = Vec::with_capacity(p_eff);
        if intercept {
            xr.push(1.0);
        }
        for rc in &reg_cols {
            match value_to_num(&rc[i]) {
                Some(v) if !v.is_nan() => xr.push(v),
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            x_mat.push(xr);
            y_mat.push(yr);
        }
    }

    let n = y_mat.len();
    if n <= p_eff {
        return Err(SasError::runtime(
            "MTEST: not enough complete observations for the multivariate fit",
        ));
    }

    // (X'X)⁻¹ and the coefficient matrix B = (X'X)⁻¹ X'Y (p_eff × q_resp).
    let xt = linalg::transpose(&x_mat);
    let xtx = linalg::matrix_mult(&xt, &x_mat);
    let xtx_inv = linalg::invert_matrix(&xtx)?;
    let xty = linalg::matrix_mult(&xt, &y_mat); // p_eff × q_resp
    let b = linalg::matrix_mult(&xtx_inv, &xty); // p_eff × q_resp

    // E = Y'Y − Y'X(X'X)⁻¹X'Y  (residual SSCP, q×q).
    let yt = linalg::transpose(&y_mat);
    let yty = linalg::matrix_mult(&yt, &y_mat); // q×q
    let ytx = linalg::matrix_mult(&yt, &x_mat); // q×p_eff
    let ytx_b = linalg::matrix_mult(&ytx, &b); // q×q  (= Y'X B)
    let mut e = vec![vec![0.0; q_resp]; q_resp];
    for i in 0..q_resp {
        for j in 0..q_resp {
            e[i][j] = yty[i][j] - ytx_b[i][j];
        }
    }

    let v = (n - p_eff) as f64; // error degrees of freedom = N − rank(X).

    for (ti, mt) in mtests.iter().enumerate() {
        // Build the hypothesis matrix L (m × p_eff). Default (no equations):
        // test that all NON-INTERCEPT coefficients are zero jointly.
        let l: Vec<Vec<f64>> = if mt.equations.is_empty() {
            let mut rows_l = Vec::new();
            let base = intercept as usize;
            for k in 0..sel_reg_names.len() {
                let mut r = vec![0.0; p_eff];
                r[base + k] = 1.0;
                rows_l.push(r);
            }
            rows_l
        } else {
            let (l, _c) = build_lc(&mt.equations, sel_reg_names, intercept)?;
            l
        };
        let m_rank = l.len();
        if m_rank == 0 {
            continue;
        }

        // H = (LB)' (L(X'X)⁻¹L')⁻¹ (LB)   (q×q).
        let lb = linalg::matrix_mult(&l, &b); // m × q
        let lt = linalg::transpose(&l); // p_eff × m
        let lxi = linalg::matrix_mult(&l, &xtx_inv); // m × p_eff
        let lxil = linalg::matrix_mult(&lxi, &lt); // m × m
        let lxil_inv = linalg::invert_matrix(&lxil)?;
        let mid = linalg::matrix_mult(&linalg::transpose(&lb), &lxil_inv); // q × m
        let h = linalg::matrix_mult(&mid, &lb); // q × q

        // Generalized eigenvalues of E⁻¹H (E SPD required).
        let lambda = match generalized_eigenvalues(&e, &h) {
            Ok(l) => l,
            Err(err) => {
                session.log.error(&format!(
                    "MTEST: error sums-of-squares matrix is singular or not positive definite ({err}); the multivariate test cannot be computed."
                ));
                return Ok(());
            }
        };

        let p = q_resp as f64; // number of responses
        let qdf = m_rank as f64; // hypothesis degrees of freedom (rank L)
        let stats = mtest_statistics(&lambda, p, qdf, v);

        let label = mt
            .label
            .clone()
            .unwrap_or_else(|| format!("{}", ti + 1));

        session.listing.blank();
        session.listing.blank();
        centered(session, &format!("Multivariate Test: {}", label));
        session.listing.blank();

        let headers: Vec<String> = vec![
            "Statistic".into(),
            "Value".into(),
            "F Value".into(),
            "Num DF".into(),
            "Den DF".into(),
            "Pr > F".into(),
        ];
        let aligns = vec![
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        let rows_out: Vec<Vec<String>> = stats
            .iter()
            .map(|s| {
                let pval = if s.df1 > 0.0 && s.df2 > 0.0 && s.f.is_finite() && s.f >= 0.0 {
                    Some((1.0 - f_cdf(s.f, s.df1, s.df2)).clamp(0.0, 1.0))
                } else {
                    None
                };
                vec![
                    s.name.to_string(),
                    fmt_fit4(s.value),
                    fmt2(s.f),
                    fmt_mtest_df(s.df1),
                    fmt_mtest_df(s.df2),
                    fmt_p(pval),
                ]
            })
            .collect();
        session.listing.write_table(&headers, &aligns, &rows_out);
    }
    Ok(())
}

/// Format a MANOVA degrees-of-freedom value: integral values plain, otherwise
/// to two decimals (Wilks' Rao Den DF can be fractional).
fn fmt_mtest_df(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.2}")
    }
}
