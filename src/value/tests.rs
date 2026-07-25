use super::*;

#[test]
fn missing_order_is_total_sas_order() {
    let underscore = Value::Missing(MissingKind::Underscore);
    let dot = Value::Missing(MissingKind::Dot);
    let a = Value::Missing(MissingKind::Letter(0));
    let z = Value::Missing(MissingKind::Letter(25));
    let neg = Value::Num(-1e300);

    assert_eq!(underscore.sas_cmp(&dot), Ordering::Less);
    assert_eq!(dot.sas_cmp(&a), Ordering::Less);
    assert_eq!(a.sas_cmp(&z), Ordering::Less);
    assert_eq!(z.sas_cmp(&neg), Ordering::Less);
    // `. = .` is true in SAS.
    assert_eq!(dot.sas_cmp(&Value::missing()), Ordering::Equal);
}

#[test]
fn char_compare_ignores_trailing_blanks() {
    let a = Value::Char("abc".into());
    let b = Value::Char("abc   ".into());
    assert_eq!(a.sas_cmp(&b), Ordering::Equal);
}

#[test]
fn best_format() {
    assert_eq!(format_best(3.0, 12), "3");
    assert_eq!(format_best(-42.0, 12), "-42");
    assert_eq!(format_best(0.5, 12), "0.5");
    assert_eq!(format_best(1.0 / 3.0, 12), "0.3333333333");
}
