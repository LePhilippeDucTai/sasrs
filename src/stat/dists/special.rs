
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
pub(super) fn betacf(a: f64, b: f64, x: f64) -> f64 {
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

/// Series representation of the lower regularized incomplete gamma P(a, x),
/// valid (convergent) for x < a + 1.
/// Promoted from common.rs, internal helper.
pub(super) fn gser(a: f64, x: f64) -> f64 {
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
pub(super) fn gcf(a: f64, x: f64) -> f64 {
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

/// ─────────────────────────── M37.3 additions ───────────────────────────

/// Digamma function ψ(x) = d/dx ln Γ(x).
///
/// Promoted verbatim (M37.3) from the DATA-step `digamma_approx` so that the
/// `DIGAMMA` SAS function stays **byte-identical**: same branch thresholds, same
/// asymptotic expression and exact operation order. The pole handling for
/// non-positive integers is the caller's responsibility (see `fn_digamma`),
/// matching the previous split.
///
/// ψ(x) ≈ ln(x) - 1/(2x) - 1/(12x²) + 1/(120x³)  (asymptotic, as historically
/// written), with the reflection formula below 0.5 and one recursion step in
/// [0.5, 1.5).
pub fn digamma(x: f64) -> f64 {
    if x < 0.5 {
        // Use reflection formula: ψ(x) = -ψ(1-x) - π/tan(πx)
        let pi = std::f64::consts::PI;
        -digamma(1.0 - x) - pi / (pi * x).tan()
    } else if x < 1.5 {
        // Use recursion: ψ(x+1) = ψ(x) + 1/x
        digamma(x + 1.0) - 1.0 / x
    } else {
        // Asymptotic expansion
        let ln_x = x.ln();
        let inv_x = 1.0 / x;
        ln_x - 0.5 * inv_x - inv_x * inv_x / 12.0 + inv_x * inv_x * inv_x / 120.0
    }
}

/// Trigamma function ψ′(x) = d²/dx² ln Γ(x) = Σ_{k≥0} 1/(x+k)².
///
/// New in M37.3 (no byte-identity constraint, so this targets ~1e-12). Strategy:
/// - For x < 0.5 use the reflection formula ψ′(1−x) + ψ′(x) = π²/sin²(πx).
/// - Otherwise push x above a threshold via the recurrence
///   ψ′(x) = ψ′(x+1) + 1/x², then apply the asymptotic series
///   ψ′(z) ≈ 1/z + 1/(2z²) + 1/(6z³) − 1/(30z⁵) + 1/(42z⁷).
///
/// The pole at non-positive integers is handled by the caller (`fn_trigamma`),
/// mirroring `digamma` / `fn_digamma`.
pub fn trigamma(x: f64) -> f64 {
    if x < 0.5 {
        // Reflection: ψ′(x) = π²/sin²(πx) − ψ′(1−x).
        let pi = std::f64::consts::PI;
        let s = (pi * x).sin();
        pi * pi / (s * s) - trigamma(1.0 - x)
    } else {
        // Recurrence: accumulate 1/z² while pushing z up to the threshold.
        let mut z = x;
        let mut acc = 0.0;
        while z < 12.0 {
            acc += 1.0 / (z * z);
            z += 1.0;
        }
        // Asymptotic expansion at z (≥ 12): 1/z + 1/(2z²) + 1/(6z³) − 1/(30z⁵) + 1/(42z⁷).
        let inv = 1.0 / z;
        let inv2 = inv * inv;
        let asymp = inv
            + 0.5 * inv2
            + inv2 * inv / 6.0
            - inv2 * inv2 * inv / 30.0
            + inv2 * inv2 * inv2 * inv / 42.0;
        acc + asymp
    }
}

/// Generic Newton-Raphson root finder for `cdf(x) = p` on the open interval
/// (lo, hi), with bisection fallback for robustness. `cdf` must be a strictly
/// increasing CDF and `pdf` its derivative. Bracket is maintained from the
/// monotonicity of the CDF; if a Newton step leaves the current bracket or the
/// derivative is degenerate, a bisection step is taken instead.
pub(super) fn newton_with_bisection<F, G>(p: f64, init: f64, lo: f64, hi: f64, cdf: F, pdf: G) -> f64
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
