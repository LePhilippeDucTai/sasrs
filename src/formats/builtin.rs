//! Formats/informats intégrés (jalon M4) — table-driven, idéal pour un
//! modèle économique, format par format avec tests unitaires exhaustifs.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! ## Formats numériques
//! - `w.d` : arrondi à d décimales, justifié droite sur w ; débordement →
//!   BESTw., puis `*` répétés si vraiment trop étroit (règle SAS).
//! - `BESTw.` : déjà approximé par `value::format_best` — déplacer ici la
//!   version finale (justifiée droite sur w pour PUT).
//! - `COMMAw.d` (séparateur milliers `,`), `DOLLARw.d` (`$` + virgules),
//!   `Zw.d` (zéros de tête), `Ew.` (scientifique), `PERCENTw.d`.
//! - Missings : `.` / `_` / `A`..`Z` justifiés droite.
//! ## Formats caractère
//! - `$w.` / `$CHARw.` : tronquer/padder à w (gauche).
//! ## Formats date/heure (valeur = jours ou secondes depuis 1960)
//! - `DATE9.` (01JAN2020), `DATE7.`, `DDMMYYw.`, `MMDDYYw.`, `YYMMDDw.`
//!   (séparateurs selon w : 8 sans, 10 avec), `MONYY7.`, `WORDDATEw.`,
//!   `DATETIMEw.d` (01JAN2020:12:34:56), `TIMEw.d` (hh:mm:ss).
//!   Conversion jours→date via chrono : 1960-01-01 + jours.
//! ## Informats
//! - `w.d` (parse f64 ; d implicite si pas de point décimal dans la
//!   source : 123 avec informat 5.2 → 1.23 — piège SAS célèbre),
//!   `COMMAw.d` (vire $ et ,), `DATE9.`/`MMDDYY10.`/`DDMMYY10.`/
//!   `YYMMDD10.` → jours depuis 1960, `TIMEw.` → secondes.
//!
//! `format_builtin`/`informat_builtin` renvoient None si le nom est
//! inconnu (le catalogue essaie alors les formats utilisateur).

#![allow(unused_variables, dead_code)]

use super::{right_justify, FormatSpec};
use crate::value::{format_best, Value};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Timelike};

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// SAS epoch: 1960-01-01
fn sas_epoch() -> NaiveDate {
    NaiveDate::from_ymd_opt(1960, 1, 1).unwrap()
}

/// Days since SAS epoch → NaiveDate (clamp to valid range).
fn days_to_date(days: f64) -> Option<NaiveDate> {
    let d = days.round() as i64;
    sas_epoch().checked_add_signed(Duration::days(d))
}

/// Seconds since SAS epoch → NaiveDateTime.
fn secs_to_datetime(secs: f64) -> Option<NaiveDateTime> {
    let epoch = sas_epoch().and_hms_opt(0, 0, 0)?;
    let whole = secs.trunc() as i64;
    epoch.checked_add_signed(Duration::seconds(whole))
}

/// Seconds-of-day → (hh, mm, ss).
fn secs_to_time(secs: f64) -> (u32, u32, u32) {
    let total = secs.abs().round() as u64;
    let hh = total / 3600;
    let mm = (total % 3600) / 60;
    let ss = total % 60;
    (hh as u32, mm as u32, ss as u32)
}

/// Short month names (uppercase), 1-indexed.
const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

/// Full month names.
const MONTHS_FULL: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Short day-of-week names (3-letter), Sunday=0.
const DOW_SHORT: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

/// Full day-of-week names, Sunday=0.
const DOW_FULL: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

// ─────────────────────────────────────────────────────────────────────────────
// New helpers for M18.1 formats
// ─────────────────────────────────────────────────────────────────────────────

/// Convert integer to Roman numerals (1–3999). Returns empty string for out-of-range.
fn to_roman(mut n: u32) -> String {
    if n == 0 || n > 3999 {
        return String::new();
    }
    const VALS: &[(u32, &str)] = &[
        (1000, "M"), (900, "CM"), (500, "D"), (400, "CD"),
        (100, "C"),  (90, "XC"), (50, "L"),  (40, "XL"),
        (10, "X"),   (9, "IX"),  (5, "V"),   (4, "IV"),
        (1, "I"),
    ];
    let mut result = String::new();
    for &(val, sym) in VALS {
        while n >= val {
            result.push_str(sym);
            n -= val;
        }
    }
    result
}

/// Convert integer to English words (simple, 0–999_999_999).
fn to_words(n: i64) -> String {
    if n == 0 {
        return "ZERO".into();
    }
    let (neg, n) = if n < 0 { (true, (-n) as u64) } else { (false, n as u64) };
    let s = to_words_unsigned(n);
    if neg {
        format!("NEGATIVE {}", s)
    } else {
        s
    }
}

fn to_words_unsigned(n: u64) -> String {
    const ONES: &[&str] = &[
        "", "ONE", "TWO", "THREE", "FOUR", "FIVE", "SIX", "SEVEN", "EIGHT", "NINE",
        "TEN", "ELEVEN", "TWELVE", "THIRTEEN", "FOURTEEN", "FIFTEEN", "SIXTEEN",
        "SEVENTEEN", "EIGHTEEN", "NINETEEN",
    ];
    const TENS: &[&str] = &[
        "", "", "TWENTY", "THIRTY", "FORTY", "FIFTY", "SIXTY", "SEVENTY", "EIGHTY", "NINETY",
    ];

    if n == 0 {
        return String::new();
    }

    if n < 20 {
        return ONES[n as usize].into();
    }
    if n < 100 {
        let t = TENS[(n / 10) as usize];
        let o = ONES[(n % 10) as usize];
        if o.is_empty() {
            return t.into();
        }
        return format!("{}-{}", t, o);
    }
    if n < 1000 {
        let h = ONES[(n / 100) as usize];
        let rest = n % 100;
        if rest == 0 {
            return format!("{} HUNDRED", h);
        }
        return format!("{} HUNDRED {}", h, to_words_unsigned(rest));
    }
    if n < 1_000_000 {
        let thousands = n / 1000;
        let rest = n % 1000;
        let t = to_words_unsigned(thousands);
        if rest == 0 {
            return format!("{} THOUSAND", t);
        }
        return format!("{} THOUSAND {}", t, to_words_unsigned(rest));
    }
    if n < 1_000_000_000 {
        let millions = n / 1_000_000;
        let rest = n % 1_000_000;
        let m = to_words_unsigned(millions);
        if rest == 0 {
            return format!("{} MILLION", m);
        }
        return format!("{} MILLION {}", m, to_words_unsigned(rest));
    }
    // Fallback for very large numbers: just show digits
    format!("{}", n)
}

/// Reduce a fraction to lowest terms.
fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Format a float as a simple fraction (limited denominator ≤ 64 for v1).
fn to_fract(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let negative = v < 0.0;
    let abs = v.abs();
    let whole = abs.floor() as u64;
    let frac = abs - whole as f64;

    if frac < 1e-9 {
        // Integer value
        let s = if whole == 0 { "0".into() } else { format!("{}", whole) };
        return if negative { format!("-{}", s) } else { s };
    }

    // Find best fraction with denominator ≤ 64
    let mut best_num = 1u64;
    let mut best_den = 1u64;
    let mut best_err = f64::MAX;
    for den in 1u64..=64 {
        let num = (frac * den as f64).round() as u64;
        let err = (frac - num as f64 / den as f64).abs();
        if err < best_err {
            best_err = err;
            best_num = num;
            best_den = den;
        }
    }

    // Reduce
    let g = gcd(best_num, best_den);
    best_num /= g;
    best_den /= g;

    let frac_str = format!("{}/{}", best_num, best_den);
    let s = if whole == 0 {
        frac_str
    } else {
        format!("{} {}", whole, frac_str)
    };
    if negative { format!("-{}", s) } else { s }
}

// ─────────────────────────────────────────────────────────────────────────────
// Numeric format helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Add thousands separators to the integer part of a formatted number.
/// `s` is expected to NOT already have commas.
fn add_commas(s: &str) -> String {
    // Handle negative sign
    let (sign, digits_and_dec) = if s.starts_with('-') {
        ("-", &s[1..])
    } else {
        ("", s)
    };
    // Split on decimal
    let (int_part, dec_part) = match digits_and_dec.find('.') {
        Some(p) => (&digits_and_dec[..p], &digits_and_dec[p..]),
        None => (digits_and_dec, ""),
    };
    // Insert commas every 3 digits from the right
    let rev: String = int_part
        .chars()
        .rev()
        .enumerate()
        .flat_map(|(i, c)| {
            if i > 0 && i % 3 == 0 {
                vec![',', c]
            } else {
                vec![c]
            }
        })
        .collect();
    let int_with_commas: String = rev.chars().rev().collect();
    format!("{}{}{}", sign, int_with_commas, dec_part)
}

/// Format a float with exactly `d` decimal places.
fn format_decimal(v: f64, d: usize) -> String {
    format!("{:.prec$}", v, prec = d)
}

/// w.d format: round to d decimals, right-justify to w.
/// On overflow: try BESTw., then fill with '*'.
fn format_wd(v: f64, w: Option<u16>, d: Option<u16>) -> String {
    let d = d.unwrap_or(0) as usize;
    let s = format_decimal(v, d);
    match w {
        None => s,
        Some(ww) => {
            let ww = ww as usize;
            if s.len() <= ww {
                right_justify(&s, ww)
            } else {
                // Try BEST fallback.
                let best = format_best(v, ww);
                if best.len() <= ww {
                    right_justify(&best, ww)
                } else {
                    // Overflow: fill with '*'.
                    "*".repeat(ww)
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// format_builtin
// ─────────────────────────────────────────────────────────────────────────────

/// Apply a builtin SAS format. Returns None if the format name is unknown.
///
/// NOTE: `Value::Missing` is handled by the catalog BEFORE this function is
/// called, so we never see it here for numeric formats. Character formats
/// still check for `Value::Char`.
pub fn format_builtin(v: &Value, spec: &FormatSpec) -> Option<String> {
    let name = spec.name.to_uppercase();

    // ── Character formats ────────────────────────────────────────────────────
    match name.as_str() {
        "$" | "$CHAR" | "$F" => {
            let s = match v {
                Value::Char(c) => c.clone(),
                Value::Num(n) => format_best(*n, 12),
                Value::Missing(k) => k.display(),
            };
            return Some(match spec.w {
                None => s,
                Some(w) => {
                    let w = w as usize;
                    let mut out = s;
                    out.truncate(w);
                    while out.len() < w {
                        out.push(' ');
                    }
                    out
                }
            });
        }

        // $QUOTE: wrap value in double quotes  "hello"
        "$QUOTE" => {
            let inner = match v {
                Value::Char(c) => c.clone(),
                Value::Num(n) => format_best(*n, 12),
                Value::Missing(k) => k.display(),
            };
            let quoted = format!("\"{}\"", inner);
            return Some(match spec.w {
                None => quoted,
                Some(w) => {
                    let w = w as usize;
                    let mut out = quoted;
                    out.truncate(w);
                    while out.len() < w {
                        out.push(' ');
                    }
                    out
                }
            });
        }

        // $HEX: each byte of the string → two uppercase hex chars
        "$HEX" => {
            let s = match v {
                Value::Char(c) => c.clone(),
                Value::Num(n) => format_best(*n, 12),
                Value::Missing(k) => k.display(),
            };
            let hex: String = s.bytes().map(|b| format!("{:02X}", b)).collect();
            return Some(match spec.w {
                None => hex,
                Some(w) => {
                    let w = w as usize;
                    let mut out = hex;
                    out.truncate(w);
                    while out.len() < w {
                        out.push(' ');
                    }
                    out
                }
            });
        }

        // $UPCASE: uppercase the string
        "$UPCASE" => {
            let s = match v {
                Value::Char(c) => c.to_uppercase(),
                Value::Num(n) => format_best(*n, 12).to_uppercase(),
                Value::Missing(k) => k.display(),
            };
            return Some(match spec.w {
                None => s,
                Some(w) => {
                    let w = w as usize;
                    let mut out = s;
                    out.truncate(w);
                    while out.len() < w {
                        out.push(' ');
                    }
                    out
                }
            });
        }

        _ => {}
    }

    // ── Numeric formats — require a numeric value ────────────────────────────
    let fval = match v {
        Value::Num(n) => *n,
        // Missing handled by catalog; if we somehow get here, return the char.
        Value::Missing(k) => {
            let ch = k.display();
            let w = spec.w.unwrap_or(1) as usize;
            return Some(right_justify(&ch, w));
        }
        Value::Char(_) => return None, // numeric format on char → unknown
    };

    match name.as_str() {
        // ── w.d (plain numeric) ──────────────────────────────────────────────
        "" => Some(format_wd(fval, spec.w, spec.d)),

        // ── BESTw. ───────────────────────────────────────────────────────────
        "BEST" => {
            let w = spec.w.unwrap_or(12) as usize;
            let s = format_best(fval, w);
            Some(right_justify(&s, w))
        }

        // ── COMMAw.d ─────────────────────────────────────────────────────────
        "COMMA" => {
            let d = spec.d.unwrap_or(0) as usize;
            let s = format_decimal(fval, d);
            let with_commas = add_commas(&s);
            match spec.w {
                None => Some(with_commas),
                Some(w) => {
                    let w = w as usize;
                    if with_commas.len() <= w {
                        Some(right_justify(&with_commas, w))
                    } else {
                        Some("*".repeat(w))
                    }
                }
            }
        }

        // ── DOLLARw.d ────────────────────────────────────────────────────────
        "DOLLAR" => {
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
                    if formatted.len() <= w {
                        Some(right_justify(&formatted, w))
                    } else {
                        Some("*".repeat(w))
                    }
                }
            }
        }

        // ── Zw.d (zero-padded) ───────────────────────────────────────────────
        "Z" => {
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
                    if full.len() <= w {
                        Some(right_justify(&full, w))
                    } else {
                        Some("*".repeat(w))
                    }
                }
            }
        }

        // ── PERCENTw.d ───────────────────────────────────────────────────────
        // SAS behavior: multiply by 100, append %. For negative, SAS uses
        // parentheses like DOLLAR: (1.23%) — we implement the simpler -1.23%
        // (documented simplification).
        "PERCENT" => {
            let d = spec.d.unwrap_or(0) as usize;
            let pct = fval * 100.0;
            let s = format!("{:.prec$}%", pct, prec = d);
            match spec.w {
                None => Some(s),
                Some(w) => {
                    let w = w as usize;
                    if s.len() <= w {
                        Some(right_justify(&s, w))
                    } else {
                        Some("*".repeat(w))
                    }
                }
            }
        }

        // ── Ew. (scientific notation) ────────────────────────────────────────
        "E" => {
            let w = spec.w.unwrap_or(12) as usize;
            let s = format!("{:E}", fval);
            if s.len() <= w {
                Some(right_justify(&s, w))
            } else {
                // Try with fewer decimal digits.
                let s2 = format!("{:.2E}", fval);
                if s2.len() <= w {
                    Some(right_justify(&s2, w))
                } else {
                    Some("*".repeat(w))
                }
            }
        }

        // ── Date formats ─────────────────────────────────────────────────────

        // DATE9. → 01JAN2020, DATE7. → 01JAN20
        "DATE" => {
            let date = days_to_date(fval)?;
            let day = date.day();
            let mon = MONTHS[(date.month() - 1) as usize];
            let year = date.year();
            let w = spec.w.unwrap_or(9) as usize;
            let s = if w >= 9 {
                format!("{:02}{}{:04}", day, mon, year)
            } else {
                // DATE7 or smaller: 2-digit year
                let yr2 = year.abs() % 100;
                format!("{:02}{}{:02}", day, mon, yr2)
            };
            Some(right_justify(&s, w))
        }

        // DDMMYYw.: w=8 → ddmmyy (no sep), w=10 → dd/mm/yyyy
        "DDMMYY" => {
            let date = days_to_date(fval)?;
            let dd = date.day();
            let mm = date.month();
            let yyyy = date.year();
            let w = spec.w.unwrap_or(8) as usize;
            let s = if w >= 10 {
                format!("{:02}/{:02}/{:04}", dd, mm, yyyy)
            } else {
                let yy = yyyy.abs() % 100;
                format!("{:02}{:02}{:02}", dd, mm, yy)
            };
            Some(right_justify(&s, w))
        }

        // MMDDYYw.: w=8 → mmddyy, w=10 → mm/dd/yyyy
        "MMDDYY" => {
            let date = days_to_date(fval)?;
            let dd = date.day();
            let mm = date.month();
            let yyyy = date.year();
            let w = spec.w.unwrap_or(8) as usize;
            let s = if w >= 10 {
                format!("{:02}/{:02}/{:04}", mm, dd, yyyy)
            } else {
                let yy = yyyy.abs() % 100;
                format!("{:02}{:02}{:02}", mm, dd, yy)
            };
            Some(right_justify(&s, w))
        }

        // YYMMDDw.: w=8 → yymmdd, w=10 → yyyy/mm/dd
        "YYMMDD" => {
            let date = days_to_date(fval)?;
            let dd = date.day();
            let mm = date.month();
            let yyyy = date.year();
            let w = spec.w.unwrap_or(8) as usize;
            let s = if w >= 10 {
                format!("{:04}/{:02}/{:02}", yyyy, mm, dd)
            } else {
                let yy = yyyy.abs() % 100;
                format!("{:02}{:02}{:02}", yy, mm, dd)
            };
            Some(right_justify(&s, w))
        }

        // MONYY7. → JAN2020
        "MONYY" => {
            let date = days_to_date(fval)?;
            let mon = MONTHS[(date.month() - 1) as usize];
            let yyyy = date.year();
            let s = format!("{}{:04}", mon, yyyy);
            let w = spec.w.unwrap_or(7) as usize;
            Some(right_justify(&s, w))
        }

        // WORDDATEw. → January 1, 2020
        "WORDDATE" => {
            let date = days_to_date(fval)?;
            let mon = MONTHS_FULL[(date.month() - 1) as usize];
            let day = date.day();
            let year = date.year();
            let s = format!("{} {}, {}", mon, day, year);
            match spec.w {
                None => Some(s),
                Some(w) => Some(right_justify(&s, w as usize)),
            }
        }

        // DATETIMEw.d → 01JAN2020:12:34:56
        "DATETIME" => {
            let dt = secs_to_datetime(fval)?;
            let day = dt.day();
            let mon = MONTHS[(dt.month() - 1) as usize];
            let year = dt.year();
            let hh = dt.hour();
            let mm = dt.minute();
            let ss = dt.second();
            let s = format!("{:02}{}{:04}:{:02}:{:02}:{:02}", day, mon, year, hh, mm, ss);
            let w = spec.w.unwrap_or(19) as usize;
            Some(right_justify(&s, w))
        }

        // TIMEw.d → hh:mm:ss
        "TIME" => {
            let (hh, mm, ss) = secs_to_time(fval);
            let s = format!("{:02}:{:02}:{:02}", hh, mm, ss);
            let w = spec.w.unwrap_or(8) as usize;
            Some(right_justify(&s, w))
        }

        // ── COMMAX: like COMMA but with period as thousands sep and comma as decimal
        // European-style: 1.234,56
        "COMMAX" => {
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
                    if formatted.len() <= w {
                        Some(right_justify(&formatted, w))
                    } else {
                        Some("*".repeat(w))
                    }
                }
            }
        }

        // ── DOLLARX: like DOLLAR but European-decimal style ($1.234,56)
        "DOLLARX" => {
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
                    if i > 0 && i % 3 == 0 { vec!['.', c] } else { vec![c] }
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
                    if formatted.len() <= w {
                        Some(right_justify(&formatted, w))
                    } else {
                        Some("*".repeat(w))
                    }
                }
            }
        }

        // ── EURO: European style with € prefix (€1.234,56)
        "EURO" | "EUROX" => {
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
                    if i > 0 && i % 3 == 0 { vec!['.', c] } else { vec![c] }
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
                    if formatted.len() <= w {
                        Some(right_justify(&formatted, w))
                    } else {
                        Some("*".repeat(w))
                    }
                }
            }
        }

        // ── NEGPAREN: negative numbers in parentheses; positives as-is
        "NEGPAREN" => {
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
                    if formatted.len() <= w {
                        Some(right_justify(&formatted, w))
                    } else {
                        Some("*".repeat(w))
                    }
                }
            }
        }

        // ── HEX: integer to hexadecimal uppercase
        "HEX" => {
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

        // ── BINARY: integer to binary representation
        "BINARY" => {
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

        // ── OCTAL: integer to octal
        "OCTAL" => {
            let n = fval.round() as u64;
            let s = format!("{:o}", n);
            match spec.w {
                None => Some(s),
                Some(w) => {
                    let w = w as usize;
                    if s.len() <= w {
                        Some(right_justify(&s, w))
                    } else {
                        Some("*".repeat(w))
                    }
                }
            }
        }

        // ── ROMAN: Roman numeral notation (1–3999)
        "ROMAN" => {
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

        // ── WORDS: English words representation
        "WORDS" => {
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

        // ── FRACT: fractional notation (0.5 → 1/2)
        "FRACT" => {
            let s = to_fract(fval);
            match spec.w {
                None => Some(s),
                Some(w) => Some(right_justify(&s, w as usize)),
            }
        }

        // ── SCIENTIFIC: scientific notation (SAS-style: 1.23E+02)
        "SCIENTIFIC" => {
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
            if s.len() <= w {
                Some(right_justify(&s, w))
            } else {
                Some("*".repeat(w))
            }
        }

        // ── Date formats (M18.1 additions) ──────────────────────────────────

        // WEEKDATE: abbreviated day of week (MON, TUE, etc.)
        "WEEKDATE" => {
            let date = days_to_date(fval)?;
            // chrono weekday: Monday=0 in num_days_from_monday(); Sunday=6
            // We need Sunday=0 for our DOW_SHORT array
            let dow = date.weekday().num_days_from_sunday() as usize;
            let s = DOW_SHORT[dow];
            let w = spec.w.unwrap_or(3) as usize;
            Some(right_justify(s, w))
        }

        // DOWNAME: full day name (Monday, Tuesday, etc.)
        "DOWNAME" => {
            let date = days_to_date(fval)?;
            let dow = date.weekday().num_days_from_sunday() as usize;
            let s = DOW_FULL[dow];
            let w = spec.w.unwrap_or(9) as usize; // "Wednesday" = 9 chars
            Some(right_justify(s, w))
        }

        // MONNAME: full month name (January, February, etc.)
        "MONNAME" => {
            let date = days_to_date(fval)?;
            let s = MONTHS_FULL[(date.month() - 1) as usize];
            let w = spec.w.unwrap_or(9) as usize; // "September" = 9 chars
            Some(right_justify(s, w))
        }

        // QTR: quarter number Q1–Q4
        "QTR" | "QTRR" => {
            let date = days_to_date(fval)?;
            let q = ((date.month() - 1) / 3) + 1;
            let s = format!("{}", q);
            let w = spec.w.unwrap_or(1) as usize;
            Some(right_justify(&s, w))
        }

        // YYQ: year + quarter (2024Q1)
        "YYQ" => {
            let date = days_to_date(fval)?;
            let q = ((date.month() - 1) / 3) + 1;
            let s = format!("{}Q{}", date.year(), q);
            let w = spec.w.unwrap_or(6) as usize;
            Some(right_justify(&s, w))
        }

        // JULIAN: Julian day format YYYYDDD (day-of-year)
        "JULIAN" => {
            let date = days_to_date(fval)?;
            let doy = date.ordinal(); // 1-based day of year
            let s = format!("{:04}{:03}", date.year(), doy);
            let w = spec.w.unwrap_or(7) as usize;
            Some(right_justify(&s, w))
        }

        // B8601DA / B8601DT / B8601TM — ISO 8601 basic (no separators)
        "B8601DA" => {
            // YYYYMMDD
            let date = days_to_date(fval)?;
            let s = format!("{:04}{:02}{:02}", date.year(), date.month(), date.day());
            let w = spec.w.unwrap_or(8) as usize;
            Some(right_justify(&s, w))
        }

        "B8601DT" => {
            // YYYYMMDDTHHmmss
            let dt = secs_to_datetime(fval)?;
            let s = format!(
                "{:04}{:02}{:02}T{:02}{:02}{:02}",
                dt.year(), dt.month(), dt.day(),
                dt.hour(), dt.minute(), dt.second()
            );
            let w = spec.w.unwrap_or(15) as usize;
            Some(right_justify(&s, w))
        }

        "B8601TM" => {
            // HHmmss
            let (hh, mm, ss) = secs_to_time(fval);
            let s = format!("{:02}{:02}{:02}", hh, mm, ss);
            let w = spec.w.unwrap_or(6) as usize;
            Some(right_justify(&s, w))
        }

        // E8601DA / E8601DT / E8601TM — ISO 8601 extended (with separators)
        "E8601DA" => {
            // YYYY-MM-DD
            let date = days_to_date(fval)?;
            let s = format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day());
            let w = spec.w.unwrap_or(10) as usize;
            Some(right_justify(&s, w))
        }

        "E8601DT" => {
            // YYYY-MM-DDTHH:mm:ss
            let dt = secs_to_datetime(fval)?;
            let s = format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
                dt.year(), dt.month(), dt.day(),
                dt.hour(), dt.minute(), dt.second()
            );
            let w = spec.w.unwrap_or(19) as usize;
            Some(right_justify(&s, w))
        }

        "E8601TM" => {
            // HH:mm:ss
            let (hh, mm, ss) = secs_to_time(fval);
            let s = format!("{:02}:{:02}:{:02}", hh, mm, ss);
            let w = spec.w.unwrap_or(8) as usize;
            Some(right_justify(&s, w))
        }

        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// informat_builtin
// ─────────────────────────────────────────────────────────────────────────────

/// Parse a SAS informat. Returns None if the informat name is unknown.
pub fn informat_builtin(s: &str, spec: &FormatSpec) -> Option<Value> {
    let name = spec.name.to_uppercase();
    let trimmed = s.trim();

    match name.as_str() {
        // ── w.d (plain numeric) ──────────────────────────────────────────────
        // THE FAMOUS PITFALL: if the source has NO decimal point and d>0,
        // divide by 10^d. If the source HAS a decimal point, ignore d.
        "" | "F" => {
            if trimmed.is_empty() || trimmed == "." {
                return Some(Value::missing());
            }
            let has_decimal = trimmed.contains('.');
            match trimmed.parse::<f64>() {
                Ok(mut v) => {
                    if !has_decimal {
                        let d = spec.d.unwrap_or(0) as u32;
                        if d > 0 {
                            v /= 10f64.powi(d as i32);
                        }
                    }
                    Some(Value::Num(v))
                }
                Err(_) => Some(Value::missing()),
            }
        }

        // ── COMMAw.d — strip $ and , then treat as w.d ───────────────────────
        "COMMA" | "DOLLAR" => {
            let cleaned: String = trimmed
                .chars()
                .filter(|&c| c != ',' && c != '$')
                .collect();
            if cleaned.is_empty() || cleaned == "." {
                return Some(Value::missing());
            }
            let has_decimal = cleaned.contains('.');
            match cleaned.parse::<f64>() {
                Ok(mut v) => {
                    if !has_decimal {
                        let d = spec.d.unwrap_or(0) as u32;
                        if d > 0 {
                            v /= 10f64.powi(d as i32);
                        }
                    }
                    Some(Value::Num(v))
                }
                Err(_) => Some(Value::missing()),
            }
        }

        // ── DATE9. → days since 1960-01-01 ───────────────────────────────────
        "DATE" => {
            // Formats: 01JAN2020 (9 chars) or 01JAN20 (7 chars)
            if trimmed.len() < 7 {
                return Some(Value::missing());
            }
            let day_str = &trimmed[..2];
            let mon_str = &trimmed[2..5].to_uppercase();
            let year_str = &trimmed[5..];
            let day: u32 = day_str.parse().ok()?;
            let month = MONTHS.iter().position(|&m| m == mon_str).map(|p| p as u32 + 1)?;
            let year: i32 = year_str.parse().ok()?;
            // 2-digit year: 00-99 → 2000-2099 (simple heuristic matching SAS)
            let year = if year_str.len() == 2 {
                if year >= 0 && year < 100 {
                    2000 + year
                } else {
                    year
                }
            } else {
                year
            };
            let date = NaiveDate::from_ymd_opt(year, month, day)?;
            let days = date.signed_duration_since(sas_epoch()).num_days() as f64;
            Some(Value::Num(days))
        }

        // ── MMDDYY10. → days since 1960-01-01 ────────────────────────────────
        "MMDDYY" => {
            // Handles both mmddyyyy (8 chars, no sep) and mm/dd/yyyy (10 chars)
            let (mm, dd, yyyy) = parse_mdy_variants(trimmed)?;
            let date = NaiveDate::from_ymd_opt(yyyy, mm, dd)?;
            let days = date.signed_duration_since(sas_epoch()).num_days() as f64;
            Some(Value::Num(days))
        }

        // ── DDMMYY10. → days since 1960-01-01 ────────────────────────────────
        "DDMMYY" => {
            let (dd, mm, yyyy) = parse_dmy_variants(trimmed)?;
            let date = NaiveDate::from_ymd_opt(yyyy, mm, dd)?;
            let days = date.signed_duration_since(sas_epoch()).num_days() as f64;
            Some(Value::Num(days))
        }

        // ── YYMMDD10. → days since 1960-01-01 ────────────────────────────────
        "YYMMDD" => {
            let (yyyy, mm, dd) = parse_ymd_variants(trimmed)?;
            let date = NaiveDate::from_ymd_opt(yyyy, mm, dd)?;
            let days = date.signed_duration_since(sas_epoch()).num_days() as f64;
            Some(Value::Num(days))
        }

        // ── TIMEw. → seconds since midnight ──────────────────────────────────
        "TIME" => {
            // hh:mm:ss or hh:mm
            let parts: Vec<&str> = trimmed.split(':').collect();
            if parts.len() < 2 {
                return Some(Value::missing());
            }
            let hh: u64 = parts[0].trim().parse().ok()?;
            let mm: u64 = parts[1].trim().parse().ok()?;
            let ss: u64 = if parts.len() >= 3 {
                parts[2].trim().parse().ok()?
            } else {
                0
            };
            let secs = hh * 3600 + mm * 60 + ss;
            Some(Value::Num(secs as f64))
        }

        // ── $CHAR / $ ─────────────────────────────────────────────────────────
        "$" | "$CHAR" | "$F" => {
            let s = match spec.w {
                None => trimmed.to_string(),
                Some(w) => {
                    let mut out = trimmed.to_string();
                    out.truncate(w as usize);
                    out
                }
            };
            Some(Value::Char(s))
        }

        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Date parsing helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Parse MM/DD/YYYY or MMDDYYYY or MMDDYY (returns (month, day, year)).
fn parse_mdy_variants(s: &str) -> Option<(u32, u32, i32)> {
    if s.contains('/') {
        // mm/dd/yyyy or mm/dd/yy
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 3 {
            return None;
        }
        let mm: u32 = parts[0].parse().ok()?;
        let dd: u32 = parts[1].parse().ok()?;
        let yyyy: i32 = expand_year(parts[2].parse().ok()?, parts[2].len());
        Some((mm, dd, yyyy))
    } else if s.len() >= 8 {
        let mm: u32 = s[..2].parse().ok()?;
        let dd: u32 = s[2..4].parse().ok()?;
        let yyyy: i32 = expand_year(s[4..].parse().ok()?, s.len() - 4);
        Some((mm, dd, yyyy))
    } else {
        None
    }
}

/// Parse DD/MM/YYYY or DDMMYYYY (returns (day, month, year)).
fn parse_dmy_variants(s: &str) -> Option<(u32, u32, i32)> {
    if s.contains('/') {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 3 {
            return None;
        }
        let dd: u32 = parts[0].parse().ok()?;
        let mm: u32 = parts[1].parse().ok()?;
        let yyyy: i32 = expand_year(parts[2].parse().ok()?, parts[2].len());
        Some((dd, mm, yyyy))
    } else if s.len() >= 8 {
        let dd: u32 = s[..2].parse().ok()?;
        let mm: u32 = s[2..4].parse().ok()?;
        let yyyy: i32 = expand_year(s[4..].parse().ok()?, s.len() - 4);
        Some((dd, mm, yyyy))
    } else {
        None
    }
}

/// Parse YYYY/MM/DD or YYYYMMDD (returns (year, month, day)).
fn parse_ymd_variants(s: &str) -> Option<(i32, u32, u32)> {
    if s.contains('/') {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 3 {
            return None;
        }
        let yyyy: i32 = expand_year(parts[0].parse().ok()?, parts[0].len());
        let mm: u32 = parts[1].parse().ok()?;
        let dd: u32 = parts[2].parse().ok()?;
        Some((yyyy, mm, dd))
    } else if s.len() >= 8 {
        let yyyy: i32 = expand_year(s[..4].parse().ok()?, 4);
        let mm: u32 = s[4..6].parse().ok()?;
        let dd: u32 = s[6..8].parse().ok()?;
        Some((yyyy, mm, dd))
    } else if s.len() == 6 {
        // yymmdd
        let yy: i32 = s[..2].parse().ok()?;
        let mm: u32 = s[2..4].parse().ok()?;
        let dd: u32 = s[4..6].parse().ok()?;
        Some((expand_year(yy, 2), mm, dd))
    } else {
        None
    }
}

/// Expand a 2-digit year to 4 digits (00-99 → 2000-2099).
fn expand_year(y: i32, len: usize) -> i32 {
    if len == 2 {
        2000 + y
    } else {
        y
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{MissingKind, Value};
    use chrono::NaiveDate;

    // Helper: make a spec
    fn spec(name: &str, w: Option<u16>, d: Option<u16>) -> FormatSpec {
        FormatSpec { name: name.to_string(), w, d }
    }

    // ── Date day-number computation (verify by chrono, not hardcoded) ─────────

    fn day_num(y: i32, m: u32, d: u32) -> f64 {
        let epoch = NaiveDate::from_ymd_opt(1960, 1, 1).unwrap();
        let date = NaiveDate::from_ymd_opt(y, m, d).unwrap();
        date.signed_duration_since(epoch).num_days() as f64
    }

    #[test]
    fn day_zero_is_epoch() {
        assert_eq!(day_num(1960, 1, 1), 0.0);
    }

    #[test]
    fn day_2020_01_01() {
        // Just verify the value is positive and reasonable (21915)
        let d = day_num(2020, 1, 1);
        assert!(d > 20000.0 && d < 25000.0, "2020-01-01 day should be ~21915, got {d}");
    }

    // ── w.d numeric format ────────────────────────────────────────────────────

    #[test]
    fn wd_no_width() {
        let v = Value::Num(3.14159);
        let s = format_builtin(&v, &spec("", None, Some(2))).unwrap();
        assert_eq!(s, "3.14");
    }

    #[test]
    fn wd_right_justified() {
        let v = Value::Num(42.0);
        let s = format_builtin(&v, &spec("", Some(8), Some(0))).unwrap();
        assert_eq!(s, "      42");
    }

    #[test]
    fn wd_decimal_rounding() {
        let v = Value::Num(1.005);
        let s = format_builtin(&v, &spec("", Some(8), Some(2))).unwrap();
        // 1.005 rounds to 1.00 or 1.01 depending on floating point; just check it fits
        assert_eq!(s.len(), 8);
    }

    #[test]
    fn wd_overflow_stars() {
        // Width 3, value 12345 → doesn't fit → stars
        let v = Value::Num(12345.0);
        let s = format_builtin(&v, &spec("", Some(3), Some(0))).unwrap();
        assert_eq!(s, "***");
    }

    #[test]
    fn wd_negative() {
        let v = Value::Num(-5.0);
        let s = format_builtin(&v, &spec("", Some(6), Some(1))).unwrap();
        assert_eq!(s, "  -5.0");
    }

    // ── BEST ─────────────────────────────────────────────────────────────────

    #[test]
    fn best12_integer() {
        let v = Value::Num(42.0);
        let s = format_builtin(&v, &spec("BEST", Some(12), None)).unwrap();
        assert_eq!(s, "          42");
    }

    #[test]
    fn best12_decimal() {
        let v = Value::Num(3.14);
        let s = format_builtin(&v, &spec("BEST", Some(12), None)).unwrap();
        assert_eq!(s.trim(), "3.14");
        assert_eq!(s.len(), 12);
    }

    // ── COMMA ─────────────────────────────────────────────────────────────────

    #[test]
    fn comma_format_thousands() {
        let v = Value::Num(1234567.0);
        let s = format_builtin(&v, &spec("COMMA", Some(12), Some(0))).unwrap();
        let trimmed = s.trim();
        assert!(trimmed.contains(','), "expected commas in: {trimmed}");
        assert_eq!(trimmed, "1,234,567");
    }

    #[test]
    fn comma_format_with_decimals() {
        let v = Value::Num(1234.5);
        let s = format_builtin(&v, &spec("COMMA", Some(10), Some(2))).unwrap();
        let trimmed = s.trim();
        assert_eq!(trimmed, "1,234.50");
    }

    #[test]
    fn comma_overflow_stars() {
        let v = Value::Num(1234567890.0);
        let s = format_builtin(&v, &spec("COMMA", Some(5), Some(0))).unwrap();
        assert_eq!(s, "*****");
    }

    // ── DOLLAR ───────────────────────────────────────────────────────────────

    #[test]
    fn dollar_format() {
        let v = Value::Num(1234.0);
        let s = format_builtin(&v, &spec("DOLLAR", Some(10), Some(2))).unwrap();
        let trimmed = s.trim();
        assert_eq!(trimmed, "$1,234.00");
    }

    #[test]
    fn dollar_negative() {
        let v = Value::Num(-50.0);
        let s = format_builtin(&v, &spec("DOLLAR", Some(10), Some(2))).unwrap();
        let trimmed = s.trim();
        assert_eq!(trimmed, "-$50.00");
    }

    // ── Z (zero-padded) ──────────────────────────────────────────────────────

    #[test]
    fn z_format_pad() {
        let v = Value::Num(42.0);
        let s = format_builtin(&v, &spec("Z", Some(5), None)).unwrap();
        assert_eq!(s, "00042");
    }

    #[test]
    fn z_format_negative() {
        let v = Value::Num(-7.0);
        let s = format_builtin(&v, &spec("Z", Some(5), None)).unwrap();
        assert_eq!(s, "-0007");
    }

    // ── PERCENT ──────────────────────────────────────────────────────────────

    #[test]
    fn percent_format() {
        let v = Value::Num(0.25);
        let s = format_builtin(&v, &spec("PERCENT", Some(8), Some(1))).unwrap();
        let trimmed = s.trim();
        assert_eq!(trimmed, "25.0%");
    }

    #[test]
    fn percent_format_no_width() {
        let v = Value::Num(1.0);
        let s = format_builtin(&v, &spec("PERCENT", None, Some(0))).unwrap();
        assert_eq!(s, "100%");
    }

    // ── E (scientific) ───────────────────────────────────────────────────────

    #[test]
    fn e_format() {
        let v = Value::Num(12345.0);
        let s = format_builtin(&v, &spec("E", Some(12), None)).unwrap();
        assert!(s.contains('E') || s.contains('e'), "expected scientific notation: {s}");
    }

    // ── $CHAR ────────────────────────────────────────────────────────────────

    #[test]
    fn char_format_truncate() {
        let v = Value::Char("HelloWorld".into());
        let s = format_builtin(&v, &spec("$CHAR", Some(5), None)).unwrap();
        assert_eq!(s, "Hello");
    }

    #[test]
    fn char_format_pad() {
        let v = Value::Char("Hi".into());
        let s = format_builtin(&v, &spec("$CHAR", Some(6), None)).unwrap();
        assert_eq!(s, "Hi    ");
    }

    #[test]
    fn char_format_dollar() {
        let v = Value::Char("abc".into());
        let s = format_builtin(&v, &spec("$", Some(8), None)).unwrap();
        assert_eq!(s, "abc     ");
    }

    // ── DATE formats ─────────────────────────────────────────────────────────

    #[test]
    fn date9_epoch() {
        // Day 0 = 1960-01-01
        let v = Value::Num(0.0);
        let s = format_builtin(&v, &spec("DATE", Some(9), None)).unwrap();
        assert_eq!(s, "01JAN1960");
    }

    #[test]
    fn date9_2020_01_01() {
        let d = day_num(2020, 1, 1);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("DATE", Some(9), None)).unwrap();
        assert_eq!(s, "01JAN2020");
    }

    #[test]
    fn date7_two_digit_year() {
        let d = day_num(2020, 6, 15);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("DATE", Some(7), None)).unwrap();
        assert_eq!(s, "15JUN20");
    }

    #[test]
    fn ddmmyy8_no_sep() {
        // w=8 → no separators, 2-digit year → "ddmmyy" (6 chars) right-justified in 8
        let d = day_num(2020, 1, 1);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("DDMMYY", Some(8), None)).unwrap();
        assert_eq!(s.len(), 8);
        assert_eq!(s.trim(), "010120"); // dd=01, mm=01, yy=20
    }

    #[test]
    fn ddmmyy10_with_sep() {
        let d = day_num(2020, 1, 1);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("DDMMYY", Some(10), None)).unwrap();
        assert_eq!(s, "01/01/2020");
    }

    #[test]
    fn mmddyy8_no_sep() {
        let d = day_num(2020, 3, 15);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("MMDDYY", Some(8), None)).unwrap();
        assert_eq!(s.len(), 8);
    }

    #[test]
    fn mmddyy10_with_sep() {
        let d = day_num(2020, 3, 15);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("MMDDYY", Some(10), None)).unwrap();
        assert_eq!(s, "03/15/2020");
    }

    #[test]
    fn yymmdd8_no_sep() {
        let d = day_num(2020, 1, 1);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("YYMMDD", Some(8), None)).unwrap();
        assert_eq!(s.len(), 8);
    }

    #[test]
    fn yymmdd10_with_sep() {
        let d = day_num(2020, 1, 1);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("YYMMDD", Some(10), None)).unwrap();
        assert_eq!(s, "2020/01/01");
    }

    #[test]
    fn monyy7() {
        let d = day_num(2020, 1, 1);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("MONYY", Some(7), None)).unwrap();
        assert_eq!(s, "JAN2020");
    }

    #[test]
    fn worddate() {
        let d = day_num(2020, 1, 1);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("WORDDATE", None, None)).unwrap();
        assert_eq!(s, "January 1, 2020");
    }

    // ── DATETIME ─────────────────────────────────────────────────────────────

    #[test]
    fn datetime_epoch() {
        // Seconds since 1960-01-01: 0 → 01JAN1960:00:00:00 (18 chars)
        let v = Value::Num(0.0);
        // w=18 = exact fit, w=19 would add a leading space (right-justified).
        let s = format_builtin(&v, &spec("DATETIME", Some(18), None)).unwrap();
        assert_eq!(s, "01JAN1960:00:00:00");
    }

    #[test]
    fn datetime_epoch_w19() {
        // w=19 → right-justified, 1 leading space
        let v = Value::Num(0.0);
        let s = format_builtin(&v, &spec("DATETIME", Some(19), None)).unwrap();
        assert_eq!(s, " 01JAN1960:00:00:00");
    }

    #[test]
    fn datetime_known_time() {
        // 2020-01-01 12:34:56 → "01JAN2020:12:34:56" (18 chars)
        let d = day_num(2020, 1, 1);
        let secs = d * 86400.0 + 12.0 * 3600.0 + 34.0 * 60.0 + 56.0;
        let v = Value::Num(secs);
        let s = format_builtin(&v, &spec("DATETIME", Some(18), None)).unwrap();
        assert_eq!(s, "01JAN2020:12:34:56");
    }

    // ── TIME ─────────────────────────────────────────────────────────────────

    #[test]
    fn time_format() {
        let v = Value::Num(45296.0); // 12:34:56
        let s = format_builtin(&v, &spec("TIME", Some(8), None)).unwrap();
        assert_eq!(s, "12:34:56");
    }

    #[test]
    fn time_midnight() {
        let v = Value::Num(0.0);
        let s = format_builtin(&v, &spec("TIME", Some(8), None)).unwrap();
        assert_eq!(s, "00:00:00");
    }

    // ── Missing value handling ─────────────────────────────────────────────
    // (Catalog intercepts Missing before format_builtin, but format_builtin
    //  also handles Missing internally for safety.)

    #[test]
    fn missing_dot_in_char_format() {
        // $ format on a missing: should produce "."
        let v = Value::Missing(MissingKind::Dot);
        let s = format_builtin(&v, &spec("$", Some(3), None)).unwrap();
        assert_eq!(s, ".  "); // left-justified in char format
    }

    #[test]
    fn missing_letter_in_numeric_format() {
        // BEST on a missing letter A → "A" right-justified
        let v = Value::Missing(MissingKind::Letter(0));
        let s = format_builtin(&v, &spec("BEST", Some(5), None)).unwrap();
        assert_eq!(s, "    A");
    }

    #[test]
    fn unknown_format_returns_none() {
        let v = Value::Num(1.0);
        let result = format_builtin(&v, &spec("XYZZY", None, None));
        assert!(result.is_none());
    }

    // ── Informats ────────────────────────────────────────────────────────────

    #[test]
    fn informat_wd_no_implicit_decimal() {
        // No d, or d=0 — no division
        let s = informat_builtin("42", &spec("", None, Some(0))).unwrap();
        assert_eq!(s, Value::Num(42.0));
    }

    #[test]
    fn informat_wd_implicit_decimal_pitfall() {
        // THE FAMOUS PITFALL: "123" with informat 5.2 → 1.23
        let s = informat_builtin("123", &spec("", Some(5), Some(2))).unwrap();
        assert_eq!(s, Value::Num(1.23));
    }

    #[test]
    fn informat_wd_explicit_decimal_ignores_d() {
        // "1.23" with informat 5.2 → 1.23 (d is ignored when point present)
        let s = informat_builtin("1.23", &spec("", Some(5), Some(2))).unwrap();
        assert_eq!(s, Value::Num(1.23));
    }

    #[test]
    fn informat_wd_dot_gives_missing() {
        let s = informat_builtin(".", &spec("", None, None)).unwrap();
        assert_eq!(s, Value::missing());
    }

    #[test]
    fn informat_wd_empty_gives_missing() {
        let s = informat_builtin("  ", &spec("", None, None)).unwrap();
        assert_eq!(s, Value::missing());
    }

    #[test]
    fn informat_comma_strips_commas() {
        let s = informat_builtin("1,234.56", &spec("COMMA", Some(10), Some(2))).unwrap();
        assert_eq!(s, Value::Num(1234.56));
    }

    #[test]
    fn informat_dollar_strips_dollar_and_commas() {
        let s = informat_builtin("$1,234", &spec("DOLLAR", Some(10), Some(0))).unwrap();
        assert_eq!(s, Value::Num(1234.0));
    }

    #[test]
    fn informat_date9_epoch() {
        // 01JAN1960 → 0.0
        let v = informat_builtin("01JAN1960", &spec("DATE", Some(9), None)).unwrap();
        assert_eq!(v, Value::Num(0.0));
    }

    #[test]
    fn informat_date9_2020() {
        let d = day_num(2020, 1, 1);
        let v = informat_builtin("01JAN2020", &spec("DATE", Some(9), None)).unwrap();
        assert_eq!(v, Value::Num(d));
    }

    #[test]
    fn informat_date9_roundtrip_with_format() {
        // Format then informat should give back same day number.
        let original = day_num(2020, 6, 15);
        let v = Value::Num(original);
        let formatted = format_builtin(&v, &spec("DATE", Some(9), None)).unwrap();
        let parsed = informat_builtin(&formatted, &spec("DATE", Some(9), None)).unwrap();
        assert_eq!(parsed, Value::Num(original));
    }

    #[test]
    fn informat_mmddyy10() {
        let d = day_num(2020, 3, 15);
        let v = informat_builtin("03/15/2020", &spec("MMDDYY", Some(10), None)).unwrap();
        assert_eq!(v, Value::Num(d));
    }

    #[test]
    fn informat_ddmmyy10() {
        let d = day_num(2020, 3, 15);
        let v = informat_builtin("15/03/2020", &spec("DDMMYY", Some(10), None)).unwrap();
        assert_eq!(v, Value::Num(d));
    }

    #[test]
    fn informat_yymmdd10() {
        let d = day_num(2020, 3, 15);
        let v = informat_builtin("2020/03/15", &spec("YYMMDD", Some(10), None)).unwrap();
        assert_eq!(v, Value::Num(d));
    }

    #[test]
    fn informat_time_hms() {
        // 12:34:56 = 45296 seconds
        let v = informat_builtin("12:34:56", &spec("TIME", Some(8), None)).unwrap();
        assert_eq!(v, Value::Num(45296.0));
    }

    #[test]
    fn informat_char() {
        let v = informat_builtin("  hello  ", &spec("$CHAR", Some(10), None)).unwrap();
        assert_eq!(v, Value::Char("hello".into()));
    }

    #[test]
    fn informat_unknown_returns_none() {
        let result = informat_builtin("42", &spec("XYZZY", None, None));
        assert!(result.is_none());
    }

    // ── add_commas helper ─────────────────────────────────────────────────────

    #[test]
    fn add_commas_simple() {
        assert_eq!(add_commas("1234567"), "1,234,567");
    }

    #[test]
    fn add_commas_with_decimals() {
        assert_eq!(add_commas("1234.56"), "1,234.56");
    }

    #[test]
    fn add_commas_negative() {
        assert_eq!(add_commas("-9876543"), "-9,876,543");
    }

    #[test]
    fn add_commas_small() {
        assert_eq!(add_commas("42"), "42");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // M18.1 — new format tests
    // ─────────────────────────────────────────────────────────────────────────

    // ── COMMAX ───────────────────────────────────────────────────────────────

    #[test]
    fn commax_basic() {
        // 123456.78 → "123.456,78" (European separators)
        let v = Value::Num(123456.78);
        let s = format_builtin(&v, &spec("COMMAX", Some(12), Some(2))).unwrap();
        let t = s.trim();
        assert_eq!(t, "123.456,78");
    }

    #[test]
    fn commax_negative() {
        let v = Value::Num(-1234.5);
        let s = format_builtin(&v, &spec("COMMAX", None, Some(1))).unwrap();
        assert_eq!(s, "-1.234,5");
    }

    #[test]
    fn commax_zero() {
        let v = Value::Num(0.0);
        let s = format_builtin(&v, &spec("COMMAX", None, Some(0))).unwrap();
        assert_eq!(s, "0");
    }

    // ── DOLLARX ──────────────────────────────────────────────────────────────

    #[test]
    fn dollarx_basic() {
        let v = Value::Num(1234.56);
        let s = format_builtin(&v, &spec("DOLLARX", Some(12), Some(2))).unwrap();
        let t = s.trim();
        assert_eq!(t, "$1.234,56");
    }

    #[test]
    fn dollarx_negative() {
        let v = Value::Num(-50.0);
        let s = format_builtin(&v, &spec("DOLLARX", None, Some(2))).unwrap();
        assert_eq!(s, "-$50,00");
    }

    // ── EURO ─────────────────────────────────────────────────────────────────

    #[test]
    fn euro_basic() {
        // €1.234,56 — note: € is multi-byte in UTF-8 so we check content
        let v = Value::Num(1234.56);
        let s = format_builtin(&v, &spec("EURO", None, Some(2))).unwrap();
        assert!(s.contains('€'), "expected € in: {s}");
        assert!(s.contains("1.234"), "expected thousands sep in: {s}");
        assert!(s.contains(",56"), "expected comma decimal in: {s}");
    }

    #[test]
    fn euro_no_decimals() {
        let v = Value::Num(1000.0);
        let s = format_builtin(&v, &spec("EURO", None, Some(0))).unwrap();
        assert_eq!(s, "€1.000");
    }

    // ── NEGPAREN ─────────────────────────────────────────────────────────────

    #[test]
    fn negparen_negative() {
        let v = Value::Num(-123.0);
        let s = format_builtin(&v, &spec("NEGPAREN", None, Some(0))).unwrap();
        assert_eq!(s, "(123)");
    }

    #[test]
    fn negparen_positive() {
        let v = Value::Num(456.0);
        let s = format_builtin(&v, &spec("NEGPAREN", None, Some(0))).unwrap();
        assert_eq!(s, "456");
    }

    #[test]
    fn negparen_zero() {
        let v = Value::Num(0.0);
        let s = format_builtin(&v, &spec("NEGPAREN", None, Some(0))).unwrap();
        assert_eq!(s, "0");
    }

    // ── HEX ──────────────────────────────────────────────────────────────────

    #[test]
    fn hex_format_255() {
        let v = Value::Num(255.0);
        let s = format_builtin(&v, &spec("HEX", None, None)).unwrap();
        assert_eq!(s, "FF");
    }

    #[test]
    fn hex_format_zero() {
        let v = Value::Num(0.0);
        let s = format_builtin(&v, &spec("HEX", None, None)).unwrap();
        assert_eq!(s, "0");
    }

    #[test]
    fn hex_format_with_width() {
        let v = Value::Num(255.0);
        let s = format_builtin(&v, &spec("HEX", Some(8), None)).unwrap();
        assert_eq!(s, "      FF");
    }

    // ── BINARY ───────────────────────────────────────────────────────────────

    #[test]
    fn binary_format_255() {
        let v = Value::Num(255.0);
        let s = format_builtin(&v, &spec("BINARY", None, None)).unwrap();
        assert_eq!(s, "11111111");
    }

    #[test]
    fn binary_format_zero() {
        let v = Value::Num(0.0);
        let s = format_builtin(&v, &spec("BINARY", None, None)).unwrap();
        assert_eq!(s, "0");
    }

    #[test]
    fn binary_format_10() {
        let v = Value::Num(10.0);
        let s = format_builtin(&v, &spec("BINARY", None, None)).unwrap();
        assert_eq!(s, "1010");
    }

    // ── OCTAL ────────────────────────────────────────────────────────────────

    #[test]
    fn octal_format_255() {
        let v = Value::Num(255.0);
        let s = format_builtin(&v, &spec("OCTAL", None, None)).unwrap();
        assert_eq!(s, "377");
    }

    #[test]
    fn octal_format_8() {
        let v = Value::Num(8.0);
        let s = format_builtin(&v, &spec("OCTAL", None, None)).unwrap();
        assert_eq!(s, "10");
    }

    // ── ROMAN ────────────────────────────────────────────────────────────────

    #[test]
    fn roman_nine() {
        let v = Value::Num(9.0);
        let s = format_builtin(&v, &spec("ROMAN", None, None)).unwrap();
        assert_eq!(s, "IX");
    }

    #[test]
    fn roman_1994() {
        let v = Value::Num(1994.0);
        let s = format_builtin(&v, &spec("ROMAN", None, None)).unwrap();
        assert_eq!(s, "MCMXCIV");
    }

    #[test]
    fn roman_one() {
        let v = Value::Num(1.0);
        let s = format_builtin(&v, &spec("ROMAN", None, None)).unwrap();
        assert_eq!(s, "I");
    }

    #[test]
    fn roman_forty() {
        let v = Value::Num(40.0);
        let s = format_builtin(&v, &spec("ROMAN", None, None)).unwrap();
        assert_eq!(s, "XL");
    }

    // ── WORDS ────────────────────────────────────────────────────────────────

    #[test]
    fn words_zero() {
        let v = Value::Num(0.0);
        let s = format_builtin(&v, &spec("WORDS", None, None)).unwrap();
        assert_eq!(s, "ZERO");
    }

    #[test]
    fn words_one() {
        let v = Value::Num(1.0);
        let s = format_builtin(&v, &spec("WORDS", None, None)).unwrap();
        assert_eq!(s, "ONE");
    }

    #[test]
    fn words_123() {
        let v = Value::Num(123.0);
        let s = format_builtin(&v, &spec("WORDS", None, None)).unwrap();
        assert_eq!(s, "ONE HUNDRED TWENTY-THREE");
    }

    #[test]
    fn words_1000() {
        let v = Value::Num(1000.0);
        let s = format_builtin(&v, &spec("WORDS", None, None)).unwrap();
        assert_eq!(s, "ONE THOUSAND");
    }

    #[test]
    fn words_negative() {
        let v = Value::Num(-5.0);
        let s = format_builtin(&v, &spec("WORDS", None, None)).unwrap();
        assert_eq!(s, "NEGATIVE FIVE");
    }

    // ── FRACT ────────────────────────────────────────────────────────────────

    #[test]
    fn fract_half() {
        let v = Value::Num(0.5);
        let s = format_builtin(&v, &spec("FRACT", None, None)).unwrap();
        assert_eq!(s, "1/2");
    }

    #[test]
    fn fract_integer() {
        let v = Value::Num(3.0);
        let s = format_builtin(&v, &spec("FRACT", None, None)).unwrap();
        assert_eq!(s, "3");
    }

    #[test]
    fn fract_zero() {
        let v = Value::Num(0.0);
        let s = format_builtin(&v, &spec("FRACT", None, None)).unwrap();
        assert_eq!(s, "0");
    }

    #[test]
    fn fract_one_third() {
        let v = Value::Num(1.0 / 3.0);
        let s = format_builtin(&v, &spec("FRACT", None, None)).unwrap();
        // Should be 1/3 or close fraction
        assert!(s.contains('/'), "expected fraction: {s}");
    }

    // ── SCIENTIFIC ───────────────────────────────────────────────────────────

    #[test]
    fn scientific_basic() {
        let v = Value::Num(123.0);
        let s = format_builtin(&v, &spec("SCIENTIFIC", Some(12), Some(2))).unwrap();
        let t = s.trim();
        // 1.23E+02
        assert!(t.contains('E'), "expected E in: {t}");
        assert!(t.contains("1.23"), "expected 1.23 in: {t}");
    }

    #[test]
    fn scientific_zero() {
        let v = Value::Num(0.0);
        let s = format_builtin(&v, &spec("SCIENTIFIC", None, Some(2))).unwrap();
        assert!(s.contains("0.00E"), "expected 0.00E in: {s}");
    }

    #[test]
    fn scientific_small() {
        let v = Value::Num(0.001);
        let s = format_builtin(&v, &spec("SCIENTIFIC", None, Some(2))).unwrap();
        assert!(s.contains("E-"), "expected E- in: {s}");
    }

    // ── $QUOTE ───────────────────────────────────────────────────────────────

    #[test]
    fn quote_basic() {
        let v = Value::Char("hello".into());
        let s = format_builtin(&v, &spec("$QUOTE", None, None)).unwrap();
        assert_eq!(s, "\"hello\"");
    }

    #[test]
    fn quote_with_width() {
        let v = Value::Char("hi".into());
        let s = format_builtin(&v, &spec("$QUOTE", Some(6), None)).unwrap();
        assert_eq!(s, "\"hi\"  ");
    }

    // ── $HEX ─────────────────────────────────────────────────────────────────

    #[test]
    fn hex_char_format() {
        let v = Value::Char("A".into());
        let s = format_builtin(&v, &spec("$HEX", None, None)).unwrap();
        assert_eq!(s, "41"); // 'A' = 0x41
    }

    #[test]
    fn hex_char_hello() {
        let v = Value::Char("hi".into());
        let s = format_builtin(&v, &spec("$HEX", None, None)).unwrap();
        assert_eq!(s, "6869"); // h=0x68, i=0x69
    }

    // ── $UPCASE ──────────────────────────────────────────────────────────────

    #[test]
    fn upcase_basic() {
        let v = Value::Char("hello world".into());
        let s = format_builtin(&v, &spec("$UPCASE", None, None)).unwrap();
        assert_eq!(s, "HELLO WORLD");
    }

    #[test]
    fn upcase_already_upper() {
        let v = Value::Char("ABC".into());
        let s = format_builtin(&v, &spec("$UPCASE", None, None)).unwrap();
        assert_eq!(s, "ABC");
    }

    // ── WEEKDATE ─────────────────────────────────────────────────────────────

    #[test]
    fn weekdate_monday() {
        // 2020-01-06 is a Monday
        let d = day_num(2020, 1, 6);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("WEEKDATE", None, None)).unwrap();
        assert_eq!(s.trim(), "MON");
    }

    #[test]
    fn weekdate_sunday() {
        // 2020-01-05 is a Sunday
        let d = day_num(2020, 1, 5);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("WEEKDATE", None, None)).unwrap();
        assert_eq!(s.trim(), "SUN");
    }

    // ── DOWNAME ──────────────────────────────────────────────────────────────

    #[test]
    fn downame_wednesday() {
        // 2020-01-08 is a Wednesday
        let d = day_num(2020, 1, 8);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("DOWNAME", None, None)).unwrap();
        assert_eq!(s.trim(), "Wednesday");
    }

    // ── MONNAME ──────────────────────────────────────────────────────────────

    #[test]
    fn monname_january() {
        let d = day_num(2020, 1, 15);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("MONNAME", None, None)).unwrap();
        assert_eq!(s.trim(), "January");
    }

    #[test]
    fn monname_september() {
        let d = day_num(2020, 9, 1);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("MONNAME", None, None)).unwrap();
        assert_eq!(s.trim(), "September");
    }

    // ── QTR ──────────────────────────────────────────────────────────────────

    #[test]
    fn qtr_q1() {
        let d = day_num(2024, 1, 15);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("QTR", None, None)).unwrap();
        assert_eq!(s, "1");
    }

    #[test]
    fn qtr_q3() {
        let d = day_num(2024, 7, 1);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("QTR", None, None)).unwrap();
        assert_eq!(s, "3");
    }

    #[test]
    fn qtr_q4() {
        let d = day_num(2024, 12, 31);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("QTR", None, None)).unwrap();
        assert_eq!(s, "4");
    }

    // ── YYQ ──────────────────────────────────────────────────────────────────

    #[test]
    fn yyq_2024q1() {
        let d = day_num(2024, 1, 15);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("YYQ", None, None)).unwrap();
        assert_eq!(s.trim(), "2024Q1");
    }

    #[test]
    fn yyq_2024q4() {
        let d = day_num(2024, 10, 1);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("YYQ", None, None)).unwrap();
        assert_eq!(s.trim(), "2024Q4");
    }

    // ── JULIAN ───────────────────────────────────────────────────────────────

    #[test]
    fn julian_new_year() {
        let d = day_num(2024, 1, 1);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("JULIAN", None, None)).unwrap();
        assert_eq!(s.trim(), "2024001");
    }

    #[test]
    fn julian_last_day() {
        let d = day_num(2024, 12, 31);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("JULIAN", None, None)).unwrap();
        let t = s.trim();
        assert!(t.starts_with("2024"), "expected 2024... in: {t}");
        assert!(t.ends_with("366"), "2024 is leap year so day 366: {t}");
    }

    // ── B8601 / E8601 ────────────────────────────────────────────────────────

    #[test]
    fn b8601da_basic() {
        let d = day_num(2020, 3, 15);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("B8601DA", None, None)).unwrap();
        assert_eq!(s.trim(), "20200315");
    }

    #[test]
    fn e8601da_basic() {
        let d = day_num(2020, 3, 15);
        let v = Value::Num(d);
        let s = format_builtin(&v, &spec("E8601DA", None, None)).unwrap();
        assert_eq!(s.trim(), "2020-03-15");
    }

    #[test]
    fn e8601dt_basic() {
        // 2020-01-01 12:34:56
        let d = day_num(2020, 1, 1);
        let secs = d * 86400.0 + 12.0 * 3600.0 + 34.0 * 60.0 + 56.0;
        let v = Value::Num(secs);
        let s = format_builtin(&v, &spec("E8601DT", None, None)).unwrap();
        assert_eq!(s.trim(), "2020-01-01T12:34:56");
    }

    #[test]
    fn b8601dt_basic() {
        let d = day_num(2020, 1, 1);
        let secs = d * 86400.0 + 12.0 * 3600.0 + 34.0 * 60.0 + 56.0;
        let v = Value::Num(secs);
        let s = format_builtin(&v, &spec("B8601DT", None, None)).unwrap();
        assert_eq!(s.trim(), "20200101T123456");
    }

    #[test]
    fn e8601tm_basic() {
        let v = Value::Num(45296.0); // 12:34:56
        let s = format_builtin(&v, &spec("E8601TM", None, None)).unwrap();
        assert_eq!(s.trim(), "12:34:56");
    }

    #[test]
    fn b8601tm_basic() {
        let v = Value::Num(45296.0); // 12:34:56
        let s = format_builtin(&v, &spec("B8601TM", None, None)).unwrap();
        assert_eq!(s.trim(), "123456");
    }

    // ── Width + missing value edge cases ─────────────────────────────────────

    #[test]
    fn hex_missing_returns_dot() {
        // Missing values: format_builtin receives them only via the Missing arm
        let v = Value::Missing(crate::value::MissingKind::Dot);
        // HEX is a numeric format; missing is handled by catalog before builtin,
        // but if it reaches builtin the Missing arm returns right-justified "."
        let s = format_builtin(&v, &spec("HEX", Some(5), None)).unwrap();
        assert_eq!(s, "    .");
    }

    #[test]
    fn roman_width_right_justified() {
        let v = Value::Num(4.0); // IV
        let s = format_builtin(&v, &spec("ROMAN", Some(8), None)).unwrap();
        assert_eq!(s, "      IV");
    }

    #[test]
    fn negparen_large_with_commas() {
        let v = Value::Num(-1234567.0);
        let s = format_builtin(&v, &spec("NEGPAREN", None, Some(0))).unwrap();
        assert_eq!(s, "(1,234,567)");
    }

    // ── Roman numeral helper unit tests ──────────────────────────────────────

    #[test]
    fn roman_helper_iv() {
        assert_eq!(to_roman(4), "IV");
    }

    #[test]
    fn roman_helper_mcmxcix() {
        assert_eq!(to_roman(1999), "MCMXCIX");
    }

    #[test]
    fn roman_helper_out_of_range() {
        assert_eq!(to_roman(0), "");
        assert_eq!(to_roman(4000), "");
    }

    // ── Words helper unit tests ───────────────────────────────────────────────

    #[test]
    fn words_million() {
        assert_eq!(to_words(1_000_000), "ONE MILLION");
    }

    #[test]
    fn words_complex() {
        assert_eq!(to_words(999), "NINE HUNDRED NINETY-NINE");
    }

    // ── Fract helper unit tests ───────────────────────────────────────────────

    #[test]
    fn fract_quarter() {
        assert_eq!(to_fract(0.25), "1/4");
    }

    #[test]
    fn fract_one_and_half() {
        assert_eq!(to_fract(1.5), "1 1/2");
    }

    #[test]
    fn fract_negative_half() {
        assert_eq!(to_fract(-0.5), "-1/2");
    }
}
