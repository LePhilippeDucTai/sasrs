use super::*;

/// M22.3 — écrit la table ODS "Summary" de PROC MEANS comme dataset SAS.
///
/// Structure (périmètre v1) : une observation par variable analysée (VAR),
/// colonne caractère `Variable` (nom de la variable) puis une colonne numérique
/// par statistique du rapport (N, Mean, StdDev, Min, Max par défaut). Les stats
/// sont calculées sur l'ensemble des lignes (pas de partition CLASS/BY en v1 :
/// si CLASS/BY sont présents, on agrège globalement et une NOTE le documente).
#[allow(clippy::too_many_arguments)]
pub(super) fn write_ods_summary(
    session: &mut Session,
    ds: &SasDataset,
    var_cols: &[usize],
    var_values: &[Vec<Value>],
    weight_values: Option<&[Value]>,
    report_stats: &[String],
    alpha: f64,
    target: &DatasetRef,
) -> Result<()> {
    let n_obs = ds.n_obs();
    let all_rows: Vec<usize> = (0..n_obs).collect();

    // Colonne caractère "Variable" : un nom de variable par ligne.
    let var_names: Vec<Option<String>> = var_cols
        .iter()
        .map(|&ci| Some(ds.vars[ci].name.clone()))
        .collect();
    let name_len = var_cols
        .iter()
        .map(|&ci| ds.vars[ci].name.len())
        .max()
        .unwrap_or(8)
        .max(8);

    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();

    columns.push(Series::new("Variable".into(), var_names).into());
    vars.push(VarMeta {
        name: "Variable".to_string(),
        ty: VarType::Char,
        length: name_len,
        format: None,
        label: None,
    });

    // Une colonne numérique par statistique demandée.
    for stat in report_stats {
        let colname = ods_summary_stat_colname(stat);
        let vals: Vec<Option<f64>> = (0..var_cols.len())
            .map(|vi| {
                let v = match weight_values {
                    Some(wv) => {
                        let (pairs, nmiss) = partition_weighted(&var_values[vi], wv, &all_rows);
                        compute_weighted(stat, &pairs, nmiss, alpha)
                    }
                    None => {
                        let (xs, nmiss) = partition_numeric(&var_values[vi], &all_rows);
                        compute(stat, &xs, nmiss, alpha)
                    }
                };
                value_to_num(&v)
            })
            .collect();
        columns.push(Series::new(colname.as_str().into(), vals).into());
        vars.push(num_var_meta(&colname));
    }

    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars };

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

/// Nom de colonne du dataset Summary pour une statistique du rapport.
/// (StdDev pour `std`/`stddev` ; libellé capitalisé pour les autres.)
pub(super) fn ods_summary_stat_colname(stat: &str) -> String {
    match stat.to_ascii_lowercase().as_str() {
        "n" => "N".to_string(),
        "nmiss" => "NMiss".to_string(),
        "mean" => "Mean".to_string(),
        "std" | "stddev" => "StdDev".to_string(),
        "min" => "Min".to_string(),
        "max" => "Max".to_string(),
        "sum" => "Sum".to_string(),
        "range" => "Range".to_string(),
        "stderr" => "StdErr".to_string(),
        "cv" => "CV".to_string(),
        "median" => "Median".to_string(),
        "clm" => "CLM".to_string(),
        "lclm" => "LowerCLMean".to_string(),
        "uclm" => "UpperCLMean".to_string(),
        // Percentile keywords (M33.3): canonical PNN / QRANGE column names.
        p @ ("p1" | "p5" | "p10" | "p20" | "p25" | "p30" | "p40" | "p50" | "p60" | "p70"
        | "p75" | "p80" | "p90" | "p95" | "p99") => p.to_uppercase(),
        "q1" => "P25".to_string(),
        "q3" => "P75".to_string(),
        "qrange" => "QRange".to_string(),
        other => {
            let mut c = other.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn write_output(
    session: &mut Session,
    ds: &SasDataset,
    class_cols: &[usize],
    class_values: &[Vec<Value>],
    var_values: &[Vec<Value>],
    var_cols: &[usize],
    weight_values: Option<&[Value]>,
    out: &MeansOutput,
    by_cols: &[crate::procs::common::ByCol],
    by_groups_list: &[(Vec<Value>, Vec<usize>)],
    alpha: f64,
    allowed_types: Option<&std::collections::BTreeSet<u64>>,
) -> Result<()> {
    let k = class_cols.len();

    // Resolve each output spec's source VAR to an index into var_cols /
    // var_values (the source column must be a VAR — decode it on demand if
    // not already in the VAR list).
    // Build a name->decoded-column map for the spec sources.
    struct Spec {
        stat: String,
        outname: String,
        col: Vec<Value>,
    }
    let mut specs: Vec<Spec> = Vec::with_capacity(out.specs.len());
    for (stat, srcvar, outname) in &out.specs {
        // Find the source column in the dataset.
        let col_idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(srcvar))
            .ok_or_else(|| {
                SasError::runtime(format!("Variable {} not found.", srcvar.to_uppercase()))
            })?;
        // Reuse already-decoded VAR column if available, else decode.
        let col = match var_cols.iter().position(|&c| c == col_idx) {
            Some(p) => var_values[p].clone(),
            None => decode_column(ds, col_idx)?,
        };
        specs.push(Spec {
            stat: stat.clone(),
            outname: outname.clone(),
            col,
        });
    }

    // Output rows accumulate as: (BY-group index, type,
    // per-class-cell-values, sort key, freq, stat-values).
    struct OutRow {
        by_idx: usize,
        ty: f64,
        // class cell value per class var (active = group key value;
        // inactive = missing of right type).
        class_cells: Vec<Value>,
        // sort key: only the active classes' values (in class order).
        sort_key: Vec<Value>,
        freq: f64,
        stats: Vec<Value>,
    }
    let mut out_rows: Vec<OutRow> = Vec::new();

    let class_refs: Vec<&Vec<Value>> = class_values.iter().collect();

    // One block of CLASS-subset rows per BY group (one group overall if no BY),
    // restricting the analysis to that BY group's rows.
    for (by_idx, (_by_key, by_rows)) in by_groups_list.iter().enumerate() {
        // Enumerate all 2^k CLASS subsets within this BY group.
        for mask in 0u32..(1u32 << k) {
            let active: Vec<usize> = (0..k).filter(|&i| (mask >> i) & 1 == 1).collect();

            // _TYPE_ : LSB corresponds to the LAST class variable.
            let mut ty: u64 = 0;
            for &i in &active {
                ty |= 1u64 << (k - 1 - i);
            }

            // WAYS / TYPES restriction (M33.3): skip _TYPE_ rows not requested.
            // None → no restriction (default path, every _TYPE_ emitted).
            if let Some(set) = allowed_types {
                if !set.contains(&ty) {
                    continue;
                }
            }

            // Group this BY group's rows by the active class variables.
            let active_refs: Vec<&Vec<Value>> = active.iter().map(|&i| class_refs[i]).collect();
            let groups = group_by_keys_subset(&active_refs, by_rows);

            for (active_key, grp_rows) in &groups {
                let mut class_cells: Vec<Value> = Vec::with_capacity(k);
                let mut ai = 0usize;
                for (i, &col_idx) in class_cols.iter().enumerate() {
                    if active.contains(&i) {
                        class_cells.push(active_key[ai].clone());
                        ai += 1;
                    } else {
                        match ds.vars[col_idx].ty {
                            VarType::Num => class_cells.push(Value::missing()),
                            VarType::Char => class_cells.push(Value::Char(String::new())),
                        }
                    }
                }

                let freq = grp_rows.len() as f64;

                let mut stat_vals: Vec<Value> = Vec::with_capacity(specs.len());
                for sp in &specs {
                    match weight_values {
                        Some(wv) => {
                            let (pairs, nmiss) = partition_weighted(&sp.col, wv, grp_rows);
                            stat_vals.push(compute_weighted(&sp.stat, &pairs, nmiss, alpha));
                        }
                        None => {
                            let (xs, nmiss) = partition_numeric(&sp.col, grp_rows);
                            stat_vals.push(compute(&sp.stat, &xs, nmiss, alpha));
                        }
                    }
                }

                out_rows.push(OutRow {
                    by_idx,
                    ty: ty as f64,
                    class_cells,
                    sort_key: active_key.clone(),
                    freq,
                    stats: stat_vals,
                });
            }
        }
    }

    // Order rows: BY group order (outer, preserved), then _TYPE_ ascending,
    // then active class-value tuple via sas_cmp.
    out_rows.sort_by(|a, b| {
        match a.by_idx.cmp(&b.by_idx) {
            Ordering::Equal => {}
            other => return other,
        }
        match a.ty.partial_cmp(&b.ty).unwrap_or(Ordering::Equal) {
            Ordering::Equal => {}
            other => return other,
        }
        for (x, y) in a.sort_key.iter().zip(&b.sort_key) {
            let c = x.sas_cmp(y);
            if c != Ordering::Equal {
                return c;
            }
        }
        Ordering::Equal
    });

    // Build the output DataFrame column-by-column.
    let n_rows = out_rows.len();
    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();

    // BY columns first (copy input VarMeta; values from the BY-group key).
    for (bi, bc) in by_cols.iter().enumerate() {
        let meta = &ds.vars[bc.col_idx];
        let series = match meta.ty {
            VarType::Num => {
                let vals: Vec<Option<f64>> = out_rows
                    .iter()
                    .map(|r| value_to_num(&by_groups_list[r.by_idx].0[bi]))
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
            VarType::Char => {
                let vals: Vec<Option<String>> = out_rows
                    .iter()
                    .map(|r| match &by_groups_list[r.by_idx].0[bi] {
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

    // CLASS columns (copy input VarMeta; encode per-row values).
    for (ci, &col_idx) in class_cols.iter().enumerate() {
        let meta = &ds.vars[col_idx];
        let series = match meta.ty {
            VarType::Num => {
                let vals: Vec<Option<f64>> = out_rows
                    .iter()
                    .map(|r| value_to_num(&r.class_cells[ci]))
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
            VarType::Char => {
                let vals: Vec<Option<String>> = out_rows
                    .iter()
                    .map(|r| match &r.class_cells[ci] {
                        Value::Char(s) if s.is_empty() => None,
                        Value::Char(s) => Some(s.clone()),
                        Value::Missing(_) => None,
                        Value::Num(_) => None,
                    })
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
        };
        columns.push(series.into());
        vars.push(meta.clone());
    }

    // _TYPE_
    let type_vals: Vec<Option<f64>> = out_rows.iter().map(|r| Some(r.ty)).collect();
    columns.push(Series::new("_TYPE_".into(), type_vals).into());
    vars.push(num_var_meta("_TYPE_"));

    // _FREQ_
    let freq_vals: Vec<Option<f64>> = out_rows.iter().map(|r| Some(r.freq)).collect();
    columns.push(Series::new("_FREQ_".into(), freq_vals).into());
    vars.push(num_var_meta("_FREQ_"));

    // One column per output spec.
    for (si, sp) in specs.iter().enumerate() {
        let vals: Vec<Option<f64>> = out_rows
            .iter()
            .map(|r| value_to_num(&r.stats[si]))
            .collect();
        columns.push(Series::new(sp.outname.as_str().into(), vals).into());
        vars.push(num_var_meta(&sp.outname));
    }

    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars };

    let out_libref = out.out.libref_or_work();
    let out_table = out.out.name.to_uppercase();
    let display = format!("{out_libref}.{out_table}");
    let n_vars = out_ds.vars.len();

    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(display.clone());

    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_rows, n_vars
    ));

    Ok(())
}

pub(super) fn num_var_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}
