use super::*;
use crate::value::{MissingKind, VarType, Value};

fn make_num_var(name: &str) -> PdvVar {
    PdvVar {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        retained: false,
        from_input: false,
        format: None,
        temporary: false,
    }
}

fn make_char_var(name: &str, length: usize) -> PdvVar {
    PdvVar {
        name: name.to_string(),
        ty: VarType::Char,
        length,
        retained: false,
        from_input: false,
        format: None,
        temporary: false,
    }
}

// -----------------------------------------------------------------------
// new() / basic wiring
// -----------------------------------------------------------------------

#[test]
fn new_pdv_is_empty() {
    let pdv = Pdv::new();
    assert!(pdv.vars().is_empty());
    assert_eq!(pdv.n_, 0);
    assert!(!pdv.error_);
}

// -----------------------------------------------------------------------
// add_var / slot — case-insensitive lookup
// -----------------------------------------------------------------------

#[test]
fn add_var_returns_sequential_slots() {
    let mut pdv = Pdv::new();
    let s0 = pdv.add_var(make_num_var("Age"));
    let s1 = pdv.add_var(make_num_var("Height"));
    assert_eq!(s0, 0);
    assert_eq!(s1, 1);
}

#[test]
fn slot_lookup_is_case_insensitive() {
    let mut pdv = Pdv::new();
    let s = pdv.add_var(make_num_var("MyVar"));
    assert_eq!(pdv.slot("MyVar"), Some(s));
    assert_eq!(pdv.slot("myvar"), Some(s));
    assert_eq!(pdv.slot("MYVAR"), Some(s));
    assert_eq!(pdv.slot("mYvAr"), Some(s));
}

#[test]
fn add_var_preserves_first_name_casing() {
    let mut pdv = Pdv::new();
    pdv.add_var(make_num_var("MyVar"));
    // Second add with different casing — first wins.
    pdv.add_var(make_num_var("MYVAR"));
    assert_eq!(pdv.vars().len(), 1);
    assert_eq!(pdv.vars()[0].name, "MyVar");
}

#[test]
fn add_var_duplicate_returns_existing_slot() {
    let mut pdv = Pdv::new();
    let s1 = pdv.add_var(make_num_var("X"));
    let s2 = pdv.add_var(make_num_var("x")); // duplicate — different case
    assert_eq!(s1, s2);
    assert_eq!(pdv.vars().len(), 1);
}

#[test]
fn slot_unknown_variable_returns_none() {
    let pdv = Pdv::new();
    assert_eq!(pdv.slot("NoSuchVar"), None);
}

// -----------------------------------------------------------------------
// _N_ and _ERROR_ are NOT in the slot index
// -----------------------------------------------------------------------

#[test]
fn automatic_variables_not_in_slot_index() {
    let pdv = Pdv::new();
    assert_eq!(pdv.slot("_N_"), None);
    assert_eq!(pdv.slot("_n_"), None);
    assert_eq!(pdv.slot("_ERROR_"), None);
    assert_eq!(pdv.slot("_error_"), None);
}

// -----------------------------------------------------------------------
// get / initial values
// -----------------------------------------------------------------------

#[test]
fn numeric_var_initialised_to_missing_dot() {
    let mut pdv = Pdv::new();
    let s = pdv.add_var(make_num_var("X"));
    assert_eq!(*pdv.get(s), Value::Missing(MissingKind::Dot));
}

#[test]
fn char_var_initialised_to_empty_string() {
    let mut pdv = Pdv::new();
    let s = pdv.add_var(make_char_var("Name", 10));
    assert_eq!(*pdv.get(s), Value::Char(String::new()));
}

// -----------------------------------------------------------------------
// set() — numeric
// -----------------------------------------------------------------------

#[test]
fn set_numeric_value() {
    let mut pdv = Pdv::new();
    let s = pdv.add_var(make_num_var("Age"));
    pdv.set(s, Value::Num(42.0));
    assert_eq!(*pdv.get(s), Value::Num(42.0));
}

#[test]
fn set_missing_to_numeric_slot() {
    let mut pdv = Pdv::new();
    let s = pdv.add_var(make_num_var("X"));
    pdv.set(s, Value::Num(1.0));
    pdv.set(s, Value::missing());
    assert_eq!(*pdv.get(s), Value::missing());
}

// -----------------------------------------------------------------------
// set() — char truncation (the key SAS semantic)
// -----------------------------------------------------------------------

#[test]
fn set_char_shorter_than_length_stored_as_is() {
    let mut pdv = Pdv::new();
    let s = pdv.add_var(make_char_var("Name", 10));
    pdv.set(s, Value::Char("Alice".to_string()));
    assert_eq!(*pdv.get(s), Value::Char("Alice".to_string()));
}

#[test]
fn set_char_exactly_length_stored_unchanged() {
    let mut pdv = Pdv::new();
    let s = pdv.add_var(make_char_var("Code", 5));
    pdv.set(s, Value::Char("ABCDE".to_string()));
    assert_eq!(*pdv.get(s), Value::Char("ABCDE".to_string()));
}

#[test]
fn set_char_longer_than_length_is_truncated_silently() {
    let mut pdv = Pdv::new();
    // Declared length = 4
    let s = pdv.add_var(make_char_var("Code", 4));
    pdv.set(s, Value::Char("ABCDEFGH".to_string()));
    // Only first 4 chars kept.
    assert_eq!(*pdv.get(s), Value::Char("ABCD".to_string()));
}

#[test]
fn set_char_truncation_counts_unicode_chars_not_bytes() {
    let mut pdv = Pdv::new();
    // 'é' is 2 bytes but 1 char.
    let s = pdv.add_var(make_char_var("X", 3));
    pdv.set(s, Value::Char("éàü!".to_string())); // 4 chars
    assert_eq!(*pdv.get(s), Value::Char("éàü".to_string()));
}

#[test]
fn set_char_trailing_blanks_are_stripped() {
    let mut pdv = Pdv::new();
    let s = pdv.add_var(make_char_var("Name", 10));
    pdv.set(s, Value::Char("Alice   ".to_string()));
    // Stored trimmed (no trailing blanks).
    assert_eq!(*pdv.get(s), Value::Char("Alice".to_string()));
}

#[test]
fn set_char_truncation_then_trailing_blank_strip() {
    let mut pdv = Pdv::new();
    // length = 6, value = "Hello " (6 chars, last is blank)
    let s = pdv.add_var(make_char_var("X", 6));
    pdv.set(s, Value::Char("Hello   IGNORED".to_string())); // truncated to "Hello " then stripped
    assert_eq!(*pdv.get(s), Value::Char("Hello".to_string()));
}

// -----------------------------------------------------------------------
// reset_non_retained()
// -----------------------------------------------------------------------

#[test]
fn reset_resets_plain_num_var_to_missing() {
    let mut pdv = Pdv::new();
    let s = pdv.add_var(make_num_var("X"));
    pdv.set(s, Value::Num(99.0));
    pdv.reset_non_retained();
    assert_eq!(*pdv.get(s), Value::missing());
}

#[test]
fn reset_resets_plain_char_var_to_empty() {
    let mut pdv = Pdv::new();
    let s = pdv.add_var(make_char_var("Name", 10));
    pdv.set(s, Value::Char("Alice".to_string()));
    pdv.reset_non_retained();
    assert_eq!(*pdv.get(s), Value::Char(String::new()));
}

#[test]
fn reset_preserves_retained_var() {
    let mut pdv = Pdv::new();
    let mut var = make_num_var("Total");
    var.retained = true;
    let s = pdv.add_var(var);
    pdv.set(s, Value::Num(42.0));
    pdv.reset_non_retained();
    // Retained — value must survive the reset.
    assert_eq!(*pdv.get(s), Value::Num(42.0));
}

#[test]
fn reset_preserves_from_input_var() {
    let mut pdv = Pdv::new();
    let mut var = make_num_var("Age");
    var.from_input = true;
    let s = pdv.add_var(var);
    pdv.set(s, Value::Num(14.0));
    pdv.reset_non_retained();
    // from_input — SET variable, survives until next read.
    assert_eq!(*pdv.get(s), Value::Num(14.0));
}

#[test]
fn reset_selective_mixed_vars() {
    let mut pdv = Pdv::new();

    // Plain var — must be reset.
    let s_plain = pdv.add_var(make_num_var("Plain"));
    pdv.set(s_plain, Value::Num(1.0));

    // Retained var — must survive.
    let mut retained_var = make_num_var("Retained");
    retained_var.retained = true;
    let s_ret = pdv.add_var(retained_var);
    pdv.set(s_ret, Value::Num(2.0));

    // from_input var — must survive.
    let mut input_var = make_char_var("Name", 20);
    input_var.from_input = true;
    let s_inp = pdv.add_var(input_var);
    pdv.set(s_inp, Value::Char("Bob".to_string()));

    pdv.reset_non_retained();

    assert_eq!(*pdv.get(s_plain), Value::missing());
    assert_eq!(*pdv.get(s_ret), Value::Num(2.0));
    assert_eq!(*pdv.get(s_inp), Value::Char("Bob".to_string()));
}

// -----------------------------------------------------------------------
// vars() order
// -----------------------------------------------------------------------

#[test]
fn vars_order_is_first_reference_order() {
    let mut pdv = Pdv::new();
    pdv.add_var(make_num_var("C"));
    pdv.add_var(make_num_var("A"));
    pdv.add_var(make_num_var("B"));
    let names: Vec<&str> = pdv.vars().iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["C", "A", "B"]);
}

// -----------------------------------------------------------------------
// Automatic fields n_ / error_
// -----------------------------------------------------------------------

#[test]
fn n_and_error_fields_are_independent_of_slot_array() {
    let mut pdv = Pdv::new();
    pdv.n_ = 5;
    pdv.error_ = true;
    assert_eq!(pdv.n_, 5);
    assert!(pdv.error_);
    // They don't appear in vars().
    assert!(pdv.vars().is_empty());
}

// -----------------------------------------------------------------------
// sas_cmp integration: char comparison ignores trailing blanks
// (PDV stores trimmed, so sas_cmp on stored values should still work)
// -----------------------------------------------------------------------

#[test]
fn stored_chars_compare_equal_despite_original_trailing_blanks() {
    use std::cmp::Ordering;
    let mut pdv = Pdv::new();
    let s1 = pdv.add_var(make_char_var("A", 20));
    let s2 = pdv.add_var(make_char_var("B", 20));
    pdv.set(s1, Value::Char("hello   ".to_string()));
    pdv.set(s2, Value::Char("hello".to_string()));
    // Both stored as "hello" (trailing blanks stripped), so equal.
    assert_eq!(pdv.get(s1).sas_cmp(pdv.get(s2)), Ordering::Equal);
}
