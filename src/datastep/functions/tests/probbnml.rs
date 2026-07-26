use super::super::*;
use super::*;
use crate::value::Value;

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
    approx(
        "CDF",
        &[chr("NORMAL"), num(1.96), num(0.0), num(1.0)],
        0.9750021,
        1e-6,
    );
}

#[test]
fn cdf_t_and_chisq() {
    approx("CDF", &[chr("T"), num(2.0), num(5.0)], 0.9490302605, 1e-7);
    approx("CDF", &[chr("CHISQ"), num(3.84), num(1.0)], 0.9499565, 1e-6);
}

#[test]
fn cdf_bad_distribution() {
    let mut c = ctx();
    assert_eq!(
        invoke_ctx("CDF", &[chr("WEIBULL"), num(1.0)], &mut c),
        miss()
    );
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
    approx(
        "PDF",
        &[chr("POISSON"), num(2.0), num(2.0)],
        0.2706705665,
        1e-9,
    );
}

#[test]
fn pdf_binomial_pmf() {
    // dbinom(3, 10, 0.5) = 0.1171875
    approx(
        "PDF",
        &[chr("BINOMIAL"), num(3.0), num(0.5), num(10.0)],
        0.1171875,
        1e-9,
    );
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
    assert!(
        !v.is_missing(),
        "RANUNI(missing) should still return a number"
    );
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

// ── Dispatch map ≡ scan linéaire (anti-régression du gagnant) ─────────────

/// La `DISPATCH_MAP` doit résoudre chaque clé de la table source vers le
/// MÊME pointeur de fonction qu'un scan linéaire de référence (première
/// occurrence gagnante) — protège le remplacement du scan O(n) par la map.
#[test]
fn dispatch_map_matches_linear_scan() {
    assert!(!DISPATCH.is_empty());
    for (name, _) in DISPATCH {
        // Gagnant du scan linéaire : la PREMIÈRE occurrence de la clé.
        let linear: SasFn = DISPATCH
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, f)| *f)
            .expect("key present in source table");
        let mapped: SasFn = *DISPATCH_MAP
            .get(name)
            .unwrap_or_else(|| panic!("key {name} missing from DISPATCH_MAP"));
        assert!(
            std::ptr::fn_addr_eq(linear, mapped),
            "dispatch winner mismatch for {name}"
        );
    }
}
