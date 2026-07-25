use super::*;

/// OUT= accumulation: rows where differences occur.
/// _TYPE_: "BASE" / "COMPARE" / "DIF" rows for each obs with diffs.
pub(super) struct OutRow {
    pub(super) obs: usize,
    pub(super) row_type: &'static str,
    pub(super) values: Vec<Option<Value>>, // one per common-var with type match
}

/// Write the OUT= differences dataset: `_TYPE_`, `_OBS_` plus the type-matched
/// common variables (BASE/COMPARE/DIF rows per differing obs). With no
/// differences, an empty dataset with just `_TYPE_`/`_OBS_` is created.
pub(super) fn write_out_dataset(
    session: &mut Session,
    out_ref: &DatasetRef,
    out_rows: &[OutRow],
    matching_vars: &[&CommonVar],
    base_ds: &SasDataset,
) -> Result<()> {
    if !out_rows.is_empty() && !matching_vars.is_empty() {
        let out_libref = out_ref.libref_or_work();
        let out_name = out_ref.name.to_uppercase();

        // Build DataFrame for OUT= dataset
        // Columns: _TYPE_, _OBS_, <common vars matching type>
        let n_rows = out_rows.len();

        let type_col: StringChunked = out_rows
            .iter()
            .map(|r| Some(r.row_type))
            .collect();
        let obs_col: Float64Chunked = out_rows
            .iter()
            .map(|r| Some(r.obs as f64))
            .collect();

        let mut columns: Vec<Column> = vec![
            Series::new("_TYPE_".into(), type_col).into(),
            Series::new("_OBS_".into(), obs_col).into(),
        ];

        let mut vars: Vec<VarMeta> = vec![
            VarMeta {
                name: "_TYPE_".to_string(),
                ty: VarType::Char,
                length: 7,
                format: None,
                label: None,
            },
            VarMeta {
                name: "_OBS_".to_string(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            },
        ];

        for (vi, mv) in matching_vars.iter().enumerate() {
            let base_meta = &base_ds.vars[mv.base_idx];
            match mv.base_type {
                VarType::Num => {
                    let col_vals: Float64Chunked = out_rows
                        .iter()
                        .map(|r| {
                            r.values[vi].as_ref().and_then(|v| match v {
                                Value::Num(f) => Some(*f),
                                _ => None,
                            })
                        })
                        .collect();
                    columns.push(
                        Series::new(mv.name.as_str().into(), col_vals).into(),
                    );
                }
                VarType::Char => {
                    let col_vals: StringChunked = out_rows
                        .iter()
                        .map(|r| {
                            r.values[vi].as_ref().and_then(|v| match v {
                                Value::Char(s) => Some(s.as_str()),
                                _ => None,
                            })
                        })
                        .collect();
                    columns.push(
                        Series::new(mv.name.as_str().into(), col_vals).into(),
                    );
                }
            }
            vars.push(VarMeta {
                name: mv.name.clone(),
                ty: mv.base_type,
                length: base_meta.length,
                format: base_meta.format.clone(),
                label: base_meta.label.clone(),
            });
        }

        let df = DataFrame::new(columns)
            .map_err(|e| SasError::runtime(format!("COMPARE OUT= build error: {e}")))?;
        let out_ds = SasDataset { df, vars };
        let out_provider = session.libs.get(&out_libref)?;
        out_provider.write(&out_name, &out_ds)?;
        session.log.note(&format!(
            "Output data set: {}.{} ({} observations).",
            out_libref,
            out_name,
            n_rows
        ));
        session.last_dataset = Some(format!("{}.{}", out_libref, out_name));
    } else if out_rows.is_empty() {
        // No diffs — create empty OUT= dataset with just _TYPE_, _OBS_
        let type_col: StringChunked = std::iter::empty::<Option<&str>>().collect();
        let obs_col: Float64Chunked = std::iter::empty::<Option<f64>>().collect();
        let columns: Vec<Column> = vec![
            Series::new("_TYPE_".into(), type_col).into(),
            Series::new("_OBS_".into(), obs_col).into(),
        ];
        let vars = vec![
            VarMeta {
                name: "_TYPE_".to_string(),
                ty: VarType::Char,
                length: 7,
                format: None,
                label: None,
            },
            VarMeta {
                name: "_OBS_".to_string(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            },
        ];
        let df = DataFrame::new(columns)
            .map_err(|e| SasError::runtime(format!("COMPARE OUT= build error: {e}")))?;
        let out_ds = SasDataset { df, vars };
        let out_libref = out_ref.libref_or_work();
        let out_name = out_ref.name.to_uppercase();
        let out_provider = session.libs.get(&out_libref)?;
        out_provider.write(&out_name, &out_ds)?;
        session.log.note(&format!(
            "Output data set: {}.{} (0 observations).",
            out_libref, out_name
        ));
        session.last_dataset = Some(format!("{}.{}", out_libref, out_name));
    }
    Ok(())
}
