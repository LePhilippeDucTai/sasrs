use super::*;

/// FACT(n): factorial (n! where n ≥ 0 integer).
/// n < 0 or non-integer → missing + error.
/// overflow → missing + warning.
pub(crate) fn fn_fact(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(f) => {
                // Check if integer
                if f.fract() != 0.0 {
                    ctx.error_flag = true;
                    ctx.invalid_data += 1;
                    return Value::missing();
                }
                // Check if non-negative
                if f < 0.0 {
                    ctx.error_flag = true;
                    ctx.invalid_data += 1;
                    return Value::missing();
                }
                let n = f as u32;
                // Compute factorial with overflow check
                let mut result = 1i64;
                for i in 2..=n as i64 {
                    match result.checked_mul(i) {
                        Some(r) => result = r,
                        None => {
                            // Overflow
                            return Value::missing();
                        }
                    }
                }
                Value::Num(result as f64)
            }
        },
    }
}

/// COMB(n, k): binomial coefficient C(n,k) = n! / (k!(n-k)!).
/// k > n or k < 0 → 0.
/// invalid inputs → missing + error.
pub(crate) fn fn_comb(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (None, _) | (_, None) => Value::missing(),
        (Some(nf), Some(kf)) => {
            // Check if integers
            if nf.fract() != 0.0 || kf.fract() != 0.0 {
                ctx.error_flag = true;
                ctx.invalid_data += 1;
                return Value::missing();
            }
            let n = nf as i64;
            let k = kf as i64;
            // Check if non-negative
            if n < 0 || k < 0 {
                ctx.error_flag = true;
                ctx.invalid_data += 1;
                return Value::missing();
            }
            // k > n → 0
            if k > n {
                return Value::Num(0.0);
            }
            // Compute C(n,k) = n! / (k!(n-k)!)
            // Use efficient formula: C(n,k) = n * (n-1) * ... * (n-k+1) / (k!)
            let k = k.min(n - k); // Use symmetry to reduce computation
            let mut result = 1i64;
            for i in 0..k {
                match result.checked_mul(n - i) {
                    Some(r) => result = r,
                    None => return Value::missing(), // Overflow
                }
                result /= i + 1;
            }
            Value::Num(result as f64)
        }
    }
}

/// PERM(n, k): permutation P(n,k) = n! / (n-k)!.
/// k > n or k < 0 → 0.
/// invalid inputs → missing + error.
pub(crate) fn fn_perm(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (None, _) | (_, None) => Value::missing(),
        (Some(nf), Some(kf)) => {
            // Check if integers
            if nf.fract() != 0.0 || kf.fract() != 0.0 {
                ctx.error_flag = true;
                ctx.invalid_data += 1;
                return Value::missing();
            }
            let n = nf as i64;
            let k = kf as i64;
            // Check if non-negative
            if n < 0 || k < 0 {
                ctx.error_flag = true;
                ctx.invalid_data += 1;
                return Value::missing();
            }
            // k > n → 0
            if k > n {
                return Value::Num(0.0);
            }
            // Compute P(n,k) = n * (n-1) * ... * (n-k+1)
            let mut result = 1i64;
            for i in 0..k {
                match result.checked_mul(n - i) {
                    Some(r) => result = r,
                    None => return Value::missing(), // Overflow
                }
            }
            Value::Num(result as f64)
        }
    }
}

/// GAMMA(x): gamma function Γ(x).
/// x ≤ 0 integer → missing + error.
/// x > 170 → infinity.
pub(crate) fn fn_gamma(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_checked(args, ctx, |x| {
        // x <= 0 integer is a pole → out of domain
        if x <= 0.0 && x.fract() == 0.0 {
            None
        } else if x > 170.0 {
            // For x > 170, Gamma(x) overflows; return infinity
            Some(f64::INFINITY)
        } else {
            // Use Stirling's approximation for large x
            // For small x, use the recurrence relation or direct computation
            Some(gamma_approx(x))
        }
    })
}

/// LGAMMA(x): log-gamma log|Γ(x)|.
/// x ≤ 0 integer → missing + error.
pub(crate) fn fn_lgamma(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_checked(args, ctx, |x| {
        if x <= 0.0 && x.fract() == 0.0 { None } else { Some(lgamma_approx(x)) }
    })
}

/// DIGAMMA(x): digamma ψ(x) = d/dx log Γ(x).
/// x ≤ 0 integer → missing + error.
pub(crate) fn fn_digamma(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_checked(args, ctx, |x| {
        if x <= 0.0 && x.fract() == 0.0 { None } else { Some(crate::stat::digamma(x)) }
    })
}

/// TRIGAMMA(x): trigamma ψ′(x) = d²/dx² log Γ(x).
/// x ≤ 0 integer → missing + error (mirrors DIGAMMA's pole handling).
pub(crate) fn fn_trigamma(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_checked(args, ctx, |x| {
        if x <= 0.0 && x.fract() == 0.0 { None } else { Some(crate::stat::trigamma(x)) }
    })
}

/// BETA(a, b): beta function B(a,b) = Γ(a)Γ(b) / Γ(a+b).
/// a, b > 0 required; invalid → missing + error.
pub(crate) fn fn_beta(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (None, _) | (_, None) => Value::missing(),
        (Some(a), Some(b)) => {
            if a <= 0.0 || b <= 0.0 {
                ctx.error_flag = true;
                ctx.invalid_data += 1;
                return Value::missing();
            }
            let result = (gamma_approx(a) * gamma_approx(b)) / gamma_approx(a + b);
            Value::Num(result)
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Helper functions for special mathematical functions
// ──────────────────────────────────────────────────────────────────────────────

/// Stirling's approximation for gamma function.
/// Uses the formula: Γ(x) ≈ √(2π) * (x/e)^x * x^(-1/2)
/// More precisely: ln Γ(x) ≈ (x - 1/2) ln(x) - x + ln(2π)/2 + 1/(12x) - ...
pub(crate) fn gamma_approx(x: f64) -> f64 {
    if x < 0.5 {
        // Use reflection formula: Γ(x) = π / (sin(πx) * Γ(1-x))
        let pi = std::f64::consts::PI;
        pi / ((pi * x).sin() * gamma_approx(1.0 - x))
    } else {
        // Stirling's approximation for x >= 0.5
        let ln_gamma = lgamma_approx(x);
        ln_gamma.exp()
    }
}

/// Log-gamma approximation using Stirling's formula.
/// ln Γ(x) ≈ (x - 1/2) ln(x) - x + ln(2π)/2 + 1/(12x) - 1/(360x^3) + ...
pub(crate) fn lgamma_approx(x: f64) -> f64 {
    if x < 0.5 {
        // Use reflection formula: ln|Γ(x)| = ln(π) - ln|sin(πx)| - ln|Γ(1-x)|
        let pi = std::f64::consts::PI;
        pi.ln() - (pi * x).sin().abs().ln() - lgamma_approx(1.0 - x)
    } else if x < 1.5 {
        // For small x, use recursion: ln Γ(x+1) = ln(x) + ln Γ(x)
        lgamma_approx(x + 1.0) - x.ln()
    } else {
        // Stirling's approximation
        let ln_2pi = (2.0 * std::f64::consts::PI).ln();
        let x_minus_half = x - 0.5;
        x_minus_half * x.ln() - x + 0.5 * ln_2pi
            + 1.0 / (12.0 * x)
            - 1.0 / (360.0 * x * x * x)
    }
}
