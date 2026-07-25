use super::super::*;
use super::*;

// ── SINH ───────────────────────────────────────────────────────────────────

#[test]
fn sinh_zero() {
    assert_eq!(invoke("SINH", &[num(0.0)]), num(0.0));
}

#[test]
fn sinh_positive() {
    let result = invoke("SINH", &[num(1.0)]);
    assert!(coerce_num(&result, &mut ctx()).unwrap() > 1.0);
}

#[test]
fn sinh_missing() {
    assert_eq!(invoke("SINH", &[miss()]), miss());
}

// ── COSH ───────────────────────────────────────────────────────────────────

#[test]
fn cosh_zero() {
    assert_eq!(invoke("COSH", &[num(0.0)]), num(1.0));
}

#[test]
fn cosh_positive() {
    let result = invoke("COSH", &[num(1.0)]);
    assert!(coerce_num(&result, &mut ctx()).unwrap() > 1.0);
}

#[test]
fn cosh_missing() {
    assert_eq!(invoke("COSH", &[miss()]), miss());
}

// ── TANH ───────────────────────────────────────────────────────────────────

#[test]
fn tanh_zero() {
    assert_eq!(invoke("TANH", &[num(0.0)]), num(0.0));
}

#[test]
fn tanh_large_positive() {
    let result = invoke("TANH", &[num(100.0)]);
    assert!((coerce_num(&result, &mut ctx()).unwrap() - 1.0).abs() < 1e-10);
}

#[test]
fn tanh_missing() {
    assert_eq!(invoke("TANH", &[miss()]), miss());
}

// ── FACT ───────────────────────────────────────────────────────────────────

#[test]
fn fact_five() {
    assert_eq!(invoke("FACT", &[num(5.0)]), num(120.0));
}

#[test]
fn fact_zero() {
    assert_eq!(invoke("FACT", &[num(0.0)]), num(1.0));
}

#[test]
fn fact_non_integer() {
    let mut c = ctx();
    let r = invoke_ctx("FACT", &[num(3.5)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

#[test]
fn fact_negative() {
    let mut c = ctx();
    let r = invoke_ctx("FACT", &[num(-1.0)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

// ── COMB ───────────────────────────────────────────────────────────────────

#[test]
fn comb_basic() {
    // C(5, 2) = 10
    assert_eq!(invoke("COMB", &[num(5.0), num(2.0)]), num(10.0));
}

#[test]
fn comb_k_greater_than_n() {
    // C(3, 5) = 0
    assert_eq!(invoke("COMB", &[num(3.0), num(5.0)]), num(0.0));
}

#[test]
fn comb_k_equals_zero() {
    // C(5, 0) = 1
    assert_eq!(invoke("COMB", &[num(5.0), num(0.0)]), num(1.0));
}

#[test]
fn comb_non_integer() {
    let mut c = ctx();
    let r = invoke_ctx("COMB", &[num(5.0), num(2.5)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

// ── PERM ───────────────────────────────────────────────────────────────────

#[test]
fn perm_basic() {
    // P(5, 2) = 20
    assert_eq!(invoke("PERM", &[num(5.0), num(2.0)]), num(20.0));
}

#[test]
fn perm_k_greater_than_n() {
    // P(3, 5) = 0
    assert_eq!(invoke("PERM", &[num(3.0), num(5.0)]), num(0.0));
}

#[test]
fn perm_k_equals_zero() {
    // P(5, 0) = 1
    assert_eq!(invoke("PERM", &[num(5.0), num(0.0)]), num(1.0));
}

#[test]
fn perm_non_integer() {
    let mut c = ctx();
    let r = invoke_ctx("PERM", &[num(5.0), num(2.5)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

// ── GAMMA ──────────────────────────────────────────────────────────────────

#[test]
fn gamma_one() {
    let result = invoke("GAMMA", &[num(1.0)]);
    let val = coerce_num(&result, &mut ctx()).unwrap();
    assert!((val - 1.0).abs() < 0.001);
}

#[test]
fn gamma_two() {
    let result = invoke("GAMMA", &[num(2.0)]);
    let val = coerce_num(&result, &mut ctx()).unwrap();
    assert!((val - 1.0).abs() < 0.001);
}

#[test]
fn gamma_zero_or_negative_integer() {
    let mut c = ctx();
    let r = invoke_ctx("GAMMA", &[num(0.0)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

#[test]
fn gamma_large_x() {
    let result = invoke("GAMMA", &[num(171.0)]);
    assert_eq!(result, num(f64::INFINITY));
}

// ── LGAMMA ────────────────────────────────────────────────────────────────

#[test]
fn lgamma_one() {
    let result = invoke("LGAMMA", &[num(1.0)]);
    let val = coerce_num(&result, &mut ctx()).unwrap();
    assert!(val.abs() < 0.001);
}

#[test]
fn lgamma_two() {
    let result = invoke("LGAMMA", &[num(2.0)]);
    let val = coerce_num(&result, &mut ctx()).unwrap();
    assert!(val.abs() < 0.001);
}

#[test]
fn lgamma_zero_or_negative_integer() {
    let mut c = ctx();
    let r = invoke_ctx("LGAMMA", &[num(-1.0)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

// ── DIGAMMA ───────────────────────────────────────────────────────────────

#[test]
fn digamma_one() {
    // ψ(1) ≈ -0.5772 (Euler-Mascheroni constant)
    let result = invoke("DIGAMMA", &[num(1.0)]);
    let val = coerce_num(&result, &mut ctx()).unwrap();
    assert!((val - (-0.5772156649)).abs() < 0.001);
}

#[test]
fn digamma_zero_integer() {
    let mut c = ctx();
    let r = invoke_ctx("DIGAMMA", &[num(0.0)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

// ── TRIGAMMA ───────────────────────────────────────────────────────────────

#[test]
fn trigamma_one() {
    // ψ′(1) = π²/6 ≈ 1.6449340668.
    let result = invoke("TRIGAMMA", &[num(1.0)]);
    let val = coerce_num(&result, &mut ctx()).unwrap();
    assert!((val - 1.6449340668).abs() < 0.001);
}

#[test]
fn trigamma_zero_integer() {
    let mut c = ctx();
    let r = invoke_ctx("TRIGAMMA", &[num(0.0)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

// ── BETA ───────────────────────────────────────────────────────────────────

#[test]
fn beta_one_one() {
    let result = invoke("BETA", &[num(1.0), num(1.0)]);
    let val = coerce_num(&result, &mut ctx()).unwrap();
    assert!((val - 1.0).abs() < 0.001);
}

#[test]
fn beta_positive() {
    let result = invoke("BETA", &[num(2.0), num(2.0)]);
    let val = coerce_num(&result, &mut ctx()).unwrap();
    assert!((val - 1.0 / 6.0).abs() < 0.001);
}

#[test]
fn beta_invalid_negative() {
    let mut c = ctx();
    let r = invoke_ctx("BETA", &[num(-1.0), num(1.0)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

// ── ROUNDZ ────────────────────────────────────────────────────────────────

#[test]
fn roundz_tie_toward_zero_positive() {
    // 2.5 should round to 2 (toward zero)
    assert_eq!(invoke("ROUNDZ", &[num(2.5)]), num(2.0));
}

#[test]
fn roundz_tie_toward_zero_negative() {
    // -2.5 should round to -2 (toward zero)
    assert_eq!(invoke("ROUNDZ", &[num(-2.5)]), num(-2.0));
}

#[test]
fn roundz_normal_positive() {
    // 2.3 should round to 2
    assert_eq!(invoke("ROUNDZ", &[num(2.3)]), num(2.0));
}

#[test]
fn roundz_with_unit() {
    // 2.55 with unit 0.1 should round to 2.5
    assert_eq!(invoke("ROUNDZ", &[num(2.55), num(0.1)]), num(2.5));
}

// ── RANGE ─────────────────────────────────────────────────────────────────

#[test]
fn range_basic() {
    assert_eq!(invoke("RANGE", &[num(1.0), num(5.0), num(3.0)]), num(4.0));
}

#[test]
fn range_ignores_missing() {
    assert_eq!(invoke("RANGE", &[miss(), num(1.0), num(5.0)]), num(4.0));
}

#[test]
fn range_all_missing() {
    assert_eq!(invoke("RANGE", &[miss(), miss()]), miss());
}

#[test]
fn range_negative() {
    assert_eq!(invoke("RANGE", &[num(-5.0), num(3.0)]), num(8.0));
}

// ── LARGEST ───────────────────────────────────────────────────────────────

#[test]
fn largest_second() {
    // 2nd largest of (3, 1, 5, 2)
    assert_eq!(invoke("LARGEST", &[num(2.0), num(3.0), num(1.0), num(5.0), num(2.0)]), num(3.0));
}

#[test]
fn largest_first() {
    // 1st largest of (3, 1, 5)
    assert_eq!(invoke("LARGEST", &[num(1.0), num(3.0), num(1.0), num(5.0)]), num(5.0));
}

#[test]
fn largest_out_of_range() {
    // 10th largest of only 3 values
    assert_eq!(invoke("LARGEST", &[num(10.0), num(1.0), num(2.0), num(3.0)]), miss());
}

#[test]
fn largest_k_zero() {
    assert_eq!(invoke("LARGEST", &[num(0.0), num(1.0), num(2.0)]), miss());
}

// ── SMALLEST ──────────────────────────────────────────────────────────────

#[test]
fn smallest_second() {
    // 2nd smallest of (3, 1, 5, 2)
    assert_eq!(invoke("SMALLEST", &[num(2.0), num(3.0), num(1.0), num(5.0), num(2.0)]), num(2.0));
}

#[test]
fn smallest_first() {
    // 1st smallest of (3, 1, 5)
    assert_eq!(invoke("SMALLEST", &[num(1.0), num(3.0), num(1.0), num(5.0)]), num(1.0));
}

#[test]
fn smallest_out_of_range() {
    // 10th smallest of only 3 values
    assert_eq!(invoke("SMALLEST", &[num(10.0), num(1.0), num(2.0), num(3.0)]), miss());
}

#[test]
fn smallest_k_negative() {
    assert_eq!(invoke("SMALLEST", &[num(-1.0), num(1.0), num(2.0)]), miss());
}

// ── ORDINAL ───────────────────────────────────────────────────────────────

#[test]
fn ordinal_first() {
    assert_eq!(invoke("ORDINAL", &[num(1.0)]), chr("1st"));
}

#[test]
fn ordinal_second() {
    assert_eq!(invoke("ORDINAL", &[num(2.0)]), chr("2nd"));
}

#[test]
fn ordinal_third() {
    assert_eq!(invoke("ORDINAL", &[num(3.0)]), chr("3rd"));
}

#[test]
fn ordinal_fourth() {
    assert_eq!(invoke("ORDINAL", &[num(4.0)]), chr("4th"));
}

#[test]
fn ordinal_eleventh() {
    assert_eq!(invoke("ORDINAL", &[num(11.0)]), chr("11th"));
}

#[test]
fn ordinal_twelfth() {
    assert_eq!(invoke("ORDINAL", &[num(12.0)]), chr("12th"));
}

#[test]
fn ordinal_thirteenth() {
    assert_eq!(invoke("ORDINAL", &[num(13.0)]), chr("13th"));
}

#[test]
fn ordinal_twenty_first() {
    assert_eq!(invoke("ORDINAL", &[num(21.0)]), chr("21st"));
}

#[test]
fn ordinal_non_integer() {
    assert_eq!(invoke("ORDINAL", &[num(3.5)]), chr(""));
}

// ── UPCASE / LOWCASE ──────────────────────────────────────────────────────

#[test]
fn upcase_nominal() {
    assert_eq!(invoke("UPCASE", &[chr("hello")]), chr("HELLO"));
}

#[test]
fn lowcase_nominal() {
    assert_eq!(invoke("LOWCASE", &[chr("HELLO")]), chr("hello"));
}

// ── TRIM ──────────────────────────────────────────────────────────────────

#[test]
fn trim_trailing_blanks() {
    assert_eq!(invoke("TRIM", &[chr("hello   ")]), chr("hello"));
}

#[test]
fn trim_all_blank_becomes_empty() {
    assert_eq!(invoke("TRIM", &[chr("   ")]), chr(""));
}

// ── STRIP ─────────────────────────────────────────────────────────────────

#[test]
fn strip_both_ends() {
    assert_eq!(invoke("STRIP", &[chr("  hello  ")]), chr("hello"));
}

// ── LEFT ──────────────────────────────────────────────────────────────────

#[test]
fn left_removes_leading_blanks() {
    assert_eq!(invoke("LEFT", &[chr("  hello")]), chr("hello"));
}

// ── LENGTH ────────────────────────────────────────────────────────────────

#[test]
fn length_without_trailing_blanks() {
    assert_eq!(invoke("LENGTH", &[chr("hello   ")]), num(5.0));
}

#[test]
fn length_blank_string_min_one() {
    assert_eq!(invoke("LENGTH", &[chr("")]), num(1.0));
    assert_eq!(invoke("LENGTH", &[chr("   ")]), num(1.0));
}
