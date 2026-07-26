use super::*;

// ───────────────────────── OUT= (TYPE=CORR) ─────────────────────────

/// Build a TYPE=CORR output dataset for `method` over the square analysis ×
/// analysis correlation matrix. Layout (SAS):
///   _TYPE_  _NAME_   <var1> <var2> ...
///   MEAN             m1     m2     ...
///   STD              s1     s2     ...
///   N                n1     n2     ...
///   CORR    var1     r11    r12    ...
///   CORR    var2     r21    r22    ...
/// MEAN/STD/N rows carry an empty `_NAME_`. The CORR block uses the same
/// pairwise-complete r computed for the listing. WEIGHT applies to Pearson.
pub(super) fn build_out_dataset(
    method: Method,
    ds: &SasDataset,
    analysis_cols: &[usize],
    decoded: &std::collections::HashMap<usize, Vec<Value>>,
    weight: Option<&[Value]>,
    n_obs: usize,
) -> Result<SasDataset> {
    let k = analysis_cols.len();
    let all_rows: Vec<usize> = (0..n_obs).collect();

    // Per-variable simple stats (unweighted MEAN/STD/N, matching SAS TYPE=CORR
    // simple-statistics rows; WEIGHT does not alter these rows in v1).
    let mut means = Vec::with_capacity(k);
    let mut stds = Vec::with_capacity(k);
    let mut ns = Vec::with_capacity(k);
    for &c in analysis_cols {
        let (xs, _) = partition_numeric(&decoded[&c], &all_rows);
        let n = xs.len();
        means.push(if n > 0 {
            Some(xs.iter().sum::<f64>() / n as f64)
        } else {
            None
        });
        stds.push(sample_std(&xs));
        ns.push(n as f64);
    }

    // CORR block: square matrix over analysis_cols.
    let cells = compute_matrix(method, analysis_cols, analysis_cols, decoded, weight);

    // Assemble row-major then transpose into columns.
    // Row order: MEAN, STD, N, then one CORR row per analysis variable.
    let n_rows = 3 + k;
    let mut type_col: Vec<Option<String>> = Vec::with_capacity(n_rows);
    let mut name_col: Vec<Option<String>> = Vec::with_capacity(n_rows);
    // One value column per analysis variable.
    let mut value_cols: Vec<Vec<Option<f64>>> = vec![Vec::with_capacity(n_rows); k];

    type_col.push(Some("MEAN".into()));
    name_col.push(None);
    for j in 0..k {
        value_cols[j].push(means[j]);
    }
    type_col.push(Some("STD".into()));
    name_col.push(None);
    for j in 0..k {
        value_cols[j].push(stds[j]);
    }
    type_col.push(Some("N".into()));
    name_col.push(None);
    for j in 0..k {
        value_cols[j].push(Some(ns[j]));
    }
    for i in 0..k {
        type_col.push(Some(CORR_TYPE.into()));
        name_col.push(Some(ds.vars[analysis_cols[i]].name.clone()));
        for j in 0..k {
            value_cols[j].push(cells[i][j].r);
        }
    }

    // Build columns: _TYPE_ (char), _NAME_ (char), then one numeric column per
    // analysis variable (original variable name preserved).
    let mut columns: Vec<Column> = Vec::with_capacity(k + 2);
    let mut vars: Vec<VarMeta> = Vec::with_capacity(k + 2);

    columns.push(Series::new("_TYPE_".into(), type_col).into());
    vars.push(char_var_meta("_TYPE_", 8));
    columns.push(Series::new("_NAME_".into(), name_col).into());
    vars.push(char_var_meta("_NAME_", 32));

    for (j, &c) in analysis_cols.iter().enumerate() {
        let name = ds.vars[c].name.clone();
        columns.push(Series::new(name.as_str().into(), std::mem::take(&mut value_cols[j])).into());
        vars.push(num_var_meta(&name));
    }

    let df = DataFrame::new(columns)?;
    Ok(SasDataset { df, vars })
}

/// Persist a built TYPE=CORR dataset to `target`, update `_LAST_`, and emit the
/// SAS creation NOTE.
pub(super) fn write_out_dataset(
    session: &mut Session,
    target: &DatasetRef,
    out_ds: SasDataset,
) -> Result<()> {
    let out_libref = target.libref_or_work();
    let out_table = target.name.to_uppercase();
    let display = format!("{out_libref}.{out_table}");
    let n_rows = out_ds.n_obs();
    let n_vars = out_ds.vars.len();

    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(display.clone());
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_rows, n_vars
    ));
    Ok(())
}
