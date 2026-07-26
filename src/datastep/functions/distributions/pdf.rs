use super::*;

// ── Densities / mass functions (used by PDF) ─────────────────────────────────

pub(crate) fn normal_pdf(x: f64, mu: f64, sigma: f64) -> f64 {
    let z = (x - mu) / sigma;
    (-0.5 * z * z).exp() / (sigma * (2.0 * std::f64::consts::PI).sqrt())
}

pub(crate) fn t_pdf(t: f64, df: f64) -> f64 {
    let c = (ln_gamma(0.5 * (df + 1.0)) - ln_gamma(0.5 * df)).exp()
        / (df * std::f64::consts::PI).sqrt();
    c * (1.0 + t * t / df).powf(-0.5 * (df + 1.0))
}

pub(crate) fn f_pdf(x: f64, ndf: f64, ddf: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let ln = 0.5 * ndf * (ndf / ddf).ln() + (0.5 * ndf - 1.0) * x.ln()
        - 0.5 * (ndf + ddf) * (1.0 + ndf * x / ddf).ln()
        - (ln_gamma(0.5 * ndf) + ln_gamma(0.5 * ddf) - ln_gamma(0.5 * (ndf + ddf)));
    ln.exp()
}

pub(crate) fn chisq_pdf(x: f64, df: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let ln = (0.5 * df - 1.0) * x.ln() - 0.5 * x - 0.5 * df * 2.0_f64.ln() - ln_gamma(0.5 * df);
    ln.exp()
}

pub(crate) fn beta_pdf(x: f64, a: f64, b: f64) -> f64 {
    if x <= 0.0 || x >= 1.0 {
        return 0.0;
    }
    let ln = (a - 1.0) * x.ln() + (b - 1.0) * (1.0 - x).ln()
        - (ln_gamma(a) + ln_gamma(b) - ln_gamma(a + b));
    ln.exp()
}

pub(crate) fn gamma_pdf(x: f64, a: f64) -> f64 {
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
pub(crate) fn binomial_pmf(p: f64, n: f64, k: f64) -> f64 {
    let n = n.round();
    let k = k.round();
    if k < 0.0 || k > n {
        return 0.0;
    }
    let ln_coeff = ln_gamma(n + 1.0) - ln_gamma(k + 1.0) - ln_gamma(n - k + 1.0);
    let ln = ln_coeff
        + (if p > 0.0 {
            k * p.ln()
        } else if k == 0.0 {
            0.0
        } else {
            return 0.0;
        })
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
pub(crate) fn poisson_pmf(lambda: f64, k: f64) -> f64 {
    let k = k.round();
    if k < 0.0 {
        return 0.0;
    }
    let ln = k * lambda.ln() - lambda - ln_gamma(k + 1.0);
    ln.exp()
}
