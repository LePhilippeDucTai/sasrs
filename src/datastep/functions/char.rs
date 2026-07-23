// ──────────────────────────────────────────────────────────────────────────────
// Character functions
// ──────────────────────────────────────────────────────────────────────────────

use super::*;

/// Default SAS SCAN delimiters.
pub(super) const SAS_SCAN_DELIMS: &str = " .<>()+&!$*);^-/,%|";

pub(super) fn fn_upcase(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => Value::Char(coerce_char(v).to_uppercase()),
    }
}

pub(super) fn fn_lowcase(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => Value::Char(coerce_char(v).to_lowercase()),
    }
}

/// TRIM: remove trailing blanks. A fully-blank string becomes "".
pub(super) fn fn_trim(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => {
            let s = coerce_char(v);
            Value::Char(s.trim_end_matches(' ').to_string())
        }
    }
}

/// STRIP: remove both leading and trailing blanks.
pub(super) fn fn_strip(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => {
            let s = coerce_char(v);
            Value::Char(s.trim().to_string())
        }
    }
}

/// LEFT: remove leading blanks (trim_start).
pub(super) fn fn_left(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => {
            let s = coerce_char(v);
            Value::Char(s.trim_start_matches(' ').to_string())
        }
    }
}

/// LENGTH: length without trailing blanks; minimum 1 even for blank string.
pub(super) fn fn_length(args: &[Value], _ctx: &mut EvalCtx) -> Value {
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
pub(super) fn fn_substr(args: &[Value], ctx: &mut EvalCtx) -> Value {
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

/// INDEX(s, sub) → 1-based position, 0 if not found.
pub(super) fn fn_index(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Num(0.0);
    }
    let s = coerce_char(&args[0]);
    let sub = coerce_char(&args[1]);
    if sub.is_empty() {
        return Value::Num(0.0);
    }
    match s.find(&sub as &str) {
        None => Value::Num(0.0),
        // byte offset, but for ASCII this equals char offset.
        // For proper Unicode, count chars.
        Some(byte_pos) => {
            let char_pos = s[..byte_pos].chars().count() + 1;
            Value::Num(char_pos as f64)
        }
    }
}

/// CAT: concatenate all args without modification.
pub(super) fn fn_cat(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    let mut result = String::new();
    for a in args {
        result.push_str(&coerce_char(a));
    }
    Value::Char(result)
}

/// CATS: strip each arg, then concatenate.
pub(super) fn fn_cats(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    let mut result = String::new();
    for a in args {
        let s = coerce_char(a);
        result.push_str(s.trim());
    }
    Value::Char(result)
}

/// CATX(sep, ...): strip each arg; skip blank args; join with separator.
pub(super) fn fn_catx(args: &[Value], _ctx: &mut EvalCtx) -> Value {
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

/// COMPRESS(s[, chars]): remove specified chars from s; default removes spaces.
pub(super) fn fn_compress(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let remove_chars: String = if args.len() >= 2 {
        coerce_char(&args[1])
    } else {
        " ".to_string()
    };
    let result: String = s.chars().filter(|c| !remove_chars.contains(*c)).collect();
    Value::Char(result)
}

/// TRANWRD(s, from, to): replace all occurrences of `from` with `to`.
pub(super) fn fn_tranwrd(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.len() < 3 {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let from = coerce_char(&args[1]);
    let to = coerce_char(&args[2]);
    if from.is_empty() {
        return Value::Char(s);
    }
    Value::Char(s.replace(&from as &str, &to as &str))
}

/// SCAN(s, n[, delims]): return nth word; n<0 means from end.
/// Default delimiters: " .<>()+&!$*);^-/,%|"
pub(super) fn fn_scan(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let n = match coerce_num(&args[1], ctx) {
        None => return Value::Char(String::new()),
        Some(f) => f as i64,
    };
    if n == 0 {
        return Value::Char(String::new());
    }
    let delims: String = if args.len() >= 3 {
        coerce_char(&args[2])
    } else {
        SAS_SCAN_DELIMS.to_string()
    };

    // Split s into words (tokens between delimiter chars).
    let words: Vec<&str> = s
        .split(|c: char| delims.contains(c))
        .filter(|w| !w.is_empty())
        .collect();

    let idx = if n > 0 {
        n as usize - 1
    } else {
        // n < 0: count from end
        let abs_n = (-n) as usize;
        if abs_n > words.len() {
            return Value::Char(String::new());
        }
        words.len() - abs_n
    };

    match words.get(idx) {
        None => Value::Char(String::new()),
        Some(w) => Value::Char(w.to_string()),
    }
}

/// FIND(s, target[, startPos[, modifiers]]): return 1-based position of first
/// occurrence of target in s, starting at startPos. If not found, return 0.
/// Modifiers: 'i' for case-insensitive.
pub(super) fn fn_find(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Num(0.0);
    }
    let s = coerce_char(&args[0]);
    let target = coerce_char(&args[1]);

    if target.is_empty() {
        return Value::Num(0.0);
    }

    let start_pos = if args.len() >= 3 {
        match coerce_num(&args[2], ctx) {
            None => return Value::Num(0.0),
            Some(f) => (f as i64).max(1),
        }
    } else {
        1
    };

    let case_insensitive = if args.len() >= 4 {
        let modifiers = coerce_char(&args[3]);
        modifiers.to_lowercase().contains('i')
    } else {
        false
    };

    let chars: Vec<char> = s.chars().collect();
    if start_pos < 1 || start_pos as usize > chars.len() {
        return Value::Num(0.0);
    }

    let search_from_char_idx = start_pos as usize;  // startPos is exclusive (1-based), skip to next char

    let target_search = if case_insensitive {
        target.to_lowercase()
    } else {
        target.clone()
    };

    // Search in the substring starting after startPos
    let search_text = chars[search_from_char_idx..].iter().collect::<String>();
    if case_insensitive && search_text.is_empty() {
        return Value::Num(0.0);
    }

    match if case_insensitive {
        search_text.to_lowercase().find(&target_search)
    } else {
        search_text.find(&target_search)
    } {
        None => Value::Num(0.0),
        Some(byte_pos) => {
            let found_char_idx = search_text[..byte_pos].chars().count();
            let char_pos = search_from_char_idx + found_char_idx + 1;
            Value::Num(char_pos as f64)
        }
    }
}

/// FINDC(s, target[, startPos[, modifiers]]): like FIND but target is a set of
/// characters; find first char from target in s.
pub(super) fn fn_findc(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Num(0.0);
    }
    let s = coerce_char(&args[0]);
    let target = coerce_char(&args[1]);

    if target.is_empty() {
        return Value::Num(0.0);
    }

    let start_pos = if args.len() >= 3 {
        match coerce_num(&args[2], ctx) {
            None => return Value::Num(0.0),
            Some(f) => (f as i64).max(1),
        }
    } else {
        1
    };

    let case_insensitive = if args.len() >= 4 {
        let modifiers = coerce_char(&args[3]);
        modifiers.to_lowercase().contains('i')
    } else {
        false
    };

    let chars: Vec<char> = s.chars().collect();
    if start_pos < 1 || start_pos as usize > chars.len() {
        return Value::Num(0.0);
    }

    let target_chars: Vec<char> = if case_insensitive {
        target.to_lowercase().chars().collect()
    } else {
        target.chars().collect()
    };

    for (i, &c) in chars.iter().enumerate().skip((start_pos - 1) as usize) {
        let test_c = if case_insensitive { c.to_lowercase().to_string() } else { c.to_string() };
        if target_chars.contains(&test_c.chars().next().unwrap_or('?')) {
            return Value::Num((i + 1) as f64);
        }
    }
    Value::Num(0.0)
}

/// COUNT(s, target[, modifiers]): count occurrences of target substring in s.
/// Modifiers: 'i' for case-insensitive.
pub(super) fn fn_count(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Num(0.0);
    }
    let s = coerce_char(&args[0]);
    let target = coerce_char(&args[1]);

    if target.is_empty() {
        return Value::Num(0.0);
    }

    let case_insensitive = if args.len() >= 3 {
        let modifiers = coerce_char(&args[2]);
        modifiers.to_lowercase().contains('i')
    } else {
        false
    };

    let search_str = if case_insensitive { s.to_lowercase() } else { s.clone() };
    let target_str = if case_insensitive { target.to_lowercase() } else { target.clone() };

    let mut count = 0;
    let mut start = 0;
    while let Some(pos) = search_str[start..].find(&target_str as &str) {
        count += 1;
        start += pos + target_str.len();
    }
    Value::Num(count as f64)
}

/// COUNTC(s, target[, modifiers]): count occurrences of any character from
/// target set in s.
pub(super) fn fn_countc(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Num(0.0);
    }
    let s = coerce_char(&args[0]);
    let target = coerce_char(&args[1]);

    if target.is_empty() {
        return Value::Num(0.0);
    }

    let case_insensitive = if args.len() >= 3 {
        let modifiers = coerce_char(&args[2]);
        modifiers.to_lowercase().contains('i')
    } else {
        false
    };

    let target_chars: Vec<char> = if case_insensitive {
        target.to_lowercase().chars().collect()
    } else {
        target.chars().collect()
    };

    let count = s.chars().filter(|c| {
        let test_c = if case_insensitive {
            c.to_lowercase().next().unwrap_or('?')
        } else {
            *c
        };
        target_chars.contains(&test_c)
    }).count();

    Value::Num(count as f64)
}

/// VERIFY(s, target[, modifiers]): return 1-based position of first character
/// in s NOT in target set. Return 0 if all chars in s are in target.
pub(super) fn fn_verify(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Num(0.0);
    }
    let s = coerce_char(&args[0]);
    let target = coerce_char(&args[1]);

    if target.is_empty() {
        return if s.is_empty() { Value::Num(0.0) } else { Value::Num(1.0) };
    }

    let case_insensitive = if args.len() >= 3 {
        let modifiers = coerce_char(&args[2]);
        modifiers.to_lowercase().contains('i')
    } else {
        false
    };

    let target_chars: Vec<char> = if case_insensitive {
        target.to_lowercase().chars().collect()
    } else {
        target.chars().collect()
    };

    for (i, c) in s.chars().enumerate() {
        let test_c = if case_insensitive {
            c.to_lowercase().next().unwrap_or('?')
        } else {
            c
        };
        if !target_chars.contains(&test_c) {
            return Value::Num((i + 1) as f64);
        }
    }
    Value::Num(0.0)
}

/// TRANSLATE(s, to, from): replace each char in from with corresponding char in to.
/// If to is shorter than from, chars in from beyond len(to) are removed.
pub(super) fn fn_translate(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.len() < 3 {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let to = coerce_char(&args[1]);
    let from = coerce_char(&args[2]);

    if from.is_empty() {
        return Value::Char(s);
    }

    let to_chars: Vec<char> = to.chars().collect();
    let from_chars: Vec<char> = from.chars().collect();

    let result: String = s.chars().map(|c| {
        match from_chars.iter().position(|&fc| fc == c) {
            Some(pos) => {
                if pos < to_chars.len() {
                    to_chars[pos]
                } else {
                    // char in from beyond len(to) → remove it
                    return '\0';  // placeholder, will be filtered
                }
            }
            None => c,
        }
    }).filter(|&c| c != '\0').collect();

    Value::Char(result)
}

/// REVERSE(s): reverse the string s.
pub(super) fn fn_reverse(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => {
            let s = coerce_char(v);
            let reversed: String = s.chars().rev().collect();
            Value::Char(reversed)
        }
    }
}

/// REPEAT(s, n): repeat string s n times. n is numeric, truncated to integer.
/// n<0 → "".
pub(super) fn fn_repeat(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    match coerce_num(&args[1], ctx) {
        None => Value::Char(String::new()),
        Some(f) => {
            let n = f.trunc() as i64;
            if n < 0 {
                Value::Char(String::new())
            } else {
                let result = s.repeat(n as usize);
                Value::Char(result)
            }
        }
    }
}

/// PROPCASE(s[, delim]): proper case — capitalize first letter of each word
/// (words separated by delim, default ' ').
pub(super) fn fn_propcase(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let delim = if args.len() >= 2 {
        coerce_char(&args[1])
    } else {
        " ".to_string()
    };

    if delim.is_empty() {
        // No delimiter: treat entire string as one word
        if s.is_empty() {
            return Value::Char(String::new());
        }
        let mut chars = s.chars();
        let first = chars.next().unwrap().to_uppercase().to_string();
        let rest: String = chars.map(|c| c.to_lowercase().to_string()).collect();
        return Value::Char(format!("{}{}", first, rest));
    }

    // Split by delimiter and capitalize each word
    let delim_chars: Vec<char> = delim.chars().collect();
    let mut result = String::new();
    let mut capitalize_next = true;

    for c in s.chars() {
        if delim_chars.contains(&c) {
            result.push(c);
            capitalize_next = true;
        } else if capitalize_next {
            for ch in c.to_uppercase() {
                result.push(ch);
            }
            capitalize_next = false;
        } else {
            for ch in c.to_lowercase() {
                result.push(ch);
            }
        }
    }

    Value::Char(result)
}

/// COMPBL(s): compress multiple blanks to single, remove leading/trailing blanks.
pub(super) fn fn_compbl(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let trimmed = s.trim();
    let result: String = trimmed
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Value::Char(result)
}

/// SUBSTRN(s, pos[, len]): like SUBSTR but out-of-bounds pos returns ""
/// WITHOUT setting _ERROR_.
pub(super) fn fn_substrn(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let chars: Vec<char> = s.chars().collect();
    let slen = chars.len() as i64;

    let pos = match args.get(1) {
        None => return Value::Char(String::new()),
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::Char(String::new()),
            Some(f) => f as i64,
        },
    };

    // Out of bounds → "" WITHOUT setting _ERROR_
    if pos < 1 || pos > slen {
        return Value::Char(String::new());
    }

    let start = (pos - 1) as usize;
    let end = if let Some(len_v) = args.get(2) {
        match coerce_num(len_v, ctx) {
            None => return Value::Char(String::new()),
            Some(l) => {
                let l = l as i64;
                if l < 0 {
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

/// CHAR(n): return character with Unicode code point n (numeric input).
/// CHAR(0) returns empty string.
pub(super) fn fn_char(args: &[Value], ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => match coerce_num(v, ctx) {
            None => Value::Char(String::new()),
            Some(f) => {
                let code = f as u32;
                if code == 0 {
                    Value::Char(String::new())
                } else {
                    match std::char::from_u32(code) {
                        Some(c) => Value::Char(c.to_string()),
                        None => Value::Char(String::new()),
                    }
                }
            }
        }
    }
}

/// RANK(s): return Unicode code point of first character of s.
pub(super) fn fn_rank(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Num(0.0),
        Some(v) => {
            let s = coerce_char(v);
            match s.chars().next() {
                None => Value::Num(0.0),
                Some(c) => Value::Num(c as u32 as f64),
            }
        }
    }
}

/// BYTE(n): alias for CHAR(n).
pub(super) fn fn_byte(args: &[Value], ctx: &mut EvalCtx) -> Value {
    fn_char(args, ctx)
}

/// WHICHC(needle, haystack1[, haystack2, ...]): return 1-based position of
/// first argument (after needle) that equals needle. Return 0 if none found.
pub(super) fn fn_whichc(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Num(0.0);
    }
    let needle = coerce_char(&args[0]);
    for (i, haystack) in args[1..].iter().enumerate() {
        if coerce_char(haystack) == needle {
            return Value::Num((i + 1) as f64);
        }
    }
    Value::Num(0.0)
}

/// CATQ(delim, item1, item2, ...): concatenate items with delimiter, quoting
/// items that contain delimiter or quotes. Escape internal quotes with double quotes.
pub(super) fn fn_catq(args: &[Value], _ctx: &mut EvalCtx) -> Value {
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

