// ──────────────────────────────────────────────────────────────────────────────
// Math functions (propagate missing)
// ──────────────────────────────────────────────────────────────────────────────

use super::*;

mod combinatorics;
mod ordering;
mod trig;

pub(crate) use combinatorics::*;
pub(crate) use ordering::*;
pub(crate) use trig::*;

pub(crate) fn fn_abs(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.abs())
}

pub(crate) fn fn_sqrt(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_checked(args, ctx, |f| if f < 0.0 { None } else { Some(f.sqrt()) })
}

pub(crate) fn fn_exp(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.exp())
}

pub(crate) fn fn_log(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_checked(args, ctx, |f| if f <= 0.0 { None } else { Some(f.ln()) })
}

pub(crate) fn fn_log2(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_checked(args, ctx, |f| if f <= 0.0 { None } else { Some(f.log2()) })
}

pub(crate) fn fn_log10(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_checked(args, ctx, |f| if f <= 0.0 { None } else { Some(f.log10()) })
}

pub(crate) fn fn_int(args: &[Value], ctx: &mut EvalCtx) -> Value {
    // INT truncates toward zero (like Rust's `as i64` cast for in-range values).
    unary_num(args, ctx, |f| f.trunc())
}

pub(crate) fn fn_round(args: &[Value], ctx: &mut EvalCtx) -> Value {
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

pub(crate) fn fn_mod(args: &[Value], ctx: &mut EvalCtx) -> Value {
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
pub(crate) fn fn_ceil(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.ceil())
}

/// FLOOR(x): largest integer ≤ x.
pub(crate) fn fn_floor(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.floor())
}

/// SIGN(x): return -1.0 for negative, 0.0 for zero, 1.0 for positive.
pub(crate) fn fn_sign(args: &[Value], ctx: &mut EvalCtx) -> Value {
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

/// ROUNDZ(x, unit): round x to nearest unit, ties to zero (vs. ROUND's half-away-from-zero).
pub(crate) fn fn_roundz(args: &[Value], ctx: &mut EvalCtx) -> Value {
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
