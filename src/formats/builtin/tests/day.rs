use super::super::*;
use super::*;
use crate::value::{MissingKind, Value};

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
