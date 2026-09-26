//! J03-P4 — tests `multibyte*` : formats/informats caractère coupent et
//! remplissent en CARACTÈRES (contrat D-001, docs/encoding.md). L'ancien
//! `out.truncate(w)` (octets) paniquait sur « byte index is not a char
//! boundary » dès qu'une largeur tombait au milieu d'un caractère.

use super::*;

// ── $w. / $CHARw. / $F ─────────────────────────────────────────────────────

#[test]
fn multibyte_char_format_no_panic_on_accent_boundary() {
    // "Café" = 5 octets ; w=4 tombait au milieu du « é » → panique avant J03-P4.
    let v = Value::Char("Café".into());
    let s = format_builtin(&v, &spec("$CHAR", Some(4), None)).unwrap();
    assert_eq!(s, "Café");
    assert_eq!(s.chars().count(), 4);
}

#[test]
fn multibyte_char_format_truncates_cjk_on_char_boundary() {
    let v = Value::Char("日本語".into());
    let s = format_builtin(&v, &spec("$", Some(2), None)).unwrap();
    assert_eq!(s, "日本");
}

#[test]
fn multibyte_char_format_pads_emoji_to_width() {
    // "😀" = 1 caractère, 4 octets : w=3 → l'emoji + deux espaces.
    let v = Value::Char("😀".into());
    let s = format_builtin(&v, &spec("$F", Some(3), None)).unwrap();
    assert_eq!(s, "😀  ");
    assert_eq!(s.chars().count(), 3);
}

#[test]
fn multibyte_char_format_truncates_emoji_run() {
    let v = Value::Char("😀😀😀".into());
    let s = format_builtin(&v, &spec("$", Some(2), None)).unwrap();
    assert_eq!(s, "😀😀");
}

#[test]
fn multibyte_char_format_preserves_trailing_spaces() {
    let v = Value::Char("ab  ".into());
    let s = format_builtin(&v, &spec("$CHAR", Some(4), None)).unwrap();
    assert_eq!(s, "ab  ");
}

#[test]
fn multibyte_char_format_wider_than_value_pads() {
    let v = Value::Char("Thé".into());
    let s = format_builtin(&v, &spec("$CHAR", Some(6), None)).unwrap();
    assert_eq!(s, "Thé   ");
    assert_eq!(s.chars().count(), 6);
}

// ── $QUOTE ─────────────────────────────────────────────────────────────────

#[test]
fn multibyte_quote_truncates_in_chars() {
    let v = Value::Char("café".into());
    // "\"café\"" = 6 caractères (7 octets) ; w=5 coupe le « é » final.
    let s = format_builtin(&v, &spec("$QUOTE", Some(5), None)).unwrap();
    assert_eq!(s, "\"café");
}

#[test]
fn multibyte_quote_pads_after_closing_quote() {
    let v = Value::Char("café".into());
    let s = format_builtin(&v, &spec("$QUOTE", Some(8), None)).unwrap();
    assert_eq!(s, "\"café\"  ");
}

// ── $HEX ───────────────────────────────────────────────────────────────────

#[test]
fn multibyte_hex_encodes_utf8_bytes_then_fits_in_chars() {
    let v = Value::Char("é".into()); // C3 A9
    let s = format_builtin(&v, &spec("$HEX", None, None)).unwrap();
    assert_eq!(s, "C3A9");
    // Truncation of the (ASCII) hex expansion still cuts at w chars.
    let s = format_builtin(&v, &spec("$HEX", Some(3), None)).unwrap();
    assert_eq!(s, "C3A");
}

// ── $UPCASE ────────────────────────────────────────────────────────────────

#[test]
fn multibyte_upcase_keeps_accent_and_fits_in_chars() {
    let v = Value::Char("café".into());
    let s = format_builtin(&v, &spec("$UPCASE", Some(4), None)).unwrap();
    assert_eq!(s, "CAFÉ");
}

#[test]
fn multibyte_upcase_no_panic_on_cjk_width() {
    let v = Value::Char("日本語".into());
    let s = format_builtin(&v, &spec("$UPCASE", Some(1), None)).unwrap();
    assert_eq!(s, "日");
}

// ── Informats ──────────────────────────────────────────────────────────────

#[test]
fn multibyte_char_informat_truncates_in_chars() {
    // L'ancien `out.truncate(w as usize)` paniquait : « Café au » a 9 octets,
    // w=4 tombait dans le « é ».
    let v = informat_builtin("Café au lait", &spec("$CHAR", Some(4), None)).unwrap();
    assert_eq!(v, Value::Char("Café".into()));
}

#[test]
fn multibyte_char_informat_pads_are_trimmed_not_counted_in_bytes() {
    // Leading blanks are trimmed by the informat, then cut in characters.
    let v = informat_builtin("  日本語です", &spec("$", Some(3), None)).unwrap();
    assert_eq!(v, Value::Char("日本語".into()));
}

#[test]
fn multibyte_date_informat_non_ascii_is_missing_no_panic() {
    // « €€€€€€€€ » = 7 caractères mais 28 octets : passait la garde byte et
    // paniquait sur la découpe [..2]/[2..5]/[5..]. Doit donner missing.
    let v = informat_builtin("€€€€€€€€", &spec("DATE", Some(9), None)).unwrap();
    assert!(matches!(v, Value::Missing(_)));

    // Sanity: a valid ASCII date still parses.
    let ok = informat_builtin("01JAN2020", &spec("DATE", Some(9), None)).unwrap();
    assert!(matches!(ok, Value::Num(_)));
}
