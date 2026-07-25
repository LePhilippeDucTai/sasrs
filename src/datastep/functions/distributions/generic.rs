use super::*;

/// Generic quantile (inverse CDF) by bisection on a monotone CDF closure.
/// `lo`/`hi` bracket the search; the function expands `hi` if necessary.
pub(crate) fn quantile_bisect<F: Fn(f64) -> f64>(cdf: F, p: f64, mut lo: f64, mut hi: f64) -> f64 {
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

pub(crate) fn mid_scale(lo: f64, hi: f64) -> f64 {
    lo.abs().max(hi.abs())
}

/// Identifies a distribution by its (case-insensitive) SAS keyword, accepting
/// common abbreviations. Returns the canonical kind or None if unrecognised.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum DistKind {
    Normal,
    T,
    F,
    Chisq,
    Beta,
    Gamma,
    Binomial,
    Poisson,
}

pub(crate) fn parse_dist(name: &str) -> Option<DistKind> {
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
pub(crate) fn dist_dispatch<F>(args: &[Value], ctx: &mut EvalCtx, compute: F) -> Value
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
pub(crate) fn dist_cdf(kind: DistKind, x: f64, p: &[f64]) -> Option<f64> {
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
pub(crate) fn dist_pdf(kind: DistKind, x: f64, p: &[f64]) -> Option<f64> {
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
pub(crate) fn dist_quantile(kind: DistKind, pr: f64, p: &[f64]) -> Option<f64> {
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
pub(crate) fn fn_cdf(args: &[Value], ctx: &mut EvalCtx) -> Value {
    dist_dispatch(args, ctx, dist_cdf)
}

/// SDF(dist, x, parm1[, parm2]): survival function 1 - CDF(x).
pub(crate) fn fn_sdf(args: &[Value], ctx: &mut EvalCtx) -> Value {
    dist_dispatch(args, ctx, |k, x, p| dist_cdf(k, x, p).map(|c| 1.0 - c))
}

/// LOGCDF(dist, x, parm1[, parm2]): log of the CDF.
pub(crate) fn fn_logcdf(args: &[Value], ctx: &mut EvalCtx) -> Value {
    dist_dispatch(args, ctx, |k, x, p| dist_cdf(k, x, p).map(|c| c.ln()))
}

/// PDF(dist, x, parm1[, parm2]): probability density (or mass) function.
pub(crate) fn fn_pdf(args: &[Value], ctx: &mut EvalCtx) -> Value {
    dist_dispatch(args, ctx, dist_pdf)
}

/// QUANTILE(dist, p, parm1[, parm2]): inverse CDF.
pub(crate) fn fn_quantile(args: &[Value], ctx: &mut EvalCtx) -> Value {
    dist_dispatch(args, ctx, dist_quantile)
}
