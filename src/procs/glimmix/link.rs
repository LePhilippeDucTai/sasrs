use super::*;

/// Standard normal pdf φ(z).
pub(super) fn norm_pdf(z: f64) -> f64 {
    (-0.5 * z * z).exp() / (std::f64::consts::TAU).sqrt()
}

pub(super) fn canonical_link(dist: Distribution) -> LinkFunction {
    match dist {
        Distribution::Normal => LinkFunction::Identity,
        Distribution::Poisson => LinkFunction::Log,
        Distribution::Binary => LinkFunction::Logit,
        Distribution::Gamma => LinkFunction::Log,
        Distribution::NegBinomial => LinkFunction::Log,
    }
}

// ───────────────────────── Link / variance ─────────────────────────

pub(super) fn inv_link(eta: f64, lf: LinkFunction) -> f64 {
    match lf {
        LinkFunction::Identity => eta,
        LinkFunction::Log => eta.exp().max(1e-10),
        LinkFunction::Logit => 1.0 / (1.0 + (-eta).exp()),
        LinkFunction::Probit => probnorm(eta).clamp(1e-12, 1.0 - 1e-12),
        LinkFunction::Cloglog => {
            // μ = 1 − exp(−exp(η))
            (1.0 - (-(eta.exp())).exp()).clamp(1e-12, 1.0 - 1e-12)
        }
    }
}

/// dμ/dη (derivative of the inverse link).
pub(super) fn dmu_deta(eta: f64, lf: LinkFunction) -> f64 {
    match lf {
        LinkFunction::Identity => 1.0,
        LinkFunction::Log => eta.exp().max(1e-10),
        LinkFunction::Logit => {
            let mu = 1.0 / (1.0 + (-eta).exp());
            (mu * (1.0 - mu)).max(1e-15)
        }
        LinkFunction::Probit => norm_pdf(eta).max(1e-15),
        LinkFunction::Cloglog => {
            // dμ/dη = exp(η − exp(η))
            (eta - eta.exp()).exp().max(1e-15)
        }
    }
}

pub(super) fn variance(mu: f64, dist: Distribution) -> f64 {
    match dist {
        Distribution::Normal => 1.0,
        Distribution::Poisson => mu.max(1e-15),
        Distribution::Binary => (mu * (1.0 - mu)).max(1e-15),
        _ => 1.0,
    }
}

// ═══════════════════════ METHOD=LAPLACE (single random intercept) ════════════
//
// True maximum-likelihood for the single random-intercept GLMM via the Laplace
// approximation: per subject s, integrate u_s ~ N(0, σ²_u) out by locating the
// per-subject mode (inner Newton on η_i = x_i'β + u) and using the curvature at
// the mode. Maximise over (β, σ²_u [, σ²_e for Normal]) with the same
// Nelder-Mead-with-restarts + coordinate-polish optimizer used elsewhere.

/// Per-observation log-density and its first/second derivatives w.r.t. the
/// linear predictor η. `scale` is the residual variance σ²_e (Normal only).
/// Returns (log f, d log f/dη, d² log f/dη²).
pub(super) fn log_density(y: f64, eta: f64, dist: Distribution, lf: LinkFunction, scale: f64) -> (f64, f64, f64) {
    match (dist, lf) {
        (Distribution::Normal, LinkFunction::Identity) => {
            let s2 = scale.max(1e-12);
            let r = y - eta;
            let lf = -0.5 * (std::f64::consts::TAU * s2).ln() - r * r / (2.0 * s2);
            (lf, r / s2, -1.0 / s2)
        }
        (Distribution::Poisson, LinkFunction::Log) => {
            let mu = eta.exp();
            let lf = y * eta - mu - ln_factorial_f(y);
            (lf, y - mu, -mu)
        }
        (Distribution::Binary, LinkFunction::Logit) => {
            let mu = 1.0 / (1.0 + (-eta).exp());
            // log f = y·η − log(1+e^η)
            let lf = y * eta - (1.0 + eta.exp()).ln();
            (lf, y - mu, -(mu * (1.0 - mu)))
        }
        _ => {
            // General binary link (probit / cloglog): use μ(η) with analytic
            // first derivative and a finite-difference second derivative.
            let mu = inv_link(eta, lf).clamp(1e-12, 1.0 - 1e-12);
            let lf_val = y * mu.ln() + (1.0 - y) * (1.0 - mu).ln();
            let d = dmu_deta(eta, lf);
            let g = (y - mu) / (mu * (1.0 - mu)) * d;
            // Second derivative via central difference on g(η).
            let h = 1e-4;
            let mu_p = inv_link(eta + h, lf).clamp(1e-12, 1.0 - 1e-12);
            let mu_m = inv_link(eta - h, lf).clamp(1e-12, 1.0 - 1e-12);
            let dp = dmu_deta(eta + h, lf);
            let dm = dmu_deta(eta - h, lf);
            let gp = (y - mu_p) / (mu_p * (1.0 - mu_p)) * dp;
            let gm = (y - mu_m) / (mu_m * (1.0 - mu_m)) * dm;
            (lf_val, g, (gp - gm) / (2.0 * h))
        }
    }
}

/// log(y!) for non-negative integer-valued y (Poisson normalising constant).
pub(super) fn ln_factorial_f(y: f64) -> f64 {
    if y <= 1.0 {
        return 0.0;
    }
    // Σ ln k for small y, Stirling-via-lgamma for larger.
    crate::stat::ln_gamma(y + 1.0)
}
