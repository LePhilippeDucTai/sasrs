//! Datasets de sortie (OUTEST=, OUTSSCP=, OUTPUT OUT=) et tracés PLOTS=/PLOT.

use super::*;

mod outest;
mod plots;

pub(super) use outest::*;
pub(super) use plots::*;

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
        Some(compute_obs_stats(
            x_mat,
            &reconstruct_y(fit),
            fit,
            n,
            p_eff,
            alpha,
            weighting,
        ))
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
        Some(compute_influence_stats(
            x_mat,
            &reconstruct_y(fit),
            fit,
            n,
            p_eff,
            weighting,
        ))
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
                            if v.is_finite() { Some(v) } else { None }
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
                            if v.is_finite() { Some(v) } else { None }
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
