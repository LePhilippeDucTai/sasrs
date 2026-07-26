use super::*;

// ── Core distribution CDFs (all return a probability in [0, 1]) ──────────────

/// Standard normal CDF Φ(x) via erfc for numerical stability in the tails.
pub(crate) fn normal_cdf_std(x: f64) -> f64 {
    0.5 * erfc(-x / std::f64::consts::SQRT_2)
}

/// Normal CDF with mean `mu` and standard deviation `sigma`.
pub(crate) fn normal_cdf(x: f64, mu: f64, sigma: f64) -> f64 {
    normal_cdf_std((x - mu) / sigma)
}

/// Student's t CDF with `df` degrees of freedom.
pub(crate) fn t_cdf(t: f64, df: f64) -> f64 {
    if df <= 0.0 {
        return f64::NAN;
    }
    let x = df / (df + t * t);
    let ib = 0.5 * betai(0.5 * df, 0.5, x);
    if t >= 0.0 { 1.0 - ib } else { ib }
}

/// F CDF with `ndf` numerator and `ddf` denominator degrees of freedom.
pub(crate) fn f_cdf(f: f64, ndf: f64, ddf: f64) -> f64 {
    if f <= 0.0 {
        return 0.0;
    }
    let x = ndf * f / (ndf * f + ddf);
    betai(0.5 * ndf, 0.5 * ddf, x)
}

/// Chi-square CDF with `df` degrees of freedom.
pub(crate) fn chisq_cdf(x: f64, df: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    lower_gamma_p(0.5 * df, 0.5 * x)
}

/// Beta CDF P(X <= x) for X ~ Beta(a, b).
pub(crate) fn beta_cdf(x: f64, a: f64, b: f64) -> f64 {
    betai(a, b, x)
}

/// Gamma CDF P(X <= x) for X ~ Gamma(shape = a, scale = 1).
pub(crate) fn gamma_cdf(x: f64, a: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    lower_gamma_p(a, x)
}

/// Binomial CDF P(X <= k) for X ~ Binomial(n, p) by exact summation of the PMF.
pub(crate) fn binomial_cdf(p: f64, n: f64, k: f64) -> f64 {
    let n = n.round();
    let k = k.floor();
    if k < 0.0 {
        return 0.0;
    }
    if k >= n {
        return 1.0;
    }
    // Use the incomplete-beta identity for stability with large n:
    // P(X <= k) = I_{1-p}(n-k, k+1)
    betai(n - k, k + 1.0, 1.0 - p)
}

/// Poisson CDF P(X <= k) for X ~ Poisson(lambda).
/// P(X <= k) = Q(k + 1, lambda) (regularized upper incomplete gamma).
pub(crate) fn poisson_cdf(lambda: f64, k: f64) -> f64 {
    let k = k.floor();
    if k < 0.0 {
        return 0.0;
    }
    if lambda <= 0.0 {
        return 1.0;
    }
    upper_gamma_q(k + 1.0, lambda)
}
