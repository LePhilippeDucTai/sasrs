use super::*;

// ───────────────────────── numeric core ─────────────────────────

/// One-sample (or paired-difference) t-test result over a complete numeric
/// sample. `t`/`p` are `None` when the test is undefined (n < 2 or zero std).
#[derive(Debug, Clone)]
pub(super) struct OneSampleResult {
    pub(super) n: usize,
    pub(super) mean: f64,
    pub(super) std: Option<f64>,
    pub(super) se: Option<f64>,
    pub(super) min: f64,
    pub(super) max: f64,
    pub(super) df: f64,
    pub(super) t: Option<f64>,
    pub(super) p: Option<f64>,
    /// Two-sided 100(1-alpha)% confidence limits for the mean.
    pub(super) mean_lcl: Option<f64>,
    pub(super) mean_ucl: Option<f64>,
    /// Two-sided 100(1-alpha)% confidence limits for the standard deviation
    /// (chi-square based). None when the test is undefined.
    pub(super) std_lcl: Option<f64>,
    pub(super) std_ucl: Option<f64>,
}

/// One-sample t-test of `values` against `h0` at significance `alpha`,
/// reporting the `sides`-appropriate probability and ALPHA-level CLs.
pub(super) fn one_sample(values: &[f64], h0: f64, alpha: f64, sides: TTestSides) -> OneSampleResult {
    let n = values.len();
    let mean = if n > 0 {
        values.iter().sum::<f64>() / n as f64
    } else {
        f64::NAN
    };
    let std = sample_std(values);
    let df = (n as f64 - 1.0).max(0.0);
    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    let (se, t, p, mean_lcl, mean_ucl, std_lcl, std_ucl) = match std {
        Some(s) if n >= 2 && s > 0.0 => {
            let se = s / (n as f64).sqrt();
            let t = (mean - h0) / se;
            let p = sided_p(t, df, sides);
            // Two-sided 100(1-alpha)% CL for the mean: mean ± t_{1-alpha/2} · se.
            let tcrit = common::t_quantile(1.0 - alpha / 2.0, df);
            let half = tcrit * se;
            // Two-sided CL for the standard deviation via chi-square:
            // [ s·√((n-1)/χ²_{1-α/2}), s·√((n-1)/χ²_{α/2}) ].
            let chi_hi = chisq_quantile(1.0 - alpha / 2.0, df);
            let chi_lo = chisq_quantile(alpha / 2.0, df);
            let (slcl, sucl) = if chi_hi > 0.0 && chi_lo > 0.0 {
                (
                    Some(s * (df / chi_hi).sqrt()),
                    Some(s * (df / chi_lo).sqrt()),
                )
            } else {
                (None, None)
            };
            (
                Some(se),
                Some(t),
                Some(p),
                Some(mean - half),
                Some(mean + half),
                slcl,
                sucl,
            )
        }
        Some(_) if n >= 2 => {
            // Constant sample (zero std): test undefined.
            (Some(0.0), None, None, None, None, None, None)
        }
        _ => (None, None, None, None, None, None, None),
    };

    OneSampleResult {
        n,
        mean,
        std,
        se,
        min: if n > 0 { min } else { f64::NAN },
        max: if n > 0 { max } else { f64::NAN },
        df,
        t,
        p,
        mean_lcl,
        mean_ucl,
        std_lcl,
        std_ucl,
    }
}

/// Two-method (Pooled + Satterthwaite) two-sample t-test, plus the folded
/// F-test for equality of variances. Groups `a` and `b` are the complete
/// numeric samples for the two CLASS levels in display order (a first).
#[derive(Debug, Clone)]
pub(super) struct TwoSampleResult {
    pub(super) n_a: usize,
    pub(super) n_b: usize,
    /// Difference of means (mean_a - mean_b); NaN when undefined.
    pub(super) diff: f64,
    /// Pooled: (t, df, p)
    pub(super) pooled: Option<(f64, f64, f64)>,
    /// Satterthwaite: (t, df, p)
    pub(super) satterthwaite: Option<(f64, f64, f64)>,
    /// Folded F test for equal variances: (F, df1, df2, p)
    pub(super) f_test: Option<(f64, f64, f64, f64)>,
    /// Pooled mean-difference CL: (lower, upper) at the ALPHA level.
    pub(super) pooled_cl: Option<(f64, f64)>,
    /// Satterthwaite mean-difference CL: (lower, upper) at the ALPHA level.
    pub(super) satt_cl: Option<(f64, f64)>,
}

pub(super) fn two_sample(a: &[f64], b: &[f64], alpha: f64, sides: TTestSides) -> TwoSampleResult {
    let n_a = a.len();
    let n_b = b.len();
    let naf = n_a as f64;
    let nbf = n_b as f64;
    let mean_a = if n_a > 0 { a.iter().sum::<f64>() / naf } else { f64::NAN };
    let mean_b = if n_b > 0 { b.iter().sum::<f64>() / nbf } else { f64::NAN };
    let std_a = sample_std(a);
    let std_b = sample_std(b);

    let diff = mean_a - mean_b;
    let (pooled, satterthwaite, pooled_cl, satt_cl) = match (std_a, std_b) {
        (Some(sa), Some(sb)) if n_a >= 2 && n_b >= 2 => {
            let va = sa * sa;
            let vb = sb * sb;
            // Pooled.
            let sp2 = ((naf - 1.0) * va + (nbf - 1.0) * vb) / (naf + nbf - 2.0);
            let se_pool = (sp2 * (1.0 / naf + 1.0 / nbf)).sqrt();
            let (pooled, pooled_cl) = if se_pool > 0.0 {
                let df = naf + nbf - 2.0;
                let t = diff / se_pool;
                let half = common::t_quantile(1.0 - alpha / 2.0, df) * se_pool;
                (
                    Some((t, df, sided_p(t, df, sides))),
                    Some((diff - half, diff + half)),
                )
            } else {
                (None, None)
            };
            // Satterthwaite.
            let se_satt = (va / naf + vb / nbf).sqrt();
            let (satt, satt_cl) = if se_satt > 0.0 {
                let num = (va / naf + vb / nbf).powi(2);
                let den = (va / naf).powi(2) / (naf - 1.0) + (vb / nbf).powi(2) / (nbf - 1.0);
                let df = num / den;
                let t = diff / se_satt;
                let half = common::t_quantile(1.0 - alpha / 2.0, df) * se_satt;
                (
                    Some((t, df, sided_p(t, df, sides))),
                    Some((diff - half, diff + half)),
                )
            } else {
                (None, None)
            };
            (pooled, satt, pooled_cl, satt_cl)
        }
        _ => (None, None, None, None),
    };

    let f_test = match (std_a, std_b) {
        (Some(sa), Some(sb)) if n_a >= 2 && n_b >= 2 && sa > 0.0 && sb > 0.0 => {
            let va = sa * sa;
            let vb = sb * sb;
            // Numerator df corresponds to the group with the LARGER variance.
            let (f, df1, df2) = if va >= vb {
                (va / vb, naf - 1.0, nbf - 1.0)
            } else {
                (vb / va, nbf - 1.0, naf - 1.0)
            };
            let cdf = f_cdf(f, df1, df2);
            let p = 2.0 * cdf.min(1.0 - cdf);
            Some((f, df1, df2, p.clamp(0.0, 1.0)))
        }
        _ => None,
    };

    TwoSampleResult {
        n_a,
        n_b,
        diff,
        pooled,
        satterthwaite,
        f_test,
        pooled_cl,
        satt_cl,
    }
}

/// p-value for a t statistic honoring the requested test `sides`:
/// - TwoTailed → Pr > |t|
/// - Upper (H1: μ > H0) → Pr > t = 1 - CDF(t)
/// - Lower (H1: μ < H0) → Pr < t = CDF(t)
pub(super) fn sided_p(t: f64, df: f64, sides: TTestSides) -> f64 {
    match sides {
        TTestSides::TwoTailed => two_sided_p(t, df),
        TTestSides::Upper => (1.0 - student_t_cdf(t, df)).clamp(0.0, 1.0),
        TTestSides::Lower => student_t_cdf(t, df).clamp(0.0, 1.0),
    }
}

/// Chi-square quantile: value `x` with P(χ²_df ≤ x) = `p`, via bisection on the
/// monotone CDF `1 - chisq_sf`. Used for the std-dev confidence limits. Robust
/// over the useful range; accuracy ~1e-8 on the probability.
pub(super) fn chisq_quantile(p: f64, df: f64) -> f64 {
    if !(0.0..=1.0).contains(&p) || df <= 0.0 {
        return f64::NAN;
    }
    if p <= 0.0 {
        return 0.0;
    }
    let cdf = |x: f64| 1.0 - common::chisq_sf(x, df);
    let mut lo = 0.0_f64;
    let mut hi = 1.0_f64;
    while cdf(hi) < p && hi < 1e12 {
        hi *= 2.0;
    }
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if cdf(mid) < p {
            lo = mid;
        } else {
            hi = mid;
        }
        if (hi - lo) <= 1e-12 * (1.0 + hi.abs()) {
            break;
        }
    }
    0.5 * (lo + hi)
}
