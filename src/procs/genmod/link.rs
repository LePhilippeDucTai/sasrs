use super::*;

/// Canonical link for each distribution (SAS 9.4 defaults).
pub(super) fn canonical_link(dist: &Distribution) -> LinkFunction {
    match dist {
        Distribution::Poisson => LinkFunction::Log,
        Distribution::Binomial => LinkFunction::Logit,
        Distribution::Normal => LinkFunction::Identity,
        // SAS GENMOD canonical link for Gamma is the reciprocal (power(-1)).
        Distribution::Gamma => LinkFunction::Reciprocal,
    }
}

// ───────────────────────── Link / variance functions ─────────────────────────

/// Apply inverse link: η → μ (mean on natural scale).
///
/// For the reciprocal link μ = 1/η; the IRLS step can drive η through 0 and
/// make μ negative, which is invalid for Gamma (μ > 0). We clamp μ to a small
/// positive floor here; the IRLS loop additionally step-halves on invalid μ.
pub(super) fn inv_link(eta: f64, lf: &LinkFunction) -> f64 {
    match lf {
        LinkFunction::Log => eta.exp().max(1e-10),
        LinkFunction::Logit => {
            let e = (-eta).exp();
            1.0 / (1.0 + e)
        }
        LinkFunction::Identity => eta,
        LinkFunction::Reciprocal => {
            if eta.abs() < 1e-12 {
                1e12
            } else {
                let mu = 1.0 / eta;
                if mu > 0.0 {
                    mu
                } else {
                    1e-10
                }
            }
        }
    }
}

/// Variance function V(μ) for the family.
pub(super) fn variance(mu: f64, dist: &Distribution) -> f64 {
    match dist {
        Distribution::Poisson => mu,
        Distribution::Binomial => {
            let v = mu * (1.0 - mu);
            v.max(1e-15)
        }
        Distribution::Normal => 1.0,
        // Gamma: V(μ) = μ².
        Distribution::Gamma => (mu * mu).max(1e-15),
    }
}

/// dη/dμ = g'(μ) where g is the link function.
pub(super) fn deta_dmu(mu: f64, lf: &LinkFunction) -> f64 {
    match lf {
        LinkFunction::Log => 1.0 / mu,
        LinkFunction::Logit => {
            let v = mu * (1.0 - mu);
            1.0 / v.max(1e-15)
        }
        LinkFunction::Identity => 1.0,
        // η = 1/μ ⇒ dη/dμ = −1/μ².
        LinkFunction::Reciprocal => -1.0 / (mu * mu).max(1e-15),
    }
}

// ───────────────────────── Deviance contribution ─────────────────────────

pub(super) fn dev_contribution_binom(y: f64, mu: f64) -> f64 {
    let t1 = if y > 0.0 { y * (y / mu).ln() } else { 0.0 };
    let t2 = if (1.0 - y) > 0.0 {
        (1.0 - y) * ((1.0 - y) / (1.0 - mu)).ln()
    } else {
        0.0
    };
    2.0 * (t1 + t2)
}
