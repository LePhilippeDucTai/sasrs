use super::*;

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
