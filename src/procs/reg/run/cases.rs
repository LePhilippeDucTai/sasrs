use super::*;

/// M36.10 run-group editing: ADD/DELETE adjust the regressor set for the
/// final fit. With neither present this returns `None` and the caller borrows
/// `model.regressors` directly (byte-identical). ADD appends
/// not-already-present names (MODEL order preserved, additions last); DELETE
/// removes matching names.
pub(crate) fn effective_regressors(entry: &RegModelEntry) -> Option<Vec<String>> {
    let model = &entry.model;
    if entry.add.is_empty() && entry.delete.is_empty() {
        None
    } else {
        let mut regs: Vec<String> = model.regressors.clone();
        for a in &entry.add {
            if !regs.iter().any(|r| r.eq_ignore_ascii_case(a)) {
                regs.push(a.clone());
            }
        }
        if !entry.delete.is_empty() {
            regs.retain(|r| !entry.delete.iter().any(|d| d.eq_ignore_ascii_case(r)));
        }
        Some(regs)
    }
}

/// Find a dataset column by (case-insensitive) name.
pub(crate) fn find_col(ds: &SasDataset, nm: &str) -> Result<usize> {
    ds.vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(nm))
        .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", nm.to_uppercase())))
}

/// Regressor resolution + decode is shared across all responses (the design
/// is common to every dependent on the MODEL LHS), so it is hoisted above the
/// per-response loop of `run_model`.
pub(crate) fn decode_regressor_columns(
    ds: &SasDataset,
    regressors: &[String],
) -> Result<Vec<Vec<crate::value::Value>>> {
    let p = regressors.len();
    let mut reg_idxs: Vec<usize> = Vec::with_capacity(p);
    for nm in regressors {
        let idx = find_col(ds, nm)?;
        if ds.vars[idx].ty != VarType::Num {
            return Err(SasError::runtime(format!(
                "Regressor {} must be numeric.",
                nm.to_uppercase()
            )));
        }
        reg_idxs.push(idx);
    }
    // --- Decode regressor columns (shared) ---
    let mut reg_cols: Vec<Vec<crate::value::Value>> = Vec::with_capacity(p);
    for &idx in &reg_idxs {
        reg_cols.push(decode_column(ds, idx)?);
    }
    Ok(reg_cols)
}

/// MQ5.2 — one response's complete-case data: the listwise-deleted regressor
/// columns and response vector, the complete-row mask, the M36.7 weighting
/// context, and the first ID variable's per-row display values.
pub(crate) struct CaseData {
    pub(crate) xcols: Vec<Vec<f64>>,
    pub(crate) y_vec: Vec<f64>,
    pub(crate) complete_mask: Vec<bool>,
    pub(crate) weighting: Option<Weighting>,
    pub(crate) id_used: Vec<String>,
}

/// MQ5.2 — gather one response's complete cases (M36.7 WEIGHT/FREQ/ID
/// bookkeeping + listwise deletion over the regressors and the response).
pub(crate) fn gather_complete_cases(
    ds: &SasDataset,
    rows: &[usize],
    weight_col: Option<&[crate::value::Value]>,
    freq_col: Option<&[crate::value::Value]>,
    id_cols: &[(String, Vec<crate::value::Value>)],
    dep_col: &[crate::value::Value],
    reg_cols: &[Vec<crate::value::Value>],
) -> CaseData {
    let p = reg_cols.len();
    // --- M36.7 weighting bookkeeping. `wf` accumulates the effective SS weight
    // w_i·f_i for each complete-case row; `total_n` accumulates Σf_i (FREQ
    // inflates the observation count / df, WEIGHT does not). `id_used` carries
    // the first ID variable's per-row display value when ID is given. When no
    // WEIGHT and no FREQ are present, `weighting` stays inactive and the whole
    // analysis is byte-identical to the prior OLS path.
    let has_weight = weight_col.is_some();
    let has_freq = freq_col.is_some();
    let mut wf: Vec<f64> = Vec::new();
    let mut total_n: f64 = 0.0;
    let mut id_used: Vec<String> = Vec::new();

    // --- Build regressor columns (numeric) and y vector (listwise deletion) ---
    // xcols[c] is the c-th regressor over the complete-case rows.
    let mut xcols: Vec<Vec<f64>> = vec![Vec::new(); p];
    let mut y_vec: Vec<f64> = Vec::new();
    let mut complete_mask: Vec<bool> = vec![false; ds.n_obs()];

    for &i in rows {
        // FREQ: truncate to integer; exclude obs with f_i < 1 or missing.
        let fi: f64 = match freq_col {
            Some(col) => match value_to_num(&col[i]) {
                Some(v) if !v.is_nan() => {
                    let t = v.trunc();
                    if t < 1.0 {
                        continue;
                    }
                    t
                }
                _ => continue,
            },
            None => 1.0,
        };
        // WEIGHT: exclude obs with w_i ≤ 0 or missing weight.
        let wi: f64 = match weight_col {
            Some(col) => match value_to_num(&col[i]) {
                Some(v) if !v.is_nan() && v > 0.0 => v,
                _ => continue,
            },
            None => 1.0,
        };
        let yi = match value_to_num(&dep_col[i]) {
            Some(v) if !v.is_nan() => v,
            _ => continue,
        };
        let mut row_vals = Vec::with_capacity(p);
        let mut ok = true;
        for rc in reg_cols {
            match value_to_num(&rc[i]) {
                Some(v) if !v.is_nan() => row_vals.push(v),
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            for (c, v) in row_vals.into_iter().enumerate() {
                xcols[c].push(v);
            }
            y_vec.push(yi);
            wf.push(wi * fi);
            total_n += fi;
            if let Some((_, col)) = id_cols.first() {
                id_used.push(id_value_cell(&col[i]));
            }
            complete_mask[i] = true;
        }
    }

    // Effective weighting context. Active when WEIGHT or FREQ is present. When
    // inactive the OLS path runs exactly as before (byte-identical). `total_n`
    // (Σf_i) is the observation count that drives df / n bookkeeping: FREQ
    // changes n and df, WEIGHT does not.
    let weighting = if has_weight || has_freq {
        Some(Weighting {
            wf: wf.clone(),
            total_n,
        })
    } else {
        None
    };
    CaseData {
        xcols,
        y_vec,
        complete_mask,
        weighting,
        id_used,
    }
}

/// PRESS statistic (M36.5): Σ wf_i·(resid_i/(1−h_i))². With WEIGHT/FREQ active
/// (M36.7) the leverage is the WEIGHTED one (h_i·w_i) and each term carries
/// wf_i, matching the weighted PRESS in the MODEL R residual summary and
/// STUDENT/Cook's D (which already use the weighted leverage). With no
/// weighting `wf` is all-ones and h is the plain OLS leverage, so this is
/// byte-identical to before.
pub(crate) fn compute_press_stat(
    model: &RegModel,
    x_mat: &[Vec<f64>],
    fit: &OlsFit,
    weighting: Option<&Weighting>,
) -> Option<f64> {
    if model.press_opt && !model.noprint {
        let h0 = leverages(&x_mat, &fit.xtx_inv);
        let ones = vec![1.0; h0.len()];
        let wf: &[f64] = weighting.as_ref().map(|w| w.wf.as_slice()).unwrap_or(&ones);
        let h: Vec<f64> = h0
            .iter()
            .zip(wf.iter())
            .map(|(&hi, &wi)| hi * wi)
            .collect();
        let press: f64 = fit
            .resid
            .iter()
            .zip(h.iter())
            .zip(wf.iter())
            .map(|((e, &hi), &wi)| {
                let d = 1.0 - hi;
                if d != 0.0 {
                    let p = e / d;
                    wi * p * p
                } else {
                    0.0
                }
            })
            .sum();
        Some(press)
    } else {
        None
    }
}

/// MQ5.2 — one response's fitted-model context, shared by the post-fit
/// option sections (printed matrices, diagnostics, TEST, plots).
pub(crate) struct RespFit<'a> {
    pub(crate) model: &'a RegModel,
    pub(crate) dep_name: &'a str,
    pub(crate) x_mat: &'a [Vec<f64>],
    pub(crate) y_vec: &'a [f64],
    pub(crate) sel_cols: &'a [Vec<f64>],
    pub(crate) sel_reg_names: &'a [String],
    pub(crate) fit: &'a OlsFit,
    pub(crate) intercept: bool,
    pub(crate) n: usize,
    pub(crate) p_eff: usize,
    pub(crate) weighting: Option<&'a Weighting>,
    pub(crate) id_first: Option<&'a [String]>,
}
