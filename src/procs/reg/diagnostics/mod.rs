//! Diagnostics : VIF/TOL, collinéarité, test de White, Durbin-Watson, ACOV.

use super::*;


mod collin;
mod hetero;

pub(crate) use collin::*;
pub(crate) use hetero::*;

/// Durbin-Watson statistic and related quantities (M36.4).
pub(crate) struct DwResult {
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
pub(crate) fn durbin_watson(
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
pub(crate) fn print_durbin_watson(dwr: &DwResult, session: &mut Session) {
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
