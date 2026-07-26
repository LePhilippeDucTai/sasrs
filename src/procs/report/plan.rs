use super::*;

/// Resolved per-column plan entry: index into the dataset, effective usage,
/// order direction, and the header text to display.
pub(super) struct ColPlan {
    pub(super) idx: usize,
    pub(super) usage: Usage,
    pub(super) dir: OrderDir,
    pub(super) header: String,
    /// `format=<fmt>` for displayed values (M33.5); `None` → default rendering.
    pub(super) format: Option<String>,
    /// `width=<n>` display width (M33.5); `None` → aligner-derived width.
    pub(super) width: Option<usize>,
    /// `spacing=<n>` blank spaces before this column (M33.5); `None` → default.
    pub(super) spacing: Option<usize>,
}

/// Resolve the column list (display order) and build the per-column plan,
/// applying DEFINEs and type defaults.
pub(super) fn build_col_plan(
    ast: &ReportAst,
    ds: &crate::dataset::SasDataset,
) -> Result<Vec<ColPlan>> {
    let col_names: Vec<String> = match &ast.columns {
        Some(list) => list.clone(),
        None => ds.vars.iter().map(|m| m.name.clone()).collect(),
    };

    let mut plan: Vec<ColPlan> = Vec::with_capacity(col_names.len());
    for cname in &col_names {
        let def = ast
            .defines
            .iter()
            .find(|d| d.var.eq_ignore_ascii_case(cname));

        // COMPUTED columns have no underlying dataset variable.
        if matches!(def.map(|d| &d.usage), Some(Usage::Computed)) {
            plan.push(ColPlan {
                idx: usize::MAX,
                usage: Usage::Computed,
                dir: def.map(|d| d.order).unwrap_or(OrderDir::Ascending),
                header: def
                    .and_then(|d| d.label.clone())
                    .unwrap_or_else(|| cname.clone()),
                format: def.and_then(|d| d.format.clone()),
                width: def.and_then(|d| d.width),
                spacing: def.and_then(|d| d.spacing),
            });
            continue;
        }

        let idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(cname))
            .ok_or_else(|| {
                SasError::runtime(format!("Variable {} not found.", cname.to_uppercase()))
            })?;

        let usage = match def {
            Some(d) => d.usage.clone(),
            None => match ds.vars[idx].ty {
                VarType::Num => Usage::Analysis("sum".to_string()),
                VarType::Char => Usage::Display,
            },
        };
        let dir = def.map(|d| d.order).unwrap_or(OrderDir::Ascending);
        let header = match def.and_then(|d| d.label.clone()) {
            Some(lbl) => lbl,
            None => ds.vars[idx].name.clone(),
        };

        plan.push(ColPlan {
            idx,
            usage,
            dir,
            header,
            format: def.and_then(|d| d.format.clone()),
            width: def.and_then(|d| d.width),
            spacing: def.and_then(|d| d.spacing),
        });
    }
    Ok(plan)
}

/// Decode every planned column once (COMPUTED columns decode to all-missing),
/// apply the WHERE predicate, and project the columns onto the surviving rows
/// so downstream code indexes 0..n_obs contiguously.
pub(super) fn decode_and_filter(
    ast: &ReportAst,
    ds: &crate::dataset::SasDataset,
    plan: &[ColPlan],
    n_obs_total: usize,
) -> Result<(Vec<Vec<Value>>, usize)> {
    let decoded_all: Vec<Vec<Value>> = plan
        .iter()
        .map(|c| {
            if c.idx == usize::MAX {
                Ok(vec![Value::missing(); n_obs_total])
            } else {
                decode_column(ds, c.idx)
            }
        })
        .collect::<Result<_>>()?;

    // WHERE: build the surviving-rows index.
    let live_rows: Vec<usize> = if let Some(cond) = &ast.where_ {
        // Build a name→decoded-column lookup over ALL dataset variables (not
        // just the planned columns) so the predicate can reference any var.
        let where_cols = decode_named_columns(ds)?;
        (0..n_obs_total)
            .filter(|&r| {
                let v = eval_row_expr(cond, &where_cols, r);
                v.truthy()
            })
            .collect()
    } else {
        (0..n_obs_total).collect()
    };
    let n_obs = live_rows.len();

    let decoded: Vec<Vec<Value>> = decoded_all
        .iter()
        .map(|col| live_rows.iter().map(|&r| col[r].clone()).collect())
        .collect();
    Ok((decoded, n_obs))
}

/// Headers + per-column alignments for the listing.
pub(super) fn build_headers(
    plan: &[ColPlan],
    ds: &crate::dataset::SasDataset,
) -> (Vec<String>, Vec<Align>) {
    let headers: Vec<String> = plan.iter().map(|c| c.header.clone()).collect();
    let aligns: Vec<Align> = plan
        .iter()
        .map(|c| match c.usage {
            Usage::Analysis(_) => Align::Right,
            Usage::Computed => Align::Right,
            _ if c.idx == usize::MAX => Align::Left,
            _ => match ds.vars[c.idx].ty {
                VarType::Num => Align::Right,
                VarType::Char => Align::Left,
            },
        })
        .collect();
    (headers, aligns)
}
