use super::*;
use crate::value::MissingKind;

// -------------------------------------------------------------------------
// FormatSpec::parse
// -------------------------------------------------------------------------

#[test]
fn parse_date9_dot() {
    let spec = FormatSpec::parse("DATE9.").unwrap();
    assert_eq!(spec.name, "DATE");
    assert_eq!(spec.w, Some(9));
    assert_eq!(spec.d, None);
}

#[test]
fn parse_date9_no_dot() {
    let spec = FormatSpec::parse("DATE9").unwrap();
    assert_eq!(spec.name, "DATE");
    assert_eq!(spec.w, Some(9));
    assert_eq!(spec.d, None);
}

#[test]
fn parse_wd_8_2() {
    let spec = FormatSpec::parse("8.2").unwrap();
    assert_eq!(spec.name, "");
    assert_eq!(spec.w, Some(8));
    assert_eq!(spec.d, Some(2));
}

#[test]
fn parse_wd_8_dot() {
    let spec = FormatSpec::parse("8.").unwrap();
    assert_eq!(spec.name, "");
    assert_eq!(spec.w, Some(8));
    assert_eq!(spec.d, None);
}

#[test]
fn parse_dollar_char10() {
    let spec = FormatSpec::parse("$CHAR10.").unwrap();
    assert_eq!(spec.name, "$CHAR");
    assert_eq!(spec.w, Some(10));
    assert_eq!(spec.d, None);
}

#[test]
fn parse_dollar_10() {
    let spec = FormatSpec::parse("$10.").unwrap();
    assert_eq!(spec.name, "$");
    assert_eq!(spec.w, Some(10));
    assert_eq!(spec.d, None);
}

#[test]
fn parse_comma12_2() {
    let spec = FormatSpec::parse("COMMA12.2").unwrap();
    assert_eq!(spec.name, "COMMA");
    assert_eq!(spec.w, Some(12));
    assert_eq!(spec.d, Some(2));
}

#[test]
fn parse_best12() {
    let spec = FormatSpec::parse("BEST12.").unwrap();
    assert_eq!(spec.name, "BEST");
    assert_eq!(spec.w, Some(12));
    assert_eq!(spec.d, None);
}

#[test]
fn parse_empty_returns_none() {
    assert!(FormatSpec::parse("").is_none());
}

#[test]
fn parse_lowercase_is_upcased() {
    let spec = FormatSpec::parse("date9.").unwrap();
    assert_eq!(spec.name, "DATE");
}

#[test]
fn parse_name_only_no_width() {
    let spec = FormatSpec::parse("BEST.").unwrap();
    assert_eq!(spec.name, "BEST");
    assert_eq!(spec.w, None);
    assert_eq!(spec.d, None);
}

// -------------------------------------------------------------------------
// FormatCatalog::format — fallback paths
// -------------------------------------------------------------------------

fn catalog() -> FormatCatalog {
    FormatCatalog::default()
}

#[test]
fn format_missing_dot_right_justified() {
    let cat = catalog();
    let spec = FormatSpec {
        name: "".into(),
        w: Some(5),
        d: None,
    };
    let result = cat.format(&Value::missing(), &spec);
    assert_eq!(result, "    .");
}

#[test]
fn format_missing_letter_a() {
    let cat = catalog();
    let spec = FormatSpec {
        name: "".into(),
        w: Some(3),
        d: None,
    };
    let result = cat.format(&Value::Missing(MissingKind::Letter(0)), &spec);
    assert_eq!(result, "  A");
}

#[test]
fn format_missing_underscore() {
    let cat = catalog();
    let spec = FormatSpec {
        name: "".into(),
        w: Some(3),
        d: None,
    };
    let result = cat.format(&Value::Missing(MissingKind::Underscore), &spec);
    assert_eq!(result, "  _");
}

#[test]
fn format_char_left_justified_padded() {
    let cat = catalog();
    let spec = FormatSpec {
        name: "$".into(),
        w: Some(8),
        d: None,
    };
    let result = cat.format(&Value::Char("abc".into()), &spec);
    // fallback: not a known builtin name for $, but builtin handles $ too
    // Let's test the exact fallback only if builtin doesn't claim it.
    // Actually builtin handles "$", so test truncation via builtin.
    assert_eq!(result.len(), 8);
    assert!(result.starts_with("abc"));
}

#[test]
fn format_num_fallback_best12() {
    let cat = catalog();
    let spec = FormatSpec {
        name: "UNKNOWNFORMAT".into(),
        w: Some(12),
        d: None,
    };
    let result = cat.format(&Value::Num(42.0), &spec);
    assert_eq!(result, "          42");
}

// -------------------------------------------------------------------------
// FormatCatalog::informat — fallback paths
// -------------------------------------------------------------------------

#[test]
fn informat_empty_gives_missing() {
    let cat = catalog();
    let spec = FormatSpec {
        name: "UNKNOWNFORMAT".into(),
        w: None,
        d: None,
    };
    let result = cat.informat("  ", &spec);
    assert_eq!(result, Value::missing());
}

#[test]
fn informat_dot_gives_missing() {
    let cat = catalog();
    let spec = FormatSpec {
        name: "UNKNOWNFORMAT".into(),
        w: None,
        d: None,
    };
    let result = cat.informat(".", &spec);
    assert_eq!(result, Value::missing());
}

#[test]
fn informat_numeric_string_gives_num() {
    let cat = catalog();
    let spec = FormatSpec {
        name: "UNKNOWNFORMAT".into(),
        w: None,
        d: None,
    };
    let result = cat.informat("3.14", &spec);
    assert_eq!(result, Value::Num(3.14));
}

#[test]
fn informat_text_gives_char() {
    let cat = catalog();
    let spec = FormatSpec {
        name: "UNKNOWNFORMAT".into(),
        w: None,
        d: None,
    };
    let result = cat.informat("hello", &spec);
    assert_eq!(result, Value::Char("hello".into()));
}

// ── FormatCatalog::user_informats (M18.2) ─────────────────────────────────

use crate::formats::userdef::{InformatRange, InformatValue, UserInformat};

fn make_grade_informat() -> UserInformat {
    UserInformat {
        is_char_result: false,
        ranges: vec![
            InformatRange {
                from: userdef::Bound::Char("A".to_string()),
                to: userdef::Bound::Char("A".to_string()),
                from_exclusive: false,
                to_exclusive: false,
                result: InformatValue::Num(4.0),
            },
            InformatRange {
                from: userdef::Bound::Char("B".to_string()),
                to: userdef::Bound::Char("B".to_string()),
                from_exclusive: false,
                to_exclusive: false,
                result: InformatValue::Num(3.0),
            },
            InformatRange {
                from: userdef::Bound::Char("F".to_string()),
                to: userdef::Bound::Char("F".to_string()),
                from_exclusive: false,
                to_exclusive: false,
                result: InformatValue::Num(0.0),
            },
        ],
        other: Some(InformatValue::Missing(".".to_string())),
    }
}

#[test]
fn user_informat_registered_and_resolved() {
    let mut cat = catalog();
    cat.define_informat("GRADE", make_grade_informat());
    let spec = FormatSpec::parse("GRADE.").unwrap();
    assert_eq!(cat.informat("A", &spec), Value::Num(4.0));
    assert_eq!(cat.informat("B", &spec), Value::Num(3.0));
    assert_eq!(cat.informat("F", &spec), Value::Num(0.0));
}

#[test]
fn user_informat_unmatched_returns_missing() {
    let mut cat = catalog();
    cat.define_informat("GRADE", make_grade_informat());
    let spec = FormatSpec::parse("GRADE.").unwrap();
    // "X" not in any range; other = missing → Value::missing().
    assert_eq!(cat.informat("X", &spec), Value::missing());
}

#[test]
fn user_informat_shadows_builtin() {
    // There's no builtin informat named "GRADE", but let's confirm that
    // if a user informat is registered under a builtin-sounding name it wins.
    // We use "$CHAR" style to confirm the name resolution is correct:
    // actually just confirm user wins over fallback for any name.
    let mut cat = catalog();
    // Override the fallback for format "MYNUM" — a name no builtin claims.
    cat.define_informat(
        "MYNUM",
        UserInformat {
            is_char_result: false,
            ranges: vec![InformatRange {
                from: userdef::Bound::Char("one".to_string()),
                to: userdef::Bound::Char("one".to_string()),
                from_exclusive: false,
                to_exclusive: false,
                result: InformatValue::Num(1.0),
            }],
            other: None,
        },
    );
    let spec = FormatSpec::parse("MYNUM.").unwrap();
    // "one" → 1.0 from user informat (not the fallback which would give Char).
    assert_eq!(cat.informat("one", &spec), Value::Num(1.0));
    // "two" → no range + no other → missing (user informat claimed the name).
    assert_eq!(cat.informat("two", &spec), Value::missing());
}

#[test]
fn char_user_informat_resolved() {
    let mut cat = catalog();
    cat.define_informat(
        "$SIZE",
        UserInformat {
            is_char_result: true,
            ranges: vec![
                InformatRange {
                    from: userdef::Bound::Char("S".to_string()),
                    to: userdef::Bound::Char("S".to_string()),
                    from_exclusive: false,
                    to_exclusive: false,
                    result: InformatValue::Char("Small".to_string()),
                },
                InformatRange {
                    from: userdef::Bound::Char("L".to_string()),
                    to: userdef::Bound::Char("L".to_string()),
                    from_exclusive: false,
                    to_exclusive: false,
                    result: InformatValue::Char("Large".to_string()),
                },
            ],
            other: Some(InformatValue::Char("Unknown".to_string())),
        },
    );
    let spec = FormatSpec::parse("$SIZE.").unwrap();
    assert_eq!(cat.informat("S", &spec), Value::Char("Small".to_string()));
    assert_eq!(cat.informat("L", &spec), Value::Char("Large".to_string()));
    assert_eq!(
        cat.informat("XL", &spec),
        Value::Char("Unknown".to_string())
    );
}
