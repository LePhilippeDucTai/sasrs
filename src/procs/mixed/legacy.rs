use super::*;

// ───────────────────────── Execute ─────────────────────────

/// Decide whether the request is exactly the legacy M28 case: a single random
/// intercept with TYPE=VC|CS, SUBJECT=, no REPEATED, and an intercept-only mean
/// (no fixed effects, no NOINT). This path is kept numerically and format
/// byte-identical to the m28 oracle.
pub(super) fn is_legacy_case(ast: &MixedAst) -> bool {
    let Some(model) = ast.model.as_ref() else {
        return false;
    };
    if !model.fixed.is_empty() || model.noint {
        return false;
    }
    if ast.repeated.is_some() {
        return false;
    }
    let Some(random) = ast.random.as_ref() else {
        return false;
    };
    if !matches!(random.cov_type, CovType::Vc | CovType::Cs) {
        return false;
    }
    if random.subject.is_none() {
        return false;
    }
    random.effects.len() == 1 && random.effects[0].eq_ignore_ascii_case("intercept")
}

pub(super) fn execute_legacy(ast: &MixedAst, session: &mut Session) -> Result<()> {
    // ── 1. Validate / guards ────────────────────────────────────────────────
    let model = ast
        .model
        .as_ref()
        .ok_or_else(|| SasError::runtime("MODEL statement required in PROC MIXED."))?;

    let random = ast.random.as_ref().ok_or_else(|| {
        SasError::runtime("PROC MIXED currently requires a RANDOM statement with SUBJECT=.")
    })?;

    let subject = random
        .subject
        .as_ref()
        .ok_or_else(|| SasError::runtime("RANDOM statement requires SUBJECT= in PROC MIXED."))?;

    // NOTEs for parse-accepted / deferred features.
    note_deferred_features_legacy(ast, model, session);

    // ── 2. Read dataset ─────────────────────────────────────────────────────
    let (ds, in_libref, in_table) = common::open_input(&ast.data, session)?;

    let n_read = ds.n_obs();

    let find_col = |nm: &str| -> Result<usize> {
        ds.vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(nm))
            .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", nm.to_uppercase())))
    };

    let resp_idx = find_col(&model.response)?;
    let subj_idx = find_col(subject)?;

    let resp_col = decode_column(&ds, resp_idx)?;
    let subj_col = decode_column(&ds, subj_idx)?;

    // ── 3. Build complete observations ──────────────────────────────────────
    let (y, subj_of, levels, n_not_used) = build_observations_legacy(&resp_col, &subj_col, n_read)?;
    let n_used = y.len();
    let n_subjects = levels.len();

    // Design matrix X: intercept-only.
    let x: Vec<Vec<f64>> = vec![vec![1.0]; n_used];

    // ── 4. Fit ──────────────────────────────────────────────────────────────
    let fit = fit_mixed(&y, &x, &subj_of, n_subjects, ast.method, ast.nobound)?;

    // Max observations per subject.
    let mut counts = vec![0usize; n_subjects];
    for &s in &subj_of {
        counts[s] += 1;
    }
    let max_obs = *counts.iter().max().unwrap_or(&0);

    // ── 5. Listing ──────────────────────────────────────────────────────────
    print_model_information_legacy(session, ast, model, random, &in_libref, &in_table);
    print_class_level_information_legacy(session, subject, &levels);
    print_dimensions_legacy(session, &fit, n_subjects, max_obs);
    print_number_of_observations_legacy(session, n_read, n_used, n_not_used);
    print_iteration_history_legacy(session, &fit);
    print_covariance_parameter_estimates_legacy(session, random, subject, &fit);
    print_fit_statistics_legacy(session, ast, &fit, n_subjects);

    // Solution for Fixed Effects.
    if model.solution {
        print_fixed_solution_legacy(session, &fit, n_subjects);
    }

    // Final NOTE if a fall-back unbalanced fit was used.
    let _ = fit.balanced;

    Ok(())
}

/// NOTEs for parse-accepted / deferred features (legacy path).
pub(super) fn note_deferred_features_legacy(
    ast: &MixedAst,
    model: &ModelSpec,
    session: &mut Session,
) {
    if ast.covtest {
        session
            .log
            .note("COVTEST is parse-accepted but not implemented in PROC MIXED.");
    }
    if ast.asycov {
        session
            .log
            .note("ASYCOV is parse-accepted but not implemented in PROC MIXED.");
    }
    if ast.nobound {
        session
            .log
            .note("NOBOUND is parse-accepted but not implemented in PROC MIXED.");
    }
    if let Some(d) = &model.ddfm {
        if d != "contain" {
            session.log.note(&format!(
                "DDFM={} is parse-accepted but not implemented; using CONTAIN.",
                d.to_uppercase()
            ));
        }
    }
    if model.nofit {
        session
            .log
            .note("NOFIT is parse-accepted but not implemented in PROC MIXED.");
    }
    if ast.repeated.is_some() {
        session
            .log
            .note("REPEATED statement is parse-accepted but not implemented in PROC MIXED.");
    }
    for lbl in &ast.estimate_labels {
        session.log.note(&format!(
            "ESTIMATE '{}' is parse-accepted but not implemented in PROC MIXED.",
            lbl
        ));
    }
    for lbl in &ast.contrast_labels {
        session.log.note(&format!(
            "CONTRAST '{}' is parse-accepted but not implemented in PROC MIXED.",
            lbl
        ));
    }
    if !ast.lsmeans.is_empty() {
        session
            .log
            .note("LSMEANS is parse-accepted but not implemented in PROC MIXED.");
    }
}

/// Complete observations for the legacy path: y, subject index per obs and
/// sorted subject levels (SAS comparison order). Guards: at least one complete
/// observation and at least 2 subjects.
pub(super) fn build_observations_legacy(
    resp_col: &[Value],
    subj_col: &[Value],
    n_read: usize,
) -> Result<(Vec<f64>, Vec<usize>, Vec<Value>, usize)> {
    let mut y: Vec<f64> = Vec::new();
    let mut subj_values: Vec<Value> = Vec::new();
    let mut n_not_used = 0usize;
    for i in 0..n_read {
        let yi = match &resp_col[i] {
            Value::Num(v) if !v.is_nan() => *v,
            _ => {
                n_not_used += 1;
                continue;
            }
        };
        if subj_col[i].is_missing() {
            n_not_used += 1;
            continue;
        }
        y.push(yi);
        subj_values.push(subj_col[i].clone());
    }

    if y.is_empty() {
        return Err(SasError::runtime(
            "No complete observations available for PROC MIXED.",
        ));
    }

    // Determine subject levels (sorted by SAS comparison order).
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
    let level_index = |v: &Value| -> usize {
        levels
            .iter()
            .position(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
            .unwrap()
    };
    let subj_of: Vec<usize> = subj_values.iter().map(|v| level_index(v)).collect();

    if levels.len() < 2 {
        return Err(SasError::runtime(
            "PROC MIXED requires at least 2 subjects.",
        ));
    }
    Ok((y, subj_of, levels, n_not_used))
}
