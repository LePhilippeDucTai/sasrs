use super::*;

/// Common variable (by name, case-insensitive), with type match analysis.
pub(super) struct CommonVar {
    pub(super) name: String,
    pub(super) base_idx: usize,
    pub(super) comp_idx: usize,
    pub(super) type_match: bool,
    pub(super) base_type: VarType,
    pub(super) comp_type: VarType,
}

/// For each common var that has matching types: track max abs diff (numeric)
/// and whether any diffs occurred (char).
pub(super) struct VarDiffSummary {
    pub(super) name: String,
    pub(super) var_type: VarType,
    pub(super) n_diffs: usize,
    pub(super) max_diff: f64, // only for numeric
}

/// Read one input dataset (BASE= or COMPARE=), forwarding provider notes.
pub(super) fn read_input(session: &mut Session, dsref: &DatasetRef) -> Result<SasDataset> {
    let libref = dsref.libref_or_work();
    let name = dsref.name.to_uppercase();
    let provider = session.libs.get(&libref)?;
    let (ds, notes) = provider.read(&name)?;
    for note in notes {
        session.log.forward(&note);
    }
    Ok(ds)
}

/// Variable analysis: names only in BASE, only in COMPARE, and the sorted
/// common-variable list (with per-variable type match).
pub(super) fn analyze_variables(
    base_ds: &SasDataset,
    comp_ds: &SasDataset,
) -> (Vec<String>, Vec<String>, Vec<CommonVar>) {
    // Build maps: name → (index, VarMeta) for each dataset
    let base_var_map: HashMap<String, usize> = base_ds
        .vars
        .iter()
        .enumerate()
        .map(|(i, v)| (v.name.to_uppercase(), i))
        .collect();
    let comp_var_map: HashMap<String, usize> = comp_ds
        .vars
        .iter()
        .enumerate()
        .map(|(i, v)| (v.name.to_uppercase(), i))
        .collect();

    // Variables only in BASE
    let mut only_base: Vec<String> = base_ds
        .vars
        .iter()
        .filter(|v| !comp_var_map.contains_key(&v.name.to_uppercase()))
        .map(|v| v.name.to_uppercase())
        .collect();
    only_base.sort();

    // Variables only in COMPARE
    let mut only_comp: Vec<String> = comp_ds
        .vars
        .iter()
        .filter(|v| !base_var_map.contains_key(&v.name.to_uppercase()))
        .map(|v| v.name.to_uppercase())
        .collect();
    only_comp.sort();

    let mut common_vars: Vec<CommonVar> = base_ds
        .vars
        .iter()
        .enumerate()
        .filter_map(|(bi, bv)| {
            let uname = bv.name.to_uppercase();
            comp_var_map.get(&uname).map(|&ci| CommonVar {
                name: uname,
                base_idx: bi,
                comp_idx: ci,
                type_match: bv.ty == comp_ds.vars[ci].ty,
                base_type: bv.ty,
                comp_type: comp_ds.vars[ci].ty,
            })
        })
        .collect();
    common_vars.sort_by(|a, b| a.name.cmp(&b.name));

    (only_base, only_comp, common_vars)
}

/// Row-by-row value comparison over the type-matched common variables.
/// Returns (nb obs with differences, per-variable diff summaries, OUT= rows —
/// empty unless `need_out`).
pub(super) fn compare_observations(
    base_ds: &SasDataset,
    comp_ds: &SasDataset,
    matching_vars: &[&CommonVar],
    n_compared: usize,
    need_out: bool,
) -> (usize, Vec<VarDiffSummary>, Vec<OutRow>) {
    let mut n_with_diffs: usize = 0;

    let mut var_diffs: Vec<VarDiffSummary> = matching_vars
        .iter()
        .map(|cv| VarDiffSummary {
            name: cv.name.clone(),
            var_type: cv.base_type,
            n_diffs: 0,
            max_diff: 0.0,
        })
        .collect();

    let mut out_rows: Vec<OutRow> = Vec::new();

    // Build column iterators for the comparison
    // We'll do row-by-row comparison using the Polars Series
    let base_df = &base_ds.df;
    let comp_df = &comp_ds.df;

    // Pre-fetch columns
    // For each common var with type match, get base+comp series
    struct ColPair {
        cv_idx: usize, // index into common_vars (only type-matched ones)
        var_type: VarType,
        base_col_idx: usize,
        comp_col_idx: usize,
    }

    let col_pairs: Vec<ColPair> = matching_vars
        .iter()
        .enumerate()
        .map(|(i, cv)| ColPair {
            cv_idx: i,
            var_type: cv.base_type,
            base_col_idx: cv.base_idx,
            comp_col_idx: cv.comp_idx,
        })
        .collect();

    for obs_idx in 0..n_compared {
        let mut obs_has_diff = false;
        let mut base_out_values: Vec<Option<Value>> = vec![None; col_pairs.len()];
        let mut comp_out_values: Vec<Option<Value>> = vec![None; col_pairs.len()];
        let mut dif_out_values: Vec<Option<Value>> = vec![None; col_pairs.len()];

        for cp in &col_pairs {
            let base_val = get_value_at(base_df, cp.base_col_idx, obs_idx, cp.var_type);
            let comp_val = get_value_at(comp_df, cp.comp_col_idx, obs_idx, cp.var_type);

            let differ = values_differ(&base_val, &comp_val);

            if differ {
                obs_has_diff = true;
                var_diffs[cp.cv_idx].n_diffs += 1;

                // Compute numeric difference for max_diff
                if cp.var_type == VarType::Num
                    && let (Value::Num(b), Value::Num(c)) = (&base_val, &comp_val)
                {
                    let diff = (b - c).abs();
                    if diff > var_diffs[cp.cv_idx].max_diff {
                        var_diffs[cp.cv_idx].max_diff = diff;
                    }
                }
            }

            if need_out {
                base_out_values[cp.cv_idx] = Some(base_val.clone());
                comp_out_values[cp.cv_idx] = Some(comp_val.clone());
                if cp.var_type == VarType::Num {
                    match (&base_val, &comp_val) {
                        (Value::Num(b), Value::Num(c)) => {
                            dif_out_values[cp.cv_idx] = Some(Value::Num(b - c));
                        }
                        _ => {
                            dif_out_values[cp.cv_idx] = Some(Value::missing());
                        }
                    }
                }
            }
        }

        if obs_has_diff {
            n_with_diffs += 1;
            if need_out {
                out_rows.push(OutRow {
                    obs: obs_idx + 1,
                    row_type: "BASE",
                    values: base_out_values,
                });
                out_rows.push(OutRow {
                    obs: obs_idx + 1,
                    row_type: "COMPARE",
                    values: comp_out_values,
                });
                out_rows.push(OutRow {
                    obs: obs_idx + 1,
                    row_type: "DIF",
                    values: dif_out_values,
                });
            }
        }
    }

    (n_with_diffs, var_diffs, out_rows)
}

/// Get a SAS Value from a DataFrame column at a given row index.
/// Only handles Num and Char (the SAS type model).
pub(super) fn get_value_at(df: &DataFrame, col_idx: usize, row_idx: usize, ty: VarType) -> Value {
    let col = &df.get_columns()[col_idx];
    match ty {
        VarType::Num => {
            let f64_col = col.as_materialized_series().f64().unwrap();
            num_to_value(f64_col.get(row_idx))
        }
        VarType::Char => {
            let str_col = col.as_materialized_series().str().unwrap();
            match str_col.get(row_idx) {
                None => Value::Char(String::new()),
                Some(s) => Value::Char(s.to_string()),
            }
        }
    }
}

/// Return true if two SAS values differ (using sas_cmp semantics).
pub(super) fn values_differ(a: &Value, b: &Value) -> bool {
    a.sas_cmp(b) != Ordering::Equal
}

pub(super) fn type_str(ty: VarType) -> &'static str {
    match ty {
        VarType::Num => "Num",
        VarType::Char => "Char",
    }
}
