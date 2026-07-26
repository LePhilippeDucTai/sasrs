//! Statements TEST / RESTRICT (M36.1) et MTEST (M36.10).

use super::*;

mod mtest;

pub(crate) use mtest::*;

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
                    )));
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
        let label = test.label.clone().unwrap_or_else(|| format!("{}", ti + 1));

        session.listing.blank();
        session.listing.blank();
        centered(
            session,
            &format!("Test {} Results for Dependent Variable {}", label, dep_name),
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
