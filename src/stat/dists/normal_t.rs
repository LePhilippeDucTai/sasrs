use super::*;

/// Student-t cumulative distribution function.
/// Promoted from common.rs; t ∈ ℝ, df > 0.
pub fn student_t_cdf(t: f64, df: f64) -> f64 {
    // P(T <= t) = 1 - 0.5 * I_{df/(df+t^2)}(df/2, 1/2) for t >= 0, mirrored.
    let x = df / (df + t * t);
    let ib = betai(df / 2.0, 0.5, x);
    if t >= 0.0 { 1.0 - 0.5 * ib } else { 0.5 * ib }
}

/// Standard normal cumulative distribution function.
/// Promoted from common.rs; N(0,1) CDF via erf.
pub fn probnorm(z: f64) -> f64 {
    0.5 * (1.0 + erf(z / std::f64::consts::SQRT_2))
}

/// Inverse CDF of standard normal (quantile).
/// Promoted from common.rs; Acklam approximation + 1 Halley step.
pub fn phi_inv(p: f64) -> f64 {
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }

    // Rational approximation coefficients (Acklam).
    const A: [f64; 6] = [
        -3.969683028665376e+01,
        2.209460984245205e+02,
        -2.759285104469687e+02,
        1.383577518672690e+02,
        -3.066479806614716e+01,
        2.506628277459239e+00,
    ];
    const B: [f64; 5] = [
        -5.447609879822406e+01,
        1.615858368580409e+02,
        -1.556989798598866e+02,
        6.680131188771972e+01,
        -1.328068155288572e+01,
    ];
    const C: [f64; 6] = [
        -7.784894002430293e-03,
        -3.223964580411365e-01,
        -2.400758277161838e+00,
        -2.549732539343734e+00,
        4.374664141464968e+00,
        2.938163982698783e+00,
    ];
    const D: [f64; 4] = [
        7.784695709041462e-03,
        3.224671290700398e-01,
        2.445134137142996e+00,
        3.754408661907416e+00,
    ];

    // Break-points for the central / tail regions.
    const P_LOW: f64 = 0.02425;
    const P_HIGH: f64 = 1.0 - P_LOW;

    let mut x = if p < P_LOW {
        // Lower tail.
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= P_HIGH {
        // Central region.
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        // Upper tail.
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };

    // One Halley refinement step: e = Φ(x) − p, u = e·√(2π)·exp(x²/2).
    let e = probnorm(x) - p;
    let u = e * (2.0 * std::f64::consts::PI).sqrt() * (0.5 * x * x).exp();
    x -= u / (1.0 + 0.5 * x * u);
    x
}

/// Student-t distribution quantile (inverse CDF).
/// Quantile of t(df) for p ∈ (0, 1). Exploits T² ~ F(1, df): for the upper
/// tail (p ≥ 0.5) the t-quantile is √(F-quantile(2p−1, 1, df)); the lower tail
/// is its negative mirror. Validation: t_quantile(0.975, 10) ≈ 2.228139.
pub fn t_quantile(p: f64, df: f64) -> f64 {
    if df <= 0.0 || !(0.0..=1.0).contains(&p) {
        return f64::NAN;
    }
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    if p == 0.5 {
        return 0.0;
    }
    if p > 0.5 {
        // Upper tail: P(T ≤ t) = p, t > 0. With f = t², P(F ≤ f) = 2p − 1.
        f_quantile(2.0 * p - 1.0, 1.0, df).sqrt()
    } else {
        // Lower tail by symmetry.
        -f_quantile(1.0 - 2.0 * p, 1.0, df).sqrt()
    }
}
