use super::super::*;
use super::*;
use crate::value::Value;

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
