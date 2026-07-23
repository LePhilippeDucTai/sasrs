// ──────────────────────────────────────────────────────────────────────────────
// Math functions (propagate missing)
// ──────────────────────────────────────────────────────────────────────────────

use super::*;

pub(super) fn fn_abs(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.abs())
}

pub(super) fn fn_sqrt(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_checked(args, ctx, |f| if f < 0.0 { None } else { Some(f.sqrt()) })
}

pub(super) fn fn_exp(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.exp())
}

pub(super) fn fn_log(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_checked(args, ctx, |f| if f <= 0.0 { None } else { Some(f.ln()) })
}

pub(super) fn fn_log2(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_checked(args, ctx, |f| if f <= 0.0 { None } else { Some(f.log2()) })
}

pub(super) fn fn_log10(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_checked(args, ctx, |f| if f <= 0.0 { None } else { Some(f.log10()) })
}

pub(super) fn fn_int(args: &[Value], ctx: &mut EvalCtx) -> Value {
    // INT truncates toward zero (like Rust's `as i64` cast for in-range values).
    unary_num(args, ctx, |f| f.trunc())
}

pub(super) fn fn_round(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(x) => {
                let unit = if args.len() >= 2 {
                    match coerce_num(&args[1], ctx) {
                        None => return Value::missing(),
                        Some(u) => u,
                    }
                } else {
                    1.0
                };
                if unit == 0.0 {
                    return Value::Num(x);
                }
                // SAS ROUND: half-away-from-zero.
                // (x / unit).round() in Rust already uses half-away-from-zero for f64.
                let rounded = (x / unit).round() * unit;
                Value::Num(rounded)
            }
        },
    }
}

pub(super) fn fn_mod(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (None, _) | (_, None) => Value::missing(),
        (Some(_), Some(b)) if b == 0.0 => {
            ctx.division_by_zero += 1;
            ctx.error_flag = true;
            Value::missing()
        }
        (Some(a), Some(b)) => {
            // SAS MOD: sign of result = sign of a (same as Rust's `%` for f64).
            Value::Num(a % b)
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// M15.2 Mathematical functions (M15.2)
// ──────────────────────────────────────────────────────────────────────────────

/// CEIL(x): smallest integer ≥ x.
pub(super) fn fn_ceil(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.ceil())
}

/// FLOOR(x): largest integer ≤ x.
pub(super) fn fn_floor(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.floor())
}

/// SIGN(x): return -1.0 for negative, 0.0 for zero, 1.0 for positive.
pub(super) fn fn_sign(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| {
        if f < 0.0 {
            -1.0
        } else if f > 0.0 {
            1.0
        } else {
            0.0
        }
    })
}

/// SIN(x): sine (x in radians).
pub(super) fn fn_sin(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.sin())
}

/// COS(x): cosine (x in radians).
pub(super) fn fn_cos(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.cos())
}

/// TAN(x): tangent (x in radians).
pub(super) fn fn_tan(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.tan())
}

/// ARSIN(x): arcsine (domain -1 to +1, result in radians).
pub(super) fn fn_arsin(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_checked(args, ctx, |f| {
        if f < -1.0 || f > 1.0 { None } else { Some(f.asin()) }
    })
}

/// ARCOS(x): arccosine (domain -1 to +1, result in radians).
pub(super) fn fn_arcos(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_checked(args, ctx, |f| {
        if f < -1.0 || f > 1.0 { None } else { Some(f.acos()) }
    })
}

/// ATAN(x): arctangent (result in radians).
pub(super) fn fn_atan(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.atan())
}

/// ATAN2(y, x): two-argument arctangent (atan(y/x) with quadrant correction).
pub(super) fn fn_atan2(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (None, _) | (_, None) => Value::missing(),
        (Some(y), Some(x)) => Value::Num(y.atan2(x)),
    }
}

/// SINH(x): hyperbolic sine.
pub(super) fn fn_sinh(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.sinh())
}

/// COSH(x): hyperbolic cosine.
pub(super) fn fn_cosh(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.cosh())
}

/// TANH(x): hyperbolic tangent.
pub(super) fn fn_tanh(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.tanh())
}

/// FACT(n): factorial (n! where n ≥ 0 integer).
/// n < 0 or non-integer → missing + error.
/// overflow → missing + warning.
pub(super) fn fn_fact(args: &[Value], ctx: &mut EvalCtx) -> Value {
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
pub(super) fn fn_comb(args: &[Value], ctx: &mut EvalCtx) -> Value {
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
pub(super) fn fn_perm(args: &[Value], ctx: &mut EvalCtx) -> Value {
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
pub(super) fn fn_gamma(args: &[Value], ctx: &mut EvalCtx) -> Value {
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
pub(super) fn fn_lgamma(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_checked(args, ctx, |x| {
        if x <= 0.0 && x.fract() == 0.0 { None } else { Some(lgamma_approx(x)) }
    })
}

/// DIGAMMA(x): digamma ψ(x) = d/dx log Γ(x).
/// x ≤ 0 integer → missing + error.
pub(super) fn fn_digamma(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_checked(args, ctx, |x| {
        if x <= 0.0 && x.fract() == 0.0 { None } else { Some(crate::stat::digamma(x)) }
    })
}

/// TRIGAMMA(x): trigamma ψ′(x) = d²/dx² log Γ(x).
/// x ≤ 0 integer → missing + error (mirrors DIGAMMA's pole handling).
pub(super) fn fn_trigamma(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_checked(args, ctx, |x| {
        if x <= 0.0 && x.fract() == 0.0 { None } else { Some(crate::stat::trigamma(x)) }
    })
}

/// BETA(a, b): beta function B(a,b) = Γ(a)Γ(b) / Γ(a+b).
/// a, b > 0 required; invalid → missing + error.
pub(super) fn fn_beta(args: &[Value], ctx: &mut EvalCtx) -> Value {
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

/// ROUNDZ(x, unit): round x to nearest unit, ties to zero (vs. ROUND's half-away-from-zero).
pub(super) fn fn_roundz(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::missing(),
            Some(x) => {
                let unit = if args.len() >= 2 {
                    match coerce_num(&args[1], ctx) {
                        None => return Value::missing(),
                        Some(u) => u,
                    }
                } else {
                    1.0
                };
                if unit == 0.0 {
                    return Value::Num(x);
                }
                // Round to nearest unit, ties toward zero
                let scaled = x / unit;
                let rounded = if scaled >= 0.0 {
                    // For positive: if fractional part < 0.5, floor; >= 0.5, ceil
                    let int_part = scaled.floor();
                    let frac_part = scaled - int_part;
                    if frac_part < 0.5 {
                        int_part
                    } else if frac_part > 0.5 {
                        int_part + 1.0
                    } else {
                        // Tie: round toward zero
                        int_part
                    }
                } else {
                    // For negative: if fractional part > -0.5, ceil; <= -0.5, floor
                    let int_part = scaled.ceil();
                    let frac_part = scaled - int_part;
                    if frac_part > -0.5 {
                        int_part
                    } else if frac_part < -0.5 {
                        int_part - 1.0
                    } else {
                        // Tie: round toward zero
                        int_part
                    }
                };
                Value::Num(rounded * unit)
            }
        },
    }
}

/// RANGE(x1, x2, ...): max(args) - min(args); missing ignored.
pub(super) fn fn_range(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let mut min_val: Option<f64> = None;
    let mut max_val: Option<f64> = None;
    for a in args {
        if let Some(f) = coerce_num(a, ctx) {
            min_val = Some(match min_val {
                None => f,
                Some(m) => if f < m { f } else { m },
            });
            max_val = Some(match max_val {
                None => f,
                Some(m) => if f > m { f } else { m },
            });
        }
    }
    match (min_val, max_val) {
        (Some(min), Some(max)) => Value::Num(max - min),
        _ => Value::missing(),
    }
}

/// LARGEST(k, x1, x2, ...): kth largest value.
/// k ≤ 0 or k > count → missing.
pub(super) fn fn_largest(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::missing();
    }
    let k_v = &args[0];
    let k = match coerce_num(k_v, ctx) {
        None => return Value::missing(),
        Some(f) => {
            if f.fract() != 0.0 {
                ctx.error_flag = true;
                ctx.invalid_data += 1;
                return Value::missing();
            }
            f as i64
        }
    };

    let mut values: Vec<f64> = Vec::new();
    for a in &args[1..] {
        if let Some(f) = coerce_num(a, ctx) {
            values.push(f);
        }
    }

    if k <= 0 || k > values.len() as i64 {
        return Value::missing();
    }

    // Sort in descending order and get kth element (1-based)
    values.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    Value::Num(values[(k - 1) as usize])
}

/// SMALLEST(k, x1, x2, ...): kth smallest value.
/// k ≤ 0 or k > count → missing.
pub(super) fn fn_smallest(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::missing();
    }
    let k_v = &args[0];
    let k = match coerce_num(k_v, ctx) {
        None => return Value::missing(),
        Some(f) => {
            if f.fract() != 0.0 {
                ctx.error_flag = true;
                ctx.invalid_data += 1;
                return Value::missing();
            }
            f as i64
        }
    };

    let mut values: Vec<f64> = Vec::new();
    for a in &args[1..] {
        if let Some(f) = coerce_num(a, ctx) {
            values.push(f);
        }
    }

    if k <= 0 || k > values.len() as i64 {
        return Value::missing();
    }

    // Sort in ascending order and get kth element (1-based)
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Value::Num(values[(k - 1) as usize])
}

/// ORDINAL(x): convert number to ordinal text ("1st", "2nd", "3rd", "4th", ...).
/// x must be integer; non-integer or invalid → empty string.
pub(super) fn fn_ordinal(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::Char(String::new()),
            Some(f) => {
                if f.fract() != 0.0 {
                    return Value::Char(String::new());
                }
                let n = f as i64;
                let suffix = if n % 100 == 11 || n % 100 == 12 || n % 100 == 13 {
                    "th"
                } else {
                    match n % 10 {
                        1 => "st",
                        2 => "nd",
                        3 => "rd",
                        _ => "th",
                    }
                };
                Value::Char(format!("{}{}", n, suffix))
            }
        },
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Helper functions for special mathematical functions
// ──────────────────────────────────────────────────────────────────────────────

/// Stirling's approximation for gamma function.
/// Uses the formula: Γ(x) ≈ √(2π) * (x/e)^x * x^(-1/2)
/// More precisely: ln Γ(x) ≈ (x - 1/2) ln(x) - x + ln(2π)/2 + 1/(12x) - ...
pub(super) fn gamma_approx(x: f64) -> f64 {
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
pub(super) fn lgamma_approx(x: f64) -> f64 {
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

