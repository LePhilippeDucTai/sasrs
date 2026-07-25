use super::*;

/// Default SAS SCAN delimiters.
pub(crate) const SAS_SCAN_DELIMS: &str = " .<>()+&!$*);^-/,%|";

pub(crate) fn fn_upcase(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    match args.first() {
        None => Value::Char(String::new()),
        Some(v) => Value::Char(coerce_char(v).to_uppercase()),
    }
}

/// INDEX(s, sub) → 1-based position, 0 if not found.
pub(crate) fn fn_index(args: &[Value], _ctx: &mut EvalCtx) -> Value {
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

/// SCAN(s, n[, delims]): return nth word; n<0 means from end.
/// Default delimiters: " .<>()+&!$*);^-/,%|"
pub(crate) fn fn_scan(args: &[Value], ctx: &mut EvalCtx) -> Value {
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
pub(crate) fn fn_find(args: &[Value], ctx: &mut EvalCtx) -> Value {
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
pub(crate) fn fn_findc(args: &[Value], ctx: &mut EvalCtx) -> Value {
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
pub(crate) fn fn_count(args: &[Value], _ctx: &mut EvalCtx) -> Value {
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
pub(crate) fn fn_countc(args: &[Value], _ctx: &mut EvalCtx) -> Value {
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
pub(crate) fn fn_verify(args: &[Value], _ctx: &mut EvalCtx) -> Value {
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

/// RANK(s): return Unicode code point of first character of s.
pub(crate) fn fn_rank(args: &[Value], _ctx: &mut EvalCtx) -> Value {
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

/// WHICHC(needle, haystack1[, haystack2, ...]): return 1-based position of
/// first argument (after needle) that equals needle. Return 0 if none found.
pub(crate) fn fn_whichc(args: &[Value], _ctx: &mut EvalCtx) -> Value {
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
