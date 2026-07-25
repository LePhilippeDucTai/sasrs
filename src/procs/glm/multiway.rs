use super::*;

/// Multiway fit results for one dependent variable: resolved factors, design
/// layout, overall ANOVA statistics and the per-term Type I / III SS.
pub(super) struct MultiwayFit {
    pub(super) factors: Vec<Factor>,
    pub(super) term_factor_idxs: Vec<Vec<usize>>,
    pub(super) col_specs: Vec<Vec<Vec<(usize, usize)>>>,
    pub(super) term_df: Vec<usize>,
    pub(super) term_labels: Vec<String>,
    pub(super) full_design: Vec<Vec<f64>>,
    pub(super) y: Vec<f64>,
    pub(super) ncols: usize,
    pub(super) y_bar: f64,
    pub(super) sst: f64,
    pub(super) sse_full: f64,
    pub(super) ssm: f64,
    pub(super) df_error: f64,
    pub(super) df_model: f64,
    pub(super) df_total: f64,
    pub(super) mse: f64,
    pub(super) msm: f64,
    pub(super) f_model: f64,
    pub(super) p_model: Option<f64>,
    pub(super) r2: f64,
    pub(super) root_mse: f64,
    pub(super) cv: f64,
    pub(super) type1_ss: Vec<f64>,
    pub(super) type3_ss: Vec<f64>,
}

/// Resolve the dependent column, apply listwise deletion, build the design,
/// fit the full model and compute the Type I / Type III SS per term.
#[allow(clippy::too_many_arguments)]
pub(super) fn fit_multiway(
    session: &mut Session,
    ds: &crate::dataset::SasDataset,
    dep_var: &str,
    class_cols: &[(String, Vec<Value>)],
    term_factor_idxs: &[Vec<usize>],
    model: &GlmModel,
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

    // Listwise deletion: require dependent present and EVERY CLASS var present.
    let mut usable_rows: Vec<usize> = Vec::new();
    for i in 0..n_obs {
        let dep_ok = matches!(value_to_num(&dep_col[i]), Some(v) if !v.is_nan());
        let cls_ok = class_cols.iter().all(|(_, c)| !c[i].is_missing());
        if dep_ok && cls_ok {
            usable_rows.push(i);
        }
    }
    let n = usable_rows.len();

    // Resolve factor levels over the usable rows (sas_cmp order, ref = last).
    let mut factors: Vec<Factor> = Vec::new();
    for (name, col) in class_cols {
        let mut levels: Vec<Value> = Vec::new();
        for &r in &usable_rows {
            let v = &col[r];
            if !levels
                .iter()
                .any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
            {
                levels.push(v.clone());
            }
        }
        levels.sort_by(|a, b| a.sas_cmp(b));
        factors.push(Factor {
            name: name.clone(),
            levels,
        });
    }

    // Per-row factor level indices.
    let row_level_idx: Vec<Vec<usize>> = usable_rows
        .iter()
        .map(|&r| {
            class_cols
                .iter()
                .enumerate()
                .map(|(fi, (_, col))| factors[fi].level_of(&col[r]))
                .collect()
        })
        .collect();

    // Response vector.
    let y: Vec<f64> = usable_rows
        .iter()
        .map(|&r| value_to_num(&dep_col[r]).unwrap())
        .collect();
    let y_bar = if n > 0 { y.iter().sum::<f64>() / n as f64 } else { f64::NAN };
    let sst: f64 = y.iter().map(|&v| (v - y_bar).powi(2)).sum();

    // Column specs: per term, list of column definitions (each a product of
    // parent (factor, dummy) pairs).
    let col_specs = term_column_specs(term_factor_idxs, &factors);
    let term_df: Vec<usize> = term_factor_idxs
        .iter()
        .map(|fis| fis.iter().map(|&fi| factors[fi].n_dummies()).product())
        .collect();

    // Precompute per-row dummy vectors for each factor.
    let row_dummy_cache: Vec<Vec<Vec<f64>>> = row_level_idx
        .iter()
        .map(|rl| row_dummies(&factors, rl))
        .collect();

    // Build a column value for a given column-spec at a given row.
    let col_value = |row: usize, spec: &[(usize, usize)]| -> f64 {
        let mut prod = 1.0;
        for &(fi, dj) in spec {
            prod *= row_dummy_cache[row][fi][dj];
        }
        prod
    };

    // Effect (sum-to-zero) coded counterpart, used ONLY for the Type III SS
    // pass. The column layout (col_specs) is identical; only the per-factor
    // contrast values differ (+1/−1/0 instead of 1/0).
    let row_effect_cache: Vec<Vec<Vec<f64>>> = row_level_idx
        .iter()
        .map(|rl| row_effects(&factors, rl))
        .collect();
    let col_value_eff = |row: usize, spec: &[(usize, usize)]| -> f64 {
        let mut prod = 1.0;
        for &(fi, dj) in spec {
            prod *= row_effect_cache[row][fi][dj];
        }
        prod
    };

    // Assemble the FULL design matrix: intercept + all terms' columns.
    let mut full_design: Vec<Vec<f64>> = vec![vec![1.0]; n];
    let mut next_col = 1usize;
    for specs in &col_specs {
        for spec in specs {
            for (row, design_row) in full_design.iter_mut().enumerate() {
                design_row.push(col_value(row, spec));
            }
            next_col += 1;
        }
    }
    let ncols = next_col;

    let sse_full = sse_of(&full_design, &y);
    let ssm = sst - sse_full;
    let df_error = (n as i64 - ncols as i64).max(0) as f64;
    let df_model: f64 = term_df.iter().map(|&d| d as f64).sum();
    let df_total = (n as f64 - 1.0).max(0.0);
    let mse = if df_error > 0.0 { sse_full / df_error } else { f64::NAN };
    let msm = if df_model > 0.0 { ssm / df_model } else { f64::NAN };
    let f_model = if mse > 0.0 && !mse.is_nan() { msm / mse } else { f64::NAN };
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

    session.log.note(&format!("There were {} observations used.", n));

    // --- Helper to build a design from a subset of terms (intercept + terms) ---
    let build_design = |term_subset: &[usize]| -> Vec<Vec<f64>> {
        let mut design: Vec<Vec<f64>> = vec![vec![1.0]; n];
        for &t in term_subset {
            for spec in &col_specs[t] {
                for (row, design_row) in design.iter_mut().enumerate() {
                    design_row.push(col_value(row, spec));
                }
            }
        }
        design
    };

    // --- Type I (sequential) SS per term ---
    let mut type1_ss: Vec<f64> = Vec::with_capacity(col_specs.len());
    {
        let mut prev_subset: Vec<usize> = Vec::new();
        let intercept_only: Vec<Vec<f64>> = vec![vec![1.0]; n];
        let mut prev_sse = sse_of(&intercept_only, &y);
        for t in 0..col_specs.len() {
            prev_subset.push(t);
            let sse_with = sse_of(&build_design(&prev_subset), &y);
            type1_ss.push((prev_sse - sse_with).max(0.0));
            prev_sse = sse_with;
        }
    }

    // --- Type III (partial) SS per term, using sum-to-zero EFFECT coding ---
    // SAS Type III SS for an effect equals the partial SS for that effect when
    // the design is built with full-rank effect coding (centered contrasts):
    // the interaction columns are then orthogonalized against lower-order
    // marginals, so dropping a term's effect-coded columns yields the SAS
    // estimable-function SS even for a main effect involved in an interaction
    // on unbalanced data. Reference-cell coding does NOT give this for a
    // lower-order term when a higher-order interaction is present.
    //
    // The effect-coded full model spans the same column space as the
    // reference-cell full model, so SSE_full is identical (asserted in tests).
    let mut type3_ss: Vec<f64> = Vec::with_capacity(col_specs.len());
    {
        // Effect-coded full model — must reproduce sse_full.
        let mut eff_full: Vec<Vec<f64>> = vec![vec![1.0]; n];
        for specs in &col_specs {
            for spec in specs {
                for (row, design_row) in eff_full.iter_mut().enumerate() {
                    design_row.push(col_value_eff(row, spec));
                }
            }
        }
        let sse_full_eff = sse_of(&eff_full, &y);
        for t in 0..col_specs.len() {
            // Build effect-coded design = full minus term t's columns.
            let mut design: Vec<Vec<f64>> = vec![vec![1.0]; n];
            for (ti, specs) in col_specs.iter().enumerate() {
                if ti == t {
                    continue;
                }
                for spec in specs {
                    for (row, design_row) in design.iter_mut().enumerate() {
                        design_row.push(col_value_eff(row, spec));
                    }
                }
            }
            let sse_drop = sse_of(&design, &y);
            type3_ss.push((sse_drop - sse_full_eff).max(0.0));
        }
    }

    // Term labels (e.g. `a*b`).
    let term_labels: Vec<String> = model
        .effect_terms
        .iter()
        .map(|t| t.join("*"))
        .collect();

    Ok(MultiwayFit {
        factors,
        term_factor_idxs: term_factor_idxs.to_vec(),
        col_specs,
        term_df,
        term_labels,
        full_design,
        y,
        ncols,
        y_bar,
        sst,
        sse_full,
        ssm,
        df_error,
        df_model,
        df_total,
        mse,
        msm,
        f_model,
        p_model,
        r2,
        root_mse,
        cv,
        type1_ss,
        type3_ss,
    })
}

/// Solve the least-squares fit and build the shared linear-combination engine
/// (M37.1) used by SOLUTION / LSMEANS.
#[allow(clippy::type_complexity)]
pub(super) fn build_lincom_engine(
    fit: &MultiwayFit,
) -> (
    Option<Vec<f64>>,
    Option<Vec<Vec<f64>>>,
    crate::procs::lincom::Coding,
    Option<crate::procs::lincom::LinCombEngine>,
) {
    let full_design = &fit.full_design;
    let y = &fit.y;
    let factors = &fit.factors;
    let term_factor_idxs = &fit.term_factor_idxs;
    let col_specs = &fit.col_specs;
    let ncols = fit.ncols;
    let df_error = fit.df_error;
    let mse = fit.mse;

    let beta = crate::stat::linalg::least_squares(full_design, y).ok();
    let xtx_inv = {
        let xt = crate::stat::linalg::transpose(full_design);
        let xtx = crate::stat::linalg::matrix_mult(&xt, full_design);
        crate::stat::linalg::invert_matrix(&xtx).ok()
    };

    // Reference-cell coding of the fitted design (M37.1). A pure function of
    // the factor layout — independent of β / covariance — so it is built
    // unconditionally and shared by both the engine and the degenerate-fit
    // fallback paths below.
    let lincom_coding = crate::procs::lincom::Coding {
        factors: factors
            .iter()
            .map(|f| (f.name.clone(), f.levels.clone()))
            .collect(),
        term_factor_idxs: term_factor_idxs.clone(),
        col_specs: col_specs.clone(),
        ncols,
    };

    // Shared linear-combination engine (M37.1): used for SOLUTION / LS-means
    // below. Built only when both β and (X'X)^-1 are available; the engine
    // carries the raw covariance and MSE separately so its arithmetic matches
    // the extracted code byte-for-byte.
    let lincom_engine = match (&beta, &xtx_inv) {
        (Some(b), Some(inv)) => Some(crate::procs::lincom::LinCombEngine::new(
            b.clone(),
            inv.clone(),
            lincom_coding.clone(),
            df_error,
            mse,
        )),
        _ => None,
    };

    (beta, xtx_inv, lincom_coding, lincom_engine)
}
