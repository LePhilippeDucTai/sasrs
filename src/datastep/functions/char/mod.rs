
mod search;
mod transform;
mod concat;

pub(crate) use search::*;
pub(crate) use transform::*;
pub(crate) use concat::*;

// ──────────────────────────────────────────────────────────────────────────────
// Character functions
// ──────────────────────────────────────────────────────────────────────────────

use super::*;

pub(crate) fn fn_lowcase(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => Value::Char(coerce_char(v).to_lowercase()),
    }
}

/// TRIM: remove trailing blanks. A fully-blank string becomes "".
pub(crate) fn fn_trim(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => {
            let s = coerce_char(v);
            Value::Char(s.trim_end_matches(' ').to_string())
        }
    }
}

/// STRIP: remove both leading and trailing blanks.
pub(crate) fn fn_strip(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => {
            let s = coerce_char(v);
            Value::Char(s.trim().to_string())
        }
    }
}

/// LEFT: remove leading blanks (trim_start).
pub(crate) fn fn_left(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => {
            let s = coerce_char(v);
            Value::Char(s.trim_start_matches(' ').to_string())
        }
    }
}

/// LENGTH: length without trailing blanks; minimum 1 even for blank string.
pub(crate) fn fn_length(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Num(1.0),
        Some(v) => {
            let s = coerce_char(v);
            // En caractères, pas en octets — convention de la crate
            // (troncature PDV et longueurs de dataset.rs comptent pareil).
            let trimmed_len = s.trim_end_matches(' ').chars().count();
            Value::Num(trimmed_len.max(1) as f64)
        }
    }
}

/// SUBSTR(s, pos[, len]) — 1-based; out of bounds → "" + _ERROR_.
pub(crate) fn fn_substr(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let chars: Vec<char> = s.chars().collect();
    let slen = chars.len() as i64;

    let pos = match args.get(1) {
        None => {
            ctx.error_flag = true;
            return Value::Char(String::new());
        }
        Some(v) => match coerce_num(v, ctx) {
            None => {
                ctx.error_flag = true;
                return Value::Char(String::new());
            }
            Some(f) => f as i64,
        },
    };

    // pos is 1-based; must be >= 1 and <= length.
    if pos < 1 || pos > slen {
        ctx.error_flag = true;
        ctx.invalid_data += 1;
        return Value::Char(String::new());
    }

    let start = (pos - 1) as usize;

    let end = if let Some(len_v) = args.get(2) {
        match coerce_num(len_v, ctx) {
            None => {
                ctx.error_flag = true;
                return Value::Char(String::new());
            }
            Some(l) => {
                let l = l as i64;
                if l < 0 {
                    ctx.error_flag = true;
                    ctx.invalid_data += 1;
                    return Value::Char(String::new());
                }
                (start + l as usize).min(chars.len())
            }
        }
    } else {
        chars.len()
    };

    let result: String = chars[start..end].iter().collect();
    Value::Char(result)
}
