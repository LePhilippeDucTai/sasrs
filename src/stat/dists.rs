//! Distributions de probabilité et fonctions statistiques maison.
//!
//! ## Plan d'implémentation (M24.1)
//!
//! **Promotion depuis `src/procs/common.rs`** (verbatim, zéro changement logique) :
//! - `ln_gamma(x)` : Lanczos approximation Γ(x), x > 0, accuracy ~1e-13.
//! - `betai(a, b, x)` : regularized incomplete beta I_x(a,b), for t-CDF, F-CDF, etc.
//! - `betacf(a, b, x)` : continued fraction for betai, accuracy tuned.
//! - `student_t_cdf(t, df)` : CDF of Student-t, t∈ℝ, df>0 via betai.
//! - `gammq(a, x)` : upper incomplete gamma Q(a,x) = 1 - P(a,x), via gser/gcf.
//! - `gser(a, x)` : ascending series P(a,x).
//! - `gcf(a, x)` : continued fraction for P(a,x).
//! - `erf(x)` : error function, used in normal CDF.
//! - `probnorm(z)` : CDF of standard normal N(0,1), z∈ℝ, via erf.
//! - `phi_inv(p)` : inverse CDF (quantile) of standard normal, p∈(0,1),
//!   Acklam approximation + 1 Halley step. Validated: phi_inv(0.975)≈1.9599640.
//! - `ln_factorial(n)` : ln(n!) via ln_gamma, for binomial coefficients.
//! - `ln_choose(n, k)` : ln(C(n,k)) = ln(n!) - ln(k!) - ln((n-k)!),
//!   no overflow on large n (stored as ln).
//!
//! **Ajout M24.1 : distributions manquantes**
//!
//! Toutes sont testées contre valeurs SAS documentées ou J.H.Maindonald et al.
//!
//! ### Chi-squared distribution
//! - `chisq_cdf(x: f64, df: f64) -> f64`
//!   CDF of χ²(df). Implemented as:
//!   ```text
//!   P(χ²(df) ≤ x) = gammq(df/2, x/2)  if x > 0 else 0.0
//!   ```
//!   Reuses existing `gammq`. Validation (SAS reference):
//!   - chisq_cdf(5.0, 2.0) ≈ 0.91795 (df=2, x=5)
//!   - chisq_cdf(3.841, 1.0) ≈ 0.95000 (critical value)
//!
//! - `chisq_quantile(p: f64, df: f64) -> f64`
//!   Inverse CDF. Implemented via Newton-Raphson on `chisq_cdf`:
//!   Initial guess: `(df + 2*p*df)^(1/3) * df` (Wilson-Hilferty approx)
//!   Loop: x_{n+1} = x_n - (chisq_cdf(x_n) - p) / chisq_pdf(x_n)
//!   where `chisq_pdf(x) = (x^(df/2-1) * exp(-x/2)) / (2^(df/2) * Γ(df/2))`
//!   Converge to ~1e-12 relative error, max ~20 iterations.
//!   Validation: chisq_quantile(0.95, 1) ≈ 3.841459.
//!
//! ### F distribution (ratio of scaled chi-squared)
//! - `f_cdf(x: f64, df1: f64, df2: f64) -> f64`
//!   CDF of F(df1, df2). Relationship:
//!   ```text
//!   P(F(d1,d2) ≤ x) = betai(d1/2, d2/2, d1*x / (d1*x + d2))
//!   ```
//!   Validation (SAS, df1=2, df2=10):
//!   - f_cdf(1.0, 2, 10) ≈ 0.40155
//!   - f_cdf(4.103, 2, 10) ≈ 0.95000 (critical value)
//!
//! - `f_quantile(p: f64, df1: f64, df2: f64) -> f64`
//!   Inverse CDF. Implemented via Newton-Raphson on f_cdf:
//!   Initial guess: `df1 / (df2 - 2.0)` (crude but reasonable)
//!   Loop: x_{n+1} = x_n - (f_cdf(x_n) - p) / f_pdf(x_n)
//!   where f_pdf requires `betai` derivative (numerical or exact).
//!   For stability: use Acklam-like approximation with refinement.
//!   Converge to ~1e-12 relative error.
//!   Validation: f_quantile(0.95, 2, 10) ≈ 4.10281.
//!
//! ### Gamma distribution
//! - `gamma_cdf(x: f64, shape: f64, scale: f64) -> f64`
//!   CDF of Gamma(α, β) parameterized as pdf ∝ x^(α-1) exp(-x/β).
//!   Implemented as: P(X ≤ x) = P(α, x/β) = gammq(α, x/β)
//!   (Note: SAS uses shape α, scale β; some sources use rate γ=1/β).
//!   Validation: Gamma(2, 1) CDF at x=2 should match exponential sum.
//!
//! - `gamma_quantile(p: f64, shape: f64, scale: f64) -> f64`
//!   Inverse CDF via Newton-Raphson on gamma_cdf.
//!   Initial guess via normal approx or Cornish-Fisher transform.
//!
//! ### Beta distribution
//! - `beta_cdf(x: f64, alpha: f64, beta: f64) -> f64`
//!   CDF of Beta(α, β) on [0, 1]. Directly:
//!   ```text
//!   P(X ≤ x) = betai(α, β, x)
//!   ```
//!   Validation: Beta(2, 2) mode at 0.5, CDF(0.5) = 0.5.
//!
//! - `beta_quantile(p: f64, alpha: f64, beta: f64) -> f64`
//!   Inverse CDF. No closed form; use Newton-Raphson on beta_cdf.
//!   Initial guess: simple bisection or approximation.
//!   Converge to ~1e-12 relative error on [0, 1].
//!
//! ## Tests
//!
//! All functions include unit tests validating against:
//! - Edge cases (x=0, p=0.5, p→0/→1)
//! - SAS PROBT/PROBF/PROBCHI/PROBGAM/PROBBETA reference values
//! - Monotonicity of CDF, identity of CDF∘quantile.
//! - Numerical stability (no NaN/Inf on valid inputs).
//!
//! Special case handling:
//! - p < 0 or p > 1 → ERROR (or clamp to [0,1])
//! - df ≤ 0 → ERROR
//! - x < 0 (for χ², gamma, beta) → 0.0
//!
//! ## Performance considerations
//!
//! All algorithms avoid iterative loops where possible (precomputed ln_gamma,
//! betai via continued fraction). Newton-Raphson limited to ~20 iterations
//! with exit on convergence; no infinite loops or pathological cases.

/// Lanczos approximation of ln Γ(x) for x > 0. Accuracy ~1e-13.
/// Promoted from common.rs.
pub fn ln_gamma(x: f64) -> f64 {
    const COF: [f64; 6] = [
        76.18009172947146,
        -86.50532032941677,
        24.01409824083091,
        -1.231739572450155,
        0.1208650973866179e-2,
        -0.5395239384953e-5,
    ];
    let mut y = x;
    let tmp = x + 5.5 - (x + 0.5) * (x + 5.5).ln();
    let mut ser = 1.000000000190015;
    for c in COF.iter() {
        y += 1.0;
        ser += c / y;
    }
    -tmp + (2.5066282746310005 * ser / x).ln()
}

/// Continued fraction for the incomplete beta function (Lentz's algorithm).
/// Promoted from common.rs.
fn betacf(a: f64, b: f64, x: f64) -> f64 {
    const MAXIT: usize = 300;
    const EPS: f64 = 3.0e-15;
    const FPMIN: f64 = 1.0e-300;

    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < FPMIN {
        d = FPMIN;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=MAXIT {
        let m = m as f64;
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            break;
        }
    }
    h
}

/// Regularized incomplete beta function I_x(a, b), x in [0,1].
/// Promoted from common.rs; used by many distributions (t, F, beta).
pub fn betai(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let ln_beta = ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b);
    let front = (a * x.ln() + b * (1.0 - x).ln() + ln_beta).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        front * betacf(a, b, x) / a
    } else {
        1.0 - front * betacf(b, a, 1.0 - x) / b
    }
}

/// Student-t cumulative distribution function.
/// Promoted from common.rs; t ∈ ℝ, df > 0.
pub fn student_t_cdf(t: f64, df: f64) -> f64 {
    // P(T <= t) = 1 - 0.5 * I_{df/(df+t^2)}(df/2, 1/2) for t >= 0, mirrored.
    let x = df / (df + t * t);
    let ib = betai(df / 2.0, 0.5, x);
    if t >= 0.0 {
        1.0 - 0.5 * ib
    } else {
        0.5 * ib
    }
}

/// Series representation of the lower regularized incomplete gamma P(a, x),
/// valid (convergent) for x < a + 1.
/// Promoted from common.rs, internal helper.
fn gser(a: f64, x: f64) -> f64 {
    const ITMAX: usize = 300;
    const EPS: f64 = 3.0e-15;
    if x <= 0.0 {
        return 0.0;
    }
    let gln = ln_gamma(a);
    let mut ap = a;
    let mut sum = 1.0 / a;
    let mut del = sum;
    for _ in 0..ITMAX {
        ap += 1.0;
        del *= x / ap;
        sum += del;
        if del.abs() < sum.abs() * EPS {
            break;
        }
    }
    sum * (-x + a * x.ln() - gln).exp()
}

/// Continued-fraction representation of the upper regularized incomplete gamma
/// Q(a, x) (Lentz's algorithm), valid (convergent) for x >= a + 1.
/// Promoted from common.rs, internal helper.
fn gcf(a: f64, x: f64) -> f64 {
    const ITMAX: usize = 300;
    const EPS: f64 = 3.0e-15;
    const FPMIN: f64 = 1.0e-300;
    let gln = ln_gamma(a);
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / FPMIN;
    let mut d = 1.0 / b;
    let mut h = d;
    for i in 1..=ITMAX {
        let an = -(i as f64) * (i as f64 - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = b + an / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            break;
        }
    }
    (-x + a * x.ln() - gln).exp() * h
}

/// Regularized upper incomplete gamma function Q(a, x) = 1 - P(a, x).
/// Promoted from common.rs; used by chi-squared CDF.
pub fn gammq(a: f64, x: f64) -> f64 {
    if x < 0.0 || a <= 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return 1.0;
    }
    if x < a + 1.0 {
        1.0 - gser(a, x)
    } else {
        gcf(a, x)
    }
}

/// Error function erf(x), via the regularized lower incomplete gamma
/// P(1/2, x²).
/// Promoted from common.rs; used in normal CDF.
pub fn erf(x: f64) -> f64 {
    if x == 0.0 {
        return 0.0;
    }
    // P(1/2, x²) = lower regularized incomplete gamma = 1 - Q(1/2, x²).
    let p = 1.0 - gammq(0.5, x * x);
    if x > 0.0 {
        p
    } else {
        -p
    }
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

/// Natural log of n! = ln Γ(n+1), for n >= 0.
/// Promoted from common.rs.
pub fn ln_factorial(n: u64) -> f64 {
    ln_gamma(n as f64 + 1.0)
}

/// Natural log of the binomial coefficient C(n, k). Returns -inf when
/// k > n (coefficient 0).
/// Promoted from common.rs.
pub fn ln_choose(n: u64, k: u64) -> f64 {
    if k > n {
        return f64::NEG_INFINITY;
    }
    ln_factorial(n) - ln_factorial(k) - ln_factorial(n - k)
}

/// ─────────────────────────── M24.1 additions ───────────────────────────

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
fn chisq_pdf(x: f64, df: f64) -> f64 {
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
    newton_with_bisection(p, x, 0.0, f64::INFINITY, |v| chisq_cdf(v, df), |v| chisq_pdf(v, df))
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
fn f_pdf(x: f64, df1: f64, df2: f64) -> f64 {
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
fn gamma_pdf(x: f64, shape: f64, scale: f64) -> f64 {
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
fn beta_pdf(x: f64, alpha: f64, beta: f64) -> f64 {
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

/// Generic Newton-Raphson root finder for `cdf(x) = p` on the open interval
/// (lo, hi), with bisection fallback for robustness. `cdf` must be a strictly
/// increasing CDF and `pdf` its derivative. Bracket is maintained from the
/// monotonicity of the CDF; if a Newton step leaves the current bracket or the
/// derivative is degenerate, a bisection step is taken instead.
fn newton_with_bisection<F, G>(p: f64, init: f64, lo: f64, hi: f64, cdf: F, pdf: G) -> f64
where
    F: Fn(f64) -> f64,
    G: Fn(f64) -> f64,
{
    let mut a = lo;
    let mut b = hi;
    let mut x = init;
    for _ in 0..100 {
        let fx = cdf(x) - p;
        // Tighten the bracket using monotonicity.
        if fx < 0.0 {
            a = x;
        } else {
            b = x;
        }
        if fx.abs() < 1e-14 {
            break;
        }
        let d = pdf(x);
        let mut next = if d.abs() > 1e-300 {
            x - fx / d
        } else {
            f64::NAN
        };
        // Fall back to bisection if Newton leaves the bracket or misbehaves.
        if !next.is_finite() || next <= a || next >= b {
            // Need a finite bracket for bisection.
            if a.is_finite() && b.is_finite() {
                next = 0.5 * (a + b);
            } else if a.is_finite() {
                // Upper bound still unbounded: expand x upward to find it.
                next = (x * 2.0).max(a + 1.0);
            } else {
                next = x;
            }
        }
        if (next - x).abs() <= 1e-13 * (1.0 + x.abs()) {
            x = next;
            break;
        }
        x = next;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * (1.0 + b.abs())
    }

    // ───────────────────────── promoted helpers ─────────────────────────

    #[test]
    fn test_ln_gamma() {
        // Γ(1)=1, Γ(5)=24, Γ(0.5)=√π.
        assert!(approx(ln_gamma(1.0), 0.0, 1e-10));
        assert!(approx(ln_gamma(5.0), 24f64.ln(), 1e-10));
        assert!(approx(ln_gamma(0.5), std::f64::consts::PI.sqrt().ln(), 1e-10));
    }

    #[test]
    fn test_betai() {
        // I_x(a,b) edge cases and symmetry I_x(a,b) = 1 - I_{1-x}(b,a).
        assert_eq!(betai(2.0, 3.0, 0.0), 0.0);
        assert_eq!(betai(2.0, 3.0, 1.0), 1.0);
        assert!(approx(betai(2.0, 3.0, 0.5), 1.0 - betai(3.0, 2.0, 0.5), 1e-12));
        // I_0.5(1,1) = 0.5 (uniform).
        assert!(approx(betai(1.0, 1.0, 0.5), 0.5, 1e-12));
    }

    #[test]
    fn test_student_t_cdf() {
        // Symmetric: CDF(0) = 0.5.
        assert!(approx(student_t_cdf(0.0, 10.0), 0.5, 1e-12));
        // df=10, t=2.228 → ~0.975 (two-tailed 0.05 critical value).
        assert!(approx(student_t_cdf(2.228138852, 10.0), 0.975, 1e-6));
        // Symmetry: CDF(-t) = 1 - CDF(t).
        assert!(approx(student_t_cdf(-1.5, 7.0), 1.0 - student_t_cdf(1.5, 7.0), 1e-12));
    }

    #[test]
    fn test_gammq() {
        // Q(a,0)=1, monotone decreasing in x.
        assert_eq!(gammq(2.0, 0.0), 1.0);
        assert!(gammq(2.0, 1.0) > gammq(2.0, 3.0));
        // Q(1,x) = exp(-x) (exponential survival).
        assert!(approx(gammq(1.0, 2.0), (-2.0f64).exp(), 1e-10));
    }

    #[test]
    fn test_erf() {
        // erf(0)=0, erf(∞)→1, odd function.
        assert_eq!(erf(0.0), 0.0);
        assert!(approx(erf(1.0), 0.8427007929, 1e-8));
        assert!(approx(erf(-0.5), -erf(0.5), 1e-12));
    }

    #[test]
    fn test_probnorm() {
        // Φ(0)=0.5, Φ(1.959964)≈0.975, Φ(-z)=1-Φ(z).
        assert!(approx(probnorm(0.0), 0.5, 1e-12));
        assert!(approx(probnorm(1.959963985), 0.975, 1e-8));
        assert!(approx(probnorm(-1.0), 1.0 - probnorm(1.0), 1e-12));
    }

    #[test]
    fn test_phi_inv() {
        // Φ⁻¹(0.5)=0, Φ⁻¹(0.975)≈1.9599640, round-trip.
        assert!(approx(phi_inv(0.5), 0.0, 1e-10));
        assert!(approx(phi_inv(0.975), 1.959963985, 1e-7));
        assert!(approx(probnorm(phi_inv(0.123)), 0.123, 1e-10));
    }

    #[test]
    fn test_ln_factorial_choose() {
        // 5! = 120, C(5,2)=10, C(10,0)=1.
        assert!(approx(ln_factorial(5).exp(), 120.0, 1e-8));
        assert!(approx(ln_choose(5, 2).exp(), 10.0, 1e-8));
        assert!(approx(ln_choose(10, 0).exp(), 1.0, 1e-8));
        assert_eq!(ln_choose(2, 5), f64::NEG_INFINITY);
    }

    // ───────────────────────── chi-squared ─────────────────────────

    #[test]
    fn test_chisq_cdf() {
        assert_eq!(chisq_cdf(0.0, 2.0), 0.0);
        assert_eq!(chisq_cdf(-1.0, 2.0), 0.0);
        // SAS reference: df=2, x=5 → 0.91791.
        assert!(approx(chisq_cdf(5.0, 2.0), 0.9179150014, 1e-6));
        // Critical value: df=1, x=3.841459 → 0.95.
        assert!(approx(chisq_cdf(3.841458821, 1.0), 0.95, 1e-6));
    }

    #[test]
    fn test_chisq_quantile() {
        // chisq_quantile(0.95, 1) ≈ 3.841459.
        assert!(approx(chisq_quantile(0.95, 1.0), 3.841458821, 1e-5));
        // df=10, 0.95 → 18.30704.
        assert!(approx(chisq_quantile(0.95, 10.0), 18.30703805, 1e-5));
        // Round-trip with CDF.
        assert!(approx(chisq_cdf(chisq_quantile(0.3, 5.0), 5.0), 0.3, 1e-8));
    }

    #[test]
    fn test_chisq_edge() {
        assert_eq!(chisq_quantile(0.0, 3.0), 0.0);
        assert!(chisq_quantile(1.0, 3.0).is_infinite());
        assert!(chisq_cdf(1.0, -1.0).is_nan());
    }

    // ───────────────────────── F distribution ─────────────────────────

    #[test]
    fn test_f_cdf() {
        assert_eq!(f_cdf(0.0, 2.0, 10.0), 0.0);
        // df1=2, df2=10, x=1: CDF = betai(1,5,1/6) = 1-(5/6)^5 = 0.59812.
        // (The 0.40155 in the header is the upper-tail survival prob 1-CDF.)
        assert!(approx(f_cdf(1.0, 2.0, 10.0), 0.5981224280, 1e-9));
        // Critical: df1=2, df2=10, x=4.102821 → 0.95.
        assert!(approx(f_cdf(4.102821015, 2.0, 10.0), 0.95, 1e-6));
    }

    #[test]
    fn test_f_quantile() {
        // f_quantile(0.95, 2, 10) ≈ 4.102821.
        assert!(approx(f_quantile(0.95, 2.0, 10.0), 4.102821015, 1e-4));
        // df1=5, df2=20, 0.95 → 2.71089.
        assert!(approx(f_quantile(0.95, 5.0, 20.0), 2.71089, 1e-3));
        // Round-trip.
        assert!(approx(f_cdf(f_quantile(0.4, 3.0, 12.0), 3.0, 12.0), 0.4, 1e-7));
    }

    #[test]
    fn test_f_edge() {
        assert_eq!(f_quantile(0.0, 2.0, 5.0), 0.0);
        assert!(f_quantile(1.0, 2.0, 5.0).is_infinite());
        assert!(f_cdf(1.0, -1.0, 5.0).is_nan());
    }

    #[test]
    fn test_t_quantile() {
        // Classic table values.
        assert!(approx(t_quantile(0.975, 10.0), 2.228138852, 1e-6));
        assert!(approx(t_quantile(0.95, 5.0), 2.015048373, 1e-6));
        // Symmetry: q(1-p) == -q(p).
        assert!(approx(t_quantile(0.025, 10.0), -2.228138852, 1e-6));
        assert_eq!(t_quantile(0.5, 7.0), 0.0);
        // Round-trip against the CDF.
        assert!(approx(student_t_cdf(t_quantile(0.8, 12.0), 12.0), 0.8, 1e-7));
        assert!(approx(student_t_cdf(t_quantile(0.3, 4.0), 4.0), 0.3, 1e-7));
        // Large df → standard normal quantile.
        assert!(approx(t_quantile(0.975, 1.0e6), 1.959963985, 1e-4));
    }

    #[test]
    fn test_t_edge() {
        assert!(t_quantile(0.0, 5.0).is_infinite() && t_quantile(0.0, 5.0) < 0.0);
        assert!(t_quantile(1.0, 5.0).is_infinite() && t_quantile(1.0, 5.0) > 0.0);
        assert!(t_quantile(0.9, -1.0).is_nan());
    }

    // ───────────────────────── gamma ─────────────────────────

    #[test]
    fn test_gamma_cdf() {
        assert_eq!(gamma_cdf(0.0, 2.0, 1.0), 0.0);
        // Gamma(1, scale) = Exponential(1/scale): CDF = 1 - exp(-x/scale).
        assert!(approx(gamma_cdf(2.0, 1.0, 1.0), 1.0 - (-2.0f64).exp(), 1e-10));
        // Gamma(2,1) at x=2: 1 - exp(-2)(1+2) = 1 - 3e^-2.
        assert!(approx(gamma_cdf(2.0, 2.0, 1.0), 1.0 - 3.0 * (-2.0f64).exp(), 1e-9));
    }

    #[test]
    fn test_gamma_quantile() {
        // Round-trip.
        assert!(approx(gamma_cdf(gamma_quantile(0.5, 2.0, 1.5), 2.0, 1.5), 0.5, 1e-9));
        assert!(approx(gamma_cdf(gamma_quantile(0.9, 3.0, 2.0), 3.0, 2.0), 0.9, 1e-9));
        // Exponential(scale=2): quantile(p) = -2 ln(1-p).
        assert!(approx(gamma_quantile(0.5, 1.0, 2.0), -2.0 * 0.5f64.ln(), 1e-7));
    }

    #[test]
    fn test_gamma_edge() {
        assert_eq!(gamma_quantile(0.0, 2.0, 1.0), 0.0);
        assert!(gamma_quantile(1.0, 2.0, 1.0).is_infinite());
        assert!(gamma_cdf(1.0, -1.0, 1.0).is_nan());
    }

    // ───────────────────────── beta ─────────────────────────

    #[test]
    fn test_beta_cdf() {
        assert_eq!(beta_cdf(0.0, 2.0, 2.0), 0.0);
        assert_eq!(beta_cdf(1.0, 2.0, 2.0), 1.0);
        // Beta(2,2) symmetric: CDF(0.5)=0.5.
        assert!(approx(beta_cdf(0.5, 2.0, 2.0), 0.5, 1e-10));
        // Beta(1,1) uniform: CDF(x)=x.
        assert!(approx(beta_cdf(0.3, 1.0, 1.0), 0.3, 1e-10));
    }

    #[test]
    fn test_beta_quantile() {
        // Beta(2,2) median = 0.5.
        assert!(approx(beta_quantile(0.5, 2.0, 2.0), 0.5, 1e-8));
        // Uniform: quantile(p)=p.
        assert!(approx(beta_quantile(0.42, 1.0, 1.0), 0.42, 1e-9));
        // Round-trip.
        assert!(approx(beta_cdf(beta_quantile(0.7, 3.0, 5.0), 3.0, 5.0), 0.7, 1e-8));
    }

    #[test]
    fn test_beta_edge() {
        assert_eq!(beta_quantile(0.0, 2.0, 2.0), 0.0);
        assert_eq!(beta_quantile(1.0, 2.0, 2.0), 1.0);
        assert!(beta_cdf(0.5, -1.0, 2.0).is_nan());
        // Quantile stays within [0,1].
        let q = beta_quantile(0.99, 2.0, 8.0);
        assert!((0.0..=1.0).contains(&q));
    }
}
