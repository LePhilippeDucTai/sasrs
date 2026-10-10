use super::*;

// ───────────────────────── Convergence criteria (J02-P3) ─────────────────────
//
// The listing used to print « Convergence criterion (GCONV=1E-8) satisfied. »
// for every fit although no gradient criterion is ever tested. Each solver now
// records the criterion it actually tests; the thresholds below are the ones
// the solvers use and the ones the listing names.

/// GLM-mode IRLS: max|Δβ| / (1 + max|β|) between two iterations (relative
/// parameter change, labelled XCONV like PROC LOGISTIC / GENMOD since J02-P1).
pub(super) const IRLS_XCONV: f64 = 1e-10;
/// Pseudo-likelihood outer loop: ‖Δψ‖ / (1 + ‖ψ‖) between two linearisations
/// (relative parameter change of the doubly iterative method, labelled PCONV).
pub(super) const PL_PCONV: f64 = 1e-6;
/// Nelder-Mead simplex: spread of the objective over the simplex below
/// FTOL·(1 + |f|) …
pub(super) const NM_FTOL: f64 = 1e-12;
/// … and largest vertex distance to the best vertex below XTOL.
pub(super) const NM_XTOL: f64 = 1e-10;
/// Golden-section search over λ = σ²_u/σ²_e: width of the bracketing interval.
pub(super) const GOLDEN_XTOL: f64 = 1e-10;

/// Convergence criterion actually tested by an estimation path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Criterion {
    /// GLM-mode IRLS ([`IRLS_XCONV`]).
    Irls,
    /// Pseudo-likelihood outer loop ([`PL_PCONV`]); an inner solver that did
    /// not converge makes the whole fit non-converged.
    PseudoLikelihood,
    /// Flag of the last Nelder-Mead run ([`NM_FTOL`], [`NM_XTOL`]).
    NelderMead,
    /// Golden-section search over λ ([`GOLDEN_XTOL`]).
    GoldenSection,
    /// Closed-form REML of the balanced one-way model: no iteration.
    ClosedForm,
}

impl Criterion {
    /// Listing line stating that the criterion was met.
    pub(super) fn satisfied_text(self) -> String {
        match self {
            Criterion::Irls => {
                format!("Convergence criterion (XCONV={IRLS_XCONV:E}) satisfied.")
            }
            Criterion::PseudoLikelihood => {
                format!("Convergence criterion (PCONV={PL_PCONV:E}) satisfied.")
            }
            Criterion::NelderMead => format!(
                "Convergence criterion (Nelder-Mead simplex: FTOL={NM_FTOL:E}, \
                 XTOL={NM_XTOL:E}) satisfied."
            ),
            Criterion::GoldenSection => format!(
                "Convergence criterion (golden-section search: XTOL={GOLDEN_XTOL:E}) satisfied."
            ),
            Criterion::ClosedForm => {
                "Closed-form REML solution (balanced data): no iteration required.".to_string()
            }
        }
    }
}

// ───────────────────────── PQL (RSPL) loop, non-normal + random ─────────────

/// Result of the full GLIMMIX fit.
pub(super) struct GlimmixFit {
    /// Fixed-effects β̂.
    pub(super) beta: Vec<f64>,
    /// Var(β̂).
    pub(super) cov_beta: Vec<Vec<f64>>,
    /// Fitted means μ_i.
    pub(super) mu: Vec<f64>,
    /// σ²_u (random intercept), present iff a G-side RANDOM effect was used.
    pub(super) sigma2_u: Option<f64>,
    /// σ²_e (residual / pseudo-residual).
    pub(super) sigma2_e: f64,
    /// -2 Res Log Pseudo-Likelihood (random case) else -2 LL placeholder.
    pub(super) neg2: f64,
    pub(super) iterations: usize,
    /// Whether the estimation criterion was actually met. The listing may only
    /// claim convergence when this is true (J02-P6).
    pub(super) converged: bool,
    /// Criterion the solver tested, named by the listing (J02-P3).
    pub(super) criterion: Criterion,
    /// σ²_u was truncated to the 0 boundary: the estimated G matrix is not
    /// positive definite (J02-P3).
    pub(super) g_not_pd: bool,
    /// The search over λ = σ²_u/σ²_e stopped at its upper bound (J02-P3).
    pub(super) lambda_capped: bool,
    /// Named covariance-parameter rows for the report. When `None`, the legacy
    /// VC display (Intercept σ²_u + Residual σ²_e) is used — byte-identical to
    /// the m28 oracle. When `Some`, these rows are printed verbatim (AR(1)/UN).
    pub(super) cov_parms: Option<Vec<CovParm>>,
}

/// A covariance-parameter row for the report (name, whether the Subject column
/// is shown, estimate).
#[derive(Clone)]
pub(super) struct CovParm {
    pub(super) name: String,
    pub(super) show_subject: bool,
    pub(super) estimate: f64,
}

/// PQL loop: linearise to a weighted mixed model at each step.
pub(super) fn fit_pql(
    y: &[f64],
    x: &[Vec<f64>],
    freq: &[f64],
    subj_of: &[usize],
    n_subjects: usize,
    dist: Distribution,
    lf: LinkFunction,
) -> Result<GlimmixFit> {
    let n = y.len();

    // Initialise β via OLS-ish IRLS (no random).
    let glm0 = fit_glm(y, x, freq, dist, lf)?;
    let mut beta = glm0.beta.clone();
    let mut u = vec![0.0_f64; n_subjects];
    let mut iterations = 0;

    for it in 0..50 {
        iterations = it + 1;
        // Working data (z, w) at current (β, u).
        let mut z = vec![0.0; n];
        let mut w = vec![0.0; n];
        for i in 0..n {
            let eta = dot(&x[i], &beta) + u[subj_of[i]];
            let mu = inv_link(eta, lf);
            let d = dmu_deta(eta, lf).max(1e-12);
            let v = variance(mu, dist);
            w[i] = freq[i] * d * d / v;
            z[i] = eta + (y[i] - mu) / d;
        }
        // Solve the weighted mixed model on (z, w): gives β, σ²_u, σ²_e, û.
        let vc = fit_vc(&z, x, subj_of, n_subjects, Some(&w))?;
        let (s2u, s2e) = (vc.sigma2_u, vc.sigma2_e);
        let beta_new = vc.beta;
        // Recover û (EBLUP) for the next linearisation:
        // û_s = σ²_u Σ_{i∈s} w_i (z_i - x_i'β) / (σ²_e + σ²_u Σ w_i).
        let mut num = vec![0.0; n_subjects];
        let mut den = vec![0.0; n_subjects];
        for i in 0..n {
            let r = z[i] - dot(&x[i], &beta_new);
            num[subj_of[i]] += w[i] * r;
            den[subj_of[i]] += w[i];
        }
        let mut u_new = vec![0.0; n_subjects];
        for s in 0..n_subjects {
            u_new[s] = s2u * num[s] / (s2e + s2u * den[s]).max(1e-12);
        }

        // Convergence must consider the FULL parameter vector (β, u): a
        // subject-separated fit can freeze β while the random effects keep
        // diverging at every linearisation (J02-P6).
        let diff_beta: f64 = beta_new
            .iter()
            .zip(&beta)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt();
        let diff_u: f64 = u_new
            .iter()
            .zip(&u)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt();
        let diff = diff_beta.max(diff_u);
        let norm_old: f64 = beta
            .iter()
            .chain(u.iter())
            .map(|b| b * b)
            .sum::<f64>()
            .sqrt();

        beta = beta_new;
        u = u_new;

        if diff / (1.0 + norm_old) < PL_PCONV {
            // Compute final μ for reporting.
            let mu: Vec<f64> = (0..n)
                .map(|i| inv_link(dot(&x[i], &beta) + u[subj_of[i]], lf))
                .collect();
            return Ok(GlimmixFit {
                beta,
                cov_beta: vc.cov_beta,
                mu,
                sigma2_u: Some(s2u),
                sigma2_e: s2e,
                neg2: vc.neg2,
                iterations,
                // The last inner search must have met its own criterion too.
                converged: vc.converged,
                criterion: Criterion::PseudoLikelihood,
                g_not_pd: vc.g_not_pd,
                lambda_capped: vc.lambda_capped,
                cov_parms: None,
            });
        }
    }

    // Did not converge within 50 — return last state.
    let mu: Vec<f64> = (0..n)
        .map(|i| inv_link(dot(&x[i], &beta) + u[subj_of[i]], lf))
        .collect();
    let vc = fit_vc(
        &{
            // recompute z one more time for variance estimates
            let mut z = vec![0.0; n];
            for i in 0..n {
                let eta = dot(&x[i], &beta) + u[subj_of[i]];
                let mu_i = inv_link(eta, lf);
                let d = dmu_deta(eta, lf).max(1e-12);
                z[i] = eta + (y[i] - mu_i) / d;
            }
            z
        },
        x,
        subj_of,
        n_subjects,
        Some(&{
            let mut w = vec![0.0; n];
            for i in 0..n {
                let eta = dot(&x[i], &beta) + u[subj_of[i]];
                let mu_i = inv_link(eta, lf);
                let d = dmu_deta(eta, lf).max(1e-12);
                let v = variance(mu_i, dist);
                w[i] = freq[i] * d * d / v;
            }
            w
        }),
    )?;
    Ok(GlimmixFit {
        beta,
        cov_beta: vc.cov_beta,
        mu,
        sigma2_u: Some(vc.sigma2_u),
        sigma2_e: vc.sigma2_e,
        neg2: vc.neg2,
        iterations,
        converged: false,
        criterion: Criterion::PseudoLikelihood,
        g_not_pd: vc.g_not_pd,
        lambda_capped: vc.lambda_capped,
        cov_parms: None,
    })
}
