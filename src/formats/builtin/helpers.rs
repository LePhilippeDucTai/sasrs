use super::*;

/// Ajuste une chaîne déjà formatée à la largeur `w` : cadrée à droite si elle
/// tient, sinon remplie d'étoiles (débordement SAS).
pub(super) fn fit_or_stars(s: &str, w: usize) -> String {
    if s.len() <= w {
        right_justify(s, w)
    } else {
        "*".repeat(w)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// SAS epoch: 1960-01-01
pub(super) fn sas_epoch() -> NaiveDate {
    NaiveDate::from_ymd_opt(1960, 1, 1).unwrap()
}

/// Days since SAS epoch → NaiveDate (clamp to valid range).
pub(super) fn days_to_date(days: f64) -> Option<NaiveDate> {
    let d = days.round() as i64;
    sas_epoch().checked_add_signed(Duration::days(d))
}

/// Seconds since SAS epoch → NaiveDateTime.
pub(super) fn secs_to_datetime(secs: f64) -> Option<NaiveDateTime> {
    let epoch = sas_epoch().and_hms_opt(0, 0, 0)?;
    let whole = secs.trunc() as i64;
    epoch.checked_add_signed(Duration::seconds(whole))
}

/// Seconds-of-day → (hh, mm, ss).
pub(super) fn secs_to_time(secs: f64) -> (u32, u32, u32) {
    let total = secs.abs().round() as u64;
    let hh = total / 3600;
    let mm = (total % 3600) / 60;
    let ss = total % 60;
    (hh as u32, mm as u32, ss as u32)
}

/// Short month names (uppercase), 1-indexed.
pub(super) const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

/// Full month names.
pub(super) const MONTHS_FULL: [&str; 12] = [
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
pub(super) const DOW_SHORT: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

/// Full day-of-week names, Sunday=0.
pub(super) const DOW_FULL: [&str; 7] = [
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
pub(super) fn to_roman(mut n: u32) -> String {
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
pub(super) fn to_words(n: i64) -> String {
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

pub(super) fn to_words_unsigned(n: u64) -> String {
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
pub(super) fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Format a float as a simple fraction (limited denominator ≤ 64 for v1).
pub(super) fn to_fract(v: f64) -> String {
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
pub(super) fn add_commas(s: &str) -> String {
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
pub(super) fn format_decimal(v: f64, d: usize) -> String {
    format!("{:.prec$}", v, prec = d)
}

/// w.d format: round to d decimals, right-justify to w.
/// On overflow: try BESTw., then fill with '*'.
pub(super) fn format_wd(v: f64, w: Option<u16>, d: Option<u16>) -> String {
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
                fit_or_stars(&best, ww)
            }
        }
    }
}
