use super::*;

/// SIN(x): sine (x in radians).
pub(crate) fn fn_sin(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.sin())
}

/// COS(x): cosine (x in radians).
pub(crate) fn fn_cos(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.cos())
}

/// TAN(x): tangent (x in radians).
pub(crate) fn fn_tan(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.tan())
}

/// ARSIN(x): arcsine (domain -1 to +1, result in radians).
pub(crate) fn fn_arsin(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_checked(args, ctx, |f| {
        if f < -1.0 || f > 1.0 { None } else { Some(f.asin()) }
    })
}

/// ARCOS(x): arccosine (domain -1 to +1, result in radians).
pub(crate) fn fn_arcos(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num_checked(args, ctx, |f| {
        if f < -1.0 || f > 1.0 { None } else { Some(f.acos()) }
    })
}

/// ATAN(x): arctangent (result in radians).
pub(crate) fn fn_atan(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.atan())
}

/// ATAN2(y, x): two-argument arctangent (atan(y/x) with quadrant correction).
pub(crate) fn fn_atan2(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (None, _) | (_, None) => Value::missing(),
        (Some(y), Some(x)) => Value::Num(y.atan2(x)),
    }
}

/// SINH(x): hyperbolic sine.
pub(crate) fn fn_sinh(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.sinh())
}

/// COSH(x): hyperbolic cosine.
pub(crate) fn fn_cosh(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.cosh())
}

/// TANH(x): hyperbolic tangent.
pub(crate) fn fn_tanh(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| f.tanh())
}
