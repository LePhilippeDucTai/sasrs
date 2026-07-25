use super::*;

/// Laplace per-subject log-likelihood contribution for random effect variance
/// σ²_u, fixed predictor `xb_i = x_i'β`, FREQ weights `w_i`. Inner Newton finds
/// û maximising h(u)=Σ w_i log f(y_i|xb_i+u) − u²/(2σ²_u); the Laplace value is
/// h(û) − 0.5 log(σ²_u) − 0.5 log(−h''(û)) (constants in 2π cancel across the
/// −u²/2σ²_u prior normaliser and the Laplace 2π factor).
pub(super) fn laplace_subject_ll(
    ys: &[f64],
    xb: &[f64],
    ws: &[f64],
    sigma2_u: f64,
    dist: Distribution,
    lf: LinkFunction,
    scale: f64,
) -> f64 {
    let s2u = sigma2_u.max(1e-12);
    // Inner Newton for the mode û.
    let mut u = 0.0_f64;
    for _ in 0..100 {
        let mut g = -u / s2u; // d/du of −u²/2σ²_u
        let mut hh = -1.0 / s2u;
        for k in 0..ys.len() {
            let (_, gi, hi) = log_density(ys[k], xb[k] + u, dist, lf, scale);
            g += ws[k] * gi;
            hh += ws[k] * hi;
        }
        if hh.abs() < 1e-300 {
            break;
        }
        let step = g / hh;
        u -= step;
        if step.abs() < 1e-10 {
            break;
        }
    }
    // Evaluate h(û) and curvature.
    let mut hval = -(u * u) / (2.0 * s2u);
    let mut hpp = -1.0 / s2u;
    for k in 0..ys.len() {
        let (lfi, _, hi) = log_density(ys[k], xb[k] + u, dist, lf, scale);
        hval += ws[k] * lfi;
        hpp += ws[k] * hi;
    }
    let neg_hpp = (-hpp).max(1e-300);
    // ∫ exp(h(u)) du ≈ exp(h(û)) · sqrt(2π / −h''); with the N(0,σ²_u) prior the
    // 2π and σ²_u normalisers combine to −0.5 ln(σ²_u) − 0.5 ln(−h'').
    hval - 0.5 * s2u.ln() - 0.5 * neg_hpp.ln()
}

/// Total Laplace −2 log-likelihood for the single-random-intercept GLMM.
pub(super) fn laplace_neg2(
    y: &[f64],
    x: &[Vec<f64>],
    freq: &[f64],
    subj_of: &[usize],
    n_subjects: usize,
    beta: &[f64],
    sigma2_u: f64,
    dist: Distribution,
    lf: LinkFunction,
    scale: f64,
) -> f64 {
    // Group rows by subject.
    let mut groups: Vec<Vec<usize>> = vec![Vec::new(); n_subjects];
    for (i, &s) in subj_of.iter().enumerate() {
        groups[s].push(i);
    }
    let mut total = 0.0;
    for g in &groups {
        let ys: Vec<f64> = g.iter().map(|&i| y[i]).collect();
        let xb: Vec<f64> = g.iter().map(|&i| dot(&x[i], beta)).collect();
        let ws: Vec<f64> = g.iter().map(|&i| freq[i]).collect();
        total += laplace_subject_ll(&ys, &xb, &ws, sigma2_u, dist, lf, scale);
    }
    -2.0 * total
}

/// Result of a Laplace ML fit.
pub(super) struct LaplaceFit {
    pub(super) beta: Vec<f64>,
    pub(super) cov_beta: Vec<Vec<f64>>,
    pub(super) mu: Vec<f64>,
    pub(super) sigma2_u: f64,
    pub(super) sigma2_e: f64,
    pub(super) neg2: f64,
    pub(super) iterations: usize,
}

/// Fit the single random-intercept GLMM by Laplace ML. Optimises over the
/// unconstrained vector (β, log σ²_u [, log σ²_e for Normal]) with Nelder-Mead
/// restarts + coordinate polish; Var(β̂) from the numeric Hessian of −2logL/2.
#[allow(clippy::too_many_arguments)]
pub(super) fn fit_laplace(
    y: &[f64],
    x: &[Vec<f64>],
    freq: &[f64],
    subj_of: &[usize],
    n_subjects: usize,
    dist: Distribution,
    lf: LinkFunction,
) -> Result<LaplaceFit> {
    let n = y.len();
    let p = x[0].len();
    let is_normal = dist == Distribution::Normal;

    // Starting values from the no-random GLM.
    let glm0 = fit_glm(y, x, freq, dist, lf)?;
    let var_y = {
        let m = y.iter().sum::<f64>() / n as f64;
        (y.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (n.max(1) as f64)).max(1e-4)
    };

    // Unconstrained layout: u[0..p] = β; u[p] = log σ²_u; (Normal) u[p+1]=log σ²_e.
    let np = p + 1 + if is_normal { 1 } else { 0 };
    let mut u0 = vec![0.0; np];
    u0[..p].copy_from_slice(&glm0.beta);
    u0[p] = (0.5 * var_y).max(1e-3).ln();
    if is_normal {
        u0[p + 1] = (0.5 * var_y).max(1e-3).ln();
    }

    let eval = |u: &[f64]| -> f64 {
        let beta = &u[..p];
        let s2u = u[p].exp();
        let scale = if is_normal { u[p + 1].exp() } else { 1.0 };
        let v = laplace_neg2(y, x, freq, subj_of, n_subjects, beta, s2u, dist, lf, scale);
        if v.is_finite() {
            v
        } else {
            1e30
        }
    };

    let mut u_best = u0.clone();
    let mut f_best = eval(&u0);
    let mut step = 0.5_f64;
    for restart in 0..8 {
        let (u_r, f_r, _iters, conv) = nelder_mead(&eval, &u_best, step, 4000, 1e-12, 1e-10);
        if f_r <= f_best {
            f_best = f_r;
            u_best = u_r;
        }
        if restart >= 2 && conv {
            break;
        }
        step *= 0.4;
    }
    polish_coord(&eval, &mut u_best, &mut f_best, 1e-10);

    let beta: Vec<f64> = u_best[..p].to_vec();
    let sigma2_u = u_best[p].exp();
    let sigma2_e = if is_normal { u_best[p + 1].exp() } else { 1.0 };

    // Var(β̂) ≈ inverse of the observed information = Hessian of (−2logL/2)=−logL
    // w.r.t. β, by central finite differences (σ's held at the optimum).
    let neg_ll = |b: &[f64]| -> f64 {
        0.5 * laplace_neg2(y, x, freq, subj_of, n_subjects, b, sigma2_u, dist, lf, sigma2_e)
    };
    let h = 1e-4;
    let mut hess = vec![vec![0.0; p]; p];
    let f0 = neg_ll(&beta);
    for a in 0..p {
        for b in a..p {
            let val = if a == b {
                let mut bp = beta.clone();
                bp[a] += h;
                let fp = neg_ll(&bp);
                let mut bm = beta.clone();
                bm[a] -= h;
                let fm = neg_ll(&bm);
                (fp - 2.0 * f0 + fm) / (h * h)
            } else {
                let mut bpp = beta.clone();
                bpp[a] += h;
                bpp[b] += h;
                let mut bpm = beta.clone();
                bpm[a] += h;
                bpm[b] -= h;
                let mut bmp = beta.clone();
                bmp[a] -= h;
                bmp[b] += h;
                let mut bmm = beta.clone();
                bmm[a] -= h;
                bmm[b] -= h;
                (neg_ll(&bpp) - neg_ll(&bpm) - neg_ll(&bmp) + neg_ll(&bmm)) / (4.0 * h * h)
            };
            hess[a][b] = val;
            hess[b][a] = val;
        }
    }
    let cov_beta = invert_matrix(&hess)?;

    let mu: Vec<f64> = (0..n).map(|i| inv_link(dot(&x[i], &beta), lf)).collect();

    Ok(LaplaceFit {
        beta,
        cov_beta,
        mu,
        sigma2_u,
        sigma2_e,
        neg2: f_best,
        iterations: 1,
    })
}
