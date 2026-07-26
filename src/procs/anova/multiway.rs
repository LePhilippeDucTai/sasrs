use super::*;

// ───────────────────── Multiway execute helpers ─────────────────────

/// Print the listing page header, the Class Level Information table and the
/// observation count for the multiway path.
pub(super) fn print_class_level_info_multiway(
    session: &mut Session,
    ast: &AnovaAst,
    class_cols: &std::collections::HashMap<String, (String, Vec<Value>)>,
    n_obs: usize,
) {
    session.listing.page_header();
    centered(session, "The ANOVA Procedure");
    session.listing.blank();

    centered(session, "Class Level Information");
    session.listing.blank();

    let cli_headers: Vec<String> = vec!["Class".into(), "Levels".into(), "Values".into()];
    let cli_aligns = vec![Align::Left, Align::Right, Align::Left];
    let mut cli_rows: Vec<Vec<String>> = Vec::new();

    for class_var in &ast.class_vars {
        let (disp_name, col) = &class_cols[&class_var.to_uppercase()];
        let levels = crate::procs::lincom::class_levels(col.iter().take(n_obs));
        let values_str: Vec<String> = levels.iter().map(value_label).collect();
        cli_rows.push(vec![
            disp_name.clone(),
            format!("{}", levels.len()),
            values_str.join(" "),
        ]);
    }
    session
        .listing
        .write_table(&cli_headers, &cli_aligns, &cli_rows);
    session.listing.blank();
    session.listing.write_line(&format!(
        "               Number of Observations Read     {}",
        n_obs
    ));
    session.listing.blank();
    session.listing.blank();
}

/// Multiway fit results for one dependent variable.
pub(super) struct MultiwayFit {
    pub(super) y: Vec<f64>,
    /// Levels per used CLASS var (sas_cmp order over usable rows).
    pub(super) var_levels: std::collections::HashMap<String, Vec<Value>>,
    /// Per-CLASS-var level codes for each usable observation.
    pub(super) var_codes: std::collections::HashMap<String, Vec<usize>>,
    pub(super) term_dfs: Vec<usize>,
    pub(super) y_bar: f64,
    pub(super) sst: f64,
    pub(super) sse_full: f64,
    pub(super) ssm: f64,
    pub(super) df_model: f64,
    pub(super) df_error: f64,
    pub(super) df_total: f64,
    pub(super) msm: f64,
    pub(super) mse: f64,
    pub(super) f_model: f64,
    pub(super) p_model: Option<f64>,
    pub(super) r2: f64,
    pub(super) root_mse: f64,
    pub(super) cv: f64,
    pub(super) type1: Vec<f64>,
    pub(super) type3: Vec<f64>,
}

/// Resolve the dependent column, apply listwise deletion, build the term
/// blocks, fit the full model and compute the Type I / Type III SS per term.
#[allow(clippy::too_many_arguments)]
pub(super) fn fit_multiway(
    session: &mut Session,
    ds: &crate::dataset::SasDataset,
    dep_var: &str,
    class_cols: &std::collections::HashMap<String, (String, Vec<Value>)>,
    used_classes: &[String],
    model: &AnovaModel,
    n_obs: usize,
) -> Result<MultiwayFit> {
    let dep_idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(dep_var))
        .ok_or_else(|| {
            SasError::runtime(format!("Variable {} not found.", dep_var.to_uppercase()))
        })?;
    if ds.vars[dep_idx].ty != VarType::Num {
        return Err(SasError::runtime(format!(
            "Dependent variable {} must be numeric.",
            dep_var.to_uppercase()
        )));
    }
    let dep_col = decode_column(ds, dep_idx)?;

    // Listwise deletion: drop rows where dep or ANY used CLASS var missing.
    let mut usable: Vec<usize> = Vec::new();
    for i in 0..n_obs {
        let dep_ok = matches!(value_to_num(&dep_col[i]), Some(v) if !v.is_nan());
        if !dep_ok {
            continue;
        }
        let cls_ok = used_classes
            .iter()
            .all(|c| !class_cols[c].1[i].is_missing());
        if cls_ok {
            usable.push(i);
        }
    }
    let n = usable.len();

    // y vector and corrected total.
    let y: Vec<f64> = usable
        .iter()
        .map(|&r| value_to_num(&dep_col[r]).unwrap())
        .collect();
    let y_bar = if n > 0 {
        y.iter().sum::<f64>() / n as f64
    } else {
        f64::NAN
    };
    let sst: f64 = y.iter().map(|&v| (v - y_bar).powi(2)).sum();

    // Levels per used CLASS var (sas_cmp order over usable rows).
    let mut var_levels: std::collections::HashMap<String, Vec<Value>> =
        std::collections::HashMap::new();
    for c in used_classes {
        let col = &class_cols[c].1;
        // Usable rows are non-missing by construction (listwise deletion).
        let levels = crate::procs::lincom::class_levels(usable.iter().map(|&r| &col[r]));
        var_levels.insert(c.clone(), levels);
    }

    // Per-CLASS-var level codes for each usable observation.
    let mut var_codes: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for c in used_classes {
        let col = &class_cols[c].1;
        let levels = &var_levels[c];
        let codes: Vec<usize> = usable
            .iter()
            .map(|&r| {
                levels
                    .iter()
                    .position(|l| l.sas_cmp(&col[r]) == std::cmp::Ordering::Equal)
                    .unwrap()
            })
            .collect();
        var_codes.insert(c.clone(), codes);
    }

    // Per-CLASS-var main-effect dummy columns (reference-cell), used for the
    // Type I sequential pass.
    let mut var_dummies: std::collections::HashMap<String, Vec<Vec<f64>>> =
        std::collections::HashMap::new();
    // Per-CLASS-var sum-to-zero (effect) coded columns, used for Type III.
    let mut var_effect: std::collections::HashMap<String, Vec<Vec<f64>>> =
        std::collections::HashMap::new();
    for c in used_classes {
        let l = var_levels[c].len();
        var_dummies.insert(c.clone(), main_effect_dummies(&var_codes[c], l, n));
        var_effect.insert(c.clone(), main_effect_effect_coded(&var_codes[c], l, n));
    }

    // Build the column block for each model term from a per-var coding map.
    // A main effect contributes its columns; an interaction term contributes
    // the elementwise products (Cartesian) across its parents' columns.
    let build_term_blocks =
        |coding: &std::collections::HashMap<String, Vec<Vec<f64>>>| -> Vec<Vec<Vec<f64>>> {
            let mut blocks: Vec<Vec<Vec<f64>>> = Vec::new();
            for term in &model.terms {
                let parents: Vec<&Vec<Vec<f64>>> =
                    term.iter().map(|p| &coding[&p.to_uppercase()]).collect();
                let mut block: Vec<Vec<f64>> = vec![vec![1.0; n]];
                for parent in &parents {
                    let mut next: Vec<Vec<f64>> = Vec::new();
                    for existing in &block {
                        for pcol in parent.iter() {
                            let prod: Vec<f64> =
                                existing.iter().zip(pcol).map(|(&a, &b)| a * b).collect();
                            next.push(prod);
                        }
                    }
                    block = next;
                }
                blocks.push(block);
            }
            blocks
        };

    // Reference-cell term blocks (Type I + Model SS) and effect-coded term
    // blocks (Type III). Both span the same column space, so SSE_full is
    // identical between them.
    let term_blocks = build_term_blocks(&var_dummies);
    let term_blocks_eff = build_term_blocks(&var_effect);

    // Per-term DF = product of (levels − 1).
    let term_dfs: Vec<usize> = model
        .terms
        .iter()
        .map(|term| {
            term.iter()
                .map(|p| var_levels[&p.to_uppercase()].len().saturating_sub(1))
                .product()
        })
        .collect();

    // Assemble a design matrix from a set of term blocks: intercept + the
    // included term blocks.
    let build_from = |blocks: &[Vec<Vec<f64>>], include: &[bool]| -> Vec<Vec<f64>> {
        let mut x = vec![vec![1.0]; n]; // intercept column
        for (t, block) in blocks.iter().enumerate() {
            if include[t] {
                for (i, row) in x.iter_mut().enumerate() {
                    for col in block {
                        row.push(col[i]);
                    }
                }
            }
        }
        x
    };
    let build_design = |include: &[bool]| -> Vec<Vec<f64>> { build_from(&term_blocks, include) };

    let n_terms = model.terms.len();
    let all_true = vec![true; n_terms];
    let full_x = build_design(&all_true);
    let full_cols = full_x[0].len();
    let sse_full = fit_sse(&full_x, &y);
    let ssm = sst - sse_full;
    let df_model = (full_cols - 1) as f64;
    let df_error = (n as f64 - full_cols as f64).max(0.0);
    let df_total = (n as f64 - 1.0).max(0.0);
    let msm = if df_model > 0.0 {
        ssm / df_model
    } else {
        f64::NAN
    };
    let mse = if df_error > 0.0 {
        sse_full / df_error
    } else {
        f64::NAN
    };
    let f_model = if mse > 0.0 && !mse.is_nan() {
        msm / mse
    } else {
        f64::NAN
    };
    let p_model = if f_model.is_nan() {
        None
    } else {
        Some((1.0 - f_cdf(f_model, df_model, df_error)).clamp(0.0, 1.0))
    };

    let r2 = if sst > 0.0 { ssm / sst } else { f64::NAN };
    let root_mse = if !mse.is_nan() { mse.sqrt() } else { f64::NAN };
    let cv = if y_bar.abs() > 1e-15 && !root_mse.is_nan() {
        root_mse / y_bar.abs() * 100.0
    } else {
        f64::NAN
    };

    // Type I (sequential): SS_t = SSE(0..t) − SSE(0..=t).
    let mut type1: Vec<f64> = Vec::with_capacity(n_terms);
    {
        let mut prev_sse = sst; // model with intercept only
        for t in 0..n_terms {
            let mut include = vec![false; n_terms];
            for inc in include.iter_mut().take(t + 1) {
                *inc = true;
            }
            let x = build_design(&include);
            let sse = fit_sse(&x, &y);
            type1.push(prev_sse - sse);
            prev_sse = sse;
        }
    }

    // Type III (partial), computed with sum-to-zero EFFECT coding so the
    // partial SS matches SAS Type III on unbalanced data. The effect-coded
    // full model spans the same column space as the reference-cell full
    // model, so its SSE equals `sse_full` (asserted in debug builds).
    let full_x_eff = build_from(&term_blocks_eff, &all_true);
    let sse_full_eff = fit_sse(&full_x_eff, &y);
    debug_assert!(
        (sse_full_eff - sse_full).abs() <= 1e-6 * (1.0 + sse_full.abs()),
        "effect-coded SSE_full {sse_full_eff} != reference-cell SSE_full {sse_full}"
    );
    let mut type3: Vec<f64> = Vec::with_capacity(n_terms);
    for t in 0..n_terms {
        let mut include = vec![true; n_terms];
        include[t] = false;
        let x = build_from(&term_blocks_eff, &include);
        let sse = fit_sse(&x, &y);
        type3.push(sse - sse_full_eff);
    }

    session
        .log
        .note(&format!("There were {} observations used.", n));

    Ok(MultiwayFit {
        y,
        var_levels,
        var_codes,
        term_dfs,
        y_bar,
        sst,
        sse_full,
        ssm,
        df_model,
        df_error,
        df_total,
        msm,
        mse,
        f_model,
        p_model,
        r2,
        root_mse,
        cv,
        type1,
        type3,
    })
}
