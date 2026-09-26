use super::*;

/// COMPRESS(s[, chars]): remove specified chars from s; default removes spaces.
pub(crate) fn fn_compress(args: &[Value], _ctx: &mut EvalCtx) -> Value {
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
pub(crate) fn fn_tranwrd(args: &[Value], _ctx: &mut EvalCtx) -> Value {
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

/// TRANSLATE(s, to, from): replace each char in from with corresponding char in to.
/// If to is shorter than from, chars in from beyond len(to) are removed.
pub(crate) fn fn_translate(args: &[Value], _ctx: &mut EvalCtx) -> Value {
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

    let result: String = s
        .chars()
        .map(|c| {
            match from_chars.iter().position(|&fc| fc == c) {
                Some(pos) => {
                    if pos < to_chars.len() {
                        to_chars[pos]
                    } else {
                        // char in from beyond len(to) → remove it
                        '\0' // placeholder, will be filtered
                    }
                }
                None => c,
            }
        })
        .filter(|&c| c != '\0')
        .collect();

    Value::Char(result)
}

/// REVERSE(s): reverse the string s.
pub(crate) fn fn_reverse(args: &[Value], _ctx: &mut EvalCtx) -> Value {
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
pub(crate) fn fn_repeat(args: &[Value], ctx: &mut EvalCtx) -> Value {
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
                // MQ9.1 — `s.repeat(n)` alloue `s.len() * n` octets : sans
                // borne, `repeat('a', 1e12)` demande 1 To et tue le process
                // (capacity overflow / OOM), au lieu de produire une valeur.
                // J03-P6 : le plafond est compté en CARACTÈRES (convention de
                // la crate depuis les jalons encodage/largeurs — J03-P3/P4),
                // pas en octets : 32 767 caractères de « é » (2 octets)
                // restent 32 767 caractères, sur une frontière de caractère.
                const MAX_CHAR_LEN: usize = 32_767;
                let s_chars = s.chars().count();
                let max_copies = (MAX_CHAR_LEN / s_chars.max(1)).max(1);
                Value::Char(s.repeat((n as usize).min(max_copies)))
            }
        }
    }
}

/// PROPCASE(s[, delim]): proper case — capitalize first letter of each word
/// (words separated by delim, default ' ').
pub(crate) fn fn_propcase(args: &[Value], _ctx: &mut EvalCtx) -> Value {
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

/// COMPBL(s): replace each occurrence of TWO OR MORE consecutive blanks by a
/// single blank (J03-P6, doc SAS : « 文字列中に 2 つ以上の空白が連続して出現する
/// たびに、COMPBL 関数はそれらを 1 つの空白に変換して、複数の空白を削除します » —
/// un blanc ISOLÉ n'est pas affecté, les blancs restants inclus). Un « blank »
/// est l'espace (0x20) : les tabulations et autres blancs Unicode ne sont PAS
/// concernés (fonction de niveau I18N 0).
pub(crate) fn fn_compbl(args: &[Value], _ctx: &mut EvalCtx) -> Value {
    if args.is_empty() {
        return Value::Char(String::new());
    }
    let s = coerce_char(&args[0]);
    let mut result = String::with_capacity(s.len());
    let mut in_blanks = false;
    for c in s.chars() {
        if c == ' ' {
            if !in_blanks {
                result.push(' ');
                in_blanks = true;
            }
            // Un 2ᵉ blanc consécutif (et les suivants) est supprimé.
        } else {
            result.push(c);
            in_blanks = false;
        }
    }
    Value::Char(result)
}

/// SUBSTRN(s, pos[, len]): like SUBSTR but out-of-bounds pos returns ""
/// WITHOUT setting _ERROR_.
pub(crate) fn fn_substrn(args: &[Value], ctx: &mut EvalCtx) -> Value {
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
pub(crate) fn fn_char(args: &[Value], ctx: &mut EvalCtx) -> Value {
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
        },
    }
}

/// BYTE(n): alias for CHAR(n).
pub(crate) fn fn_byte(args: &[Value], ctx: &mut EvalCtx) -> Value {
    fn_char(args, ctx)
}
