use super::*;

// ──────────────────────────────────────────────────────────────────────────────
// Probability distribution functions (M15.4)
//
// Wrappers around the numerical helpers defined earlier in this file
// (normal/t/F/chi-square/beta/gamma CDFs, incomplete-beta / incomplete-gamma,
// binomial & Poisson, plus the generic bisection quantile). All return numeric
// probabilities in [0, 1] (CDF/SDF) or densities (PDF) or quantiles (QUANTILE).
// Missing arguments propagate to a missing result.
// ──────────────────────────────────────────────────────────────────────────────

/// PROBNORM(x): standard normal CDF Φ(x).
pub(crate) fn fn_probnorm(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, normal_cdf_std)
}

/// PROBT(t, df): Student's t CDF P(T <= t). df must be > 0.
pub(crate) fn fn_probt(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (Some(t), Some(df)) if df > 0.0 => Value::Num(t_cdf(t, df)),
        (None, _) | (_, None) => Value::missing(),
        _ => {
            ctx.error_flag = true;
            ctx.invalid_data += 1;
            Value::missing()
        }
    }
}

/// PROBF(f, ndf, ddf): F CDF P(F <= f). ndf, ddf must be > 0.
pub(crate) fn fn_probf(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 3 {
        return Value::missing();
    }
    match (
        coerce_num(&args[0], ctx),
        coerce_num(&args[1], ctx),
        coerce_num(&args[2], ctx),
    ) {
        (Some(f), Some(ndf), Some(ddf)) if ndf > 0.0 && ddf > 0.0 => Value::Num(f_cdf(f, ndf, ddf)),
        (None, _, _) | (_, None, _) | (_, _, None) => Value::missing(),
        _ => {
            ctx.error_flag = true;
            ctx.invalid_data += 1;
            Value::missing()
        }
    }
}

/// PROBCHI(x, df): chi-square CDF P(χ² <= x). df must be > 0.
pub(crate) fn fn_probchi(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (Some(x), Some(df)) if df > 0.0 => Value::Num(chisq_cdf(x, df)),
        (None, _) | (_, None) => Value::missing(),
        _ => {
            ctx.error_flag = true;
            ctx.invalid_data += 1;
            Value::missing()
        }
    }
}

/// PROBBETA(x, a, b): Beta CDF P(X <= x). a, b must be > 0.
pub(crate) fn fn_probbeta(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 3 {
        return Value::missing();
    }
    match (
        coerce_num(&args[0], ctx),
        coerce_num(&args[1], ctx),
        coerce_num(&args[2], ctx),
    ) {
        (Some(x), Some(a), Some(b)) if a > 0.0 && b > 0.0 => Value::Num(beta_cdf(x, a, b)),
        (None, _, _) | (_, None, _) | (_, _, None) => Value::missing(),
        _ => {
            ctx.error_flag = true;
            ctx.invalid_data += 1;
            Value::missing()
        }
    }
}

/// PROBGAM(x, a): Gamma CDF P(X <= x) for shape = a, scale = 1. a must be > 0.
pub(crate) fn fn_probgam(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (Some(x), Some(a)) if a > 0.0 => Value::Num(gamma_cdf(x, a)),
        (None, _) | (_, None) => Value::missing(),
        _ => {
            ctx.error_flag = true;
            ctx.invalid_data += 1;
            Value::missing()
        }
    }
}

/// PROBBNML(p, n, k): binomial CDF P(X <= k) for X ~ Binomial(n, p).
pub(crate) fn fn_probbnml(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 3 {
        return Value::missing();
    }
    match (
        coerce_num(&args[0], ctx),
        coerce_num(&args[1], ctx),
        coerce_num(&args[2], ctx),
    ) {
        (Some(p), Some(n), Some(k)) if (0.0..=1.0).contains(&p) && n >= 0.0 && k >= 0.0 => {
            Value::Num(binomial_cdf(p, n, k))
        }
        (None, _, _) | (_, None, _) | (_, _, None) => Value::missing(),
        _ => {
            ctx.error_flag = true;
            ctx.invalid_data += 1;
            Value::missing()
        }
    }
}

/// POISSON(lambda, k): Poisson CDF P(X <= k) for X ~ Poisson(λ).
pub(crate) fn fn_poisson(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (Some(lambda), Some(k)) if lambda >= 0.0 && k >= 0.0 => Value::Num(poisson_cdf(lambda, k)),
        (None, _) | (_, None) => Value::missing(),
        _ => {
            ctx.error_flag = true;
            ctx.invalid_data += 1;
            Value::missing()
        }
    }
}
