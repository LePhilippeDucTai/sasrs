use super::*;

// ───────────────────────── OUT= (TYPE=CORR) ─────────────────────────

/// One BY group's analysis data (J08-P2): decoded columns filtered to the
/// group's rows (indices 0..n_obs within the group), the group's weight
/// slice, and the BY key values. With no BY there is a single group spanning
/// all rows and an empty key.
pub(super) struct CorrOutGroup {
    pub(super) by_key: Vec<Value>,
    pub(super) decoded: std::collections::HashMap<usize, Vec<Value>>,
    pub(super) weight: Option<Vec<Value>>,
    pub(super) n_obs: usize,
}

/// Build a TYPE=CORR output dataset for `method` over the square analysis ×
/// analysis correlation matrix, once per BY group. Layout (SAS, issue #18) :
///   [BY vars] _TYPE_  _NAME_   <var1> <var2> ...
///             N                n1     n2     ...
///             MEAN             m1     m2     ...
///             STD              s1     s2     ...
///             SUM              t1     t2     ...
///             MIN              l1     l2     ...
///             MAX              u1     u2     ...
///             CORR    var1     r11    r12    ...
///             CORR    var2     r21    r22    ...
/// The six descriptive rows carry an empty `_NAME_` and follow the order of
/// the listing Simple Statistics (N, Mean, Std Dev, Sum, Minimum, Maximum).
/// The CORR block uses the same
/// pairwise-complete r computed for the listing. WEIGHT applies to Pearson.
/// With BY (J08-P2) the block repeats per group, the BY variables in head
/// columns; without BY the layout is byte-identical to the pre-J08 form.
pub(super) fn build_out_dataset(
    method: Method,
    ds: &SasDataset,
    analysis_cols: &[usize],
    by_cols: &[common::ByCol],
    groups: &[CorrOutGroup],
) -> Result<SasDataset> {
    let k = analysis_cols.len();
    let block_rows = 6 + k;

    let mut type_col: Vec<Option<String>> = Vec::new();
    let mut name_col: Vec<Option<String>> = Vec::new();
    // One value column per analysis variable.
    let mut value_cols: Vec<Vec<Option<f64>>> = vec![Vec::new(); k];
    // One column per BY variable, one value per output row.
    let mut by_cols_vals: Vec<Vec<Value>> = vec![Vec::new(); by_cols.len()];

    for g in groups {
        let all_rows: Vec<usize> = (0..g.n_obs).collect();

        // Per-variable simple stats (unweighted, matching the SAS TYPE=CORR
        // simple-statistics rows — same values as the listing Simple
        // Statistics block; WEIGHT does not alter these rows in v1).
        let mut means = Vec::with_capacity(k);
        let mut stds = Vec::with_capacity(k);
        let mut ns = Vec::with_capacity(k);
        let mut sums = Vec::with_capacity(k);
        let mut mins = Vec::with_capacity(k);
        let mut maxs = Vec::with_capacity(k);
        for &c in analysis_cols {
            let (xs, _) = partition_numeric(&g.decoded[&c], &all_rows);
            let n = xs.len();
            means.push(if n > 0 {
                Some(xs.iter().sum::<f64>() / n as f64)
            } else {
                None
            });
            stds.push(sample_std(&xs));
            ns.push(n as f64);
            sums.push(if n > 0 {
                Some(xs.iter().sum::<f64>())
            } else {
                None
            });
            mins.push(xs.iter().copied().fold(None::<f64>, |a, x| {
                Some(match a {
                    Some(m) if m < x => m,
                    _ => x,
                })
            }));
            maxs.push(xs.iter().copied().fold(None::<f64>, |a, x| {
                Some(match a {
                    Some(m) if m > x => m,
                    _ => x,
                })
            }));
        }

        // CORR block: square matrix over analysis_cols.
        let cells = compute_matrix(
            method,
            analysis_cols,
            analysis_cols,
            &g.decoded,
            g.weight.as_deref(),
        );

        // Assemble row-major then transpose into columns.
        // Row order (SAS): N, MEAN, STD, SUM, MIN, MAX, then one CORR row
        // per analysis variable.
        type_col.push(Some("N".into()));
        name_col.push(None);
        for j in 0..k {
            value_cols[j].push(Some(ns[j]));
        }
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
        type_col.push(Some("SUM".into()));
        name_col.push(None);
        for j in 0..k {
            value_cols[j].push(sums[j]);
        }
        type_col.push(Some("MIN".into()));
        name_col.push(None);
        for j in 0..k {
            value_cols[j].push(mins[j]);
        }
        type_col.push(Some("MAX".into()));
        name_col.push(None);
        for j in 0..k {
            value_cols[j].push(maxs[j]);
        }
        for i in 0..k {
            type_col.push(Some(CORR_TYPE.into()));
            name_col.push(Some(ds.vars[analysis_cols[i]].name.clone()));
            for j in 0..k {
                value_cols[j].push(cells[i][j].r);
            }
        }

        // BY columns: the group key repeated on each row of the block.
        for (bi, bv) in by_cols_vals.iter_mut().enumerate() {
            bv.extend(std::iter::repeat_n(g.by_key[bi].clone(), block_rows));
        }
    }

    // Build columns: BY variables first (J08-P2), then _TYPE_ (char), _NAME_
    // (char), then one numeric column per analysis variable (original
    // variable name preserved).
    let mut columns: Vec<Column> = Vec::with_capacity(k + 2 + by_cols.len());
    let mut vars: Vec<VarMeta> = Vec::with_capacity(k + 2 + by_cols.len());

    for (bc, bv) in by_cols.iter().zip(by_cols_vals) {
        let meta = &ds.vars[bc.col_idx];
        let series = match meta.ty {
            VarType::Num => {
                let vals: Vec<Option<f64>> = bv.iter().map(value_to_num).collect();
                Series::new(meta.name.as_str().into(), vals)
            }
            VarType::Char => {
                let vals: Vec<Option<String>> = bv
                    .iter()
                    .map(|v| match v {
                        Value::Char(s) if s.is_empty() => None,
                        Value::Char(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
        };
        columns.push(series.into());
        vars.push(meta.clone());
    }

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
