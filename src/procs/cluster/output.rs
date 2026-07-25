use super::*;

/// Write the OUTTREE= (dendrogram) dataset.
///
/// One observation per node = (#leaves) + (#merges) rows:
///  - leaves: `_NAME_` = the singleton label (ID value or `OBn`), `_FREQ_`=1,
///    `_HEIGHT_`=0, and the original VAR values;
///  - clusters: `_NAME_` = `CL<n>`, `_FREQ_` = number of obs, `_HEIGHT_` = the
///    join criterion (here the cumulative RSQ-complement, monotone increasing).
///
/// Columns emitted (the common SAS core): `_NAME_`, `_PARENT_`, `_NCL_`,
/// `_FREQ_`, `_HEIGHT_`, plus one numeric column per VAR (leaf coordinates).
pub(super) fn write_outtree(
    out_ref: &DatasetRef,
    labels: &[String],
    decoded: &[Vec<f64>],
    var_names: &[String],
    history: &[MergeStep],
    session: &mut Session,
) -> Result<()> {
    use crate::dataset::{SasDataset, VarMeta};
    use polars::prelude::*;
    use std::collections::HashMap;

    // Parent of every node, filled while walking the merge history. The two
    // nodes joined at a merge get that merge's cluster name as their parent.
    let mut parent: HashMap<String, String> = HashMap::new();
    for step in history {
        let cl = if step.ncl == 0 {
            "CL1".to_string()
        } else {
            format!("CL{}", step.ncl)
        };
        parent.insert(step.joined_a.clone(), cl.clone());
        parent.insert(step.joined_b.clone(), cl);
    }

    // Row accumulators (one entry per observation in the OUTTREE dataset).
    let mut name_vals: Vec<String> = Vec::new();
    let mut parent_vals: Vec<Option<String>> = Vec::new();
    let mut ncl_vals: Vec<Option<f64>> = Vec::new();
    let mut freq_vals: Vec<f64> = Vec::new();
    let mut height_vals: Vec<f64> = Vec::new();
    // One coordinate column per VAR; leaves carry the value, clusters are missing.
    let n_var = var_names.len();
    let mut coord_vals: Vec<Vec<Option<f64>>> = vec![Vec::new(); n_var];

    // Leaves first.
    for (i, lab) in labels.iter().enumerate() {
        name_vals.push(lab.clone());
        parent_vals.push(parent.get(lab).cloned());
        ncl_vals.push(None);
        freq_vals.push(1.0);
        height_vals.push(0.0);
        for v in 0..n_var {
            coord_vals[v].push(Some(decoded[v][i]));
        }
    }

    // Then one row per merge (cluster node).
    for step in history {
        let cl = if step.ncl == 0 {
            "CL1".to_string()
        } else {
            format!("CL{}", step.ncl)
        };
        name_vals.push(cl.clone());
        parent_vals.push(parent.get(&cl).cloned());
        ncl_vals.push(Some(step.ncl as f64));
        freq_vals.push(step.freq as f64);
        // Join height: cumulative within-cluster SS as a fraction of the total
        // (1 - RSQ). This is monotone increasing along successive merges.
        height_vals.push((1.0 - step.rsq).max(0.0));
        for v in 0..n_var {
            coord_vals[v].push(None);
        }
    }

    let mut columns: Vec<Column> = Vec::new();
    columns.push(Series::new("_NAME_".into(), name_vals).into());
    columns.push(Series::new("_PARENT_".into(), parent_vals).into());
    columns.push(Series::new("_NCL_".into(), ncl_vals).into());
    columns.push(Series::new("_FREQ_".into(), freq_vals).into());
    columns.push(Series::new("_HEIGHT_".into(), height_vals).into());
    for (v, name) in var_names.iter().enumerate() {
        columns.push(Series::new(name.as_str().into(), coord_vals[v].clone()).into());
    }

    let df = DataFrame::new(columns)
        .map_err(|e| SasError::runtime(format!("CLUSTER OUTTREE= build failed: {e}")))?;

    let mut vars: Vec<VarMeta> = vec![
        VarMeta { name: "_NAME_".into(), ty: VarType::Char, length: 32, format: None, label: None },
        VarMeta { name: "_PARENT_".into(), ty: VarType::Char, length: 32, format: None, label: None },
        VarMeta { name: "_NCL_".into(), ty: VarType::Num, length: 8, format: None, label: None },
        VarMeta { name: "_FREQ_".into(), ty: VarType::Num, length: 8, format: None, label: None },
        VarMeta { name: "_HEIGHT_".into(), ty: VarType::Num, length: 8, format: None, label: None },
    ];
    for name in var_names {
        vars.push(VarMeta {
            name: name.clone(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        });
    }

    let out_ds = SasDataset { df, vars };
    let out_libref = out_ref.libref_or_work();
    let out_table = out_ref.name.to_uppercase();
    let out_display = format!("{out_libref}.{out_table}");
    let n_rows = out_ds.n_obs();
    let n_vars = out_ds.vars.len();
    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(out_display.clone());
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        out_display, n_rows, n_vars
    ));
    Ok(())
}

pub(super) fn label_of_value(v: &crate::value::Value) -> String {
    use crate::value::{format_best, Value};
    match v {
        Value::Char(s) => s.trim().to_string(),
        Value::Num(f) => format_best(*f, 12).trim().to_string(),
        Value::Missing(k) => k.display(),
    }
}
