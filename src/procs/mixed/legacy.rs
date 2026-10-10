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

    // Convergence / boundary diagnostics (J02-P6): the listing below only
    // claims convergence when the search actually met its criterion, and the
    // SAS-documented NOTEs are emitted instead of silence.
    if !fit.converged {
        session
            .log
            .warning("Convergence was not attained within the iteration limit in PROC MIXED.");
    }
    if fit.g_not_pd {
        // "NOTE: Estimated G matrix is not positive definite." — SAS Usage
        // Note 22614; Kiernan, Tao & Gibbs (2012), "Tips and Strategies for
        // Mixed Modeling with SAS/STAT Procedures" (SGF 332-2012).
        session
            .log
            .note("Estimated G matrix is not positive definite.");
    }
    if fit.lambda_capped {
        session.log.note(
            "The variance component ratio search reached its boundary (lambda=1000) \
             in PROC MIXED; the estimate may be unreliable.",
        );
    }

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
    print_iteration_history_legacy(session, ast.method, &fit);
    print_covariance_parameter_estimates_legacy(session, random, subject, &fit);
    print_fit_statistics_legacy(session, ast, &fit, n_subjects);

    // Solution for Fixed Effects.
    if model.solution {
        print_fixed_solution_legacy(session, &fit, n_subjects);
    }

    Ok(())
}

/// Observations complètes du chemin legacy : `(y, indice de sujet par obs,
/// niveaux de sujet triés, nombre d'obs écartées)`.
type LegacyObservations = (Vec<f64>, Vec<usize>, Vec<Value>, usize);

/// Complete observations for the legacy path: y, subject index per obs and
/// sorted subject levels (SAS comparison order). Guards: at least one complete
/// observation and at least 2 subjects.
pub(super) fn build_observations_legacy(
    resp_col: &[Value],
    subj_col: &[Value],
    n_read: usize,
) -> Result<LegacyObservations> {
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
    let subj_of: Vec<usize> = subj_values.iter().map(level_index).collect();

    if levels.len() < 2 {
        return Err(SasError::runtime(
            "PROC MIXED requires at least 2 subjects.",
        ));
    }
    Ok((y, subj_of, levels, n_not_used))
}
