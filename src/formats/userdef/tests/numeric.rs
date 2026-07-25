use super::*;

// --- numeric single-value lookup ---

#[test]
fn numeric_single_value_match() {
    let uf = UserFormat {
        is_char: false,
        ranges: vec![num_single(1.0, "Male"), num_single(2.0, "Female")],
        other: None,
    };
    assert_eq!(uf.lookup(&Value::Num(1.0)), Some("Male"));
    assert_eq!(uf.lookup(&Value::Num(2.0)), Some("Female"));
    assert_eq!(uf.lookup(&Value::Num(3.0)), None);
}

#[test]
fn numeric_single_value_list() {
    // Multiple single-value ranges simulate a list like 1,2,3='Group'
    let uf = UserFormat {
        is_char: false,
        ranges: vec![
            num_single(1.0, "Group"),
            num_single(2.0, "Group"),
            num_single(3.0, "Group"),
        ],
        other: Some("Other".to_string()),
    };
    assert_eq!(uf.lookup(&Value::Num(1.0)), Some("Group"));
    assert_eq!(uf.lookup(&Value::Num(2.0)), Some("Group"));
    assert_eq!(uf.lookup(&Value::Num(3.0)), Some("Group"));
    assert_eq!(uf.lookup(&Value::Num(99.0)), Some("Other"));
}

#[test]
fn numeric_inclusive_range() {
    // 1-3 = 'Low'
    let uf = UserFormat {
        is_char: false,
        ranges: vec![num_range(1.0, 3.0, false, false, "Low")],
        other: Some("High".to_string()),
    };
    assert_eq!(uf.lookup(&Value::Num(1.0)), Some("Low"));
    assert_eq!(uf.lookup(&Value::Num(2.5)), Some("Low"));
    assert_eq!(uf.lookup(&Value::Num(3.0)), Some("Low"));
    assert_eq!(uf.lookup(&Value::Num(4.0)), Some("High"));
    assert_eq!(uf.lookup(&Value::Num(0.9)), Some("High"));
}

#[test]
fn numeric_low_to_exclusive_upper() {
    // low -< 5 = 'Below5'
    let uf = UserFormat {
        is_char: false,
        ranges: vec![Range {
            from: Bound::Low,
            to: Bound::Num(5.0),
            from_exclusive: false,
            to_exclusive: true,
            label: "Below5".to_string(),
        }],
        other: Some("AtLeast5".to_string()),
    };
    assert_eq!(uf.lookup(&Value::Num(-100.0)), Some("Below5"));
    assert_eq!(uf.lookup(&Value::Num(4.999)), Some("Below5"));
    assert_eq!(uf.lookup(&Value::Num(5.0)), Some("AtLeast5"));
    assert_eq!(uf.lookup(&Value::Num(6.0)), Some("AtLeast5"));
}

#[test]
fn numeric_exclusive_lower_to_high() {
    // 5 <-high = 'Above5'
    let uf = UserFormat {
        is_char: false,
        ranges: vec![Range {
            from: Bound::Num(5.0),
            to: Bound::High,
            from_exclusive: true,
            to_exclusive: false,
            label: "Above5".to_string(),
        }],
        other: Some("AtMost5".to_string()),
    };
    assert_eq!(uf.lookup(&Value::Num(5.001)), Some("Above5"));
    assert_eq!(uf.lookup(&Value::Num(100.0)), Some("Above5"));
    assert_eq!(uf.lookup(&Value::Num(5.0)), Some("AtMost5"));
    assert_eq!(uf.lookup(&Value::Num(0.0)), Some("AtMost5"));
}

#[test]
fn other_fallback() {
    let uf = UserFormat {
        is_char: false,
        ranges: vec![num_single(1.0, "One")],
        other: Some("Unknown".to_string()),
    };
    assert_eq!(uf.lookup(&Value::Num(99.0)), Some("Unknown"));
}

#[test]
fn missing_value_falls_to_other() {
    let uf = UserFormat {
        is_char: false,
        ranges: vec![num_single(1.0, "One")],
        other: Some("Miss".to_string()),
    };
    assert_eq!(uf.lookup(&Value::missing()), Some("Miss"));
}

#[test]
fn char_single_value_format() {
    let uf = UserFormat {
        is_char: true,
        ranges: vec![
            Range {
                from: Bound::Char("PAR".to_string()),
                to: Bound::Char("PAR".to_string()),
                from_exclusive: false,
                to_exclusive: false,
                label: "Paris".to_string(),
            },
            Range {
                from: Bound::Char("NYC".to_string()),
                to: Bound::Char("NYC".to_string()),
                from_exclusive: false,
                to_exclusive: false,
                label: "New York".to_string(),
            },
        ],
        other: Some("Unknown City".to_string()),
    };
    assert_eq!(uf.lookup(&Value::Char("PAR".to_string())), Some("Paris"));
    assert_eq!(uf.lookup(&Value::Char("NYC".to_string())), Some("New York"));
    assert_eq!(uf.lookup(&Value::Char("LON".to_string())), Some("Unknown City"));
    // trailing-blank insensitive
    assert_eq!(uf.lookup(&Value::Char("PAR   ".to_string())), Some("Paris"));
}

#[test]
fn user_informat_numeric_single_values() {
    // invalue grade 'A'=4 'B'=3 'C'=2 'D'=1 'F'=0;
    let ui = UserInformat {
        is_char_result: false,
        ranges: vec![
            invalue_single("A", InformatValue::Num(4.0)),
            invalue_single("B", InformatValue::Num(3.0)),
            invalue_single("C", InformatValue::Num(2.0)),
            invalue_single("D", InformatValue::Num(1.0)),
            invalue_single("F", InformatValue::Num(0.0)),
        ],
        other: None,
    };
    assert_eq!(ui.lookup("A"), Some(Value::Num(4.0)));
    assert_eq!(ui.lookup("B"), Some(Value::Num(3.0)));
    assert_eq!(ui.lookup("F"), Some(Value::Num(0.0)));
    // Unmatched with no other → None.
    assert_eq!(ui.lookup("X"), None);
}

#[test]
fn user_informat_char_result() {
    // invalue $size 'S'='Small' 'M'='Medium' 'L'='Large';
    let ui = UserInformat {
        is_char_result: true,
        ranges: vec![
            invalue_single("S", InformatValue::Char("Small".to_string())),
            invalue_single("M", InformatValue::Char("Medium".to_string())),
            invalue_single("L", InformatValue::Char("Large".to_string())),
        ],
        other: Some(InformatValue::Char("Unknown".to_string())),
    };
    assert_eq!(ui.lookup("S"), Some(Value::Char("Small".to_string())));
    assert_eq!(ui.lookup("M"), Some(Value::Char("Medium".to_string())));
    assert_eq!(ui.lookup("L"), Some(Value::Char("Large".to_string())));
    assert_eq!(ui.lookup("XL"), Some(Value::Char("Unknown".to_string())));
}

#[test]
fn user_informat_trailing_blanks_ignored() {
    // Keys with trailing blanks should still match (trim_end comparison).
    let ui = UserInformat {
        is_char_result: false,
        ranges: vec![invalue_single("A", InformatValue::Num(4.0))],
        other: None,
    };
    assert_eq!(ui.lookup("A   "), Some(Value::Num(4.0)));
}

#[test]
fn user_informat_range_matching() {
    // invalue score 'A'-'C'=1 'D'-'F'=0;
    let ui = UserInformat {
        is_char_result: false,
        ranges: vec![
            invalue_range("A", "C", InformatValue::Num(1.0)),
            invalue_range("D", "F", InformatValue::Num(0.0)),
        ],
        other: Some(InformatValue::Missing(".".to_string())),
    };
    assert_eq!(ui.lookup("A"), Some(Value::Num(1.0)));
    assert_eq!(ui.lookup("B"), Some(Value::Num(1.0)));
    assert_eq!(ui.lookup("C"), Some(Value::Num(1.0)));
    assert_eq!(ui.lookup("D"), Some(Value::Num(0.0)));
    assert_eq!(ui.lookup("F"), Some(Value::Num(0.0)));
    // "G" doesn't match either range — falls to other (missing).
    let result = ui.lookup("G").unwrap();
    assert_eq!(result, Value::missing());
}

#[test]
fn user_informat_exclusive_range() {
    // 'A'-<'C' → matches A, B but NOT C.
    let ui = UserInformat {
        is_char_result: false,
        ranges: vec![InformatRange {
            from: Bound::Char("A".to_string()),
            to: Bound::Char("C".to_string()),
            from_exclusive: false,
            to_exclusive: true,
            result: InformatValue::Num(1.0),
        }],
        other: Some(InformatValue::Num(0.0)),
    };
    assert_eq!(ui.lookup("A"), Some(Value::Num(1.0)));
    assert_eq!(ui.lookup("B"), Some(Value::Num(1.0)));
    assert_eq!(ui.lookup("C"), Some(Value::Num(0.0))); // excluded
}

#[test]
fn user_informat_low_high_bounds() {
    // low-'M'=1 'N'-high=2
    let ui = UserInformat {
        is_char_result: false,
        ranges: vec![
            InformatRange {
                from: Bound::Low,
                to: Bound::Char("M".to_string()),
                from_exclusive: false,
                to_exclusive: false,
                result: InformatValue::Num(1.0),
            },
            InformatRange {
                from: Bound::Char("N".to_string()),
                to: Bound::High,
                from_exclusive: false,
                to_exclusive: false,
                result: InformatValue::Num(2.0),
            },
        ],
        other: None,
    };
    assert_eq!(ui.lookup("A"), Some(Value::Num(1.0)));
    assert_eq!(ui.lookup("M"), Some(Value::Num(1.0)));
    assert_eq!(ui.lookup("N"), Some(Value::Num(2.0)));
    assert_eq!(ui.lookup("Z"), Some(Value::Num(2.0)));
}

#[test]
fn user_informat_other_fallback() {
    let ui = UserInformat {
        is_char_result: false,
        ranges: vec![invalue_single("A", InformatValue::Num(1.0))],
        other: Some(InformatValue::Num(99.0)),
    };
    assert_eq!(ui.lookup("Z"), Some(Value::Num(99.0)));
}

#[test]
fn user_informat_same_keyword_char_result() {
    // _SAME_ for char informat → copy input verbatim.
    let ui = UserInformat {
        is_char_result: true,
        ranges: vec![InformatRange {
            from: Bound::Low,
            to: Bound::High,
            from_exclusive: false,
            to_exclusive: false,
            result: InformatValue::Same,
        }],
        other: None,
    };
    assert_eq!(ui.lookup("Hello"), Some(Value::Char("Hello".to_string())));
    // Trailing blanks stripped.
    assert_eq!(ui.lookup("Hi  "), Some(Value::Char("Hi".to_string())));
}

#[test]
fn user_informat_same_keyword_num_result() {
    // _SAME_ for numeric informat → parse input as f64.
    let ui = UserInformat {
        is_char_result: false,
        ranges: vec![InformatRange {
            from: Bound::Low,
            to: Bound::High,
            from_exclusive: false,
            to_exclusive: false,
            result: InformatValue::Same,
        }],
        other: None,
    };
    assert_eq!(ui.lookup("42"), Some(Value::Num(42.0)));
    // Non-numeric → missing.
    assert_eq!(ui.lookup("abc"), Some(Value::missing()));
}

#[test]
fn user_informat_missing_result() {
    let ui = UserInformat {
        is_char_result: false,
        ranges: vec![invalue_single(".", InformatValue::Missing(".".to_string()))],
        other: None,
    };
    assert_eq!(ui.lookup("."), Some(Value::missing()));
}

#[test]
fn user_informat_no_match_no_other_returns_none() {
    let ui = UserInformat {
        is_char_result: false,
        ranges: vec![invalue_single("A", InformatValue::Num(1.0))],
        other: None,
    };
    assert_eq!(ui.lookup("Z"), None);
}
