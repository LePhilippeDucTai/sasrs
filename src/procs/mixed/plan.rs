use super::*;

// ═════════════════════ General execute path ═════════════════════

// ───────────────────── General-path execute helpers ─────────────────────

/// Covariance-model plan for the general path.
pub(super) enum Plan {
    Repeated(CovType, String),
    RandomVc(String, CovType),
}

/// Determine the covariance model.
/// Priority: a REPEATED AR(1)/UN structure, else a RANDOM intercept VC/CS.
pub(super) fn determine_plan(ast: &MixedAst) -> Result<Plan> {
    let repeated = ast.repeated.as_ref();
    let random = ast.random.as_ref();

    let plan = if let Some(rep) = repeated {
        match rep.cov_type {
            CovType::Ar1 | CovType::Un => {
                let subj = rep.subject.as_ref().ok_or_else(|| {
                    SasError::runtime("REPEATED TYPE=AR(1)/UN requires SUBJECT= in PROC MIXED.")
                })?;
                Plan::Repeated(rep.cov_type, subj.clone())
            }
            CovType::Vc | CovType::Cs => {
                return Err(SasError::runtime(
                    "REPEATED TYPE=VC/CS is not yet implemented in PROC MIXED.",
                ));
            }
        }
    } else if let Some(rnd) = random {
        let is_intercept =
            rnd.effects.len() == 1 && rnd.effects[0].eq_ignore_ascii_case("intercept");
        if !is_intercept {
            return Err(SasError::runtime(
                "Only RANDOM INTERCEPT is implemented in PROC MIXED.",
            ));
        }
        let subj = rnd.subject.as_ref().ok_or_else(|| {
            SasError::runtime("RANDOM statement requires SUBJECT= in PROC MIXED.")
        })?;
        match rnd.cov_type {
            CovType::Vc | CovType::Cs => Plan::RandomVc(subj.clone(), rnd.cov_type),
            CovType::Ar1 | CovType::Un => {
                return Err(SasError::runtime(
                    "TYPE=AR(1)/UN on a RANDOM intercept is not yet implemented; \
                     use a REPEATED statement.",
                ));
            }
        }
    } else {
        return Err(SasError::runtime(
            "PROC MIXED currently requires a RANDOM or REPEATED statement with SUBJECT=.",
        ));
    };

    Ok(plan)
}

/// Complete observations after listwise deletion, with subject indexing.
pub(super) struct GenObs {
    pub(super) y: Vec<f64>,
    pub(super) kept_fixed: Vec<(String, Vec<Value>)>,
    /// Subject levels (sas_cmp order).
    pub(super) levels: Vec<Value>,
    pub(super) subj_of: Vec<usize>,
    pub(super) within_idx: Vec<usize>,
    pub(super) max_obs: usize,
    pub(super) n_not_used: usize,
    /// Row indices (in the input data set) of the observations used.
    pub(super) keep: Vec<usize>,
}

/// Listwise deletion + subject-level indexing over the decoded columns.
pub(super) fn build_observations_gen(
    resp_col: &[Value],
    subj_col: &[Value],
    fixed_cols: &[(String, Vec<Value>)],
    n_read: usize,
) -> Result<GenObs> {
    let mut keep: Vec<usize> = Vec::new();
    let mut n_not_used = 0usize;
    for i in 0..n_read {
        let y_ok = matches!(&resp_col[i], Value::Num(v) if !v.is_nan());
        let subj_ok = !subj_col[i].is_missing();
        let fixed_ok = fixed_cols.iter().all(|(_, c)| !c[i].is_missing());
        if y_ok && subj_ok && fixed_ok {
            keep.push(i);
        } else {
            n_not_used += 1;
        }
    }
    let n_used = keep.len();
    if n_used == 0 {
        return Err(SasError::runtime(
            "No complete observations available for PROC MIXED.",
        ));
    }

    let y: Vec<f64> = keep
        .iter()
        .map(|&i| match &resp_col[i] {
            Value::Num(v) => *v,
            _ => f64::NAN,
        })
        .collect();
    let subj_values: Vec<Value> = keep.iter().map(|&i| subj_col[i].clone()).collect();
    let kept_fixed: Vec<(String, Vec<Value>)> = fixed_cols
        .iter()
        .map(|(nm, c)| (nm.clone(), keep.iter().map(|&i| c[i].clone()).collect()))
        .collect();

    // Subject levels (sas_cmp order).
    let mut levels: Vec<Value> = Vec::new();
    for v in &subj_values {
        if !levels
            .iter()
            .any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
        {
            levels.push(v.clone());
        }
    }
    levels.sort_by(|a, b| a.sas_cmp(b));
    let n_subjects = levels.len();
    if n_subjects < 2 {
        return Err(SasError::runtime(
            "PROC MIXED requires at least 2 subjects.",
        ));
    }
    let level_index = |v: &Value| -> usize {
        levels
            .iter()
            .position(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
            .unwrap()
    };
    let subj_of: Vec<usize> = subj_values.iter().map(level_index).collect();

    // Within-subject position (order of appearance) and per-subject counts.
    let mut counts = vec![0usize; n_subjects];
    let mut within_idx = vec![0usize; n_used];
    for i in 0..n_used {
        let s = subj_of[i];
        within_idx[i] = counts[s];
        counts[s] += 1;
    }
    let max_obs = *counts.iter().max().unwrap_or(&0);

    Ok(GenObs {
        y,
        kept_fixed,
        levels,
        subj_of,
        within_idx,
        max_obs,
        n_not_used,
        keep,
    })
}

/// J02-P2 — check that the levels of the REPEATED effect agree with the order
/// of appearance used to index R: within each subject they must be strictly
/// increasing (global sorted order of the levels) and, for TYPE=UN (block
/// position = level) or TYPE=AR(1) (lag = level distance), without gaps.
/// The complete, sorted case is unaffected.
pub(super) fn check_repeated_effect(
    cov_type: CovType,
    effect: &str,
    values: &[Value],
    subjects: &[Value],
    subj_of: &[usize],
    within_idx: &[usize],
) -> Result<()> {
    if values.iter().any(Value::is_missing) {
        return Err(SasError::runtime(format!(
            "Missing values of the REPEATED effect {} are not supported in PROC MIXED; \
             they can affect results and cannot be ignored (planned: roadmap-avancee J06-P2).",
            effect.to_uppercase()
        )));
    }
    let mut levels: Vec<Value> = Vec::new();
    for v in values {
        if !levels
            .iter()
            .any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
        {
            levels.push(v.clone());
        }
    }
    levels.sort_by(|a, b| a.sas_cmp(b));
    let rank = |v: &Value| -> usize {
        levels
            .iter()
            .position(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
            .unwrap()
    };
    let rank: Vec<usize> = values.iter().map(rank).collect();
    let mut last: Vec<Option<usize>> = vec![None; subjects.len()];
    for (i, &r) in rank.iter().enumerate() {
        let s = subj_of[i];
        if let Some(prev) = last[s]
            && r <= prev
        {
            return Err(SasError::runtime(format!(
                "The levels of the REPEATED effect {} are not strictly increasing within \
                 subject {}; PROC MIXED indexes R by order of appearance, which would not \
                 match the effect levels (planned: roadmap-avancee J06-P2).",
                effect.to_uppercase(),
                value_label(&subjects[s])
            )));
        }
        last[s] = Some(r);
    }
    let mut first: Vec<Option<usize>> = vec![None; subjects.len()];
    for (i, &r) in rank.iter().enumerate() {
        let s = subj_of[i];
        let base = match cov_type {
            CovType::Un => 0,
            _ => *first[s].get_or_insert(r),
        };
        if r - base != within_idx[i] {
            return Err(SasError::runtime(format!(
                "Subject {} lacks levels of the REPEATED effect {}; with TYPE={} PROC MIXED \
                 indexes R by order of appearance, which would not match the effect levels \
                 (planned: roadmap-avancee J06-P2).",
                value_label(&subjects[s]),
                effect.to_uppercase(),
                if cov_type == CovType::Un {
                    "UN"
                } else {
                    "AR(1)"
                }
            )));
        }
    }
    Ok(())
}

pub(super) fn execute_general(ast: &MixedAst, session: &mut Session) -> Result<()> {
    let model = ast
        .model
        .as_ref()
        .ok_or_else(|| SasError::runtime("MODEL statement required in PROC MIXED."))?;

    // Determine the covariance model.
    let plan = determine_plan(ast)?;

    // ── Read dataset ────────────────────────────────────────────────────────
    let (ds, in_libref, in_table) = common::open_input(&ast.data, session)?;
    let n_read = ds.n_obs();

    let find_col = |nm: &str| -> Result<usize> {
        ds.vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(nm))
            .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", nm.to_uppercase())))
    };

    let resp_idx = find_col(&model.response)?;
    let resp_col = decode_column(&ds, resp_idx)?;

    let subject = match &plan {
        Plan::Repeated(_, s) => s.clone(),
        Plan::RandomVc(s, _) => s.clone(),
    };
    let subj_idx = find_col(&subject)?;
    let subj_col = decode_column(&ds, subj_idx)?;

    // Decode all variables referenced by the fixed effects.
    let mut fixed_cols: Vec<(String, Vec<Value>)> = Vec::new();
    for eff in &model.fixed {
        let idx = find_col(eff)?;
        fixed_cols.push((eff.clone(), decode_column(&ds, idx)?));
    }

    // ── Build complete observations (listwise deletion) ─────────────────────
    let GenObs {
        y,
        kept_fixed,
        levels,
        subj_of,
        within_idx,
        max_obs,
        n_not_used,
        keep,
    } = build_observations_gen(&resp_col, &subj_col, &fixed_cols, n_read)?;
    let n_used = y.len();
    let n_subjects = levels.len();

    // J02-P2 — the repeated effect used to be skipped: R is indexed by order
    // of appearance within a subject, so the given effect must agree with it.
    if let Plan::Repeated(cov_type, _) = &plan
        && let Some(effect) = ast.repeated.as_ref().and_then(|r| r.effect.as_ref())
    {
        let col = decode_column(&ds, find_col(effect)?)?;
        let kept: Vec<Value> = keep.iter().map(|&i| col[i].clone()).collect();
        check_repeated_effect(*cov_type, effect, &kept, &levels, &subj_of, &within_idx)?;
    }

    // ── Fixed-effects design ────────────────────────────────────────────────
    let design = build_design(
        &kept_fixed,
        &ast.class_vars,
        &model.fixed,
        model.noint,
        n_used,
    )?;
    if design.is_empty() {
        return Err(SasError::runtime(
            "PROC MIXED MODEL has no fixed-effects columns (NOINT with no effects).",
        ));
    }
    let p = design.len();
    let labels: Vec<String> = design.iter().map(|d| d.label.clone()).collect();
    let x: Vec<Vec<f64>> = (0..n_used)
        .map(|i| design.iter().map(|c| c.values[i]).collect())
        .collect();

    // ── Determine covariance model + initial unconstrained params ───────────
    let (cov, u0): (GenCov, Vec<f64>) = initial_cov_params(&plan, &y, max_obs);

    // ── Optimize ────────────────────────────────────────────────────────────
    let fit = fit_gen(&y, &x, cov, &subj_of, &within_idx, ast.method, &u0)?;
    if !fit.converged {
        // SAS MIXED reports non-convergence as a WARNING, not a NOTE
        // (Kiernan, Tao & Gibbs 2012, SGF 332-2012: "WARNING: Did not
        // converge." / Convergence Status).
        session
            .log
            .warning("Convergence was not attained within the iteration limit in PROC MIXED.");
    }

    // ── Listing ─────────────────────────────────────────────────────────────
    print_model_information_gen(session, ast, model, &plan, cov, &in_libref, &in_table);
    print_class_level_information_gen(session, ast, &subject, &levels, n_subjects, &kept_fixed);

    let n_cov = n_cov_params(cov);
    print_dimensions_gen(session, cov, n_cov, p, n_subjects, max_obs);
    print_number_of_observations_gen(session, n_read, n_used, n_not_used);
    print_convergence_status_gen(session, &fit);

    // Covariance Parameter Estimates.
    let is_cs = matches!(&plan, Plan::RandomVc(_, CovType::Cs));
    print_covariance_parameter_estimates_gen(session, cov, &fit, &subject, is_cs);

    print_fit_statistics_gen(session, ast, &fit, n_cov, n_used, p, n_subjects);

    // Solution for Fixed Effects.
    if model.solution {
        print_fixed_solutions_gen(session, &fit, &labels, p, n_subjects);
    }

    Ok(())
}

/// Rows for the "Covariance Parameter Estimates" table in the general path.
pub(super) fn cov_parm_rows(
    cov: GenCov,
    theta: &[f64],
    subject: &str,
    is_cs: bool,
) -> Vec<Vec<String>> {
    match cov {
        GenCov::RandomVc => {
            let name = if is_cs { "CS" } else { "Intercept" };
            vec![
                vec![name.into(), subject.to_string(), fmt4(theta[0])],
                vec!["Residual".into(), String::new(), fmt4(theta[1])],
            ]
        }
        GenCov::RepeatedAr1 => {
            // AR(1) → ρ (=theta[0]); Residual → σ² (=theta[1]).
            vec![
                vec!["AR(1)".into(), subject.to_string(), fmt4(theta[0])],
                vec!["Residual".into(), String::new(), fmt4(theta[1])],
            ]
        }
        GenCov::RepeatedUn { t } => {
            let mut rows = Vec::new();
            let mut k = 0;
            for r in 0..t {
                for c in 0..=r {
                    rows.push(vec![
                        format!("UN({},{})", r + 1, c + 1),
                        subject.to_string(),
                        fmt4(theta[k]),
                    ]);
                    k += 1;
                }
            }
            rows
        }
    }
}
