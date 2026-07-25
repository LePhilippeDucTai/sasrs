use super::*;

/// Gamma distribution cumulative distribution function.
/// CDF of Gamma(shape, scale) for x ≥ 0, pdf ∝ x^(shape-1) exp(-x/scale).
/// Implemented as 1 - gammq(shape, x/scale).
pub fn gamma_cdf(x: f64, shape: f64, scale: f64) -> f64 {
    if shape <= 0.0 || scale <= 0.0 {
        return f64::NAN;
    }
    if x <= 0.0 {
        return 0.0;
    }
    1.0 - gammq(shape, x / scale)
}

/// Gamma distribution probability density function (internal).
pub(super) fn gamma_pdf(x: f64, shape: f64, scale: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    // ln pdf = (shape-1)*ln x - x/scale - shape*ln(scale) - lnΓ(shape)
    let ln_pdf =
        (shape - 1.0) * x.ln() - x / scale - shape * scale.ln() - ln_gamma(shape);
    ln_pdf.exp()
}

/// Gamma distribution quantile (inverse CDF).
/// Quantile of Gamma(shape, scale) for p ∈ (0, 1) via Newton-Raphson with
/// bisection fallback.
pub fn gamma_quantile(p: f64, shape: f64, scale: f64) -> f64 {
    if shape <= 0.0 || scale <= 0.0 || !(0.0..=1.0).contains(&p) {
        return f64::NAN;
    }
    if p <= 0.0 {
        return 0.0;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    // Gamma(shape, scale) quantile = scale * chisq_quantile(p, 2*shape) / 2.
    let mut x = scale * chisq_quantile(p, 2.0 * shape) / 2.0;
    if !x.is_finite() || x <= 0.0 {
        x = shape * scale;
    }
    newton_with_bisection(
        p,
        x,
        0.0,
        f64::INFINITY,
        |v| gamma_cdf(v, shape, scale),
        |v| gamma_pdf(v, shape, scale),
    )
}

/// Beta distribution cumulative distribution function.
/// CDF of Beta(α, β) on [0, 1]. Directly betai(α, β, x).
pub fn beta_cdf(x: f64, alpha: f64, beta: f64) -> f64 {
    if alpha <= 0.0 || beta <= 0.0 {
        return f64::NAN;
    }
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    betai(alpha, beta, x)
}

/// Beta distribution probability density function (internal).
pub(super) fn beta_pdf(x: f64, alpha: f64, beta: f64) -> f64 {
    if x <= 0.0 || x >= 1.0 {
        return 0.0;
    }
    let ln_b = ln_gamma(alpha) + ln_gamma(beta) - ln_gamma(alpha + beta);
    let ln_pdf = (alpha - 1.0) * x.ln() + (beta - 1.0) * (1.0 - x).ln() - ln_b;
    ln_pdf.exp()
}

/// Beta distribution quantile (inverse CDF).
/// Quantile of Beta(α, β) for p ∈ (0, 1) via Newton-Raphson with bisection
/// fallback. Domain bounded to [0, 1].
pub fn beta_quantile(p: f64, alpha: f64, beta: f64) -> f64 {
    if alpha <= 0.0 || beta <= 0.0 || !(0.0..=1.0).contains(&p) {
        return f64::NAN;
    }
    if p <= 0.0 {
        return 0.0;
    }
    if p >= 1.0 {
        return 1.0;
    }
    // Initial guess: mean-based, clamped to the open interval.
    let mut x = alpha / (alpha + beta);
    if !(0.0..1.0).contains(&x) {
        x = 0.5;
    }
    newton_with_bisection(
        p,
        x,
        0.0,
        1.0,
        |v| beta_cdf(v, alpha, beta),
        |v| beta_pdf(v, alpha, beta),
    )
}
