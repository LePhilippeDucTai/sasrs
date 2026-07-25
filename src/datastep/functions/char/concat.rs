use super::*;

/// CAT: concatenate all args without modification.
pub(crate) fn fn_cat(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    let mut result = String::new();
    for a in args {
        result.push_str(&coerce_char(a));
    }
    Value::Char(result)
}

/// CATS: strip each arg, then concatenate.
pub(crate) fn fn_cats(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    let mut result = String::new();
    for a in args {
        let s = coerce_char(a);
        result.push_str(s.trim());
    }
    Value::Char(result)
}

/// CATX(sep, ...): strip each arg; skip blank args; join with separator.
pub(crate) fn fn_catx(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Char(String::new());
    }
    let sep = coerce_char(&args[0]);
    let parts: Vec<String> = args[1..]
        .iter()
        .map(|a| coerce_char(a).trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    Value::Char(parts.join(&sep))
}

/// CATQ(delim, item1, item2, ...): concatenate items with delimiter, quoting
/// items that contain delimiter or quotes. Escape internal quotes with double quotes.
pub(crate) fn fn_catq(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Char(String::new());
    }
    let delim = coerce_char(&args[0]);
    let mut result = Vec::new();

    for item in &args[1..] {
        let s = coerce_char(item);
        let needs_quoting = s.contains(&delim) || s.contains('"');
        let quoted = if needs_quoting {
            let escaped = s.replace('"', "\"\"");
            format!("\"{}\"", escaped)
        } else {
            s
        };
        result.push(quoted);
    }

    Value::Char(result.join(&delim))
}
