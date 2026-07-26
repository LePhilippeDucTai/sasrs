use super::*;

// ───────────────────────── OUTEST= / OUTSSCP= datasets (M36.8) ─────────────────────────

/// One fitted model's contribution to the OUTEST= dataset (M36.8). Captures
/// everything needed to emit the PARMS row (and, with modifiers, the COV/SEB
/// rows and the EDF / TABLEOUT columns). Parameter values are stored by name so
/// the writer can align them into the union of parameter columns across models.
pub(crate) struct OutEstEntry {
    /// Model label, e.g. "MODEL1".
    model_label: String,
    /// Dependent variable name.
    depvar: String,
    /// Root MSE.
    pub(crate) rmse: f64,
    /// Parameter names in design order: "Intercept" (if present) then regressors.
    param_names: Vec<String>,
    /// Parameter estimates aligned with `param_names`.
    pub(crate) beta: Vec<f64>,
    /// Standard errors aligned with `param_names`.
    se: Vec<f64>,
    /// Covariance matrix MSE·(X'X)⁻¹ (p_eff×p_eff), aligned with `param_names`.
    covb: Vec<Vec<f64>>,
    /// Number of regressors in the model (_IN_).
    pub(crate) n_in: usize,
    /// Number of parameters (_P_).
    pub(crate) n_p: usize,
    /// Error degrees of freedom (_EDF_).
    pub(crate) edf: f64,
    /// Lower/upper confidence bounds per parameter (TABLEOUT), aligned with
    /// `param_names`.
    lb: Vec<f64>,
    ub: Vec<f64>,
    /// M36.9 — back-transformed RIDGE / RIDGEVIF / IPC rows for this model
    /// (empty unless RIDGE=/PCOMIT= requested). Each carries the row `_TYPE_`,
    /// the `_RIDGE_`/`_PCOMIT_` selector value, and per-parameter values aligned
    /// with `param_names` (intercept first when present).
    pub(crate) ridge_ipc: Vec<RidgeIpcRow>,
}

/// M36.9 — one OUTEST= row produced by ridge / IPC regression. `kind` is the
/// `_TYPE_` value ("RIDGE", "RIDGEVIF", or "IPC"); `ridge_k`/`pcomit_m` carry the
/// `_RIDGE_`/`_PCOMIT_` selector (exactly one is `Some`); `values` are aligned
/// with the entry's `param_names` (Intercept first when present). For RIDGEVIF
/// rows the intercept slot is `None` (VIF undefined for the intercept).
#[derive(Clone)]
pub(crate) struct RidgeIpcRow {
    pub(crate) kind: &'static str,
    pub(crate) ridge_k: Option<f64>,
    pub(crate) pcomit_m: Option<f64>,
    pub(crate) values: Vec<Option<f64>>,
}

/// Build an OUTEST= entry from a completed fit (M36.8).
pub(crate) fn build_outest_entry(
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
    let lb: Vec<f64> = (0..p_eff).map(|j| fit.beta[j] - t_crit * se[j]).collect();
    let ub: Vec<f64> = (0..p_eff).map(|j| fit.beta[j] + t_crit * se[j]).collect();
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
pub(crate) fn write_outest(
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
        e.param_names
            .iter()
            .position(|p| p.eq_ignore_ascii_case(nm))
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
pub(crate) fn write_outsscp(
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
pub(crate) fn char_meta(name: &str, length: usize) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Char,
        length,
        format: None,
        label: None,
    }
}
