use super::*;

/// Emit the SAS BY heading line into the listing: `var1=val1 var2=val2`.
pub(super) fn emit_by_heading(session: &mut Session, by_names: &[String], by_key: &[Value]) {
    let parts: Vec<String> = by_names
        .iter()
        .zip(by_key)
        .map(|(name, v)| {
            let cell = match v {
                Value::Num(f) => format_best(*f, 12),
                Value::Missing(k) => k.display(),
                Value::Char(s) => s.trim_end().to_string(),
            };
            format!("{}={}", name, cell)
        })
        .collect();
    session.listing.write_line(&parts.join(" "));
    session.listing.blank();
}

/// Compute one OUTPUT statistic for a single variable over the group's
/// non-missing values `xs` (sorted in `sorted`), the missing count, and the
/// total row count. Returns `None` (→ SAS missing) when undefined.
pub(super) fn output_stat(stat: &str, xs: &[f64], sorted: &[f64], n_missing: usize) -> Option<f64> {
    let n = xs.len();
    let mean = if n > 0 {
        Some(xs.iter().sum::<f64>() / n as f64)
    } else {
        None
    };
    match stat {
        "n" => Some(n as f64),
        "nmiss" => Some(n_missing as f64),
        "sum" => Some(xs.iter().sum()),
        "mean" => mean,
        "std" | "stddev" => sample_std(xs),
        "var" => sample_std(xs).map(|s| s * s),
        "min" | "p0" => sorted.first().copied(),
        "max" | "p100" => sorted.last().copied(),
        "median" | "p50" => quantile_def5(sorted, 0.50),
        "q1" | "p25" => quantile_def5(sorted, 0.25),
        "q3" | "p75" => quantile_def5(sorted, 0.75),
        "p1" => quantile_def5(sorted, 0.01),
        "p5" => quantile_def5(sorted, 0.05),
        "p10" => quantile_def5(sorted, 0.10),
        "p90" => quantile_def5(sorted, 0.90),
        "p95" => quantile_def5(sorted, 0.95),
        "p99" => quantile_def5(sorted, 0.99),
        "range" => {
            if n > 0 {
                Some(sorted[n - 1] - sorted[0])
            } else {
                None
            }
        }
        "qrange" => match (quantile_def5(sorted, 0.75), quantile_def5(sorted, 0.25)) {
            (Some(a), Some(b)) => Some(a - b),
            _ => None,
        },
        _ => None,
    }
}

/// Build and write the OUTPUT OUT= dataset: one row per BY group (one overall
/// when no BY), with BY variables followed by the requested statistic columns
/// (each statistic keyword paired positionally with the VAR list).
#[allow(clippy::too_many_arguments)]
pub(super) fn write_output(
    session: &mut Session,
    ds: &SasDataset,
    var_cols: &[usize],
    var_values: &[Vec<Value>],
    out: &UnivariateOutput,
    by_cols: &[crate::procs::common::ByCol],
    by_groups_list: &[(Vec<Value>, Vec<usize>)],
) -> Result<()> {
    // Validate: each spec must not request more output names than there are
    // analysis variables (positional pairing with VAR list).
    for (stat, names) in &out.specs {
        if names.len() > var_cols.len() {
            return Err(SasError::runtime(format!(
                "The OUTPUT statement requests {} names for statistic {} but only {} \
                 analysis variable(s) are available.",
                names.len(),
                stat.to_uppercase(),
                var_cols.len()
            )));
        }
    }

    let n_rows = by_groups_list.len();
    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();

    // BY columns first (one value per BY group).
    for (bi, bc) in by_cols.iter().enumerate() {
        let meta = &ds.vars[bc.col_idx];
        let series = match meta.ty {
            VarType::Num => {
                let vals: Vec<Option<f64>> = by_groups_list
                    .iter()
                    .map(|(key, _)| value_to_num(&key[bi]))
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
            VarType::Char => {
                let vals: Vec<Option<String>> = by_groups_list
                    .iter()
                    .map(|(key, _)| match &key[bi] {
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

    // Precompute, per BY group, the (xs, sorted, n_missing) per analysis var.
    struct VarStats {
        xs: Vec<f64>,
        sorted: Vec<f64>,
        n_missing: usize,
    }
    let mut per_group: Vec<Vec<VarStats>> = Vec::with_capacity(by_groups_list.len());
    for (_key, grp_rows) in by_groups_list {
        let mut per_var: Vec<VarStats> = Vec::with_capacity(var_cols.len());
        for vv in var_values.iter() {
            let mut xs: Vec<f64> = Vec::with_capacity(grp_rows.len());
            let mut n_missing = 0usize;
            for &row in grp_rows {
                match value_to_num(&vv[row]) {
                    Some(f) if !f.is_nan() => xs.push(f),
                    _ => n_missing += 1,
                }
            }
            let mut sorted = xs.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            per_var.push(VarStats {
                xs,
                sorted,
                n_missing,
            });
        }
        per_group.push(per_var);
    }

    // One statistic column per (spec, paired analysis variable).
    for (stat, names) in &out.specs {
        for (vi, outname) in names.iter().enumerate() {
            let vals: Vec<Option<f64>> = per_group
                .iter()
                .map(|pv| {
                    let vs = &pv[vi];
                    output_stat(stat, &vs.xs, &vs.sorted, vs.n_missing)
                })
                .collect();
            columns.push(Series::new(outname.as_str().into(), vals).into());
            vars.push(num_var_meta(outname));
        }
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
