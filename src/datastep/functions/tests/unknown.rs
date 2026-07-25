use super::super::*;
use super::*;

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
