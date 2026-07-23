// ──────────────────────────────────────────────────────────────────────────────
// Numerical helpers for probability distributions (M15.4)
//
// These are intentionally self-contained (no external crates). The accuracy
// target is ~1e-9 absolute, sufficient to match documented SAS/R/scipy results
// to the displayed precision.
// ──────────────────────────────────────────────────────────────────────────────

use super::*;

/// High-precision log-gamma via the Lanczos approximation (g = 7, n = 9).
/// Valid for x > 0. Far more accurate than the truncated-Stirling
/// `lgamma_approx` used by the GAMMA/LGAMMA functions, which matters when it
/// feeds the incomplete-beta / incomplete-gamma series below.
pub(super) fn ln_gamma(x: f64) -> f64 {
    // Lanczos coefficients (g = 7).
    const G: f64 = 7.0;
    const C: [f64; 9] = [
        0.999_999_999_999_809_93,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_13,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        // Reflection: Γ(x)Γ(1-x) = π / sin(πx)
        let pi = std::f64::consts::PI;
        (pi / (pi * x).sin()).ln() - ln_gamma(1.0 - x)
    } else {
        let x = x - 1.0;
        let mut a = C[0];
        let t = x + G + 0.5;
        for (i, &c) in C.iter().enumerate().skip(1) {
            a += c / (x + i as f64);
        }
        0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
    }
}

/// Error function erf(x) via the relation erf(x) = 2Φ(x√2) - 1. Implemented
/// directly through the regularized incomplete gamma function for robustness.
pub(super) fn erf(x: f64) -> f64 {
    if x == 0.0 {
        0.0
    } else if x > 0.0 {
        lower_gamma_p(0.5, x * x)
    } else {
        -lower_gamma_p(0.5, x * x)
    }
}

/// Complementary error function erfc(x) = 1 - erf(x).
pub(super) fn erfc(x: f64) -> f64 {
    1.0 - erf(x)
}

/// Regularized lower incomplete gamma P(a, x) = γ(a, x) / Γ(a).
/// Uses the series expansion for x < a+1 and the continued fraction otherwise
/// (Numerical Recipes). Returns a value in [0, 1].
pub(super) fn lower_gamma_p(a: f64, x: f64) -> f64 {
    if x <= 0.0 || a <= 0.0 {
        return 0.0;
    }
    if x < a + 1.0 {
        gamma_p_series(a, x)
    } else {
        1.0 - gamma_q_cf(a, x)
    }
}

/// Regularized upper incomplete gamma Q(a, x) = 1 - P(a, x).
pub(super) fn upper_gamma_q(a: f64, x: f64) -> f64 {
    1.0 - lower_gamma_p(a, x)
}

/// Series expansion for P(a, x), valid for x < a + 1.
pub(super) fn gamma_p_series(a: f64, x: f64) -> f64 {
    let gln = ln_gamma(a);
    let mut ap = a;
    let mut sum = 1.0 / a;
    let mut del = sum;
    for _ in 0..1000 {
        ap += 1.0;
        del *= x / ap;
        sum += del;
        if del.abs() < sum.abs() * 1e-15 {
            break;
        }
    }
    sum * (-x + a * x.ln() - gln).exp()
}

/// Continued-fraction evaluation of Q(a, x), valid for x >= a + 1.
pub(super) fn gamma_q_cf(a: f64, x: f64) -> f64 {
    let gln = ln_gamma(a);
    let tiny = 1e-300;
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / tiny;
    let mut d = 1.0 / b;
    let mut h = d;
    for i in 1..1000 {
        let an = -(i as f64) * (i as f64 - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < tiny {
            d = tiny;
        }
        c = b + an / c;
        if c.abs() < tiny {
            c = tiny;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-15 {
            break;
        }
    }
    (-x + a * x.ln() - gln).exp() * h
}

/// Regularized incomplete beta function I_x(a, b). Returns a value in [0, 1].
/// Uses Lentz's continued fraction (Numerical Recipes) with the standard
/// symmetry swap for fast convergence.
pub(super) fn betai(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let bt = (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b)
        + a * x.ln()
        + b * (1.0 - x).ln())
    .exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        bt * betacf(a, b, x) / a
    } else {
        1.0 - bt * betacf(b, a, 1.0 - x) / b
    }
}

/// Continued fraction for the incomplete beta function (Lentz's algorithm).
pub(super) fn betacf(a: f64, b: f64, x: f64) -> f64 {
    let tiny = 1e-300;
    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < tiny {
        d = tiny;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..1000 {
        let m = m as f64;
        let m2 = 2.0 * m;
        // Even step.
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < tiny {
            d = tiny;
        }
        c = 1.0 + aa / c;
        if c.abs() < tiny {
            c = tiny;
        }
        d = 1.0 / d;
        h *= d * c;
        // Odd step.
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < tiny {
            d = tiny;
        }
        c = 1.0 + aa / c;
        if c.abs() < tiny {
            c = tiny;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-15 {
            break;
        }
    }
    h
}

// ── Core distribution CDFs (all return a probability in [0, 1]) ──────────────

/// Standard normal CDF Φ(x) via erfc for numerical stability in the tails.
pub(super) fn normal_cdf_std(x: f64) -> f64 {
    0.5 * erfc(-x / std::f64::consts::SQRT_2)
}

/// Normal CDF with mean `mu` and standard deviation `sigma`.
pub(super) fn normal_cdf(x: f64, mu: f64, sigma: f64) -> f64 {
    normal_cdf_std((x - mu) / sigma)
}

/// Student's t CDF with `df` degrees of freedom.
pub(super) fn t_cdf(t: f64, df: f64) -> f64 {
    if df <= 0.0 {
        return f64::NAN;
    }
    let x = df / (df + t * t);
    let ib = 0.5 * betai(0.5 * df, 0.5, x);
    if t >= 0.0 {
        1.0 - ib
    } else {
        ib
    }
}

/// F CDF with `ndf` numerator and `ddf` denominator degrees of freedom.
pub(super) fn f_cdf(f: f64, ndf: f64, ddf: f64) -> f64 {
    if f <= 0.0 {
        return 0.0;
    }
    let x = ndf * f / (ndf * f + ddf);
    betai(0.5 * ndf, 0.5 * ddf, x)
}

/// Chi-square CDF with `df` degrees of freedom.
pub(super) fn chisq_cdf(x: f64, df: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    lower_gamma_p(0.5 * df, 0.5 * x)
}

/// Beta CDF P(X <= x) for X ~ Beta(a, b).
pub(super) fn beta_cdf(x: f64, a: f64, b: f64) -> f64 {
    betai(a, b, x)
}

/// Gamma CDF P(X <= x) for X ~ Gamma(shape = a, scale = 1).
pub(super) fn gamma_cdf(x: f64, a: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    lower_gamma_p(a, x)
}

/// Binomial CDF P(X <= k) for X ~ Binomial(n, p) by exact summation of the PMF.
pub(super) fn binomial_cdf(p: f64, n: f64, k: f64) -> f64 {
    let n = n.round();
    let k = k.floor();
    if k < 0.0 {
        return 0.0;
    }
    if k >= n {
        return 1.0;
    }
    // Use the incomplete-beta identity for stability with large n:
    // P(X <= k) = I_{1-p}(n-k, k+1)
    betai(n - k, k + 1.0, 1.0 - p)
}

/// Poisson CDF P(X <= k) for X ~ Poisson(lambda).
/// P(X <= k) = Q(k + 1, lambda) (regularized upper incomplete gamma).
pub(super) fn poisson_cdf(lambda: f64, k: f64) -> f64 {
    let k = k.floor();
    if k < 0.0 {
        return 0.0;
    }
    if lambda <= 0.0 {
        return 1.0;
    }
    upper_gamma_q(k + 1.0, lambda)
}

// ── Densities / mass functions (used by PDF) ─────────────────────────────────

pub(super) fn normal_pdf(x: f64, mu: f64, sigma: f64) -> f64 {
    let z = (x - mu) / sigma;
    (-0.5 * z * z).exp() / (sigma * (2.0 * std::f64::consts::PI).sqrt())
}

pub(super) fn t_pdf(t: f64, df: f64) -> f64 {
    let c = (ln_gamma(0.5 * (df + 1.0)) - ln_gamma(0.5 * df)).exp()
        / (df * std::f64::consts::PI).sqrt();
    c * (1.0 + t * t / df).powf(-0.5 * (df + 1.0))
}

pub(super) fn f_pdf(x: f64, ndf: f64, ddf: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let ln = 0.5 * ndf * (ndf / ddf).ln()
        + (0.5 * ndf - 1.0) * x.ln()
        - 0.5 * (ndf + ddf) * (1.0 + ndf * x / ddf).ln()
        - (ln_gamma(0.5 * ndf) + ln_gamma(0.5 * ddf) - ln_gamma(0.5 * (ndf + ddf)));
    ln.exp()
}

pub(super) fn chisq_pdf(x: f64, df: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let ln = (0.5 * df - 1.0) * x.ln() - 0.5 * x - 0.5 * df * 2.0_f64.ln() - ln_gamma(0.5 * df);
    ln.exp()
}

pub(super) fn beta_pdf(x: f64, a: f64, b: f64) -> f64 {
    if x <= 0.0 || x >= 1.0 {
        return 0.0;
    }
    let ln = (a - 1.0) * x.ln() + (b - 1.0) * (1.0 - x).ln()
        - (ln_gamma(a) + ln_gamma(b) - ln_gamma(a + b));
    ln.exp()
}

pub(super) fn gamma_pdf(x: f64, a: f64) -> f64 {
    if x < 0.0 {
        return 0.0;
    }
    if x == 0.0 {
        return if a < 1.0 {
            f64::INFINITY
        } else if a == 1.0 {
            1.0
        } else {
            0.0
        };
    }
    let ln = (a - 1.0) * x.ln() - x - ln_gamma(a);
    ln.exp()
}

/// Binomial PMF P(X = k).
pub(super) fn binomial_pmf(p: f64, n: f64, k: f64) -> f64 {
    let n = n.round();
    let k = k.round();
    if k < 0.0 || k > n {
        return 0.0;
    }
    let ln_coeff = ln_gamma(n + 1.0) - ln_gamma(k + 1.0) - ln_gamma(n - k + 1.0);
    let ln = ln_coeff
        + (if p > 0.0 { k * p.ln() } else if k == 0.0 { 0.0 } else { return 0.0 })
        + (if p < 1.0 {
            (n - k) * (1.0 - p).ln()
        } else if (n - k) == 0.0 {
            0.0
        } else {
            return 0.0;
        });
    ln.exp()
}

/// Poisson PMF P(X = k).
pub(super) fn poisson_pmf(lambda: f64, k: f64) -> f64 {
    let k = k.round();
    if k < 0.0 {
        return 0.0;
    }
    let ln = k * lambda.ln() - lambda - ln_gamma(k + 1.0);
    ln.exp()
}

/// Generic quantile (inverse CDF) by bisection on a monotone CDF closure.
/// `lo`/`hi` bracket the search; the function expands `hi` if necessary.
pub(super) fn quantile_bisect<F: Fn(f64) -> f64>(cdf: F, p: f64, mut lo: f64, mut hi: f64) -> f64 {
    // Expand the upper bound until it brackets the target.
    let mut guard = 0;
    while cdf(hi) < p && guard < 200 {
        hi += (hi - lo).abs().max(1.0);
        guard += 1;
    }
    // Expand the lower bound for distributions on the whole real line.
    guard = 0;
    while cdf(lo) > p && guard < 200 {
        lo -= (hi - lo).abs().max(1.0);
        guard += 1;
    }
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if cdf(mid) < p {
            lo = mid;
        } else {
            hi = mid;
        }
        if (hi - lo).abs() < 1e-12 * (1.0 + mid_scale(lo, hi)) {
            break;
        }
    }
    0.5 * (lo + hi)
}

pub(super) fn mid_scale(lo: f64, hi: f64) -> f64 {
    lo.abs().max(hi.abs())
}

// ──────────────────────────────────────────────────────────────────────────────
// Probability distribution functions (M15.4)
//
// Wrappers around the numerical helpers defined earlier in this file
// (normal/t/F/chi-square/beta/gamma CDFs, incomplete-beta / incomplete-gamma,
// binomial & Poisson, plus the generic bisection quantile). All return numeric
// probabilities in [0, 1] (CDF/SDF) or densities (PDF) or quantiles (QUANTILE).
// Missing arguments propagate to a missing result.
// ──────────────────────────────────────────────────────────────────────────────

/// PROBNORM(x): standard normal CDF Φ(x).
pub(super) fn fn_probnorm(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, normal_cdf_std)
}

/// PROBT(t, df): Student's t CDF P(T <= t). df must be > 0.
pub(super) fn fn_probt(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (Some(t), Some(df)) if df > 0.0 => Value::Num(t_cdf(t, df)),
        (None, _) | (_, None) => Value::missing(),
        _ => {
            ctx.error_flag = true;
            ctx.invalid_data += 1;
            Value::missing()
        }
    }
}

/// PROBF(f, ndf, ddf): F CDF P(F <= f). ndf, ddf must be > 0.
pub(super) fn fn_probf(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 3 {
        return Value::missing();
    }
    match (
        coerce_num(&args[0], ctx),
        coerce_num(&args[1], ctx),
        coerce_num(&args[2], ctx),
    ) {
        (Some(f), Some(ndf), Some(ddf)) if ndf > 0.0 && ddf > 0.0 => {
            Value::Num(f_cdf(f, ndf, ddf))
        }
        (None, _, _) | (_, None, _) | (_, _, None) => Value::missing(),
        _ => {
            ctx.error_flag = true;
            ctx.invalid_data += 1;
            Value::missing()
        }
    }
}

/// PROBCHI(x, df): chi-square CDF P(χ² <= x). df must be > 0.
pub(super) fn fn_probchi(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (Some(x), Some(df)) if df > 0.0 => Value::Num(chisq_cdf(x, df)),
        (None, _) | (_, None) => Value::missing(),
        _ => {
            ctx.error_flag = true;
            ctx.invalid_data += 1;
            Value::missing()
        }
    }
}

/// PROBBETA(x, a, b): Beta CDF P(X <= x). a, b must be > 0.
pub(super) fn fn_probbeta(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 3 {
        return Value::missing();
    }
    match (
        coerce_num(&args[0], ctx),
        coerce_num(&args[1], ctx),
        coerce_num(&args[2], ctx),
    ) {
        (Some(x), Some(a), Some(b)) if a > 0.0 && b > 0.0 => Value::Num(beta_cdf(x, a, b)),
        (None, _, _) | (_, None, _) | (_, _, None) => Value::missing(),
        _ => {
            ctx.error_flag = true;
            ctx.invalid_data += 1;
            Value::missing()
        }
    }
}

/// PROBGAM(x, a): Gamma CDF P(X <= x) for shape = a, scale = 1. a must be > 0.
pub(super) fn fn_probgam(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (Some(x), Some(a)) if a > 0.0 => Value::Num(gamma_cdf(x, a)),
        (None, _) | (_, None) => Value::missing(),
        _ => {
            ctx.error_flag = true;
            ctx.invalid_data += 1;
            Value::missing()
        }
    }
}

/// PROBBNML(p, n, k): binomial CDF P(X <= k) for X ~ Binomial(n, p).
pub(super) fn fn_probbnml(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 3 {
        return Value::missing();
    }
    match (
        coerce_num(&args[0], ctx),
        coerce_num(&args[1], ctx),
        coerce_num(&args[2], ctx),
    ) {
        (Some(p), Some(n), Some(k)) if (0.0..=1.0).contains(&p) && n >= 0.0 && k >= 0.0 => {
            Value::Num(binomial_cdf(p, n, k))
        }
        (None, _, _) | (_, None, _) | (_, _, None) => Value::missing(),
        _ => {
            ctx.error_flag = true;
            ctx.invalid_data += 1;
            Value::missing()
        }
    }
}

/// POISSON(lambda, k): Poisson CDF P(X <= k) for X ~ Poisson(λ).
pub(super) fn fn_poisson(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 2 {
        return Value::missing();
    }
    match (coerce_num(&args[0], ctx), coerce_num(&args[1], ctx)) {
        (Some(lambda), Some(k)) if lambda >= 0.0 && k >= 0.0 => {
            Value::Num(poisson_cdf(lambda, k))
        }
        (None, _) | (_, None) => Value::missing(),
        _ => {
            ctx.error_flag = true;
            ctx.invalid_data += 1;
            Value::missing()
        }
    }
}

/// Identifies a distribution by its (case-insensitive) SAS keyword, accepting
/// common abbreviations. Returns the canonical kind or None if unrecognised.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum DistKind {
    Normal,
    T,
    F,
    Chisq,
    Beta,
    Gamma,
    Binomial,
    Poisson,
}

pub(super) fn parse_dist(name: &str) -> Option<DistKind> {
    let up = name.trim().to_uppercase();
    // SAS accepts several spellings; match on prefixes used by the docs.
    match up.as_str() {
        "NORMAL" | "GAUSS" | "N" => Some(DistKind::Normal),
        "T" => Some(DistKind::T),
        "F" => Some(DistKind::F),
        "CHISQUARE" | "CHISQ" | "CHISQUAR" => Some(DistKind::Chisq),
        "BETA" => Some(DistKind::Beta),
        "GAMMA" => Some(DistKind::Gamma),
        "BINOMIAL" | "BINOM" => Some(DistKind::Binomial),
        "POISSON" => Some(DistKind::Poisson),
        _ => None,
    }
}

/// Shared front-end for CDF/SDF/LOGCDF/PDF/QUANTILE: parses the distribution
/// keyword and the numeric value/parameters, then dispatches to `compute`.
/// `compute` receives (kind, x, parms) and returns the raw f64 result.
pub(super) fn dist_dispatch<F>(args: &[Value], ctx: &mut EvalCtx, compute: F) -> Value
where
    F: Fn(DistKind, f64, &[f64]) -> Option<f64>,
{
    if args.len() < 2 {
        return Value::missing();
    }
    let name = coerce_char(&args[0]);
    let Some(kind) = parse_dist(&name) else {
        ctx.error_flag = true;
        ctx.invalid_data += 1;
        return Value::missing();
    };
    let x = match coerce_num(&args[1], ctx) {
        None => return Value::missing(),
        Some(v) => v,
    };
    let mut parms = Vec::new();
    for a in &args[2..] {
        match coerce_num(a, ctx) {
            None => return Value::missing(),
            Some(v) => parms.push(v),
        }
    }
    match compute(kind, x, &parms) {
        Some(r) => Value::Num(r),
        None => {
            ctx.error_flag = true;
            ctx.invalid_data += 1;
            Value::missing()
        }
    }
}

/// Evaluates the CDF of `kind` at `x` with the given parameter list.
/// Parameter conventions follow SAS:
///   NORMAL(mu=parm0, sigma=parm1), T(df), F(ndf, ddf), CHISQ(df),
///   BETA(a, b), GAMMA(a), BINOMIAL(p, n), POISSON(lambda).
pub(super) fn dist_cdf(kind: DistKind, x: f64, p: &[f64]) -> Option<f64> {
    match kind {
        DistKind::Normal => {
            let mu = p.first().copied().unwrap_or(0.0);
            let sigma = p.get(1).copied().unwrap_or(1.0);
            if sigma <= 0.0 {
                return None;
            }
            Some(normal_cdf(x, mu, sigma))
        }
        DistKind::T => {
            let df = *p.first()?;
            if df <= 0.0 {
                return None;
            }
            Some(t_cdf(x, df))
        }
        DistKind::F => {
            let ndf = *p.first()?;
            let ddf = *p.get(1)?;
            if ndf <= 0.0 || ddf <= 0.0 {
                return None;
            }
            Some(f_cdf(x, ndf, ddf))
        }
        DistKind::Chisq => {
            let df = *p.first()?;
            if df <= 0.0 {
                return None;
            }
            Some(chisq_cdf(x, df))
        }
        DistKind::Beta => {
            let a = *p.first()?;
            let b = *p.get(1)?;
            if a <= 0.0 || b <= 0.0 {
                return None;
            }
            Some(beta_cdf(x, a, b))
        }
        DistKind::Gamma => {
            let a = *p.first()?;
            if a <= 0.0 {
                return None;
            }
            Some(gamma_cdf(x, a))
        }
        DistKind::Binomial => {
            let prob = *p.first()?;
            let n = *p.get(1)?;
            if !(0.0..=1.0).contains(&prob) || n < 0.0 {
                return None;
            }
            Some(binomial_cdf(prob, n, x))
        }
        DistKind::Poisson => {
            let lambda = *p.first()?;
            if lambda < 0.0 {
                return None;
            }
            Some(poisson_cdf(lambda, x))
        }
    }
}

/// Evaluates the PDF (continuous) or PMF (discrete) of `kind` at `x`.
pub(super) fn dist_pdf(kind: DistKind, x: f64, p: &[f64]) -> Option<f64> {
    match kind {
        DistKind::Normal => {
            let mu = p.first().copied().unwrap_or(0.0);
            let sigma = p.get(1).copied().unwrap_or(1.0);
            if sigma <= 0.0 {
                return None;
            }
            Some(normal_pdf(x, mu, sigma))
        }
        DistKind::T => {
            let df = *p.first()?;
            if df <= 0.0 {
                return None;
            }
            Some(t_pdf(x, df))
        }
        DistKind::F => {
            let ndf = *p.first()?;
            let ddf = *p.get(1)?;
            if ndf <= 0.0 || ddf <= 0.0 {
                return None;
            }
            Some(f_pdf(x, ndf, ddf))
        }
        DistKind::Chisq => {
            let df = *p.first()?;
            if df <= 0.0 {
                return None;
            }
            Some(chisq_pdf(x, df))
        }
        DistKind::Beta => {
            let a = *p.first()?;
            let b = *p.get(1)?;
            if a <= 0.0 || b <= 0.0 {
                return None;
            }
            Some(beta_pdf(x, a, b))
        }
        DistKind::Gamma => {
            let a = *p.first()?;
            if a <= 0.0 {
                return None;
            }
            Some(gamma_pdf(x, a))
        }
        DistKind::Binomial => {
            let prob = *p.first()?;
            let n = *p.get(1)?;
            if !(0.0..=1.0).contains(&prob) || n < 0.0 {
                return None;
            }
            Some(binomial_pmf(prob, n, x))
        }
        DistKind::Poisson => {
            let lambda = *p.first()?;
            if lambda < 0.0 {
                return None;
            }
            Some(poisson_pmf(lambda, x))
        }
    }
}

/// Inverse CDF (quantile) of `kind` at probability `pr` via bisection on the
/// monotone CDF. `pr` must lie in (0, 1).
pub(super) fn dist_quantile(kind: DistKind, pr: f64, p: &[f64]) -> Option<f64> {
    if pr <= 0.0 || pr >= 1.0 {
        return None;
    }
    let cdf = |x: f64| dist_cdf(kind, x, p);
    // Validate the parameters once (so a bad spec returns None rather than
    // looping); use the midpoint of a reasonable bracket as a probe.
    cdf(0.5)?;
    let closure = |x: f64| cdf(x).unwrap_or(f64::NAN);
    let q = match kind {
        DistKind::Normal | DistKind::T => quantile_bisect(closure, pr, -10.0, 10.0),
        DistKind::Beta => quantile_bisect(closure, pr, 0.0, 1.0),
        // Non-negative supports: F, chi-square, gamma, binomial, Poisson.
        _ => quantile_bisect(closure, pr, 0.0, 10.0),
    };
    Some(q)
}

/// CDF(dist, x, parm1[, parm2]): general cumulative distribution function.
pub(super) fn fn_cdf(args: &[Value], ctx: &mut EvalCtx) -> Value {
    dist_dispatch(args, ctx, dist_cdf)
}

/// SDF(dist, x, parm1[, parm2]): survival function 1 - CDF(x).
pub(super) fn fn_sdf(args: &[Value], ctx: &mut EvalCtx) -> Value {
    dist_dispatch(args, ctx, |k, x, p| dist_cdf(k, x, p).map(|c| 1.0 - c))
}

/// LOGCDF(dist, x, parm1[, parm2]): log of the CDF.
pub(super) fn fn_logcdf(args: &[Value], ctx: &mut EvalCtx) -> Value {
    dist_dispatch(args, ctx, |k, x, p| dist_cdf(k, x, p).map(|c| c.ln()))
}

/// PDF(dist, x, parm1[, parm2]): probability density (or mass) function.
pub(super) fn fn_pdf(args: &[Value], ctx: &mut EvalCtx) -> Value {
    dist_dispatch(args, ctx, dist_pdf)
}

/// QUANTILE(dist, p, parm1[, parm2]): inverse CDF.
pub(super) fn fn_quantile(args: &[Value], ctx: &mut EvalCtx) -> Value {
    dist_dispatch(args, ctx, dist_quantile)
}

