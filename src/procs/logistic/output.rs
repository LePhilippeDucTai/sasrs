use super::*;

// ───────────────────────── CLASS / OUTPUT helpers ─────────────────────────

/// Print "Class Level Information" (one row per CLASS variable, listing levels
/// in `sas_cmp` order). No-op when no CLASS variable is present, so the binary
/// no-CLASS listing is byte-identical to the pre-M34.6 output.
pub(super) fn write_class_level_info(session: &mut Session, design: &Design) {
    let has_class = design.effects.iter().any(|e| e.is_class);
    if !has_class {
        return;
    }
    centered(session, "Class Level Information");
    session.listing.blank();

    let headers: Vec<String> = vec!["Class".into(), "Value".into()];
    let aligns = vec![Align::Left, Align::Left];
    let mut rows: Vec<Vec<String>> = Vec::new();
    for eff in &design.effects {
        if !eff.is_class {
            continue;
        }
        // Reconstruct the full ordered level list: non-reference levels then ref.
        let mut labels: Vec<String> = eff.levels.iter().map(value_label).collect();
        labels.push(eff.ref_label.clone());
        rows.push(vec![eff.name.clone(), labels.join(" ")]);
    }
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
}

/// Write OUTPUT OUT= datasets: complete-case input rows plus the requested
/// predicted-probability and/or XBETA columns. Mirrors `reg.rs::write_outputs`
/// (creation NOTE + `_LAST_`).
pub(super) fn write_outputs(
    outputs: &[LogisticOutput],
    ds: &SasDataset,
    complete_mask: &[bool],
    predicted: &[f64],
    xbeta: &[f64],
    session: &mut Session,
) -> Result<()> {
    if outputs.is_empty() {
        return Ok(());
    }

    let complete_indices: Vec<usize> = complete_mask
        .iter()
        .enumerate()
        .filter_map(|(i, &c)| if c { Some(i) } else { None })
        .collect();

    for out_spec in outputs {
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
                            Value::Char(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect();
                    columns.push(Series::new(ds.vars[col_idx].name.as_str().into(), data).into());
                }
            }
            out_vars.push(ds.vars[col_idx].clone());
        }

        if let Some(pred_name) = &out_spec.predicted {
            let data: Vec<Option<f64>> = predicted.iter().map(|&v| Some(v)).collect();
            columns.push(Series::new(pred_name.as_str().into(), data).into());
            out_vars.push(num_var_meta(pred_name));
        }
        if let Some(xb_name) = &out_spec.xbeta {
            let data: Vec<Option<f64>> = xbeta.iter().map(|&v| Some(v)).collect();
            columns.push(Series::new(xb_name.as_str().into(), data).into());
            out_vars.push(num_var_meta(xb_name));
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
