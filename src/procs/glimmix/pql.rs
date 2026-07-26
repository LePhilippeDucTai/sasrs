use super::*;

// ───────────────────────── PQL (RSPL) loop, non-normal + random ─────────────

/// Result of the full GLIMMIX fit.
pub(super) struct GlimmixFit {
    /// Fixed-effects β̂.
    pub(super) beta: Vec<f64>,
    /// Var(β̂).
    pub(super) cov_beta: Vec<Vec<f64>>,
    /// Fitted means μ_i.
    pub(super) mu: Vec<f64>,
    /// σ²_u (random intercept), present iff a RANDOM statement was used.
    pub(super) sigma2_u: Option<f64>,
    /// σ²_e (residual / pseudo-residual).
    pub(super) sigma2_e: f64,
    /// -2 Res Log Pseudo-Likelihood (random case) else -2 LL placeholder.
    pub(super) neg2: f64,
    pub(super) iterations: usize,
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
        let (s2u, s2e, beta_new, cov, n2) = fit_vc(&z, x, subj_of, n_subjects, Some(&w))?;
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

        let diff: f64 = beta_new
            .iter()
            .zip(&beta)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt();
        let norm_old: f64 = beta.iter().map(|b| b * b).sum::<f64>().sqrt();

        beta = beta_new;
        u = u_new;

        if diff / (1.0 + norm_old) < 1e-6 {
            // Compute final μ for reporting.
            let mu: Vec<f64> = (0..n)
                .map(|i| inv_link(dot(&x[i], &beta) + u[subj_of[i]], lf))
                .collect();
            return Ok(GlimmixFit {
                beta,
                cov_beta: cov,
                mu,
                sigma2_u: Some(s2u),
                sigma2_e: s2e,
                neg2: n2,
                iterations,
                cov_parms: None,
            });
        }
    }

    // Did not converge within 50 — return last state.
    let mu: Vec<f64> = (0..n)
        .map(|i| inv_link(dot(&x[i], &beta) + u[subj_of[i]], lf))
        .collect();
    let (s2u, s2e, _, cov_beta, n2) = fit_vc(
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
        cov_beta,
        mu,
        sigma2_u: Some(s2u),
        sigma2_e: s2e,
        neg2: n2,
        iterations,
        cov_parms: None,
    })
}
