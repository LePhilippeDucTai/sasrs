//! Datasets de sortie (OUTEST=, OUTSSCP=, OUTPUT OUT=) et tracés PLOTS=/PLOT.

use super::*;

// ───────────────────────── OUTEST= / OUTSSCP= datasets (M36.8) ─────────────────────────

/// One fitted model's contribution to the OUTEST= dataset (M36.8). Captures
/// everything needed to emit the PARMS row (and, with modifiers, the COV/SEB
/// rows and the EDF / TABLEOUT columns). Parameter values are stored by name so
/// the writer can align them into the union of parameter columns across models.
pub(super) struct OutEstEntry {
    /// Model label, e.g. "MODEL1".
    model_label: String,
    /// Dependent variable name.
    depvar: String,
    /// Root MSE.
    pub(super) rmse: f64,
    /// Parameter names in design order: "Intercept" (if present) then regressors.
    param_names: Vec<String>,
    /// Parameter estimates aligned with `param_names`.
    pub(super) beta: Vec<f64>,
    /// Standard errors aligned with `param_names`.
    se: Vec<f64>,
    /// Covariance matrix MSE·(X'X)⁻¹ (p_eff×p_eff), aligned with `param_names`.
    covb: Vec<Vec<f64>>,
    /// Number of regressors in the model (_IN_).
    pub(super) n_in: usize,
    /// Number of parameters (_P_).
    pub(super) n_p: usize,
    /// Error degrees of freedom (_EDF_).
    pub(super) edf: f64,
    /// Lower/upper confidence bounds per parameter (TABLEOUT), aligned with
    /// `param_names`.
    lb: Vec<f64>,
    ub: Vec<f64>,
    /// M36.9 — back-transformed RIDGE / RIDGEVIF / IPC rows for this model
    /// (empty unless RIDGE=/PCOMIT= requested). Each carries the row `_TYPE_`,
    /// the `_RIDGE_`/`_PCOMIT_` selector value, and per-parameter values aligned
    /// with `param_names` (intercept first when present).
    pub(super) ridge_ipc: Vec<RidgeIpcRow>,
}

/// M36.9 — one OUTEST= row produced by ridge / IPC regression. `kind` is the
/// `_TYPE_` value ("RIDGE", "RIDGEVIF", or "IPC"); `ridge_k`/`pcomit_m` carry the
/// `_RIDGE_`/`_PCOMIT_` selector (exactly one is `Some`); `values` are aligned
/// with the entry's `param_names` (Intercept first when present). For RIDGEVIF
/// rows the intercept slot is `None` (VIF undefined for the intercept).
#[derive(Clone)]
pub(super) struct RidgeIpcRow {
    pub(super) kind: &'static str,
    pub(super) ridge_k: Option<f64>,
    pub(super) pcomit_m: Option<f64>,
    pub(super) values: Vec<Option<f64>>,
}

/// Build an OUTEST= entry from a completed fit (M36.8).
pub(super) fn build_outest_entry(
    model_label: &str,
    dep_name: &str,
    reg_names: &[String],
    fit: &OlsFit,
    intercept: bool,
    n_used: f64,
    alpha: f64,
) -> OutEstEntry {
    let p_eff = fit.beta.len();
    let edf = n_used - p_eff as f64;
    let mse = if edf > 0.0 { fit.sse / edf } else { f64::NAN };
    let param_names: Vec<String> = (0..p_eff)
        .map(|j| design_label(j, reg_names, intercept))
        .collect();
    let covb = covb_matrix(&fit.xtx_inv, mse);
    let se: Vec<f64> = (0..p_eff).map(|j| covb[j][j].max(0.0).sqrt()).collect();
    let t_crit = t_quantile(1.0 - alpha / 2.0, edf);
    let lb: Vec<f64> = (0..p_eff)
        .map(|j| fit.beta[j] - t_crit * se[j])
        .collect();
    let ub: Vec<f64> = (0..p_eff)
        .map(|j| fit.beta[j] + t_crit * se[j])
        .collect();
    OutEstEntry {
        model_label: model_label.to_string(),
        depvar: dep_name.to_string(),
        rmse: mse.sqrt(),
        param_names,
        beta: fit.beta.clone(),
        se,
        covb,
        n_in: reg_names.len(),
        n_p: p_eff,
        edf,
        lb,
        ub,
        ridge_ipc: Vec::new(),
    }
}

/// Write the OUTEST= dataset from the accumulated per-model entries (M36.8).
/// Variables: `_MODEL_`, `_TYPE_`, `_DEPVAR_`, [`_NAME_` when COVOUT/OUTSEB],
/// `_RMSE_`, [EDF cols], one column per parameter (union over models, source
/// order), the dependent column (=-1), then [TABLEOUT cols]. One PARMS row per
/// model; COVOUT/OUTSEB add extra rows per model.
pub(super) fn write_outest(
    spec: &OutEst,
    entries: &[OutEstEntry],
    session: &mut Session,
) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let with_name = spec.covout || spec.outseb;
    // M36.9: whether any RIDGE / IPC rows are present, and which selector
    // columns (`_RIDGE_` / `_PCOMIT_`) the dataset needs.
    let with_ridge = entries
        .iter()
        .any(|e| e.ridge_ipc.iter().any(|r| r.ridge_k.is_some()));
    let with_pcomit = entries
        .iter()
        .any(|e| e.ridge_ipc.iter().any(|r| r.pcomit_m.is_some()));

    // Union of parameter columns (source order): Intercept first if any model
    // has it, then regressors in first-seen order.
    let mut param_cols: Vec<String> = Vec::new();
    for e in entries {
        for nm in &e.param_names {
            if !param_cols.iter().any(|c| c.eq_ignore_ascii_case(nm)) {
                param_cols.push(nm.clone());
            }
        }
    }
    // Union of dependent-variable columns (marked −1 in their model's PARMS row).
    let mut dep_cols: Vec<String> = Vec::new();
    for e in entries {
        if !dep_cols.iter().any(|c| c.eq_ignore_ascii_case(&e.depvar))
            && !param_cols.iter().any(|c| c.eq_ignore_ascii_case(&e.depvar))
        {
            dep_cols.push(e.depvar.clone());
        }
    }

    // Row accumulators.
    let mut model_c: Vec<Option<String>> = Vec::new();
    let mut type_c: Vec<Option<String>> = Vec::new();
    let mut depvar_c: Vec<Option<String>> = Vec::new();
    let mut name_c: Vec<Option<String>> = Vec::new();
    let mut rmse_c: Vec<Option<f64>> = Vec::new();
    let mut in_c: Vec<Option<f64>> = Vec::new();
    let mut p_c: Vec<Option<f64>> = Vec::new();
    let mut edf_c: Vec<Option<f64>> = Vec::new();
    // One numeric vector per parameter column and per dependent column.
    let mut param_vals: Vec<Vec<Option<f64>>> = vec![Vec::new(); param_cols.len()];
    let mut dep_vals: Vec<Vec<Option<f64>>> = vec![Vec::new(); dep_cols.len()];
    let mut lb_c: Vec<Option<f64>> = Vec::new();
    let mut ub_c: Vec<Option<f64>> = Vec::new();
    // M36.9 — `_RIDGE_` / `_PCOMIT_` selector columns (None on non-ridge/IPC rows).
    let mut ridge_c: Vec<Option<f64>> = Vec::new();
    let mut pcomit_c: Vec<Option<f64>> = Vec::new();

    // Helper: index of a parameter name within an entry (case-insensitive).
    let entry_param = |e: &OutEstEntry, nm: &str| -> Option<usize> {
        e.param_names.iter().position(|p| p.eq_ignore_ascii_case(nm))
    };

    for e in entries {
        // --- PARMS row ---
        model_c.push(Some(e.model_label.clone()));
        type_c.push(Some("PARMS".to_string()));
        depvar_c.push(Some(e.depvar.clone()));
        name_c.push(Some(String::new()));
        rmse_c.push(Some(e.rmse));
        in_c.push(Some(e.n_in as f64));
        p_c.push(Some(e.n_p as f64));
        edf_c.push(Some(e.edf));
        for (ci, cn) in param_cols.iter().enumerate() {
            param_vals[ci].push(entry_param(e, cn).map(|k| e.beta[k]));
        }
        for (ci, cn) in dep_cols.iter().enumerate() {
            // The dependent column for this model's response is −1; others miss.
            dep_vals[ci].push(if cn.eq_ignore_ascii_case(&e.depvar) {
                Some(-1.0)
            } else {
                None
            });
        }
        lb_c.push(None);
        ub_c.push(None);

        // --- TABLEOUT: L95B / U95B rows for the estimates (documented subset:
        // the parameter confidence bounds at the model α). SAS emits dedicated
        // _TYPE_ rows; we add an L95B and a U95B row carrying the bounds in the
        // parameter columns.
        if spec.tableout {
            for (is_upper, src) in [(false, &e.lb), (true, &e.ub)] {
                model_c.push(Some(e.model_label.clone()));
                type_c.push(Some(if is_upper { "U95B" } else { "L95B" }.to_string()));
                depvar_c.push(Some(e.depvar.clone()));
                name_c.push(Some(String::new()));
                rmse_c.push(Some(e.rmse));
                in_c.push(Some(e.n_in as f64));
                p_c.push(Some(e.n_p as f64));
                edf_c.push(Some(e.edf));
                for (ci, cn) in param_cols.iter().enumerate() {
                    param_vals[ci].push(entry_param(e, cn).map(|k| src[k]));
                }
                for ci in 0..dep_cols.len() {
                    dep_vals[ci].push(None);
                }
                lb_c.push(None);
                ub_c.push(None);
            }
        }

        // --- COVOUT: one row per parameter with that row of the covariance
        // matrix (MSE·(X'X)⁻¹), _TYPE_="COV", _NAME_=parameter name.
        if spec.covout {
            for (k, pn) in e.param_names.iter().enumerate() {
                model_c.push(Some(e.model_label.clone()));
                type_c.push(Some("COV".to_string()));
                depvar_c.push(Some(e.depvar.clone()));
                name_c.push(Some(pn.clone()));
                rmse_c.push(Some(e.rmse));
                in_c.push(Some(e.n_in as f64));
                p_c.push(Some(e.n_p as f64));
                edf_c.push(Some(e.edf));
                for (ci, cn) in param_cols.iter().enumerate() {
                    param_vals[ci].push(entry_param(e, cn).map(|j| e.covb[k][j]));
                }
                for ci in 0..dep_cols.len() {
                    dep_vals[ci].push(None);
                }
                lb_c.push(None);
                ub_c.push(None);
            }
        }

        // --- OUTSEB: a row of standard errors, _TYPE_="SEB".
        if spec.outseb {
            model_c.push(Some(e.model_label.clone()));
            type_c.push(Some("SEB".to_string()));
            depvar_c.push(Some(e.depvar.clone()));
            name_c.push(Some(String::new()));
            rmse_c.push(Some(e.rmse));
            in_c.push(Some(e.n_in as f64));
            p_c.push(Some(e.n_p as f64));
            edf_c.push(Some(e.edf));
            for (ci, cn) in param_cols.iter().enumerate() {
                param_vals[ci].push(entry_param(e, cn).map(|k| e.se[k]));
            }
            for ci in 0..dep_cols.len() {
                dep_vals[ci].push(None);
            }
            lb_c.push(None);
            ub_c.push(None);
        }

        // --- M36.9 RIDGE / RIDGEVIF / IPC rows. The PARMS/COV/SEB rows above
        // never touch the selector columns, so bring them up to the current row
        // count with `None` first, then emit one row per ridge/IPC entry.
        if !e.ridge_ipc.is_empty() {
            while ridge_c.len() < model_c.len() {
                ridge_c.push(None);
            }
            while pcomit_c.len() < model_c.len() {
                pcomit_c.push(None);
            }
            for rr in &e.ridge_ipc {
                model_c.push(Some(e.model_label.clone()));
                type_c.push(Some(rr.kind.to_string()));
                depvar_c.push(Some(e.depvar.clone()));
                name_c.push(Some(String::new()));
                // RIDGEVIF rows carry no RMSE; RIDGE/IPC reuse the OLS RMSE.
                rmse_c.push(if rr.kind == "RIDGEVIF" {
                    None
                } else {
                    Some(e.rmse)
                });
                in_c.push(Some(e.n_in as f64));
                p_c.push(Some(e.n_p as f64));
                edf_c.push(Some(e.edf));
                for (ci, cn) in param_cols.iter().enumerate() {
                    // Align the row's per-parameter values (Intercept first when
                    // present) onto the union parameter columns by name.
                    let v = entry_param(e, cn).and_then(|k| rr.values.get(k).copied().flatten());
                    param_vals[ci].push(v);
                }
                for ci in 0..dep_cols.len() {
                    dep_vals[ci].push(None);
                }
                lb_c.push(None);
                ub_c.push(None);
                ridge_c.push(rr.ridge_k);
                pcomit_c.push(rr.pcomit_m);
            }
        }
    }
    // Pad the selector columns to the final row count (entries without any
    // ridge/IPC rows never pushed to them).
    while ridge_c.len() < model_c.len() {
        ridge_c.push(None);
    }
    while pcomit_c.len() < model_c.len() {
        pcomit_c.push(None);
    }

    // Assemble the dataset columns in SAS order.
    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();
    columns.push(Series::new("_MODEL_".into(), model_c).into());
    vars.push(char_meta("_MODEL_", 32));
    columns.push(Series::new("_TYPE_".into(), type_c).into());
    vars.push(char_meta("_TYPE_", 8));
    columns.push(Series::new("_DEPVAR_".into(), depvar_c).into());
    vars.push(char_meta("_DEPVAR_", 32));
    if with_name {
        columns.push(Series::new("_NAME_".into(), name_c).into());
        vars.push(char_meta("_NAME_", 32));
    }
    // M36.9 — selector columns precede _RMSE_ when ridge / IPC rows are present.
    if with_ridge {
        columns.push(Series::new("_RIDGE_".into(), ridge_c).into());
        vars.push(num_var_meta("_RIDGE_"));
    }
    if with_pcomit {
        columns.push(Series::new("_PCOMIT_".into(), pcomit_c).into());
        vars.push(num_var_meta("_PCOMIT_"));
    }
    columns.push(Series::new("_RMSE_".into(), rmse_c).into());
    vars.push(num_var_meta("_RMSE_"));
    if spec.edf {
        columns.push(Series::new("_IN_".into(), in_c).into());
        vars.push(num_var_meta("_IN_"));
        columns.push(Series::new("_P_".into(), p_c).into());
        vars.push(num_var_meta("_P_"));
        columns.push(Series::new("_EDF_".into(), edf_c).into());
        vars.push(num_var_meta("_EDF_"));
    }
    for (ci, cn) in param_cols.iter().enumerate() {
        columns.push(Series::new(cn.as_str().into(), param_vals[ci].clone()).into());
        vars.push(num_var_meta(cn));
    }
    for (ci, cn) in dep_cols.iter().enumerate() {
        columns.push(Series::new(cn.as_str().into(), dep_vals[ci].clone()).into());
        vars.push(num_var_meta(cn));
    }
    if spec.tableout {
        columns.push(Series::new("_LB_".into(), lb_c).into());
        vars.push(num_var_meta("_LB_"));
        columns.push(Series::new("_UB_".into(), ub_c).into());
        vars.push(num_var_meta("_UB_"));
    }

    let out_df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df: out_df, vars };
    let out_libref = spec.out.libref_or_work();
    let out_table = spec.out.name.to_uppercase();
    let display = format!("{out_libref}.{out_table}");
    let n_rows = out_ds.n_obs();
    let n_vars_out = out_ds.vars.len();
    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(display.clone());
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_rows, n_vars_out
    ));
    Ok(())
}

/// Build and write the OUTSSCP= dataset for one analysis (M36.8). `var_names`
/// are the analysis variables (regressors then dependent); `cols` the parallel
/// complete-case columns. The SSCP matrix has an `Intercept` row/col (= n on the
/// diagonal, column sums off-diagonal) unless `intercept` is false. Layout:
/// `_TYPE_`="SSCP", `_NAME_`=row label, then one numeric column per variable
/// (Intercept, regressors, dependent) holding X'X augmented with X'Y / Y'Y.
pub(super) fn write_outsscp(
    out: &DatasetRef,
    reg_names: &[String],
    dep_name: &str,
    cols: &[Vec<f64>],
    intercept: bool,
    session: &mut Session,
) -> Result<()> {
    // Order of the analysis columns in `cols`: regressors then dependent. Build
    // a combined matrix V (rows = obs) whose columns are [Intercept?, regressors,
    // dependent], then A = VᵀV.
    let n = cols.first().map(|c| c.len()).unwrap_or(0);
    // Column labels for the matrix (Intercept first when present).
    let mut labels: Vec<String> = Vec::new();
    if intercept {
        labels.push("Intercept".to_string());
    }
    for nm in reg_names {
        labels.push(nm.clone());
    }
    labels.push(dep_name.to_string());

    let m = labels.len();
    // value(i, col): the col-th analysis value for row i (Intercept = 1).
    let col_value = |row: usize, label_idx: usize| -> f64 {
        if intercept && label_idx == 0 {
            1.0
        } else {
            // cols index: subtract the intercept offset.
            let ci = if intercept { label_idx - 1 } else { label_idx };
            cols[ci][row]
        }
    };
    let mut a = vec![vec![0.0; m]; m];
    for r in 0..m {
        for c in 0..m {
            let mut s = 0.0;
            for i in 0..n {
                s += col_value(i, r) * col_value(i, c);
            }
            a[r][c] = s;
        }
    }

    // Build the dataset: _TYPE_ (char), _NAME_ (char), then one numeric column
    // per analysis variable.
    let mut columns: Vec<Column> = Vec::with_capacity(m + 2);
    let mut vars: Vec<VarMeta> = Vec::with_capacity(m + 2);
    let type_col: Vec<Option<String>> = (0..m).map(|_| Some("SSCP".to_string())).collect();
    let name_col: Vec<Option<String>> = labels.iter().map(|l| Some(l.clone())).collect();
    columns.push(Series::new("_TYPE_".into(), type_col).into());
    vars.push(char_meta("_TYPE_", 8));
    columns.push(Series::new("_NAME_".into(), name_col).into());
    vars.push(char_meta("_NAME_", 32));
    for (c, lbl) in labels.iter().enumerate() {
        let data: Vec<Option<f64>> = (0..m).map(|r| Some(a[r][c])).collect();
        columns.push(Series::new(lbl.as_str().into(), data).into());
        vars.push(num_var_meta(lbl));
    }

    let out_df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df: out_df, vars };
    let out_libref = out.libref_or_work();
    let out_table = out.name.to_uppercase();
    let display = format!("{out_libref}.{out_table}");
    let n_rows = out_ds.n_obs();
    let n_vars_out = out_ds.vars.len();
    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(display.clone());
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_rows, n_vars_out
    ));
    Ok(())
}

/// VarMeta for a character output column (M36.8).
fn char_meta(name: &str, length: usize) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Char,
        length,
        format: None,
        label: None,
    }
}

// ───────────────────────── OUTPUT dataset ─────────────────────────

/// Write the OUTPUT dataset(s) associated with this model, using the model's
/// fit (complete cases only).
#[allow(clippy::too_many_arguments)]
pub(super) fn write_outputs(
    entry: &RegModelEntry,
    ds: &SasDataset,
    complete_mask: &[bool],
    n: usize,
    fit: &OlsFit,
    x_mat: &[Vec<f64>],
    p_eff: usize,
    alpha: f64,
    reg_names: &[String],
    intercept: bool,
    weighting: Option<&Weighting>,
    session: &mut Session,
) -> Result<()> {
    if entry.outputs.is_empty() {
        return Ok(());
    }

    // Per-observation std errors / limits, computed lazily once if any OUTPUT
    // requests a leverage-derived column. Keeps the P=/R=-only path allocation-
    // free and byte-identical to before.
    let needs_stats = entry.outputs.iter().any(|o| {
        o.stdp.is_some()
            || o.stdi.is_some()
            || o.stdr.is_some()
            || o.lcl.is_some()
            || o.ucl.is_some()
            || o.lclm.is_some()
            || o.uclm.is_some()
    });
    let obs_stats: Option<Vec<ObsStat>> = if needs_stats {
        Some(compute_obs_stats(x_mat, &reconstruct_y(fit), fit, n, p_eff, alpha, weighting))
    } else {
        None
    };

    // Influence diagnostics, computed lazily once if any OUTPUT requests a
    // STUDENT/RSTUDENT/COOKD/H/PRESS/DFFITS/COVRATIO/DFBETAS column.
    let needs_infl = entry.outputs.iter().any(|o| {
        o.student.is_some()
            || o.rstudent.is_some()
            || o.cookd.is_some()
            || o.h.is_some()
            || o.press.is_some()
            || o.dffits.is_some()
            || o.covratio.is_some()
            || o.dfbetas.is_some()
    });
    let infl_stats: Option<Vec<InfluenceStat>> = if needs_infl {
        Some(compute_influence_stats(x_mat, &reconstruct_y(fit), fit, n, p_eff, weighting))
    } else {
        None
    };

    let mut complete_indices: Vec<usize> = Vec::with_capacity(n);
    for (i, &is_complete) in complete_mask.iter().enumerate() {
        if is_complete {
            complete_indices.push(i);
        }
    }

    for out_spec in &entry.outputs {
        let n_cols = ds.vars.len();
        let mut columns: Vec<Column> = Vec::with_capacity(n_cols + 2);
        let mut out_vars: Vec<VarMeta> = Vec::with_capacity(n_cols + 2);

        for col_idx in 0..n_cols {
            let col_vals = decode_column(ds, col_idx)?;
            match ds.vars[col_idx].ty {
                VarType::Num => {
                    let data: Vec<Option<f64>> = complete_indices
                        .iter()
                        .map(|&i| value_to_num(&col_vals[i]))
                        .collect();
                    columns.push(Series::new(ds.vars[col_idx].name.as_str().into(), data).into());
                }
                VarType::Char => {
                    let data: Vec<Option<String>> = complete_indices
                        .iter()
                        .map(|&i| match &col_vals[i] {
                            crate::value::Value::Char(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect();
                    columns.push(Series::new(ds.vars[col_idx].name.as_str().into(), data).into());
                }
            }
            out_vars.push(ds.vars[col_idx].clone());
        }

        if let Some(pred_name) = &out_spec.predicted {
            let data: Vec<Option<f64>> = fit.y_hat.iter().map(|&v| Some(v)).collect();
            columns.push(Series::new(pred_name.as_str().into(), data).into());
            out_vars.push(num_var_meta(pred_name));
        }
        if let Some(resid_name) = &out_spec.residual {
            let data: Vec<Option<f64>> = fit.resid.iter().map(|&v| Some(v)).collect();
            columns.push(Series::new(resid_name.as_str().into(), data).into());
            out_vars.push(num_var_meta(resid_name));
        }
        // M36.2 — leverage-derived OUTPUT columns. Each is appended in the order
        // SAS lists them on the OUTPUT statement keyword set.
        if let Some(stats) = &obs_stats {
            let mut push_col = |name: &Option<String>, f: &dyn Fn(&ObsStat) -> f64| {
                if let Some(nm) = name {
                    let data: Vec<Option<f64>> = stats.iter().map(|s| Some(f(s))).collect();
                    columns.push(Series::new(nm.as_str().into(), data).into());
                    out_vars.push(num_var_meta(nm));
                }
            };
            push_col(&out_spec.stdp, &|s| s.stdp);
            push_col(&out_spec.stdi, &|s| s.stdi);
            push_col(&out_spec.stdr, &|s| s.stdr);
            push_col(&out_spec.lclm, &|s| s.lclm);
            push_col(&out_spec.uclm, &|s| s.uclm);
            push_col(&out_spec.lcl, &|s| s.lcl);
            push_col(&out_spec.ucl, &|s| s.ucl);
        }
        // M36.3 — influence-diagnostic OUTPUT columns. Non-finite (undefined)
        // values become SAS missing (None).
        if let Some(stats) = &infl_stats {
            let mut push_col = |name: &Option<String>, f: &dyn Fn(&InfluenceStat) -> f64| {
                if let Some(nm) = name {
                    let data: Vec<Option<f64>> = stats
                        .iter()
                        .map(|s| {
                            let v = f(s);
                            if v.is_finite() {
                                Some(v)
                            } else {
                                None
                            }
                        })
                        .collect();
                    columns.push(Series::new(nm.as_str().into(), data).into());
                    out_vars.push(num_var_meta(nm));
                }
            };
            push_col(&out_spec.student, &|s| s.student);
            push_col(&out_spec.rstudent, &|s| s.rstudent);
            push_col(&out_spec.cookd, &|s| s.cookd);
            push_col(&out_spec.h, &|s| s.h);
            push_col(&out_spec.press, &|s| s.press);
            push_col(&out_spec.dffits, &|s| s.dffits);
            push_col(&out_spec.covratio, &|s| s.covratio);
            // DFBETAS= prefix → one column per parameter named `<prefix>_<var>`
            // (Intercept first if present).
            if let Some(prefix) = &out_spec.dfbetas {
                for j in 0..p_eff {
                    let var = if intercept {
                        if j == 0 {
                            "Intercept".to_string()
                        } else {
                            reg_names[j - 1].clone()
                        }
                    } else {
                        reg_names[j].clone()
                    };
                    let col_name = format!("{}_{}", prefix, var);
                    let data: Vec<Option<f64>> = stats
                        .iter()
                        .map(|s| {
                            let v = s.dfbetas[j];
                            if v.is_finite() {
                                Some(v)
                            } else {
                                None
                            }
                        })
                        .collect();
                    columns.push(Series::new(col_name.as_str().into(), data).into());
                    out_vars.push(num_var_meta(&col_name));
                }
            }
        }

        let out_df = DataFrame::new(columns)?;
        let out_ds = SasDataset {
            df: out_df,
            vars: out_vars,
        };

        let out_libref = out_spec.out.libref_or_work();
        let out_table = out_spec.out.name.to_uppercase();
        let display = format!("{out_libref}.{out_table}");
        let n_rows = out_ds.n_obs();
        let n_vars_out = out_ds.vars.len();
        session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
        session.last_dataset = Some(display.clone());
        session.log.note(&format!(
            "The data set {} has {} observations and {} variables.",
            display, n_rows, n_vars_out
        ));
    }

    Ok(())
}

/// Generate (or defer) the automatic residuals-vs-predicted diagnostic plot
/// after a MODEL statement, when `ods_graphics.enabled` is true (the caller
/// checks this). Default build: a deferral NOTE; `--features graphics`: a
/// `reg_{N}` scatter image (x = predicted, y = residual).
pub(super) fn reg_diagnostic_plot(session: &mut Session, y_hat: &[f64], resid: &[f64]) {
    #[cfg(not(feature = "graphics"))]
    {
        let _ = (y_hat, resid);
        session
            .log
            .note("REG diagnostics: image deferred (compile with --features graphics).");
    }

    #[cfg(feature = "graphics")]
    {
        use crate::graphics::render::{draw_to_file, DrawingSpec, PlotType};

        let data: Vec<(f64, f64)> = y_hat
            .iter()
            .zip(resid.iter())
            .filter(|(p, r)| p.is_finite() && r.is_finite())
            .map(|(p, r)| (*p, *r))
            .collect();
        let spec = DrawingSpec {
            title: "The REG Procedure".to_string(),
            x_label: "Predicted Value".to_string(),
            y_label: "Residual".to_string(),
            plot_type: PlotType::Scatter,
            data,
            x_categorical: vec![],
        };

        session.graphics_image_count += 1;
        let stem = session
            .ods_graphics
            .file_stem
            .clone()
            .unwrap_or_else(|| "reg".to_string());
        let fmt = session.ods_graphics.image_format;
        let name = format!(
            "{}_{}.{}",
            stem,
            session.graphics_image_count,
            fmt.extension()
        );
        let path = session.ods_graphics.output_dir.join(&name);
        match draw_to_file(
            &spec,
            &path,
            session.ods_graphics.width,
            session.ods_graphics.height,
            fmt,
        ) {
            Ok((w, h)) => {
                session
                    .log
                    .note(&format!("Output '{}' ({}x{}) written.", name, w, h));
            }
            Err(e) => {
                session
                    .log
                    .note(&format!("WARNING: could not write image {}: {}", name, e));
            }
        }
    }
}

/// M36.11 — render (or defer) the explicit `PLOTS=(…)` diagnostic request set
/// after a MODEL fit. Default build: one plural-invariant deferral NOTE listing
/// the requested plot count. `--features graphics`: each requested plot family
/// (DIAGNOSTICS, RESIDUALS, FIT) is rendered as a separate `reg_{N}` image
/// (panel components are emitted unpacked). `ALL` expands to all three families.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_plot_requests(
    req: &PlotRequests,
    x_mat: &[Vec<f64>],
    y: &[f64],
    fit: &OlsFit,
    n: usize,
    p_eff: usize,
    alpha: f64,
    sel_reg_names: &[String],
    intercept: bool,
    weighting: Option<&Weighting>,
    session: &mut Session,
) {
    let want_diag = req.all || req.diagnostics;
    let want_resid = req.all || req.residuals;
    let want_fit = req.all || req.fit;

    #[cfg(not(feature = "graphics"))]
    {
        let _ = (x_mat, y, fit, n, p_eff, alpha, sel_reg_names, intercept, weighting);
        let count = [want_diag, want_resid, want_fit]
            .iter()
            .filter(|b| **b)
            .count();
        session.log.note(&format!(
            "REG PLOTS= request: {} plot(s) deferred (compile with --features graphics).",
            count
        ));
        return;
    }

    #[cfg(feature = "graphics")]
    {
        use crate::graphics::render::{
            Decorations, DrawingSpec, Overlay, PlotType, SeriesColor,
        };

        let obs = compute_obs_stats(x_mat, y, fit, n, p_eff, alpha, weighting);
        let infl = compute_influence_stats(x_mat, y, fit, n, p_eff, weighting);

        // Helper: residual-vs-predicted scatter (the core diagnostics panel cell).
        if want_diag {
            // (1) Residual vs predicted.
            let data: Vec<(f64, f64)> = fit
                .y_hat
                .iter()
                .zip(fit.resid.iter())
                .filter(|(p, r)| p.is_finite() && r.is_finite())
                .map(|(p, r)| (*p, *r))
                .collect();
            render_reg_image(
                session,
                &DrawingSpec {
                    title: "Fit Diagnostics — Residual by Predicted".to_string(),
                    x_label: "Predicted Value".to_string(),
                    y_label: "Residual".to_string(),
                    plot_type: PlotType::Scatter,
                    data,
                    x_categorical: vec![],
                },
                &Decorations::default(),
            );

            // (2) RStudent vs predicted.
            let data: Vec<(f64, f64)> = infl
                .iter()
                .filter(|s| s.y_hat.is_finite() && s.rstudent.is_finite())
                .map(|s| (s.y_hat, s.rstudent))
                .collect();
            render_reg_image(
                session,
                &DrawingSpec {
                    title: "Fit Diagnostics — RStudent by Predicted".to_string(),
                    x_label: "Predicted Value".to_string(),
                    y_label: "RStudent".to_string(),
                    plot_type: PlotType::Scatter,
                    data,
                    x_categorical: vec![],
                },
                &Decorations::default(),
            );

            // (3) Cook's D vs leverage.
            let data: Vec<(f64, f64)> = infl
                .iter()
                .filter(|s| s.h.is_finite() && s.cookd.is_finite())
                .map(|s| (s.h, s.cookd))
                .collect();
            render_reg_image(
                session,
                &DrawingSpec {
                    title: "Fit Diagnostics — Cook's D by Leverage".to_string(),
                    x_label: "Leverage".to_string(),
                    y_label: "Cook's D".to_string(),
                    plot_type: PlotType::Scatter,
                    data,
                    x_categorical: vec![],
                },
                &Decorations::default(),
            );

            // (4) Normal QQ-plot of residuals: sorted residuals vs normal scores.
            let mut rs: Vec<f64> = fit.resid.iter().copied().filter(|r| r.is_finite()).collect();
            rs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let m = rs.len();
            let qq: Vec<(f64, f64)> = rs
                .iter()
                .enumerate()
                .map(|(i, &r)| {
                    let p = (i as f64 + 0.5) / m as f64;
                    (normal_quantile(p), r)
                })
                .collect();
            render_reg_image(
                session,
                &DrawingSpec {
                    title: "Fit Diagnostics — Normal Q-Q".to_string(),
                    x_label: "Quantile".to_string(),
                    y_label: "Residual".to_string(),
                    plot_type: PlotType::Scatter,
                    data: qq,
                    x_categorical: vec![],
                },
                &Decorations::default(),
            );
        }

        // RESIDUALS — residual vs each regressor column.
        if want_resid {
            let off = usize::from(intercept);
            for (j, name) in sel_reg_names.iter().enumerate() {
                let col = off + j;
                let data: Vec<(f64, f64)> = (0..n)
                    .filter_map(|i| {
                        let xv = x_mat[i][col];
                        let rv = fit.resid[i];
                        (xv.is_finite() && rv.is_finite()).then_some((xv, rv))
                    })
                    .collect();
                render_reg_image(
                    session,
                    &DrawingSpec {
                        title: format!("Residual by {}", name),
                        x_label: name.clone(),
                        y_label: "Residual".to_string(),
                        plot_type: PlotType::Scatter,
                        data,
                        x_categorical: vec![],
                    },
                    &Decorations::default(),
                );
            }
        }

        // FIT — single-regressor fit plot with the regression line and the
        // CLM (mean) / CLI (individual) confidence bands. Only meaningful when
        // there is exactly one regressor (plus the intercept).
        if want_fit {
            let off = usize::from(intercept);
            if sel_reg_names.len() == 1 {
                let col = off; // the single regressor column
                let mut pts: Vec<(f64, ObsStat)> = (0..n)
                    .filter(|&i| x_mat[i][col].is_finite())
                    .map(|i| (x_mat[i][col], obs[i].clone()))
                    .collect();
                pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

                let data: Vec<(f64, f64)> =
                    pts.iter().map(|(x, o)| (*x, o.y)).collect();
                let fit_line: Vec<(f64, f64)> =
                    pts.iter().map(|(x, o)| (*x, o.y_hat)).collect();
                let clm_lo: Vec<(f64, f64)> =
                    pts.iter().map(|(x, o)| (*x, o.lclm)).collect();
                let clm_hi: Vec<(f64, f64)> =
                    pts.iter().map(|(x, o)| (*x, o.uclm)).collect();
                let cli_lo: Vec<(f64, f64)> =
                    pts.iter().map(|(x, o)| (*x, o.lcl)).collect();
                let cli_hi: Vec<(f64, f64)> =
                    pts.iter().map(|(x, o)| (*x, o.ucl)).collect();
                let deco = Decorations {
                    overlays: vec![
                        Overlay { data: fit_line, color: SeriesColor::Blue, line: true, marker: false },
                        Overlay { data: clm_lo, color: SeriesColor::Green, line: true, marker: false },
                        Overlay { data: clm_hi, color: SeriesColor::Green, line: true, marker: false },
                        Overlay { data: cli_lo, color: SeriesColor::Orange, line: true, marker: false },
                        Overlay { data: cli_hi, color: SeriesColor::Orange, line: true, marker: false },
                    ],
                    x_range: None,
                    y_range: None,
                };
                render_reg_image(
                    session,
                    &DrawingSpec {
                        title: format!("Fit Plot by {}", sel_reg_names[0]),
                        x_label: sel_reg_names[0].clone(),
                        y_label: "Response".to_string(),
                        plot_type: PlotType::Scatter,
                        data,
                        x_categorical: vec![],
                    },
                    &deco,
                );
            } else {
                // Multi-regressor fit plot is not meaningful; fall back to a
                // response-vs-predicted scatter (the SAS panel's analogue).
                let data: Vec<(f64, f64)> = (0..n)
                    .filter(|&i| fit.y_hat[i].is_finite() && y[i].is_finite())
                    .map(|i| (fit.y_hat[i], y[i]))
                    .collect();
                render_reg_image(
                    session,
                    &DrawingSpec {
                        title: "Fit Plot — Observed by Predicted".to_string(),
                        x_label: "Predicted Value".to_string(),
                        y_label: "Response".to_string(),
                        plot_type: PlotType::Scatter,
                        data,
                        x_categorical: vec![],
                    },
                    &Decorations::default(),
                );
            }
        }
    }
}

/// M36.11 — render (or defer) the traditional `PLOT y*x …;` scatters after a
/// MODEL fit. Default build: one plural-invariant deferral NOTE per statement
/// listing the pair count. `--features graphics`: each (y,x) pair is rendered as
/// a separate `reg_{N}` scatter, resolving the `PREDICTED.`/`RESIDUAL.` special
/// variables from the fit and plain names from the design matrix.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_plot_statements(
    pairs: &[PlotPair],
    dep_name: &str,
    x_mat: &[Vec<f64>],
    fit: &OlsFit,
    sel_reg_names: &[String],
    intercept: bool,
    session: &mut Session,
) {
    #[cfg(not(feature = "graphics"))]
    {
        let _ = (dep_name, x_mat, fit, sel_reg_names, intercept);
        session.log.note(&format!(
            "REG PLOT statement: {} scatter(s) deferred (compile with --features graphics).",
            pairs.len()
        ));
        return;
    }

    #[cfg(feature = "graphics")]
    {
        use crate::graphics::render::{Decorations, DrawingSpec, PlotType};

        // Resolve a PlotVar to a per-observation value series + an axis label.
        let resolve = |v: &PlotVar| -> Option<(Vec<f64>, String)> {
            match v {
                PlotVar::Predicted => Some((fit.y_hat.clone(), "Predicted Value".to_string())),
                PlotVar::Residual => Some((fit.resid.clone(), "Residual".to_string())),
                PlotVar::Named(name) => {
                    let up = name.to_ascii_uppercase();
                    if up == dep_name.to_ascii_uppercase() {
                        // The dependent: reconstruct y = ŷ + resid.
                        let yv: Vec<f64> = fit
                            .y_hat
                            .iter()
                            .zip(fit.resid.iter())
                            .map(|(p, r)| p + r)
                            .collect();
                        return Some((yv, name.clone()));
                    }
                    // A regressor column.
                    let off = usize::from(intercept);
                    sel_reg_names
                        .iter()
                        .position(|r| r.eq_ignore_ascii_case(&up))
                        .map(|j| {
                            let col = off + j;
                            let vals: Vec<f64> = (0..x_mat.len()).map(|i| x_mat[i][col]).collect();
                            (vals, name.clone())
                        })
                }
            }
        };

        for pair in pairs {
            let (Some((ys, ylab)), Some((xs, xlab))) = (resolve(&pair.y), resolve(&pair.x)) else {
                session.log.note(
                    "REG PLOT statement: a variable could not be resolved; that plot is skipped.",
                );
                continue;
            };
            let data: Vec<(f64, f64)> = xs
                .iter()
                .zip(ys.iter())
                .filter(|(x, y)| x.is_finite() && y.is_finite())
                .map(|(x, y)| (*x, *y))
                .collect();
            render_reg_image(
                session,
                &DrawingSpec {
                    title: format!("Plot of {} by {}", ylab, xlab),
                    x_label: xlab,
                    y_label: ylab,
                    plot_type: PlotType::Scatter,
                    data,
                    x_categorical: vec![],
                },
                &Decorations::default(),
            );
        }
    }
}

/// M36.11 (graphics only) — render one `DrawingSpec`+`Decorations` into the next
/// `reg_{N}.{fmt}` image in `ods_graphics.output_dir`, mirroring the naming /
/// counter / NOTE convention of `reg_diagnostic_plot`.
#[cfg(feature = "graphics")]
fn render_reg_image(
    session: &mut Session,
    spec: &crate::graphics::render::DrawingSpec,
    deco: &crate::graphics::render::Decorations,
) {
    use crate::graphics::render::draw_to_file_ext;

    session.graphics_image_count += 1;
    let stem = session
        .ods_graphics
        .file_stem
        .clone()
        .unwrap_or_else(|| "reg".to_string());
    let fmt = session.ods_graphics.image_format;
    let name = format!("{}_{}.{}", stem, session.graphics_image_count, fmt.extension());
    let path = session.ods_graphics.output_dir.join(&name);
    match draw_to_file_ext(
        spec,
        deco,
        &path,
        session.ods_graphics.width,
        session.ods_graphics.height,
        fmt,
    ) {
        Ok((w, h)) => {
            session
                .log
                .note(&format!("Output '{}' ({}x{}) written.", name, w, h));
        }
        Err(e) => {
            session
                .log
                .note(&format!("WARNING: could not write image {}: {}", name, e));
        }
    }
}

/// M36.11 (graphics only) — approximate standard-normal quantile (inverse CDF)
/// via the Beasley-Springer/Moro algorithm, for the QQ-plot's theoretical
/// scores. Accuracy is ample for plotting; not used on any printed path.
#[cfg(feature = "graphics")]
fn normal_quantile(p: f64) -> f64 {
    // Clamp away from the open-interval endpoints.
    let p = p.clamp(1e-9, 1.0 - 1e-9);
    // Rational approximation (Acklam) — relative error < 1.15e-9.
    const A: [f64; 6] = [
        -3.969683028665376e+01, 2.209460984245205e+02, -2.759285104469687e+02,
        1.383577518672690e+02, -3.066479806614716e+01, 2.506628277459239e+00,
    ];
    const B: [f64; 5] = [
        -5.447609879822406e+01, 1.615858368580409e+02, -1.556989798598866e+02,
        6.680131188771972e+01, -1.328068155288572e+01,
    ];
    const C: [f64; 6] = [
        -7.784894002430293e-03, -3.223964580411365e-01, -2.400758277161838e+00,
        -2.549732539343734e+00, 4.374664141464968e+00, 2.938163982698783e+00,
    ];
    const D: [f64; 4] = [
        7.784695709041462e-03, 3.224671290700398e-01, 2.445134137142996e+00,
        3.754408661907416e+00,
    ];
    let plow = 0.02425;
    let phigh = 1.0 - plow;
    if p < plow {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= phigh {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    }
}
