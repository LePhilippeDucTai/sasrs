use super::super::*;
use super::*;
use crate::value::Value;

// ── SUBSTR ────────────────────────────────────────────────────────────────

#[test]
fn substr_nominal() {
    assert_eq!(
        invoke("SUBSTR", &[chr("Hello"), num(2.0), num(3.0)]),
        chr("ell")
    );
}

#[test]
fn substr_no_length() {
    assert_eq!(invoke("SUBSTR", &[chr("Hello"), num(3.0)]), chr("llo"));
}

#[test]
fn substr_out_of_bounds_flags_error() {
    let mut c = ctx();
    let r = invoke_ctx("SUBSTR", &[chr("abc"), num(0.0)], &mut c);
    assert_eq!(r, chr(""));
    assert!(c.error_flag);
}

// ── INDEX ─────────────────────────────────────────────────────────────────

#[test]
fn index_found() {
    assert_eq!(
        invoke("INDEX", &[chr("Hello World"), chr("World")]),
        num(7.0)
    );
}

#[test]
fn index_not_found() {
    assert_eq!(invoke("INDEX", &[chr("Hello"), chr("xyz")]), num(0.0));
}

// ── CAT / CATS / CATX ────────────────────────────────────────────────────

#[test]
fn cat_concatenates_raw() {
    assert_eq!(
        invoke("CAT", &[chr("Hello "), chr("World")]),
        chr("Hello World")
    );
}

#[test]
fn cats_strips_each() {
    assert_eq!(
        invoke("CATS", &[chr("  Hello  "), chr("  World  ")]),
        chr("HelloWorld")
    );
}

#[test]
fn catx_sep_skips_blank() {
    // CATX("-", "a", "", "c") = "a-c"
    assert_eq!(
        invoke("CATX", &[chr("-"), chr("a"), chr(""), chr("c")]),
        chr("a-c")
    );
}

// ── COMPRESS ──────────────────────────────────────────────────────────────

#[test]
fn compress_default_removes_spaces() {
    assert_eq!(invoke("COMPRESS", &[chr("hello world")]), chr("helloworld"));
}

#[test]
fn compress_custom_chars() {
    assert_eq!(
        invoke("COMPRESS", &[chr("hello123"), chr("123")]),
        chr("hello")
    );
}

// ── TRANWRD ───────────────────────────────────────────────────────────────

#[test]
fn tranwrd_replaces_substring() {
    assert_eq!(
        invoke("TRANWRD", &[chr("Hello World"), chr("World"), chr("Rust")]),
        chr("Hello Rust")
    );
}

// ── SCAN ──────────────────────────────────────────────────────────────────

#[test]
fn scan_first_word() {
    assert_eq!(
        invoke("SCAN", &[chr("hello world foo"), num(1.0)]),
        chr("hello")
    );
}

#[test]
fn scan_second_word() {
    assert_eq!(
        invoke("SCAN", &[chr("hello world foo"), num(2.0)]),
        chr("world")
    );
}

#[test]
fn scan_negative_index_from_end() {
    // n=-1 → last word
    assert_eq!(
        invoke("SCAN", &[chr("hello world foo"), num(-1.0)]),
        chr("foo")
    );
}

#[test]
fn scan_out_of_range() {
    assert_eq!(invoke("SCAN", &[chr("hello world"), num(5.0)]), chr(""));
}

#[test]
fn scan_custom_delim() {
    assert_eq!(
        invoke("SCAN", &[chr("a,b,c"), num(2.0), chr(",")]),
        chr("b")
    );
}

// ── TODAY / DATE ──────────────────────────────────────────────────────────

#[test]
fn today_returns_numeric() {
    let mut c = ctx();
    let r = call("TODAY", &[], &mut c).unwrap();
    // Croise les deux chemins de calcul de date : la valeur de TODAY()
    // redécodée par le chemin JDN doit donner une année plausible
    // (>= 2026, l'horloge ne recule pas) — attrape toute erreur
    // d'offset d'époque 1960/1970.
    match r {
        Value::Num(f) => {
            let (y, _, _) = sas_date_to_ymd(f as i64);
            assert!(y >= 2026, "TODAY() decodes to year {y}");
        }
        _ => panic!("expected numeric"),
    }
    let r2 = call("DATE", &[], &mut c).unwrap();
    assert_eq!(r, r2);
}

// ── MDY ───────────────────────────────────────────────────────────────────

#[test]
fn mdy_nominal() {
    // 1960-01-01 should be day 0.
    let r = invoke("MDY", &[num(1.0), num(1.0), num(1960.0)]);
    assert_eq!(r, num(0.0));
}

#[test]
fn mdy_known_date() {
    // 2000-01-01 = SAS date 14610.
    let r = invoke("MDY", &[num(1.0), num(1.0), num(2000.0)]);
    assert_eq!(r, num(14610.0));
}

#[test]
fn mdy_invalid_date_returns_missing() {
    let mut c = ctx();
    let r = invoke_ctx("MDY", &[num(13.0), num(1.0), num(2000.0)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

// ── YEAR / MONTH / DAY / WEEKDAY ─────────────────────────────────────────

#[test]
fn year_from_sas_date() {
    // 14610 = 2000-01-01
    assert_eq!(invoke("YEAR", &[num(14610.0)]), num(2000.0));
}

#[test]
fn month_from_sas_date() {
    assert_eq!(invoke("MONTH", &[num(14610.0)]), num(1.0));
}

#[test]
fn day_from_sas_date() {
    assert_eq!(invoke("DAY", &[num(14610.0)]), num(1.0));
}

#[test]
fn weekday_sas_date_0_is_friday() {
    // 1960-01-01 = SAS date 0 = Friday = 6 in SAS (Sun=1).
    assert_eq!(invoke("WEEKDAY", &[num(0.0)]), num(6.0));
}

#[test]
fn weekday_known_sunday() {
    // 2000-01-02 = Sunday. SAS date 14611.
    assert_eq!(invoke("WEEKDAY", &[num(14611.0)]), num(1.0));
}

// ── Case insensitivity ────────────────────────────────────────────────────

#[test]
fn function_names_case_insensitive() {
    assert_eq!(invoke("sum", &[num(1.0), num(2.0)]), num(3.0));
    assert_eq!(invoke("Abs", &[num(-3.0)]), num(3.0));
}

// ── PUT / INPUT : délégation au moteur de formats (M4) ────────────────
// Le 2e argument est le TOKEN de format poussé en Value::Char par le
// parser (cf. parse_call dans expr.rs).

#[test]
fn put_dollar_format_returns_char() {
    // PUT(1234.5, dollar10.2) → "$1,234.50".
    let r = invoke("PUT", &[num(1234.5), chr("dollar10.2")]);
    match r {
        Value::Char(s) => assert!(s.contains("$1,234.50"), "expected '$1,234.50' inside {s:?}"),
        _ => panic!("PUT must return character, got {r:?}"),
    }
}

#[test]
fn put_date_format_returns_char() {
    // 2020-01-01 = 21915 jours après 1960-01-01 (croise avec MDY).
    assert_eq!(
        invoke("MDY", &[num(1.0), num(1.0), num(2020.0)]),
        num(21915.0)
    );
    let r = invoke("PUT", &[num(21915.0), chr("date9.")]);
    match r {
        Value::Char(s) => assert!(s.contains("01JAN2020"), "expected '01JAN2020' inside {s:?}"),
        _ => panic!("PUT must return character, got {r:?}"),
    }
}

#[test]
fn put_invalid_format_returns_empty() {
    // Token non parsable → chaîne vide (pas de panique).
    assert_eq!(invoke("PUT", &[num(1.0), chr("")]), chr(""));
}

#[test]
fn put_wrong_arity_returns_empty() {
    assert_eq!(invoke("PUT", &[num(1.0)]), chr(""));
}

#[test]
fn put_user_format_via_function() {
    // PUT(1, sexfmt.) → "Male" using user-defined format.
    use crate::formats::userdef::{Bound, Range, UserFormat};
    let mut cat = crate::formats::FormatCatalog::default();
    cat.define(
        "SEXFMT",
        UserFormat {
            is_char: false,
            ranges: vec![
                Range {
                    from: Bound::Num(1.0),
                    to: Bound::Num(1.0),
                    from_exclusive: false,
                    to_exclusive: false,
                    label: "Male".to_string(),
                },
                Range {
                    from: Bound::Num(2.0),
                    to: Bound::Num(2.0),
                    from_exclusive: false,
                    to_exclusive: false,
                    label: "Female".to_string(),
                },
            ],
            other: Some("Unknown".to_string()),
        },
    );
    let mut c = EvalCtx {
        format_catalog: std::rc::Rc::new(cat),
        ..EvalCtx::default()
    };
    assert_eq!(
        invoke_ctx("PUT", &[num(1.0), chr("sexfmt.")], &mut c),
        chr("Male")
    );
    assert_eq!(
        invoke_ctx("PUT", &[num(2.0), chr("sexfmt.")], &mut c),
        chr("Female")
    );
    assert_eq!(
        invoke_ctx("PUT", &[num(99.0), chr("sexfmt.")], &mut c),
        chr("Unknown")
    );
}

#[test]
fn input_implicit_decimal() {
    // INPUT("123", 5.2) → 1.23 (le `.2` impose 2 décimales implicites).
    assert_eq!(invoke("INPUT", &[chr("123"), chr("5.2")]), num(1.23));
}

#[test]
fn input_date_informat() {
    // INPUT("01JAN2020", date9.) → 21915.
    assert_eq!(
        invoke("INPUT", &[chr("01JAN2020"), chr("date9.")]),
        num(21915.0)
    );
}

#[test]
fn input_wrong_arity_returns_missing() {
    assert_eq!(invoke("INPUT", &[chr("123")]), miss());
}

#[test]
fn input_user_informat_numeric_via_function() {
    // INPUT("A", grade.) → 4.0 using user-defined informat.
    let mut c = make_ctx_with_grade_informat();
    assert_eq!(
        invoke_ctx("INPUT", &[chr("A"), chr("grade.")], &mut c),
        num(4.0)
    );
    assert_eq!(
        invoke_ctx("INPUT", &[chr("B"), chr("grade.")], &mut c),
        num(3.0)
    );
    assert_eq!(
        invoke_ctx("INPUT", &[chr("F"), chr("grade.")], &mut c),
        num(0.0)
    );
}

#[test]
fn input_user_informat_unmatched_returns_missing() {
    // "X" not in grade informat; other=. → missing.
    let mut c = make_ctx_with_grade_informat();
    assert_eq!(
        invoke_ctx("INPUT", &[chr("X"), chr("grade.")], &mut c),
        miss()
    );
}

#[test]
fn input_user_char_informat_via_function() {
    // INPUT("S", $size.) → "Small" using char user-defined informat.
    let mut c = make_ctx_with_size_char_informat();
    assert_eq!(
        invoke_ctx("INPUT", &[chr("S"), chr("$size.")], &mut c),
        chr("Small")
    );
    assert_eq!(
        invoke_ctx("INPUT", &[chr("L"), chr("$size.")], &mut c),
        chr("Large")
    );
    assert_eq!(
        invoke_ctx("INPUT", &[chr("XL"), chr("$size.")], &mut c),
        chr("Unknown")
    );
}

#[test]
fn intck_day_diff() {
    let d1 = sas_day(2020, 1, 1);
    let d2 = sas_day(2020, 1, 11);
    assert_eq!(invoke("INTCK", &[chr("day"), num(d1), num(d2)]), num(10.0));
}

#[test]
fn intck_month() {
    // 15jan2020 → 01mar2020 = 2 month boundaries.
    let d1 = sas_day(2020, 1, 15);
    let d2 = sas_day(2020, 3, 1);
    assert_eq!(invoke("INTCK", &[chr("month"), num(d1), num(d2)]), num(2.0));
}

#[test]
fn intck_qtr() {
    // jan2020 (Q1) → jul2020 (Q3) = 2 quarter boundaries.
    let d1 = sas_day(2020, 1, 15);
    let d2 = sas_day(2020, 7, 1);
    assert_eq!(invoke("INTCK", &[chr("qtr"), num(d1), num(d2)]), num(2.0));
}

#[test]
fn intck_year() {
    let d1 = sas_day(2018, 6, 1);
    let d2 = sas_day(2021, 3, 1);
    assert_eq!(invoke("INTCK", &[chr("year"), num(d1), num(d2)]), num(3.0));
}

#[test]
fn intck_week_boundary() {
    // SAS day 0 = Friday; day 2 (1960-01-03) = Sunday → new SAS week.
    assert_eq!(
        invoke("INTCK", &[chr("week"), num(0.0), num(2.0)]),
        num(1.0)
    );
    // days 0..6 within the SAS week of day 0: day 0 (Fri) → day 1 (Sat)
    // are in the same week (Sunday boundary not crossed).
    assert_eq!(
        invoke("INTCK", &[chr("week"), num(0.0), num(1.0)]),
        num(0.0)
    );
}

#[test]
fn intck_week_negative() {
    // Going backward across a Sunday boundary.
    assert_eq!(
        invoke("INTCK", &[chr("week"), num(2.0), num(0.0)]),
        num(-1.0)
    );
}

#[test]
fn intck_unknown_interval_is_missing() {
    let mut c = ctx();
    let r = invoke_ctx("INTCK", &[chr("fortnight"), num(0.0), num(14.0)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

#[test]
fn intck_missing_date_is_missing() {
    assert_eq!(invoke("INTCK", &[chr("day"), miss(), num(5.0)]), miss());
}
