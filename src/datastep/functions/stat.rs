// ──────────────────────────────────────────────────────────────────────────────
// Statistical functions (ignore missings)
// ──────────────────────────────────────────────────────────────────────────────

use super::*;

pub(super) fn fn_sum(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let mut total = 0.0f64;
    let mut n_valid = 0usize;
    for a in args {
        if let Some(f) = coerce_num(a, ctx) {
            total += f;
            n_valid += 1;
        }
    }
    if n_valid == 0 {
        Value::missing()
    } else {
        Value::Num(total)
    }
}

pub(super) fn fn_mean(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let mut total = 0.0f64;
    let mut n_valid = 0usize;
    for a in args {
        if let Some(f) = coerce_num(a, ctx) {
            total += f;
            n_valid += 1;
        }
    }
    if n_valid == 0 {
        Value::missing()
    } else {
        Value::Num(total / n_valid as f64)
    }
}

pub(super) fn fn_min(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let mut min_val: Option<f64> = None;
    for a in args {
        if let Some(f) = coerce_num(a, ctx) {
            min_val = Some(match min_val {
                None => f,
                Some(m) => if f < m { f } else { m },
            });
        }
    }
    match min_val {
        None => Value::missing(),
        Some(f) => Value::Num(f),
    }
}

pub(super) fn fn_max(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let mut max_val: Option<f64> = None;
    for a in args {
        if let Some(f) = coerce_num(a, ctx) {
            max_val = Some(match max_val {
                None => f,
                Some(m) => if f > m { f } else { m },
            });
        }
    }
    match max_val {
        None => Value::missing(),
        Some(f) => Value::Num(f),
    }
}

pub(super) fn fn_n(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    let count = args.iter().filter(|a| !a.is_missing()).count();
    Value::Num(count as f64)
}

pub(super) fn fn_nmiss(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    let count = args.iter().filter(|a| a.is_missing()).count();
    Value::Num(count as f64)
}

pub(super) fn fn_coalesce(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    for a in args {
        if !a.is_missing() {
            return a.clone();
        }
    }
    Value::missing()
}

pub(super) fn fn_missing(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if let Some(a) = args.first() {
        Value::Num(if a.is_missing() { 1.0 } else { 0.0 })
    } else {
        Value::missing()
    }
}

