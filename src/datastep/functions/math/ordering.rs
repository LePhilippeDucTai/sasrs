use super::*;

/// RANGE(x1, x2, ...): max(args) - min(args); missing ignored.
pub(crate) fn fn_range(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let mut min_val: Option<f64> = None;
    let mut max_val: Option<f64> = None;
    for a in args {
        if let Some(f) = coerce_num(a, ctx) {
            min_val = Some(match min_val {
                None => f,
                Some(m) => {
                    if f < m {
                        f
                    } else {
                        m
                    }
                }
            });
            max_val = Some(match max_val {
                None => f,
                Some(m) => {
                    if f > m {
                        f
                    } else {
                        m
                    }
                }
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
pub(crate) fn fn_largest(args: &[Value], ctx: &mut EvalCtx) -> Value {
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
pub(crate) fn fn_smallest(args: &[Value], ctx: &mut EvalCtx) -> Value {
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
pub(crate) fn fn_ordinal(args: &[Value], ctx: &mut EvalCtx) -> Value {
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
