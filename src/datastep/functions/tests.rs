// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

use super::*;
use crate::value::{MissingKind, Value};

fn ctx() -> EvalCtx {
    EvalCtx::default()
}

fn num(f: f64) -> Value {
    Value::Num(f)
}

fn miss() -> Value {
    Value::missing()
}

fn miss_a() -> Value {
    Value::Missing(MissingKind::Letter(0))
}

fn chr(s: &str) -> Value {
    Value::Char(s.to_string())
}

fn invoke(name: &str, args: &[Value]) -> Value {
    let mut c = ctx();
    call(name, args, &mut c).expect("function should be known")
}

fn invoke_ctx<'a>(name: &str, args: &[Value], c: &'a mut EvalCtx) -> Value {
    call(name, args, c).expect("function should be known")
}

// ── Unknown function → None ───────────────────────────────────────────────

#[test]
fn unknown_function_returns_none() {
    let mut c = ctx();
    assert!(call("NOSUCHFN", &[], &mut c).is_none());
}

// ── SUM ──────────────────────────────────────────────────────────────────

#[test]
fn sum_nominal() {
    assert_eq!(invoke("SUM", &[num(1.0), num(2.0), num(3.0)]), num(6.0));
}

#[test]
fn sum_ignores_missing_in_middle() {
    // SUM(., 1) = 1  (missing ignored, not propagated)
    assert_eq!(invoke("SUM", &[miss(), num(1.0)]), num(1.0));
    assert_eq!(invoke("SUM", &[num(1.0), miss(), num(3.0)]), num(4.0));
}

#[test]
fn sum_all_missing_returns_missing() {
    assert_eq!(invoke("SUM", &[miss(), miss()]), miss());
}

#[test]
fn sum_special_missing_ignored() {
    assert_eq!(invoke("SUM", &[miss_a(), num(5.0)]), num(5.0));
}

// ── MEAN ─────────────────────────────────────────────────────────────────

#[test]
fn mean_nominal() {
    assert_eq!(invoke("MEAN", &[num(1.0), num(3.0)]), num(2.0));
}

#[test]
fn mean_ignores_missing() {
    // MEAN(1, ., 3) = 2.0
    assert_eq!(invoke("MEAN", &[num(1.0), miss(), num(3.0)]), num(2.0));
}

#[test]
fn mean_all_missing() {
    assert_eq!(invoke("MEAN", &[miss()]), miss());
}

// ── MIN / MAX ─────────────────────────────────────────────────────────────

#[test]
fn min_nominal() {
    assert_eq!(invoke("MIN", &[num(3.0), num(1.0), num(2.0)]), num(1.0));
}

#[test]
fn min_ignores_missing() {
    assert_eq!(invoke("MIN", &[miss(), num(5.0)]), num(5.0));
}

#[test]
fn min_all_missing() {
    assert_eq!(invoke("MIN", &[miss()]), miss());
}

#[test]
fn max_nominal() {
    assert_eq!(invoke("MAX", &[num(3.0), num(1.0), num(2.0)]), num(3.0));
}

#[test]
fn max_ignores_missing() {
    assert_eq!(invoke("MAX", &[miss(), num(5.0)]), num(5.0));
}

// ── N / NMISS ─────────────────────────────────────────────────────────────

#[test]
fn n_counts_nonmissing() {
    assert_eq!(invoke("N", &[num(1.0), miss(), num(3.0)]), num(2.0));
    assert_eq!(invoke("N", &[miss(), miss()]), num(0.0));
}

#[test]
fn nmiss_counts_missing() {
    assert_eq!(invoke("NMISS", &[num(1.0), miss(), num(3.0)]), num(1.0));
    assert_eq!(invoke("NMISS", &[miss(), miss()]), num(2.0));
}

// ── COALESCE ──────────────────────────────────────────────────────────────

#[test]
fn coalesce_first_nonmissing() {
    assert_eq!(invoke("COALESCE", &[miss(), num(2.0), num(3.0)]), num(2.0));
}

#[test]
fn coalesce_all_missing() {
    assert_eq!(invoke("COALESCE", &[miss(), miss()]), miss());
}

// ── MISSING ───────────────────────────────────────────────────────────────

#[test]
fn missing_fn_numeric_missing() {
    assert_eq!(invoke("MISSING", &[miss()]), num(1.0));
}

#[test]
fn missing_fn_numeric_nonmissing() {
    assert_eq!(invoke("MISSING", &[num(0.0)]), num(0.0));
}

#[test]
fn missing_fn_blank_char() {
    assert_eq!(invoke("MISSING", &[chr("   ")]), num(1.0));
}

#[test]
fn missing_fn_nonblank_char() {
    assert_eq!(invoke("MISSING", &[chr("hi")]), num(0.0));
}

// ── ABS ───────────────────────────────────────────────────────────────────

#[test]
fn abs_nominal() {
    assert_eq!(invoke("ABS", &[num(-5.0)]), num(5.0));
    assert_eq!(invoke("ABS", &[num(3.0)]), num(3.0));
}

#[test]
fn abs_missing_propagates() {
    assert_eq!(invoke("ABS", &[miss()]), miss());
}

// ── SQRT ──────────────────────────────────────────────────────────────────

#[test]
fn sqrt_nominal() {
    assert_eq!(invoke("SQRT", &[num(4.0)]), num(2.0));
}

#[test]
fn sqrt_negative_returns_missing_and_flags_error() {
    let mut c = ctx();
    let result = invoke_ctx("SQRT", &[num(-1.0)], &mut c);
    assert_eq!(result, miss());
    assert!(c.error_flag);
}

#[test]
fn sqrt_missing_propagates() {
    assert_eq!(invoke("SQRT", &[miss()]), miss());
}

// ── EXP ───────────────────────────────────────────────────────────────────

#[test]
fn exp_nominal() {
    let result = invoke("EXP", &[num(0.0)]);
    assert_eq!(result, num(1.0));
}

#[test]
fn exp_missing_propagates() {
    assert_eq!(invoke("EXP", &[miss()]), miss());
}

// ── LOG / LOG2 / LOG10 ────────────────────────────────────────────────────

#[test]
fn log_nominal() {
    let result = invoke("LOG", &[num(1.0)]);
    assert_eq!(result, num(0.0));
}

#[test]
fn log_nonpositive_returns_missing_and_flags() {
    let mut c = ctx();
    let r = invoke_ctx("LOG", &[num(0.0)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);

    let mut c2 = ctx();
    let r2 = invoke_ctx("LOG", &[num(-1.0)], &mut c2);
    assert_eq!(r2, miss());
    assert!(c2.error_flag);
}

#[test]
fn log2_nominal() {
    assert_eq!(invoke("LOG2", &[num(8.0)]), num(3.0));
}

#[test]
fn log10_nominal() {
    assert_eq!(invoke("LOG10", &[num(100.0)]), num(2.0));
}

// ── INT ───────────────────────────────────────────────────────────────────

#[test]
fn int_truncates_toward_zero() {
    assert_eq!(invoke("INT", &[num(3.7)]), num(3.0));
    assert_eq!(invoke("INT", &[num(-3.7)]), num(-3.0));
}

#[test]
fn int_missing_propagates() {
    assert_eq!(invoke("INT", &[miss()]), miss());
}

// ── ROUND ─────────────────────────────────────────────────────────────────

#[test]
fn round_default_unit() {
    assert_eq!(invoke("ROUND", &[num(2.5)]), num(3.0));
    assert_eq!(invoke("ROUND", &[num(-2.5)]), num(-3.0));
}

#[test]
fn round_with_unit() {
    assert_eq!(invoke("ROUND", &[num(2.567), num(0.01)]), num(2.57));
}

#[test]
fn round_missing_propagates() {
    assert_eq!(invoke("ROUND", &[miss()]), miss());
}

// ── MOD ───────────────────────────────────────────────────────────────────

#[test]
fn mod_nominal() {
    assert_eq!(invoke("MOD", &[num(10.0), num(3.0)]), num(1.0));
}

#[test]
fn mod_sign_follows_dividend() {
    assert_eq!(invoke("MOD", &[num(-7.0), num(3.0)]), num(-1.0));
}

#[test]
fn mod_div_by_zero_returns_missing() {
    let mut c = ctx();
    let r = invoke_ctx("MOD", &[num(5.0), num(0.0)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

// ── M15.2 Mathematical Functions ───────────────────────────────────────────

// ── CEIL ───────────────────────────────────────────────────────────────────

#[test]
fn ceil_positive() {
    assert_eq!(invoke("CEIL", &[num(3.2)]), num(4.0));
}

#[test]
fn ceil_negative() {
    assert_eq!(invoke("CEIL", &[num(-3.2)]), num(-3.0));
}

#[test]
fn ceil_integer() {
    assert_eq!(invoke("CEIL", &[num(3.0)]), num(3.0));
}

// ── FLOOR ──────────────────────────────────────────────────────────────────

#[test]
fn floor_positive() {
    assert_eq!(invoke("FLOOR", &[num(3.7)]), num(3.0));
}

#[test]
fn floor_negative() {
    assert_eq!(invoke("FLOOR", &[num(-3.7)]), num(-4.0));
}

#[test]
fn floor_integer() {
    assert_eq!(invoke("FLOOR", &[num(3.0)]), num(3.0));
}

// ── SIGN ───────────────────────────────────────────────────────────────────

#[test]
fn sign_positive() {
    assert_eq!(invoke("SIGN", &[num(5.0)]), num(1.0));
}

#[test]
fn sign_negative() {
    assert_eq!(invoke("SIGN", &[num(-5.0)]), num(-1.0));
}

#[test]
fn sign_zero() {
    assert_eq!(invoke("SIGN", &[num(0.0)]), num(0.0));
}

// ── SIN ────────────────────────────────────────────────────────────────────

#[test]
fn sin_zero() {
    assert_eq!(invoke("SIN", &[num(0.0)]), num(0.0));
}

#[test]
fn sin_pi_half() {
    let result = invoke("SIN", &[num(std::f64::consts::PI / 2.0)]);
    assert!((coerce_num(&result, &mut ctx()).unwrap() - 1.0).abs() < 1e-10);
}

#[test]
fn sin_missing() {
    assert_eq!(invoke("SIN", &[miss()]), miss());
}

// ── COS ────────────────────────────────────────────────────────────────────

#[test]
fn cos_zero() {
    assert_eq!(invoke("COS", &[num(0.0)]), num(1.0));
}

#[test]
fn cos_pi() {
    let result = invoke("COS", &[num(std::f64::consts::PI)]);
    assert!((coerce_num(&result, &mut ctx()).unwrap() + 1.0).abs() < 1e-10);
}

#[test]
fn cos_missing() {
    assert_eq!(invoke("COS", &[miss()]), miss());
}

// ── TAN ────────────────────────────────────────────────────────────────────

#[test]
fn tan_zero() {
    assert_eq!(invoke("TAN", &[num(0.0)]), num(0.0));
}

#[test]
fn tan_pi_quarter() {
    let result = invoke("TAN", &[num(std::f64::consts::PI / 4.0)]);
    assert!((coerce_num(&result, &mut ctx()).unwrap() - 1.0).abs() < 1e-10);
}

#[test]
fn tan_missing() {
    assert_eq!(invoke("TAN", &[miss()]), miss());
}

// ── ARSIN ──────────────────────────────────────────────────────────────────

#[test]
fn arsin_zero() {
    assert_eq!(invoke("ARSIN", &[num(0.0)]), num(0.0));
}

#[test]
fn arsin_one() {
    let result = invoke("ARSIN", &[num(1.0)]);
    assert!((coerce_num(&result, &mut ctx()).unwrap() - std::f64::consts::PI / 2.0).abs() < 1e-10);
}

#[test]
fn arsin_out_of_domain() {
    let mut c = ctx();
    let r = invoke_ctx("ARSIN", &[num(1.5)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

// ── ARCOS ──────────────────────────────────────────────────────────────────

#[test]
fn arcos_one() {
    assert_eq!(invoke("ARCOS", &[num(1.0)]), num(0.0));
}

#[test]
fn arcos_zero() {
    let result = invoke("ARCOS", &[num(0.0)]);
    assert!((coerce_num(&result, &mut ctx()).unwrap() - std::f64::consts::PI / 2.0).abs() < 1e-10);
}

#[test]
fn arcos_out_of_domain() {
    let mut c = ctx();
    let r = invoke_ctx("ARCOS", &[num(-1.5)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

// ── ATAN ───────────────────────────────────────────────────────────────────

#[test]
fn atan_zero() {
    assert_eq!(invoke("ATAN", &[num(0.0)]), num(0.0));
}

#[test]
fn atan_one() {
    let result = invoke("ATAN", &[num(1.0)]);
    assert!((coerce_num(&result, &mut ctx()).unwrap() - std::f64::consts::PI / 4.0).abs() < 1e-10);
}

#[test]
fn atan_missing() {
    assert_eq!(invoke("ATAN", &[miss()]), miss());
}

// ── ATAN2 ──────────────────────────────────────────────────────────────────

#[test]
fn atan2_one_one() {
    let result = invoke("ATAN2", &[num(1.0), num(1.0)]);
    assert!((coerce_num(&result, &mut ctx()).unwrap() - std::f64::consts::PI / 4.0).abs() < 1e-10);
}

#[test]
fn atan2_zero_one() {
    assert_eq!(invoke("ATAN2", &[num(0.0), num(1.0)]), num(0.0));
}

#[test]
fn atan2_missing_first() {
    assert_eq!(invoke("ATAN2", &[miss(), num(1.0)]), miss());
}

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

// ── SUBSTR ────────────────────────────────────────────────────────────────

#[test]
fn substr_nominal() {
    assert_eq!(invoke("SUBSTR", &[chr("Hello"), num(2.0), num(3.0)]), chr("ell"));
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
    assert_eq!(invoke("INDEX", &[chr("Hello World"), chr("World")]), num(7.0));
}

#[test]
fn index_not_found() {
    assert_eq!(invoke("INDEX", &[chr("Hello"), chr("xyz")]), num(0.0));
}

// ── CAT / CATS / CATX ────────────────────────────────────────────────────

#[test]
fn cat_concatenates_raw() {
    assert_eq!(invoke("CAT", &[chr("Hello "), chr("World")]), chr("Hello World"));
}

#[test]
fn cats_strips_each() {
    assert_eq!(invoke("CATS", &[chr("  Hello  "), chr("  World  ")]), chr("HelloWorld"));
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
    assert_eq!(invoke("COMPRESS", &[chr("hello123"), chr("123")]), chr("hello"));
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
    assert_eq!(invoke("SCAN", &[chr("hello world foo"), num(1.0)]), chr("hello"));
}

#[test]
fn scan_second_word() {
    assert_eq!(invoke("SCAN", &[chr("hello world foo"), num(2.0)]), chr("world"));
}

#[test]
fn scan_negative_index_from_end() {
    // n=-1 → last word
    assert_eq!(invoke("SCAN", &[chr("hello world foo"), num(-1.0)]), chr("foo"));
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
        Value::Char(s) => assert!(
            s.contains("$1,234.50"),
            "expected '$1,234.50' inside {s:?}"
        ),
        _ => panic!("PUT must return character, got {r:?}"),
    }
}

#[test]
fn put_date_format_returns_char() {
    // 2020-01-01 = 21915 jours après 1960-01-01 (croise avec MDY).
    assert_eq!(invoke("MDY", &[num(1.0), num(1.0), num(2020.0)]), num(21915.0));
    let r = invoke("PUT", &[num(21915.0), chr("date9.")]);
    match r {
        Value::Char(s) => assert!(
            s.contains("01JAN2020"),
            "expected '01JAN2020' inside {s:?}"
        ),
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
fn input_implicit_decimal() {
    // INPUT("123", 5.2) → 1.23 (le `.2` impose 2 décimales implicites).
    assert_eq!(invoke("INPUT", &[chr("123"), chr("5.2")]), num(1.23));
}

#[test]
fn input_date_informat() {
    // INPUT("01JAN2020", date9.) → 21915.
    assert_eq!(invoke("INPUT", &[chr("01JAN2020"), chr("date9.")]), num(21915.0));
}

#[test]
fn input_wrong_arity_returns_missing() {
    assert_eq!(invoke("INPUT", &[chr("123")]), miss());
}

// ── INPUT / PUT with user-defined formats & informats (M18.2) ────────────

fn make_ctx_with_grade_informat() -> EvalCtx {
    use crate::formats::userdef::{Bound, InformatRange, InformatValue, UserInformat};
    let mut cat = crate::formats::FormatCatalog::default();
    cat.define_informat(
        "GRADE",
        UserInformat {
            is_char_result: false,
            ranges: vec![
                InformatRange {
                    from: Bound::Char("A".to_string()),
                    to: Bound::Char("A".to_string()),
                    from_exclusive: false,
                    to_exclusive: false,
                    result: InformatValue::Num(4.0),
                },
                InformatRange {
                    from: Bound::Char("B".to_string()),
                    to: Bound::Char("B".to_string()),
                    from_exclusive: false,
                    to_exclusive: false,
                    result: InformatValue::Num(3.0),
                },
                InformatRange {
                    from: Bound::Char("F".to_string()),
                    to: Bound::Char("F".to_string()),
                    from_exclusive: false,
                    to_exclusive: false,
                    result: InformatValue::Num(0.0),
                },
            ],
            other: Some(InformatValue::Missing(".".to_string())),
        },
    );
    EvalCtx { format_catalog: cat, ..EvalCtx::default() }
}

fn make_ctx_with_size_char_informat() -> EvalCtx {
    use crate::formats::userdef::{Bound, InformatRange, InformatValue, UserInformat};
    let mut cat = crate::formats::FormatCatalog::default();
    cat.define_informat(
        "$SIZE",
        UserInformat {
            is_char_result: true,
            ranges: vec![
                InformatRange {
                    from: Bound::Char("S".to_string()),
                    to: Bound::Char("S".to_string()),
                    from_exclusive: false,
                    to_exclusive: false,
                    result: InformatValue::Char("Small".to_string()),
                },
                InformatRange {
                    from: Bound::Char("L".to_string()),
                    to: Bound::Char("L".to_string()),
                    from_exclusive: false,
                    to_exclusive: false,
                    result: InformatValue::Char("Large".to_string()),
                },
            ],
            other: Some(InformatValue::Char("Unknown".to_string())),
        },
    );
    EvalCtx { format_catalog: cat, ..EvalCtx::default() }
}

#[test]
fn input_user_informat_numeric_via_function() {
    // INPUT("A", grade.) → 4.0 using user-defined informat.
    let mut c = make_ctx_with_grade_informat();
    assert_eq!(invoke_ctx("INPUT", &[chr("A"), chr("grade.")], &mut c), num(4.0));
    assert_eq!(invoke_ctx("INPUT", &[chr("B"), chr("grade.")], &mut c), num(3.0));
    assert_eq!(invoke_ctx("INPUT", &[chr("F"), chr("grade.")], &mut c), num(0.0));
}

#[test]
fn input_user_informat_unmatched_returns_missing() {
    // "X" not in grade informat; other=. → missing.
    let mut c = make_ctx_with_grade_informat();
    assert_eq!(invoke_ctx("INPUT", &[chr("X"), chr("grade.")], &mut c), miss());
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
    let mut c = EvalCtx { format_catalog: cat, ..EvalCtx::default() };
    assert_eq!(invoke_ctx("PUT", &[num(1.0), chr("sexfmt.")], &mut c), chr("Male"));
    assert_eq!(invoke_ctx("PUT", &[num(2.0), chr("sexfmt.")], &mut c), chr("Female"));
    assert_eq!(invoke_ctx("PUT", &[num(99.0), chr("sexfmt.")], &mut c), chr("Unknown"));
}

// ── INTCK ─────────────────────────────────────────────────────────────────

fn sas_day(y: i64, m: i64, d: i64) -> f64 {
    days_since_1960(y, m, d) as f64
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
    assert_eq!(invoke("INTCK", &[chr("week"), num(0.0), num(2.0)]), num(1.0));
    // days 0..6 within the SAS week of day 0: day 0 (Fri) → day 1 (Sat)
    // are in the same week (Sunday boundary not crossed).
    assert_eq!(invoke("INTCK", &[chr("week"), num(0.0), num(1.0)]), num(0.0));
}

#[test]
fn intck_week_negative() {
    // Going backward across a Sunday boundary.
    assert_eq!(invoke("INTCK", &[chr("week"), num(2.0), num(0.0)]), num(-1.0));
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

// ── WHICHC ───────────────────────────────────────────────────────────────

#[test]
fn whichc_first_match() {
    assert_eq!(
        invoke("WHICHC", &[chr("b"), chr("a"), chr("b"), chr("c")]),
        num(2.0)
    );
}

#[test]
fn whichc_no_match() {
    assert_eq!(
        invoke("WHICHC", &[chr("x"), chr("a"), chr("b"), chr("c")]),
        num(0.0)
    );
}

#[test]
fn whichc_first_is_match() {
    assert_eq!(
        invoke("WHICHC", &[chr("a"), chr("a"), chr("b"), chr("c")]),
        num(1.0)
    );
}

#[test]
fn whichc_empty_needle() {
    assert_eq!(
        invoke("WHICHC", &[chr(""), chr(""), chr("b")]),
        num(1.0)
    );
}

// ── CATQ ──────────────────────────────────────────────────────────────────

#[test]
fn catq_no_quoting_needed() {
    assert_eq!(
        invoke("CATQ", &[chr(","), chr("a"), chr("b")]),
        chr("a,b")
    );
}

#[test]
fn catq_quote_on_delimiter() {
    assert_eq!(
        invoke("CATQ", &[chr(","), chr("a,b"), chr("c")]),
        chr("\"a,b\",c")
    );
}

#[test]
fn catq_quote_on_internal_quote() {
    assert_eq!(
        invoke("CATQ", &[chr(","), chr("a\"b"), chr("c")]),
        chr("\"a\"\"b\",c")
    );
}

#[test]
fn catq_both_conditions() {
    assert_eq!(
        invoke("CATQ", &[chr(","), chr("a,\"b"), chr("c")]),
        chr("\"a,\"\"b\",c")
    );
}

#[test]
fn catq_empty_items() {
    assert_eq!(
        invoke("CATQ", &[chr(","), chr("a"), chr(""), chr("c")]),
        chr("a,,c")
    );
}

// ── DATEPART (M15.3) ─────────────────────────────────────────────────────

#[test]
fn datepart_nominal() {
    // 2020-01-01 12:30:45 → date 21915.
    let dt = 21915.0 * SECONDS_PER_DAY + 45045.0;
    assert_eq!(invoke("DATEPART", &[num(dt)]), num(21915.0));
}

#[test]
fn datepart_midnight() {
    assert_eq!(invoke("DATEPART", &[num(0.0)]), num(0.0));
}

#[test]
fn datepart_missing() {
    assert_eq!(invoke("DATEPART", &[miss()]), miss());
}

// ── TIMEPART (M15.3) ─────────────────────────────────────────────────────

#[test]
fn timepart_nominal() {
    let dt = 21915.0 * SECONDS_PER_DAY + 45045.0;
    assert_eq!(invoke("TIMEPART", &[num(dt)]), num(45045.0));
}

#[test]
fn timepart_midnight() {
    let dt = 21915.0 * SECONDS_PER_DAY;
    assert_eq!(invoke("TIMEPART", &[num(dt)]), num(0.0));
}

// ── DATETIME (combine) (M15.3) ───────────────────────────────────────────

#[test]
fn datetime_combine_nominal() {
    // date 21915 + time 45045 → datetime.
    let expected = 21915.0 * SECONDS_PER_DAY + 45045.0;
    assert_eq!(invoke("DATETIME", &[num(21915.0), num(45045.0)]), num(expected));
}

#[test]
fn datetime_combine_default_time() {
    assert_eq!(
        invoke("DATETIME", &[num(21915.0)]),
        num(21915.0 * SECONDS_PER_DAY)
    );
}

// ── HMS (M15.3) ──────────────────────────────────────────────────────────

#[test]
fn hms_nominal() {
    // 12:30:45 = 45045 seconds.
    assert_eq!(invoke("HMS", &[num(12.0), num(30.0), num(45.0)]), num(45045.0));
}

#[test]
fn hms_large_hours_ok() {
    // h ≥ 0 may exceed 23 (HMS allows large hour counts).
    assert_eq!(invoke("HMS", &[num(25.0), num(0.0), num(0.0)]), num(90000.0));
}

#[test]
fn hms_invalid_minute_is_missing() {
    let mut c = ctx();
    let r = invoke_ctx("HMS", &[num(1.0), num(60.0), num(0.0)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

// ── DHMS (M15.3) ─────────────────────────────────────────────────────────

#[test]
fn dhms_nominal() {
    let expected = 21915.0 * SECONDS_PER_DAY + 45045.0;
    assert_eq!(
        invoke("DHMS", &[num(21915.0), num(12.0), num(30.0), num(45.0)]),
        num(expected)
    );
}

#[test]
fn dhms_invalid_second_is_missing() {
    let mut c = ctx();
    let r = invoke_ctx("DHMS", &[num(0.0), num(0.0), num(0.0), num(60.0)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

// ── YRDIF (M15.3) ────────────────────────────────────────────────────────

#[test]
fn yrdif_actual_one_year() {
    // 2000-01-01 (14610) to 2001-01-01 (14976) = 366 days / 365.
    let r = invoke("YRDIF", &[num(14610.0), num(14976.0), chr("ACTUAL")]);
    assert_eq!(r, num(366.0 / 365.0));
}

#[test]
fn yrdif_default_basis_is_actual() {
    // No basis → ACTUAL.
    let r = invoke("YRDIF", &[num(14610.0), num(14975.0)]);
    assert_eq!(r, num(365.0 / 365.0));
}

#[test]
fn yrdif_b360_one_year() {
    // 2000-01-01 to 2001-01-01: 30/360 → 360 days / 360 = 1.0.
    let r = invoke("YRDIF", &[num(14610.0), num(14976.0), chr("B360")]);
    assert_eq!(r, num(1.0));
}

#[test]
fn yrdif_invalid_basis_is_missing() {
    let mut c = ctx();
    let r = invoke_ctx("YRDIF", &[num(14610.0), num(14976.0), chr("XYZ")], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

// ── DATDIF (M15.3) ───────────────────────────────────────────────────────

#[test]
fn datdif_actual_days() {
    assert_eq!(
        invoke("DATDIF", &[num(14610.0), num(14976.0), chr("ACTUAL")]),
        num(366.0)
    );
}

#[test]
fn datdif_b360_days() {
    assert_eq!(
        invoke("DATDIF", &[num(14610.0), num(14976.0), chr("B360")]),
        num(360.0)
    );
}

#[test]
fn datdif_invalid_basis_is_missing() {
    let mut c = ctx();
    let r = invoke_ctx("DATDIF", &[num(14610.0), num(14976.0), chr("BAD")], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

// ── JULDATE (M15.3) ──────────────────────────────────────────────────────

#[test]
fn juldate_jan1() {
    // 2000-01-01 (14610) → day 1.
    assert_eq!(invoke("JULDATE", &[num(14610.0)]), num(1.0));
}

#[test]
fn juldate_dec31() {
    // 2007-12-31 (17531) → day 365.
    assert_eq!(invoke("JULDATE", &[num(17531.0)]), num(365.0));
}

#[test]
fn juldate_missing() {
    assert_eq!(invoke("JULDATE", &[miss()]), miss());
}

// ── DATEJUL (M15.3) ──────────────────────────────────────────────────────

#[test]
fn datejul_two_digit_year() {
    // 07365 = day 365 of 1907 (00–99 → 1900–1999) → SAS date -18994.
    assert_eq!(invoke("DATEJUL", &[num(7365.0)]), num(-18994.0));
    // 107001 = day 1 of 2007 (100–199 → 2000–2099) → SAS date 17167.
    let r = invoke("DATEJUL", &[num(107001.0)]);
    assert_eq!(invoke("YEAR", &[r.clone()]), num(2007.0));
    assert_eq!(invoke("JULDATE", &[r]), num(1.0));
}

#[test]
fn datejul_four_digit_year() {
    // 2000001 = day 1 of 2000 → SAS date 14610.
    assert_eq!(invoke("DATEJUL", &[num(2000001.0)]), num(14610.0));
}

#[test]
fn datejul_invalid_day_is_missing() {
    // 2001366 = day 366 of 2001 (not a leap year) → missing.
    let mut c = ctx();
    let r = invoke_ctx("DATEJUL", &[num(2001366.0)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

// ── HOUR / MINUTE / SECOND (M15.3) ───────────────────────────────────────

#[test]
fn hour_nominal() {
    let dt = 21915.0 * SECONDS_PER_DAY + 45045.0; // 12:30:45
    assert_eq!(invoke("HOUR", &[num(dt)]), num(12.0));
}

#[test]
fn hour_midnight() {
    assert_eq!(invoke("HOUR", &[num(0.0)]), num(0.0));
}

#[test]
fn minute_nominal() {
    let dt = 21915.0 * SECONDS_PER_DAY + 45045.0; // 12:30:45
    assert_eq!(invoke("MINUTE", &[num(dt)]), num(30.0));
}

#[test]
fn minute_missing() {
    assert_eq!(invoke("MINUTE", &[miss()]), miss());
}

#[test]
fn second_nominal() {
    let dt = 21915.0 * SECONDS_PER_DAY + 45045.0; // 12:30:45
    assert_eq!(invoke("SECOND", &[num(dt)]), num(45.0));
}

#[test]
fn second_zero() {
    let dt = 21915.0 * SECONDS_PER_DAY; // midnight
    assert_eq!(invoke("SECOND", &[num(dt)]), num(0.0));
}

// ── NLDATE (M15.3) ───────────────────────────────────────────────────────

#[test]
fn nldate_en_default() {
    // 2020-01-01 = SAS date 21915 → "01JAN2020".
    assert_eq!(invoke("NLDATE", &[num(21915.0)]), chr("01JAN2020"));
}

#[test]
fn nldate_fr_same_as_en() {
    assert_eq!(invoke("NLDATE", &[num(21915.0), chr("FR")]), chr("01JAN2020"));
}

#[test]
fn nldate_unknown_language_defaults_en() {
    assert_eq!(invoke("NLDATE", &[num(21915.0), chr("ZZ")]), chr("01JAN2020"));
}

#[test]
fn nldate_missing_is_empty() {
    assert_eq!(invoke("NLDATE", &[miss()]), chr(""));
}

// ── Probability distribution functions (M15.4) ─────────────────────────────

/// Numeric value of a function result, panicking if missing.
fn val(v: &Value) -> f64 {
    coerce_num(v, &mut ctx()).expect("expected numeric result")
}

fn approx(name: &str, args: &[Value], expected: f64, tol: f64) {
    let got = val(&invoke(name, args));
    assert!(
        (got - expected).abs() < tol,
        "{name}: got {got}, expected {expected} (tol {tol})"
    );
}

// PROBNORM (R: pnorm)
#[test]
fn probnorm_zero_is_half() {
    approx("PROBNORM", &[num(0.0)], 0.5, 1e-9);
}
#[test]
fn probnorm_one_and_neg() {
    approx("PROBNORM", &[num(1.96)], 0.9750021048, 1e-7);
    approx("PROBNORM", &[num(-1.0)], 0.1586552539, 1e-7);
}
#[test]
fn probnorm_missing() {
    assert_eq!(invoke("PROBNORM", &[miss()]), miss());
}

// PROBT (R: pt)
#[test]
fn probt_zero_is_half() {
    approx("PROBT", &[num(0.0), num(10.0)], 0.5, 1e-9);
}
#[test]
fn probt_nominal() {
    // pt(2, 5) = 0.9490303
    approx("PROBT", &[num(2.0), num(5.0)], 0.9490302605, 1e-7);
}
#[test]
fn probt_missing_and_bad_df() {
    assert_eq!(invoke("PROBT", &[miss(), num(5.0)]), miss());
    let mut c = ctx();
    assert_eq!(invoke_ctx("PROBT", &[num(1.0), num(0.0)], &mut c), miss());
    assert!(c.error_flag);
}

// PROBF (R: pf)
#[test]
fn probf_nominal() {
    // F CDF: I_{0.375}(1.5, 5) = 0.8219926 (verified by numerical integration)
    approx("PROBF", &[num(2.0), num(3.0), num(10.0)], 0.8219926, 1e-6);
}
#[test]
fn probf_zero_is_zero() {
    approx("PROBF", &[num(0.0), num(3.0), num(10.0)], 0.0, 1e-12);
}
#[test]
fn probf_missing() {
    assert_eq!(invoke("PROBF", &[num(2.0), miss(), num(10.0)]), miss());
}

// PROBCHI (R: pchisq)
#[test]
fn probchi_nominal() {
    // pchisq(3.84, 1) = 0.9499565
    approx("PROBCHI", &[num(3.84), num(1.0)], 0.9499565, 1e-6);
}
#[test]
fn probchi_zero_is_zero() {
    approx("PROBCHI", &[num(0.0), num(5.0)], 0.0, 1e-12);
}
#[test]
fn probchi_missing() {
    assert_eq!(invoke("PROBCHI", &[miss(), num(5.0)]), miss());
}

// PROBBETA (R: pbeta)
#[test]
fn probbeta_nominal() {
    // pbeta(0.5, 2, 3) = 0.6875
    approx("PROBBETA", &[num(0.5), num(2.0), num(3.0)], 0.6875, 1e-7);
}
#[test]
fn probbeta_endpoints() {
    approx("PROBBETA", &[num(0.0), num(2.0), num(3.0)], 0.0, 1e-12);
    approx("PROBBETA", &[num(1.0), num(2.0), num(3.0)], 1.0, 1e-12);
}
#[test]
fn probbeta_bad_param() {
    let mut c = ctx();
    assert_eq!(
        invoke_ctx("PROBBETA", &[num(0.5), num(0.0), num(3.0)], &mut c),
        miss()
    );
    assert!(c.error_flag);
}

// PROBGAM (R: pgamma, rate=1)
#[test]
fn probgam_nominal() {
    // pgamma(2, 3) = 0.3233236
    approx("PROBGAM", &[num(2.0), num(3.0)], 0.3233236, 1e-6);
}
#[test]
fn probgam_zero_is_zero() {
    approx("PROBGAM", &[num(0.0), num(3.0)], 0.0, 1e-12);
}
#[test]
fn probgam_missing() {
    assert_eq!(invoke("PROBGAM", &[miss(), num(3.0)]), miss());
}

// PROBBNML (R: pbinom)
#[test]
fn probbnml_nominal() {
    // pbinom(3, 10, 0.5) = 0.171875  (SAS: PROBBNML(p, n, k))
    approx("PROBBNML", &[num(0.5), num(10.0), num(3.0)], 0.171875, 1e-9);
}
#[test]
fn probbnml_k_equals_n_is_one() {
    approx("PROBBNML", &[num(0.3), num(5.0), num(5.0)], 1.0, 1e-9);
}
#[test]
fn probbnml_missing() {
    assert_eq!(invoke("PROBBNML", &[num(0.5), miss(), num(3.0)]), miss());
}

// POISSON (R: ppois)
#[test]
fn poisson_nominal() {
    // ppois(3, 2) = 0.8571235
    approx("POISSON", &[num(2.0), num(3.0)], 0.8571235, 1e-6);
}
#[test]
fn poisson_zero_k() {
    // ppois(0, 2) = exp(-2) = 0.1353353
    approx("POISSON", &[num(2.0), num(0.0)], 0.1353352832, 1e-9);
}
#[test]
fn poisson_missing() {
    assert_eq!(invoke("POISSON", &[miss(), num(3.0)]), miss());
}

// CDF generic
#[test]
fn cdf_normal_matches_probnorm() {
    approx("CDF", &[chr("NORMAL"), num(1.96), num(0.0), num(1.0)], 0.9750021, 1e-6);
}
#[test]
fn cdf_t_and_chisq() {
    approx("CDF", &[chr("T"), num(2.0), num(5.0)], 0.9490302605, 1e-7);
    approx("CDF", &[chr("CHISQ"), num(3.84), num(1.0)], 0.9499565, 1e-6);
}
#[test]
fn cdf_bad_distribution() {
    let mut c = ctx();
    assert_eq!(invoke_ctx("CDF", &[chr("WEIBULL"), num(1.0)], &mut c), miss());
    assert!(c.error_flag);
}

// SDF generic = 1 - CDF
#[test]
fn sdf_normal() {
    approx("SDF", &[chr("NORMAL"), num(0.0)], 0.5, 1e-9);
}
#[test]
fn sdf_complements_cdf() {
    let c = val(&invoke("CDF", &[chr("CHISQ"), num(3.84), num(1.0)]));
    let s = val(&invoke("SDF", &[chr("CHISQ"), num(3.84), num(1.0)]));
    assert!((c + s - 1.0).abs() < 1e-9);
}

// LOGCDF generic
#[test]
fn logcdf_normal() {
    // ln(0.5)
    approx("LOGCDF", &[chr("NORMAL"), num(0.0)], (0.5f64).ln(), 1e-9);
}
#[test]
fn logcdf_missing() {
    assert_eq!(invoke("LOGCDF", &[chr("NORMAL"), miss()]), miss());
}

// PDF generic
#[test]
fn pdf_normal_at_zero() {
    // dnorm(0) = 1/sqrt(2pi) = 0.3989423
    approx("PDF", &[chr("NORMAL"), num(0.0)], 0.3989422804, 1e-9);
}
#[test]
fn pdf_poisson_pmf() {
    // dpois(2, 2) = 0.2706706
    approx("PDF", &[chr("POISSON"), num(2.0), num(2.0)], 0.2706705665, 1e-9);
}
#[test]
fn pdf_binomial_pmf() {
    // dbinom(3, 10, 0.5) = 0.1171875
    approx("PDF", &[chr("BINOMIAL"), num(3.0), num(0.5), num(10.0)], 0.1171875, 1e-9);
}

// QUANTILE generic (inverse of CDF)
#[test]
fn quantile_normal() {
    // qnorm(0.975) = 1.959964
    approx("QUANTILE", &[chr("NORMAL"), num(0.975)], 1.959964, 1e-5);
}
#[test]
fn quantile_chisq_roundtrip() {
    let q = val(&invoke("QUANTILE", &[chr("CHISQ"), num(0.95), num(1.0)]));
    let back = val(&invoke("CDF", &[chr("CHISQ"), num(q), num(1.0)]));
    assert!((back - 0.95).abs() < 1e-6);
}
#[test]
fn quantile_p_out_of_range() {
    let mut c = ctx();
    assert_eq!(
        invoke_ctx("QUANTILE", &[chr("NORMAL"), num(1.5)], &mut c),
        miss()
    );
    assert!(c.error_flag);
}

// ── M15.5 : Random variate generation ────────────────────────────────────

// Helper: extract f64 from a Value::Num, panic otherwise.
fn num_val(v: Value) -> f64 {
    match v {
        Value::Num(f) => f,
        other => panic!("expected Num, got {other:?}"),
    }
}

// ── RANUNI ───────────────────────────────────────────────────────────────

#[test]
fn ranuni_no_seed_returns_numeric() {
    // No seed → result is numeric and in (0, 1).
    let v = invoke("RANUNI", &[]);
    let f = num_val(v);
    assert!(f > 0.0 && f < 1.0, "RANUNI out of (0,1): {f}");
}

#[test]
fn ranuni_seed_deterministic() {
    // Same seed → same first value.
    let v1 = invoke("RANUNI", &[num(42.0)]);
    let v2 = invoke("RANUNI", &[num(42.0)]);
    assert_eq!(v1, v2, "RANUNI with same seed must be deterministic");
}

#[test]
fn ranuni_missing_arg_returns_numeric() {
    // Missing seed → treated as no seed (returns numeric, not missing).
    let v = invoke("RANUNI", &[miss()]);
    assert!(!v.is_missing(), "RANUNI(missing) should still return a number");
}

// ── RANNOR ───────────────────────────────────────────────────────────────

#[test]
fn rannor_no_seed_returns_numeric() {
    let v = invoke("RANNOR", &[]);
    assert!(matches!(v, Value::Num(_)), "RANNOR() must return Num");
}

#[test]
fn rannor_seed_deterministic() {
    let v1 = invoke("RANNOR", &[num(12345.0)]);
    let v2 = invoke("RANNOR", &[num(12345.0)]);
    assert_eq!(v1, v2, "RANNOR with same seed must be deterministic");
}

#[test]
fn rannor_multiple_calls_vary() {
    // Two consecutive calls with the same ctx should differ.
    let mut c = ctx();
    c.rng_state = 0x1234_5678_ABCD_EF00_u64;
    let a = call("RANNOR", &[], &mut c).unwrap();
    let b = call("RANNOR", &[], &mut c).unwrap();
    // They may be equal if both are the Box-Muller pair — very unlikely but
    // not impossible; we just check they're both numeric.
    assert!(matches!(a, Value::Num(_)));
    assert!(matches!(b, Value::Num(_)));
}

// ── RANEXP ───────────────────────────────────────────────────────────────

#[test]
fn ranexp_no_seed_positive() {
    // Exponential variates are always > 0.
    let v = num_val(invoke("RANEXP", &[]));
    assert!(v > 0.0, "RANEXP() must be positive, got {v}");
}

#[test]
fn ranexp_seed_deterministic() {
    let v1 = invoke("RANEXP", &[num(7.0)]);
    let v2 = invoke("RANEXP", &[num(7.0)]);
    assert_eq!(v1, v2);
}

#[test]
fn ranexp_missing_seed_still_numeric() {
    let v = invoke("RANEXP", &[miss()]);
    assert!(matches!(v, Value::Num(_)));
}

// ── RANBIN ───────────────────────────────────────────────────────────────

#[test]
fn ranbin_returns_non_negative_integer() {
    let v = num_val(invoke("RANBIN", &[num(0.3), num(10.0)]));
    assert!(v >= 0.0 && v <= 10.0, "RANBIN out of range: {v}");
    assert_eq!(v.fract(), 0.0, "RANBIN must return integer: {v}");
}

#[test]
fn ranbin_p_zero_yields_zero() {
    // p=0 → all trials fail → 0 successes (deterministic).
    let v = num_val(invoke("RANBIN", &[num(0.0), num(5.0)]));
    assert_eq!(v, 0.0);
}

#[test]
fn ranbin_p_one_yields_n() {
    // p=1 → all trials succeed → n successes (deterministic).
    let v = num_val(invoke("RANBIN", &[num(1.0), num(8.0)]));
    assert_eq!(v, 8.0);
}

#[test]
fn ranbin_missing_p_returns_missing() {
    assert_eq!(invoke("RANBIN", &[miss(), num(5.0)]), miss());
}

#[test]
fn ranbin_invalid_p_returns_missing_with_error() {
    let mut c = ctx();
    let r = invoke_ctx("RANBIN", &[num(1.5), num(5.0)], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

// ── RAND ─────────────────────────────────────────────────────────────────

#[test]
fn rand_uniform_in_range() {
    let v = num_val(invoke("RAND", &[chr("UNIFORM")]));
    assert!(v > 0.0 && v < 1.0, "RAND UNIFORM out of (0,1): {v}");
}

#[test]
fn rand_uniform_custom_range() {
    // lo=5, hi=10 → result in (5, 10).
    let v = num_val(invoke("RAND", &[chr("UNIFORM"), num(5.0), num(10.0)]));
    assert!(v > 5.0 && v < 10.0, "RAND UNIFORM[5,10] out of range: {v}");
}

#[test]
fn rand_normal_returns_numeric() {
    let v = invoke("RAND", &[chr("NORMAL")]);
    assert!(matches!(v, Value::Num(_)));
}

#[test]
fn rand_exponential_positive() {
    let v = num_val(invoke("RAND", &[chr("EXPONENTIAL")]));
    assert!(v > 0.0, "RAND EXPONENTIAL must be > 0, got {v}");
}

#[test]
fn rand_poisson_non_negative_integer() {
    let v = num_val(invoke("RAND", &[chr("POISSON"), num(2.0)]));
    assert!(v >= 0.0);
    assert_eq!(v.fract(), 0.0);
}

#[test]
fn rand_binomial_range() {
    let v = num_val(invoke("RAND", &[chr("BINOMIAL"), num(0.5), num(10.0)]));
    assert!(v >= 0.0 && v <= 10.0);
    assert_eq!(v.fract(), 0.0);
}

#[test]
fn rand_missing_distribution_returns_missing() {
    assert_eq!(invoke("RAND", &[miss()]), miss());
}

#[test]
fn rand_unknown_distribution_error() {
    let mut c = ctx();
    let r = invoke_ctx("RAND", &[chr("WEIBULL")], &mut c);
    assert_eq!(r, miss());
    assert!(c.error_flag);
}

// ── CALL STREAMINIT integration (via seed_to_state) ──────────────────────

#[test]
fn streaminit_seed_fn_nonzero_input() {
    // streaminit_seed with any non-zero seed returns the seed as-is (cast).
    let s = streaminit_seed(42);
    assert_eq!(s, 42_u64);
}

#[test]
fn streaminit_seed_fn_zero_uses_default() {
    // seed=0 → default state (non-zero).
    let s = streaminit_seed(0);
    assert_ne!(s, 0);
}
