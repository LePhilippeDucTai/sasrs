use super::*;

// ───────────────────────── Execute ─────────────────────────

// ───────────────────────── Execute helpers ─────────────────────────

/// METHOD / DIST / LINK / RANDOM guards plus NOTEs for parse-accepted but
/// deferred features.
pub(super) fn check_guards(ast: &GlimmixAst, model: &ModelSpec, session: &mut Session) -> Result<()> {
    // METHOD guards. LAPLACE is supported for a single VC random intercept;
    // QUAD remains deferred (documented NOTE).
    match ast.method {
        Method::Rspl => {}
        Method::Quad => {
            return Err(SasError::runtime(
                "METHOD=QUAD is not yet implemented for PROC GLIMMIX; use METHOD=LAPLACE or RSPL.",
            ));
        }
        Method::Laplace => {
            // LAPLACE requires a single random intercept with TYPE=VC.
            match &ast.random {
                None => {}
                Some(r) => {
                    let is_intercept = r.effects.len() == 1
                        && r.effects[0].eq_ignore_ascii_case("intercept");
                    if !is_intercept
                        || !matches!(r.cov_type, CovType::Vc | CovType::Cs)
                        || r.subject.is_none()
                    {
                        return Err(SasError::runtime(
                            "METHOD=LAPLACE in PROC GLIMMIX is limited to a single \
                             RANDOM INTERCEPT with TYPE=VC and SUBJECT=; AR(1)/UN or \
                             multiple random effects are not supported under LAPLACE.",
                        ));
                    }
                }
            }
        }
    }

    // DIST guards.
    match model.dist {
        Distribution::Normal | Distribution::Poisson | Distribution::Binary => {}
        Distribution::Gamma => {
            return Err(SasError::runtime(
                "DIST=GAMMA is not yet implemented for PROC GLIMMIX.",
            ));
        }
        Distribution::NegBinomial => {
            return Err(SasError::runtime(
                "DIST=NEGBINOMIAL is not yet implemented for PROC GLIMMIX.",
            ));
        }
    }

    // LINK guards. Probit/Cloglog are valid only for the binary distribution.
    match model.link {
        LinkFunction::Identity | LinkFunction::Log | LinkFunction::Logit => {}
        LinkFunction::Probit | LinkFunction::Cloglog => {
            if model.dist != Distribution::Binary {
                return Err(SasError::runtime(
                    "LINK=PROBIT/CLOGLOG requires DIST=BINARY in PROC GLIMMIX.",
                ));
            }
        }
    }

    // RANDOM guards.
    if let Some(r) = &ast.random {
        // AR(1)/UN are accepted as within-subject (repeated) covariance
        // structures and require SUBJECT= to order observations.
        let is_intercept =
            r.effects.len() == 1 && r.effects[0].eq_ignore_ascii_case("intercept");
        if !is_intercept {
            return Err(SasError::runtime(
                "Only RANDOM INTERCEPT is implemented in PROC GLIMMIX.",
            ));
        }
        if r.subject.is_none() {
            return Err(SasError::runtime(
                "RANDOM statement requires SUBJECT= in PROC GLIMMIX.",
            ));
        }
    }

    // NOTEs for parse-accepted / deferred features.
    for lbl in &ast.estimate_labels {
        session.log.note(&format!(
            "ESTIMATE '{}' is parse-accepted but not implemented in PROC GLIMMIX.",
            lbl
        ));
    }
    for lbl in &ast.contrast_labels {
        session.log.note(&format!(
            "CONTRAST '{}' is parse-accepted but not implemented in PROC GLIMMIX.",
            lbl
        ));
    }
    if !ast.lsmeans.is_empty() {
        session
            .log
            .note("LSMEANS is parse-accepted but not implemented in PROC GLIMMIX.");
    }
    if ast.weight_var.is_some() {
        session
            .log
            .note("WEIGHT statement is parse-accepted but not implemented in PROC GLIMMIX.");
    }

    Ok(())
}

/// Determine the binomial event level (EVENT= / DESCENDING / default).
/// Returns `None` for non-binary distributions.
pub(super) fn determine_event_level(
    model: &ModelSpec,
    resp_col: &[Value],
    n_read: usize,
) -> Result<Option<Value>> {
    let mut event_level: Option<Value> = None;
    if model.dist == Distribution::Binary {
        let levels = crate::procs::lincom::class_levels(resp_col.iter().take(n_read));
        if levels.len() != 2 {
            return Err(SasError::runtime(format!(
                "Response variable {} must have exactly 2 non-missing levels for DIST=BINARY (found {}).",
                model.response.to_uppercase(),
                levels.len()
            )));
        }
        let lvl: Value = if let Some(ev) = &model.event {
            levels
                .iter()
                .find(|l| value_matches_event(l, ev))
                .cloned()
                .ok_or_else(|| {
                    SasError::runtime(format!(
                        "Event value '{}' not found in response variable {}.",
                        ev,
                        model.response.to_uppercase()
                    ))
                })?
        } else if model.descending {
            levels[1].clone()
        } else {
            levels[0].clone()
        };
        event_level = Some(lvl);
    }
    Ok(event_level)
}

/// Observations kept after listwise deletion, with the encoded response.
pub(super) struct KeptObs {
    pub(super) y: Vec<f64>,
    pub(super) freq: Vec<f64>,
    pub(super) subj_values: Vec<Value>,
    pub(super) kept_fixed: Vec<(String, Vec<Value>)>,
    pub(super) n_not_used: usize,
}

/// Listwise deletion + response encoding over the raw decoded columns.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_observations(
    model: &ModelSpec,
    class_vars: &[String],
    resp_col: &[Value],
    fixed_cols_full: &[(String, Vec<Value>)],
    freq_col: &Option<Vec<Value>>,
    subj_col: &Option<Vec<Value>>,
    event_level: &Option<Value>,
    n_read: usize,
) -> KeptObs {
    // Which fixed effects are CLASS variables (decoded char/num levels) vs.
    // continuous (must be numeric).
    let is_class_var = |nm: &str| class_vars.iter().any(|c| c.eq_ignore_ascii_case(nm));

    let mut y: Vec<f64> = Vec::new();
    let mut freq: Vec<f64> = Vec::new();
    let mut subj_values: Vec<Value> = Vec::new();
    let mut kept_fixed: Vec<(String, Vec<Value>)> = fixed_cols_full
        .iter()
        .map(|(nm, _)| (nm.clone(), Vec::new()))
        .collect();
    let mut n_not_used = 0usize;

    for i in 0..n_read {
        if resp_col[i].is_missing() {
            n_not_used += 1;
            continue;
        }
        // FREQ weight.
        let w = match &freq_col {
            Some(fc) => match value_to_num(&fc[i]) {
                Some(f) if !f.is_nan() && f > 0.0 => f,
                _ => {
                    n_not_used += 1;
                    continue;
                }
            },
            None => 1.0,
        };
        // Validate fixed-effect predictors: CLASS vars just need non-missing,
        // continuous vars must be numeric & non-missing.
        let mut ok = true;
        for (nm, col) in fixed_cols_full {
            let v = &col[i];
            if is_class_var(nm) {
                if v.is_missing() {
                    ok = false;
                    break;
                }
            } else {
                match value_to_num(v) {
                    Some(f) if !f.is_nan() => {}
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
        }
        if !ok {
            n_not_used += 1;
            continue;
        }
        // Subject.
        if let Some(sc) = &subj_col {
            if sc[i].is_missing() {
                n_not_used += 1;
                continue;
            }
        }
        // Response encoding.
        let yi = if model.dist == Distribution::Binary {
            let ev = event_level.as_ref().unwrap();
            if resp_col[i].sas_cmp(ev) == std::cmp::Ordering::Equal {
                1.0
            } else {
                0.0
            }
        } else {
            match value_to_num(&resp_col[i]) {
                Some(v) if !v.is_nan() => v,
                _ => {
                    n_not_used += 1;
                    continue;
                }
            }
        };
        // Commit the kept observation.
        y.push(yi);
        freq.push(w);
        if let Some(sc) = &subj_col {
            subj_values.push(sc[i].clone());
        }
        for (k, (_, col)) in fixed_cols_full.iter().enumerate() {
            kept_fixed[k].1.push(col[i].clone());
        }
    }

    KeptObs {
        y,
        freq,
        subj_values,
        kept_fixed,
        n_not_used,
    }
}

/// Map subject values to 0-based level indices (levels in `sas_cmp` order).
pub(super) fn index_subjects(subj_values: &[Value], has_subject: bool) -> (Vec<usize>, Vec<Value>) {
    if has_subject {
        let mut levels: Vec<Value> = Vec::new();
        for v in subj_values {
            if !levels.iter().any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal) {
                levels.push(v.clone());
            }
        }
        levels.sort_by(|a, b| a.sas_cmp(b));
        let idx: Vec<usize> = subj_values
            .iter()
            .map(|v| {
                levels
                    .iter()
                    .position(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
                    .unwrap()
            })
            .collect();
        (idx, levels)
    } else {
        (Vec::new(), Vec::new())
    }
}

/// Shared inputs of the estimation dispatch.
pub(super) struct FitContext<'a> {
    pub(super) y: &'a [f64],
    pub(super) x: &'a [Vec<f64>],
    pub(super) freq: &'a [f64],
    pub(super) subj_of: &'a [usize],
    pub(super) within_idx: &'a [usize],
    pub(super) n_subjects: usize,
    pub(super) n_total: f64,
}

/// Estimation dispatch: GLM (no random), Laplace ML, repeated AR(1)/UN,
/// Normal REML, or the PQL loop.
pub(super) fn compute_fit(
    model: &ModelSpec,
    ctx: &FitContext,
    rep_cov: Option<RepCov>,
    use_laplace: bool,
    has_random: bool,
) -> Result<GlimmixFit> {
    let &FitContext {
        y,
        x,
        freq,
        subj_of,
        within_idx,
        n_subjects,
        n_total,
    } = ctx;
    let n_used = y.len();
    let p = x[0].len();

    let fit: GlimmixFit = if !has_random {
        // No random effects → IRLS GLM (≡ ordinary GLM MLE under any METHOD).
        let g = fit_glm(y, x, freq, model.dist, model.link)?;
        let sigma2_e = if model.dist == Distribution::Normal {
            // residual variance = MSE.
            let sse: f64 = (0..n_used).map(|i| freq[i] * (y[i] - g.mu[i]).powi(2)).sum();
            sse / (n_total - p as f64).max(1.0)
        } else {
            1.0
        };
        GlimmixFit {
            beta: g.beta,
            cov_beta: g.cov_beta,
            mu: g.mu,
            sigma2_u: None,
            sigma2_e,
            neg2: 0.0,
            iterations: g.iterations,
            cov_parms: None,
        }
    } else if use_laplace {
        // METHOD=LAPLACE single random intercept → true ML by Laplace.
        let lf = fit_laplace(
            y, x, freq, subj_of, n_subjects, model.dist, model.link,
        )?;
        GlimmixFit {
            beta: lf.beta,
            cov_beta: lf.cov_beta,
            mu: lf.mu,
            sigma2_u: Some(lf.sigma2_u),
            sigma2_e: lf.sigma2_e,
            neg2: lf.neg2,
            iterations: lf.iterations,
            cov_parms: None,
        }
    } else if let Some(rep) = rep_cov {
        // RANDOM with TYPE=AR(1)/UN: the within-subject repeated covariance R
        // is fit as a weighted LMM at each RSPL step. For Normal/Identity this
        // is the exact REML (no PQL iteration); for non-normal links we run the
        // RSPL working-variate loop with R as the working covariance.
        fit_rspl_rep(
            y, x, freq, subj_of, within_idx, rep, model.dist, model.link,
        )?
    } else if model.dist == Distribution::Normal {
        // Normal + random → PQL == REML, closed-form / profile.
        let (s2u, s2e, beta, cov, neg2) =
            fit_vc(y, x, subj_of, n_subjects, None)?;
        let mu = (0..n_used).map(|i| dot(&x[i], &beta)).collect();
        GlimmixFit {
            beta,
            cov_beta: cov,
            mu,
            sigma2_u: Some(s2u),
            sigma2_e: s2e,
            neg2,
            iterations: 1,
            cov_parms: None,
        }
    } else {
        // Non-normal + random → full PQL loop (VC).
        fit_pql(y, x, freq, subj_of, n_subjects, model.dist, model.link)?
    };

    Ok(fit)
}
