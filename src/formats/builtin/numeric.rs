use super::*;

/// "BEST" arm of `format_builtin`.
pub(super) fn fmt_best(fval: f64, spec: &FormatSpec) -> Option<String> {
    let w = spec.w.unwrap_or(12) as usize;
    let s = format_best(fval, w);
    Some(right_justify(&s, w))
}

/// "COMMA" arm of `format_builtin`.
pub(super) fn fmt_comma(fval: f64, spec: &FormatSpec) -> Option<String> {
    let d = spec.d.unwrap_or(0) as usize;
    let s = format_decimal(fval, d);
    let with_commas = add_commas(&s);
    match spec.w {
        None => Some(with_commas),
        Some(w) => {
            let w = w as usize;
            Some(fit_or_stars(&with_commas, w))
        }
    }
}

/// "DOLLAR" arm of `format_builtin`.
pub(super) fn fmt_dollar(fval: f64, spec: &FormatSpec) -> Option<String> {
    let d = spec.d.unwrap_or(0) as usize;
    // Dollar sign goes before sign for negatives in SAS: -$1,234 → handle below.
    let negative = fval < 0.0;
    let abs_val = fval.abs();
    let s = format_decimal(abs_val, d);
    let with_commas = add_commas(&s);
    let formatted = if negative {
        format!("-${}", with_commas)
    } else {
        format!("${}", with_commas)
    };
    match spec.w {
        None => Some(formatted),
        Some(w) => {
            let w = w as usize;
            Some(fit_or_stars(&formatted, w))
        }
    }
}

/// "Z" arm of `format_builtin`.
pub(super) fn fmt_z(fval: f64, spec: &FormatSpec) -> Option<String> {
    let d = spec.d.unwrap_or(0) as usize;
    let negative = fval < 0.0;
    let abs_val = fval.abs();
    let s = format_decimal(abs_val, d);
    match spec.w {
        None => Some(s),
        Some(w) => {
            let w = w as usize;
            // Sign takes 1 char if negative.
            let pad_target = if negative { w.saturating_sub(1) } else { w };
            let padded = format!("{:0>width$}", s, width = pad_target);
            let full = if negative {
                format!("-{}", padded)
            } else {
                padded
            };
            Some(fit_or_stars(&full, w))
        }
    }
}

/// "PERCENT" arm of `format_builtin`.
pub(super) fn fmt_percent(fval: f64, spec: &FormatSpec) -> Option<String> {
    let d = spec.d.unwrap_or(0) as usize;
    let pct = fval * 100.0;
    let s = format!("{:.prec$}%", pct, prec = d);
    match spec.w {
        None => Some(s),
        Some(w) => {
            let w = w as usize;
            Some(fit_or_stars(&s, w))
        }
    }
}

/// "E" arm of `format_builtin`.
pub(super) fn fmt_e(fval: f64, spec: &FormatSpec) -> Option<String> {
    let w = spec.w.unwrap_or(12) as usize;
    let s = format!("{:E}", fval);
    if s.len() <= w {
        Some(right_justify(&s, w))
    } else {
        // Try with fewer decimal digits.
        let s2 = format!("{:.2E}", fval);
        Some(fit_or_stars(&s2, w))
    }
}

/// "COMMAX" arm of `format_builtin`.
pub(super) fn fmt_commax(fval: f64, spec: &FormatSpec) -> Option<String> {
    let d = spec.d.unwrap_or(0) as usize;
    let negative = fval < 0.0;
    let abs_val = fval.abs();
    // Format with d decimals
    let s = format!("{:.prec$}", abs_val, prec = d);
    // Split on '.' (Rust decimal point)
    let (int_part, dec_part) = match s.find('.') {
        Some(p) => (&s[..p], &s[p + 1..]),
        None => (s.as_str(), ""),
    };
    // Add periods as thousands separators
    let rev: String = int_part
        .chars()
        .rev()
        .enumerate()
        .flat_map(|(i, c)| {
            if i > 0 && i % 3 == 0 {
                vec!['.', c]
            } else {
                vec![c]
            }
        })
        .collect();
    let int_with_sep: String = rev.chars().rev().collect();
    let formatted = if d > 0 {
        if negative {
            format!("-{},{}", int_with_sep, dec_part)
        } else {
            format!("{},{}", int_with_sep, dec_part)
        }
    } else {
        if negative {
            format!("-{}", int_with_sep)
        } else {
            int_with_sep
        }
    };
    match spec.w {
        None => Some(formatted),
        Some(w) => {
            let w = w as usize;
            Some(fit_or_stars(&formatted, w))
        }
    }
}

/// "DOLLARX" arm of `format_builtin`.
pub(super) fn fmt_dollarx(fval: f64, spec: &FormatSpec) -> Option<String> {
    let d = spec.d.unwrap_or(0) as usize;
    let negative = fval < 0.0;
    let abs_val = fval.abs();
    let s = format!("{:.prec$}", abs_val, prec = d);
    let (int_part, dec_part) = match s.find('.') {
        Some(p) => (&s[..p], &s[p + 1..]),
        None => (s.as_str(), ""),
    };
    // Thousands with periods
    let rev: String = int_part
        .chars()
        .rev()
        .enumerate()
        .flat_map(|(i, c)| {
            if i > 0 && i % 3 == 0 {
                vec!['.', c]
            } else {
                vec![c]
            }
        })
        .collect();
    let int_with_sep: String = rev.chars().rev().collect();
    let formatted = if d > 0 {
        if negative {
            format!("-${},{}", int_with_sep, dec_part)
        } else {
            format!("${},{}", int_with_sep, dec_part)
        }
    } else {
        if negative {
            format!("-${}", int_with_sep)
        } else {
            format!("${}", int_with_sep)
        }
    };
    match spec.w {
        None => Some(formatted),
        Some(w) => {
            let w = w as usize;
            Some(fit_or_stars(&formatted, w))
        }
    }
}

/// "EURO" | "EUROX" arm of `format_builtin`.
pub(super) fn fmt_euro(fval: f64, spec: &FormatSpec) -> Option<String> {
    let d = spec.d.unwrap_or(0) as usize;
    let negative = fval < 0.0;
    let abs_val = fval.abs();
    let s = format!("{:.prec$}", abs_val, prec = d);
    let (int_part, dec_part) = match s.find('.') {
        Some(p) => (&s[..p], &s[p + 1..]),
        None => (s.as_str(), ""),
    };
    let rev: String = int_part
        .chars()
        .rev()
        .enumerate()
        .flat_map(|(i, c)| {
            if i > 0 && i % 3 == 0 {
                vec!['.', c]
            } else {
                vec![c]
            }
        })
        .collect();
    let int_with_sep: String = rev.chars().rev().collect();
    let formatted = if d > 0 {
        if negative {
            format!("-€{},{}", int_with_sep, dec_part)
        } else {
            format!("€{},{}", int_with_sep, dec_part)
        }
    } else {
        if negative {
            format!("-€{}", int_with_sep)
        } else {
            format!("€{}", int_with_sep)
        }
    };
    match spec.w {
        None => Some(formatted),
        Some(w) => {
            let w = w as usize;
            Some(fit_or_stars(&formatted, w))
        }
    }
}

/// "NEGPAREN" arm of `format_builtin`.
pub(super) fn fmt_negparen(fval: f64, spec: &FormatSpec) -> Option<String> {
    let d = spec.d.unwrap_or(0) as usize;
    let formatted = if fval < 0.0 {
        let abs_val = fval.abs();
        let s = format!("{:.prec$}", abs_val, prec = d);
        let with_commas = add_commas(&s);
        format!("({})", with_commas)
    } else {
        let s = format!("{:.prec$}", fval, prec = d);
        add_commas(&s)
    };
    match spec.w {
        None => Some(formatted),
        Some(w) => {
            let w = w as usize;
            Some(fit_or_stars(&formatted, w))
        }
    }
}

/// "HEX" arm of `format_builtin`.
pub(super) fn fmt_hex(fval: f64, spec: &FormatSpec) -> Option<String> {
    let n = fval.round() as i64;
    let s = if n < 0 {
        // SAS HEX format renders negative as two's complement in 16 hex digits
        format!("{:016X}", n as u64)
    } else {
        format!("{:X}", n)
    };
    match spec.w {
        None => Some(s),
        Some(w) => {
            let w = w as usize;
            if s.len() <= w {
                Some(right_justify(&s, w))
            } else {
                Some(s[s.len() - w..].to_string()) // keep rightmost
            }
        }
    }
}

/// "BINARY" arm of `format_builtin`.
pub(super) fn fmt_binary(fval: f64, spec: &FormatSpec) -> Option<String> {
    let n = fval.round() as i64;
    let s = if n < 0 {
        format!("{:064b}", n as u64)
    } else {
        format!("{:b}", n)
    };
    match spec.w {
        None => Some(s),
        Some(w) => {
            let w = w as usize;
            if s.len() <= w {
                Some(right_justify(&s, w))
            } else {
                Some(s[s.len() - w..].to_string())
            }
        }
    }
}

/// "OCTAL" arm of `format_builtin`.
pub(super) fn fmt_octal(fval: f64, spec: &FormatSpec) -> Option<String> {
    let n = fval.round() as u64;
    let s = format!("{:o}", n);
    match spec.w {
        None => Some(s),
        Some(w) => {
            let w = w as usize;
            Some(fit_or_stars(&s, w))
        }
    }
}

/// "ROMAN" arm of `format_builtin`.
pub(super) fn fmt_roman(fval: f64, spec: &FormatSpec) -> Option<String> {
    let n = fval.round() as u32;
    let s = to_roman(n);
    if s.is_empty() {
        // out of range → use numeric fallback
        let fallback = format!("{}", fval.round() as i64);
        return Some(match spec.w {
            None => fallback,
            Some(w) => right_justify(&fallback, w as usize),
        });
    }
    match spec.w {
        None => Some(s),
        Some(w) => Some(right_justify(&s, w as usize)),
    }
}

/// "WORDS" arm of `format_builtin`.
pub(super) fn fmt_words(fval: f64, spec: &FormatSpec) -> Option<String> {
    let n = fval.round() as i64;
    let s = to_words(n);
    match spec.w {
        None => Some(s),
        Some(w) => {
            let w = w as usize;
            let mut out = s;
            if out.len() > w {
                out.truncate(w);
            } else {
                while out.len() < w {
                    out.push(' ');
                }
            }
            Some(out)
        }
    }
}

/// "FRACT" arm of `format_builtin`.
pub(super) fn fmt_fract(fval: f64, spec: &FormatSpec) -> Option<String> {
    let s = to_fract(fval);
    match spec.w {
        None => Some(s),
        Some(w) => Some(right_justify(&s, w as usize)),
    }
}

/// "SCIENTIFIC" arm of `format_builtin`.
pub(super) fn fmt_scientific(fval: f64, spec: &FormatSpec) -> Option<String> {
    let d = spec.d.unwrap_or(2) as usize;
    let w = spec.w.unwrap_or(12) as usize;
    // Format as xExx style: coefficient with d decimals, exponent with sign and 2 digits
    let s = if fval == 0.0 {
        format!("{:.prec$}E+00", 0.0, prec = d)
    } else {
        let exp = fval.abs().log10().floor() as i32;
        let coeff = fval / 10f64.powi(exp);
        if exp >= 0 {
            format!("{:.prec$}E+{:02}", coeff, exp, prec = d)
        } else {
            format!("{:.prec$}E-{:02}", coeff, -exp, prec = d)
        }
    };
    Some(fit_or_stars(&s, w))
}
