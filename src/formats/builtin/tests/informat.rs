use super::super::*;
use super::*;
use crate::value::Value;

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

#[test]
fn negparen_large_with_commas() {
    let v = Value::Num(-1234567.0);
    let s = format_builtin(&v, &spec("NEGPAREN", None, Some(0))).unwrap();
    assert_eq!(s, "(1,234,567)");
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

#[test]
fn roman_width_right_justified() {
    let v = Value::Num(4.0); // IV
    let s = format_builtin(&v, &spec("ROMAN", Some(8), None)).unwrap();
    assert_eq!(s, "      IV");
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

// ── Words helper unit tests ───────────────────────────────────────────────

#[test]
fn words_million() {
    assert_eq!(to_words(1_000_000), "ONE MILLION");
}

#[test]
fn words_complex() {
    assert_eq!(to_words(999), "NINE HUNDRED NINETY-NINE");
}
