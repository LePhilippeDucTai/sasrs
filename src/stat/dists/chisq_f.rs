use super::*;

// ─────────────────────────── M24.1 additions ───────────────────────────

/// Chi-squared cumulative distribution function.
/// CDF of χ²(df) for x ≥ 0. Implemented as 1 - gammq(df/2, x/2).
pub fn chisq_cdf(x: f64, df: f64) -> f64 {
    if df <= 0.0 {
        return f64::NAN;
    }
    if x <= 0.0 {
        return 0.0;
    }
    1.0 - gammq(df / 2.0, x / 2.0)
}

/// Chi-squared probability density function (internal, for Newton-Raphson).
pub(super) fn chisq_pdf(x: f64, df: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let k = df / 2.0;
    // ln pdf = (k-1)*ln x - x/2 - k*ln2 - lnΓ(k)
    let ln_pdf = (k - 1.0) * x.ln() - x / 2.0 - k * std::f64::consts::LN_2 - ln_gamma(k);
    ln_pdf.exp()
}

/// Chi-squared quantile (inverse CDF).
/// Quantile of χ²(df) for p ∈ (0, 1) via Newton-Raphson with bisection
/// fallback. Initial guess: Wilson-Hilferty approximation.
pub fn chisq_quantile(p: f64, df: f64) -> f64 {
    if df <= 0.0 || !(0.0..=1.0).contains(&p) {
        return f64::NAN;
    }
    if p <= 0.0 {
        return 0.0;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    // Wilson-Hilferty: χ² ≈ df * (1 - 2/(9df) + z*sqrt(2/(9df)))³.
    let z = phi_inv(p);
    let a = 2.0 / (9.0 * df);
    let mut x = df * (1.0 - a + z * a.sqrt()).powi(3);
    if !x.is_finite() || x <= 0.0 {
        x = df.max(1e-3);
    }
    newton_with_bisection(
        p,
        x,
        0.0,
        f64::INFINITY,
        |v| chisq_cdf(v, df),
        |v| chisq_pdf(v, df),
    )
}

/// F distribution cumulative distribution function.
/// CDF of F(df1, df2) for x ≥ 0. Implemented via betai.
pub fn f_cdf(x: f64, df1: f64, df2: f64) -> f64 {
    if df1 <= 0.0 || df2 <= 0.0 {
        return f64::NAN;
    }
    if x <= 0.0 {
        return 0.0;
    }
    betai(df1 / 2.0, df2 / 2.0, df1 * x / (df1 * x + df2))
}

/// F distribution probability density function (internal).
pub(super) fn f_pdf(x: f64, df1: f64, df2: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let d1 = df1 / 2.0;
    let d2 = df2 / 2.0;
    // ln pdf = d1*ln(df1) + d2*ln(df2) + (d1-1)*ln x
    //          - (d1+d2)*ln(df1*x+df2) - lnB(d1,d2)
    let ln_b = ln_gamma(d1) + ln_gamma(d2) - ln_gamma(d1 + d2);
    let ln_pdf = d1 * df1.ln() + d2 * df2.ln() + (d1 - 1.0) * x.ln()
        - (d1 + d2) * (df1 * x + df2).ln()
        - ln_b;
    ln_pdf.exp()
}

/// F distribution quantile (inverse CDF).
/// Quantile of F(df1, df2) for p ∈ (0, 1) via Newton-Raphson with bisection
/// fallback.
pub fn f_quantile(p: f64, df1: f64, df2: f64) -> f64 {
    if df1 <= 0.0 || df2 <= 0.0 || !(0.0..=1.0).contains(&p) {
        return f64::NAN;
    }
    if p <= 0.0 {
        return 0.0;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    // Initial guess: use the chi-square ratio approximation x ≈ χ²_{p}(df1)/df1.
    let mut x = chisq_quantile(p, df1) / df1;
    if !x.is_finite() || x <= 0.0 {
        x = 1.0;
    }
    newton_with_bisection(
        p,
        x,
        0.0,
        f64::INFINITY,
        |v| f_cdf(v, df1, df2),
        |v| f_pdf(v, df1, df2),
    )
}
