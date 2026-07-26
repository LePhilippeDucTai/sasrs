use super::*;

#[test]
fn picture_nine_selectors_print_leading_zeros() {
    // '99999' is a fixed-width zero-padded integer picture.
    let p = pic_low_high("99999", PictureDirectives::default());
    assert_eq!(p.render(&Value::Num(42.0)).as_deref(), Some("00042"));
    assert_eq!(p.render(&Value::Num(12345.0)).as_deref(), Some("12345"));
}

#[test]
fn picture_zero_selectors_suppress_leading_zeros() {
    // '00000' suppresses leading zeros (fill = space).
    let p = pic_low_high("00000", PictureDirectives::default());
    assert_eq!(p.render(&Value::Num(42.0)).as_deref(), Some("   42"));
    assert_eq!(p.render(&Value::Num(12345.0)).as_deref(), Some("12345"));
}

#[test]
fn picture_mixed_zero_then_nine() {
    // '009' — two suppressed leading positions, one always-printed unit.
    let p = pic_low_high("009", PictureDirectives::default());
    assert_eq!(p.render(&Value::Num(5.0)).as_deref(), Some("  5"));
    assert_eq!(p.render(&Value::Num(0.0)).as_deref(), Some("  0"));
    assert_eq!(p.render(&Value::Num(123.0)).as_deref(), Some("123"));
}

#[test]
fn picture_literal_separators_date() {
    // '99/99/9999' on a packed date-like integer.
    let p = pic_low_high("99/99/9999", PictureDirectives::default());
    // 12252020 → 12/25/2020
    assert_eq!(
        p.render(&Value::Num(12252020.0)).as_deref(),
        Some("12/25/2020")
    );
}

#[test]
fn picture_comma_separator() {
    let p = pic_low_high("000,000,009", PictureDirectives::default());
    assert_eq!(
        p.render(&Value::Num(1234567.0)).as_deref(),
        Some("  1,234,567")
    );
    // Leading separators dropped when value is small.
    assert_eq!(p.render(&Value::Num(5.0)).as_deref(), Some("          5"));
}

#[test]
fn picture_decimal_auto_mult() {
    // '009.99' auto-derives MULT=100 from the two fractional selectors.
    let p = pic_low_high("009.99", PictureDirectives::default());
    assert_eq!(p.render(&Value::Num(1.5)).as_deref(), Some("  1.50"));
}

#[test]
fn picture_decimal_values() {
    let p = pic_low_high("009.99", PictureDirectives::default());
    assert_eq!(p.render(&Value::Num(1.5)).as_deref(), Some("  1.50"));
    assert_eq!(p.render(&Value::Num(12.34)).as_deref(), Some(" 12.34"));
    assert_eq!(p.render(&Value::Num(0.0)).as_deref(), Some("  0.00"));
}

#[test]
fn picture_prefix() {
    let mut dir = PictureDirectives::default();
    dir.prefix = Some("$".to_string());
    let p = pic_low_high("000,000,009.99", dir);
    assert_eq!(p.render(&Value::Num(1234.5)).as_deref(), Some("$1,234.50"));
}

#[test]
fn picture_explicit_mult() {
    // MULT=100 turns a proportion into a percentage of digits.
    let mut dir = PictureDirectives::default();
    dir.mult = Some(100.0);
    let p = pic_low_high("009.9%", dir);
    // 0.125 * 100 = 12.5 → with one fractional selector (auto would be 10,
    // but explicit MULT=100 wins): scaled = round(0.125*100)=12 → '12%'?
    // n=0.125, mult=100 → 12.5 rounds to 13 → digits "13" → 1 frac selector
    // expects 1 fractional digit; selectors=4 (0,0,9,9). padded "0013" →
    // '  1.3%'.
    assert_eq!(p.render(&Value::Num(0.125)).as_deref(), Some("  1.3%"));
}

#[test]
fn picture_fill_character() {
    let mut dir = PictureDirectives::default();
    dir.fill = Some('*');
    let p = pic_low_high("00000", dir);
    assert_eq!(p.render(&Value::Num(42.0)).as_deref(), Some("***42"));
}

#[test]
fn picture_negative_number() {
    let p = pic_low_high("009.99", PictureDirectives::default());
    // Magnitude rendered, leading '-'.
    assert_eq!(p.render(&Value::Num(-12.34)).as_deref(), Some("- 12.34"));
}

#[test]
fn picture_range_selection() {
    let p = UserPicture {
        ranges: vec![
            PictureRange {
                from: Bound::Num(0.0),
                to: Bound::Num(9.0),
                from_exclusive: false,
                to_exclusive: false,
                template: "9".to_string(),
                directives: PictureDirectives::default(),
            },
            PictureRange {
                from: Bound::Num(10.0),
                to: Bound::High,
                from_exclusive: false,
                to_exclusive: false,
                template: "999".to_string(),
                directives: PictureDirectives::default(),
            },
        ],
        other: None,
    };
    assert_eq!(p.render(&Value::Num(5.0)).as_deref(), Some("5"));
    assert_eq!(p.render(&Value::Num(42.0)).as_deref(), Some("042"));
    // Out of all ranges, no other → None.
    assert_eq!(p.render(&Value::Num(-1.0)), None);
}

#[test]
fn picture_other_fallback() {
    let p = UserPicture {
        ranges: vec![PictureRange {
            from: Bound::Num(0.0),
            to: Bound::Num(9.0),
            from_exclusive: false,
            to_exclusive: false,
            template: "9".to_string(),
            directives: PictureDirectives::default(),
        }],
        other: Some(("0000".to_string(), PictureDirectives::default())),
    };
    assert_eq!(p.render(&Value::Num(5.0)).as_deref(), Some("5"));
    assert_eq!(p.render(&Value::Num(123.0)).as_deref(), Some(" 123"));
}

#[test]
fn picture_non_numeric_returns_none() {
    let p = pic_low_high("999", PictureDirectives::default());
    assert_eq!(p.render(&Value::Char("x".to_string())), None);
}
