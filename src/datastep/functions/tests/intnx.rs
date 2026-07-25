use super::*;

// ── INTNX ─────────────────────────────────────────────────────────────────

#[test]
fn intnx_month_beginning_default() {
    // INTNX('month', 15jan2020, 1) → 01feb2020.
    let start = sas_day(2020, 1, 15);
    assert_eq!(
        invoke("INTNX", &[chr("month"), num(start), num(1.0)]),
        num(sas_day(2020, 2, 1))
    );
}

#[test]
fn intnx_month_same() {
    // INTNX('month', 15jan2020, 1, 'same') → 15feb2020.
    let start = sas_day(2020, 1, 15);
    assert_eq!(
        invoke("INTNX", &[chr("month"), num(start), num(1.0), chr("same")]),
        num(sas_day(2020, 2, 15))
    );
}

#[test]
fn intnx_month_end() {
    // INTNX('month', 15jan2020, 1, 'e') → 29feb2020 (leap year).
    let start = sas_day(2020, 1, 15);
    assert_eq!(
        invoke("INTNX", &[chr("month"), num(start), num(1.0), chr("e")]),
        num(sas_day(2020, 2, 29))
    );
}

#[test]
fn intnx_month_same_clamps_to_last_day() {
    // 31jan2020 + 1 month, 'same' → clamp to 29feb2020.
    let start = sas_day(2020, 1, 31);
    assert_eq!(
        invoke("INTNX", &[chr("month"), num(start), num(1.0), chr("same")]),
        num(sas_day(2020, 2, 29))
    );
}

#[test]
fn intnx_year_end() {
    // INTNX('year', d, 0, 'e') → 31dec of that year.
    let start = sas_day(2020, 5, 17);
    assert_eq!(
        invoke("INTNX", &[chr("year"), num(start), num(0.0), chr("e")]),
        num(sas_day(2020, 12, 31))
    );
}

#[test]
fn intnx_year_beginning() {
    let start = sas_day(2020, 5, 17);
    assert_eq!(
        invoke("INTNX", &[chr("year"), num(start), num(0.0)]),
        num(sas_day(2020, 1, 1))
    );
}

#[test]
fn intnx_qtr_beginning() {
    // 17may2020 is in Q2 (apr-jun); +1 qtr → Q3 → 01jul2020.
    let start = sas_day(2020, 5, 17);
    assert_eq!(
        invoke("INTNX", &[chr("qtr"), num(start), num(1.0)]),
        num(sas_day(2020, 7, 1))
    );
}

#[test]
fn intnx_qtr_end() {
    // Q2 of 2020, 0 increment, end → 30jun2020.
    let start = sas_day(2020, 5, 17);
    assert_eq!(
        invoke("INTNX", &[chr("qtr"), num(start), num(0.0), chr("e")]),
        num(sas_day(2020, 6, 30))
    );
}

#[test]
fn intnx_day() {
    let start = sas_day(2020, 1, 1);
    assert_eq!(
        invoke("INTNX", &[chr("day"), num(start), num(10.0)]),
        num(sas_day(2020, 1, 11))
    );
}

#[test]
fn intnx_week_beginning() {
    // SAS day 0 = Friday; its week begins Sunday day -5 (1959-12-27).
    // +0 weeks, B → day -5.
    assert_eq!(
        invoke("INTNX", &[chr("week"), num(0.0), num(0.0)]),
        num(-5.0)
    );
    // +1 week beginning → next Sunday = day 2 (1960-01-03).
    assert_eq!(
        invoke("INTNX", &[chr("week"), num(0.0), num(1.0)]),
        num(2.0)
    );
}

#[test]
fn intnx_week_same_weekday() {
    // day 0 = Friday; +1 week 'same' → next Friday = day 7.
    assert_eq!(
        invoke("INTNX", &[chr("week"), num(0.0), num(1.0), chr("s")]),
        num(7.0)
    );
}

#[test]
fn intnx_unknown_interval_is_missing() {
    let mut c = ctx();
    let r = invoke_ctx("INTNX", &[chr("decade"), num(0.0), num(1.0)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

#[test]
fn intnx_missing_date_is_missing() {
    assert_eq!(invoke("INTNX", &[chr("month"), miss(), num(1.0)]), miss());
}

// ── FIND ──────────────────────────────────────────────────────────────────

#[test]
fn find_basic() {
    assert_eq!(invoke("FIND", &[chr("hello world"), chr("world")]), num(7.0));
}

#[test]
fn find_not_found() {
    assert_eq!(invoke("FIND", &[chr("hello"), chr("xyz")]), num(0.0));
}

#[test]
fn find_with_start_pos() {
    // Find "o" starting from position 5 in "hello world"
    assert_eq!(invoke("FIND", &[chr("hello world"), chr("o"), num(5.0)]), num(8.0));
}

#[test]
fn find_case_insensitive() {
    assert_eq!(invoke("FIND", &[chr("Hello World"), chr("WORLD"), num(1.0), chr("i")]), num(7.0));
}

#[test]
fn find_empty_target() {
    assert_eq!(invoke("FIND", &[chr("hello"), chr("")]), num(0.0));
}

// ── FINDC ─────────────────────────────────────────────────────────────────

#[test]
fn findc_basic() {
    assert_eq!(invoke("FINDC", &[chr("hello"), chr("lo")]), num(3.0));
}

#[test]
fn findc_not_found() {
    assert_eq!(invoke("FINDC", &[chr("hello"), chr("xyz")]), num(0.0));
}

#[test]
fn findc_with_start_pos() {
    assert_eq!(invoke("FINDC", &[chr("hello"), chr("lo"), num(4.0)]), num(4.0));
}

#[test]
fn findc_case_insensitive() {
    assert_eq!(invoke("FINDC", &[chr("Hello"), chr("EL"), num(1.0), chr("i")]), num(2.0));
}

// ── COUNT ────────────────────────────────────────────────────────────────

#[test]
fn count_basic() {
    assert_eq!(invoke("COUNT", &[chr("hello hello"), chr("hello")]), num(2.0));
}

#[test]
fn count_zero() {
    assert_eq!(invoke("COUNT", &[chr("hello"), chr("xyz")]), num(0.0));
}

#[test]
fn count_overlapping() {
    assert_eq!(invoke("COUNT", &[chr("aaa"), chr("aa")]), num(1.0));
}

#[test]
fn count_case_insensitive() {
    assert_eq!(invoke("COUNT", &[chr("Hello hello"), chr("HELLO"), chr("i")]), num(2.0));
}

// ── COUNTC ───────────────────────────────────────────────────────────────

#[test]
fn countc_basic() {
    assert_eq!(invoke("COUNTC", &[chr("hello"), chr("lo")]), num(3.0));
}

#[test]
fn countc_zero() {
    assert_eq!(invoke("COUNTC", &[chr("hello"), chr("xyz")]), num(0.0));
}

#[test]
fn countc_all_chars_in_set() {
    assert_eq!(invoke("COUNTC", &[chr("aaa"), chr("a")]), num(3.0));
}

#[test]
fn countc_case_insensitive() {
    assert_eq!(invoke("COUNTC", &[chr("Hello"), chr("EL"), chr("i")]), num(3.0));
}

// ── VERIFY ───────────────────────────────────────────────────────────────

#[test]
fn verify_basic() {
    assert_eq!(invoke("VERIFY", &[chr("hello"), chr("helo")]), num(0.0));
}

#[test]
fn verify_first_not_in_set() {
    assert_eq!(invoke("VERIFY", &[chr("xhello"), chr("helo")]), num(1.0));
}

#[test]
fn verify_middle_not_in_set() {
    assert_eq!(invoke("VERIFY", &[chr("hello world"), chr("hello")]), num(6.0));
}

#[test]
fn verify_empty_target() {
    assert_eq!(invoke("VERIFY", &[chr("hello"), chr("")]), num(1.0));
    assert_eq!(invoke("VERIFY", &[chr(""), chr("")]), num(0.0));
}

// ── TRANSLATE ────────────────────────────────────────────────────────────

#[test]
fn translate_basic() {
    assert_eq!(invoke("TRANSLATE", &[chr("hello"), chr("HELLO"), chr("hello")]), chr("HELLO"));
}

#[test]
fn translate_partial_mapping() {
    assert_eq!(invoke("TRANSLATE", &[chr("hello"), chr("12"), chr("he")]), chr("12llo"));
}

#[test]
fn translate_removal() {
    assert_eq!(invoke("TRANSLATE", &[chr("hello"), chr("1"), chr("helo")]), chr("1"));
}

#[test]
fn translate_no_change() {
    assert_eq!(invoke("TRANSLATE", &[chr("hello"), chr("abc"), chr("xyz")]), chr("hello"));
}

// ── REVERSE ───────────────────────────────────────────────────────────────

#[test]
fn reverse_basic() {
    assert_eq!(invoke("REVERSE", &[chr("hello")]), chr("olleh"));
}

#[test]
fn reverse_empty() {
    assert_eq!(invoke("REVERSE", &[chr("")]), chr(""));
}

#[test]
fn reverse_single_char() {
    assert_eq!(invoke("REVERSE", &[chr("a")]), chr("a"));
}

// ── REPEAT ────────────────────────────────────────────────────────────────

#[test]
fn repeat_basic() {
    assert_eq!(invoke("REPEAT", &[chr("ab"), num(3.0)]), chr("ababab"));
}

#[test]
fn repeat_zero_times() {
    assert_eq!(invoke("REPEAT", &[chr("hello"), num(0.0)]), chr(""));
}

#[test]
fn repeat_negative_times() {
    assert_eq!(invoke("REPEAT", &[chr("hello"), num(-5.0)]), chr(""));
}

#[test]
fn repeat_single_time() {
    assert_eq!(invoke("REPEAT", &[chr("hello"), num(1.0)]), chr("hello"));
}

#[test]
fn repeat_truncates_decimal() {
    assert_eq!(invoke("REPEAT", &[chr("a"), num(3.7)]), chr("aaa"));
}

// ── PROPCASE ──────────────────────────────────────────────────────────────

#[test]
fn propcase_basic() {
    assert_eq!(invoke("PROPCASE", &[chr("hello world")]), chr("Hello World"));
}

#[test]
fn propcase_mixed_case() {
    assert_eq!(invoke("PROPCASE", &[chr("HELLO world")]), chr("Hello World"));
}

#[test]
fn propcase_custom_delimiter() {
    assert_eq!(invoke("PROPCASE", &[chr("hello-world"), chr("-")]), chr("Hello-World"));
}

#[test]
fn propcase_empty() {
    assert_eq!(invoke("PROPCASE", &[chr("")]), chr(""));
}

#[test]
fn propcase_single_word() {
    assert_eq!(invoke("PROPCASE", &[chr("hello")]), chr("Hello"));
}

// ── COMPBL ───────────────────────────────────────────────────────────────

#[test]
fn compbl_multiple_spaces() {
    assert_eq!(invoke("COMPBL", &[chr("hello    world")]), chr("hello world"));
}

#[test]
fn compbl_leading_trailing() {
    assert_eq!(invoke("COMPBL", &[chr("  hello world  ")]), chr("hello world"));
}

#[test]
fn compbl_mixed_whitespace() {
    assert_eq!(invoke("COMPBL", &[chr("hello  \t  world")]), chr("hello world"));
}

#[test]
fn compbl_empty() {
    assert_eq!(invoke("COMPBL", &[chr("")]), chr(""));
}

// ── SUBSTRN ───────────────────────────────────────────────────────────────

#[test]
fn substrn_basic() {
    assert_eq!(invoke("SUBSTRN", &[chr("hello"), num(2.0), num(3.0)]), chr("ell"));
}

#[test]
fn substrn_no_length() {
    assert_eq!(invoke("SUBSTRN", &[chr("hello"), num(3.0)]), chr("llo"));
}

#[test]
fn substrn_out_of_bounds_no_error() {
    let mut c = ctx();
    let r = invoke_ctx("SUBSTRN", &[chr("abc"), num(10.0)], &mut c);
    assert_eq!(r, chr(""));
    assert!(!c.error_flag);  // Unlike SUBSTR, no error flag
}

#[test]
fn substrn_pos_zero_no_error() {
    let mut c = ctx();
    let r = invoke_ctx("SUBSTRN", &[chr("abc"), num(0.0)], &mut c);
    assert_eq!(r, chr(""));
    assert!(!c.error_flag);
}

// ── CHAR ──────────────────────────────────────────────────────────────────

#[test]
fn char_ascii() {
    assert_eq!(invoke("CHAR", &[num(65.0)]), chr("A"));
}

#[test]
fn char_space() {
    assert_eq!(invoke("CHAR", &[num(32.0)]), chr(" "));
}

#[test]
fn char_zero() {
    assert_eq!(invoke("CHAR", &[num(0.0)]), chr(""));
}

#[test]
fn char_unicode() {
    assert_eq!(invoke("CHAR", &[num(233.0)]), chr("é"));
}

// ── RANK ──────────────────────────────────────────────────────────────────

#[test]
fn rank_ascii() {
    assert_eq!(invoke("RANK", &[chr("A")]), num(65.0));
}

#[test]
fn rank_space() {
    assert_eq!(invoke("RANK", &[chr(" ")]), num(32.0));
}

#[test]
fn rank_empty() {
    assert_eq!(invoke("RANK", &[chr("")]), num(0.0));
}

#[test]
fn rank_first_char_only() {
    assert_eq!(invoke("RANK", &[chr("ABC")]), num(65.0));
}

#[test]
fn rank_unicode() {
    assert_eq!(invoke("RANK", &[chr("é")]), num(233.0));
}

// ── BYTE ──────────────────────────────────────────────────────────────────

#[test]
fn byte_basic() {
    assert_eq!(invoke("BYTE", &[num(65.0)]), chr("A"));
}

#[test]
fn byte_same_as_char() {
    assert_eq!(invoke("BYTE", &[num(72.0)]), invoke("CHAR", &[num(72.0)]));
}
