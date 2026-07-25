use super::*;

/// Pre-rendered report state shared by the plain and BY-group renderers.
pub(crate) struct RenderCtx<'a> {
    pub(super) headers: Vec<String>,
    pub(super) aligns: Vec<Align>,
    pub(super) id_cells: Vec<Vec<String>>,
    pub(super) col_cells: Vec<Vec<String>>,
    pub(super) sum_indices: &'a [usize],
    pub(super) sum_values: Vec<Vec<Value>>,
    pub(super) sum_render_pos: Vec<Option<usize>>,
    /// Render the leading `Obs` column (no NOOBS and no ID).
    pub(super) obs_col: bool,
    pub(super) double: bool,
    pub(super) n_flag: bool,
}

impl RenderCtx<'_> {
    /// Build one rendered row for input row `row_i`.
    pub(super) fn build_row(&self, row_i: usize) -> Vec<String> {
        let mut row: Vec<String> = Vec::with_capacity(self.headers.len());
        for cells in &self.id_cells {
            row.push(cells[row_i].clone());
        }
        if self.obs_col {
            row.push((row_i + 1).to_string());
        }
        for cells in &self.col_cells {
            row.push(cells[row_i].clone());
        }
        row
    }

    /// Build a totals row over a set of input rows. Returns None when there
    /// are no SUM variables. The total of a column is the SAS SUM (missing
    /// values ignored; all-missing → missing `.`).
    pub(super) fn build_totals(&self, rows: &[usize]) -> Option<Vec<String>> {
        if self.sum_indices.is_empty() {
            return None;
        }
        let mut out = vec![String::new(); self.headers.len()];
        for (k, vals) in self.sum_values.iter().enumerate() {
            let mut acc = 0.0_f64;
            let mut any = false;
            for &r in rows {
                if let Value::Num(f) = vals[r] {
                    acc += f;
                    any = true;
                }
            }
            let cell = if any {
                format_best(acc, 12)
            } else {
                ".".to_string()
            };
            if let Some(pos) = self.sum_render_pos[k] {
                out[pos] = cell;
            }
        }
        Some(out)
    }
}

/// No BY: single section over all rows.
pub(crate) fn render_plain(session: &mut Session, ctx: &RenderCtx<'_>, n_obs: usize) {
    let rows: Vec<Vec<String>> = (0..n_obs).map(|r| ctx.build_row(r)).collect();
    let all: Vec<usize> = (0..n_obs).collect();
    let totals = ctx.build_totals(&all);
    session
        .listing
        .write_table_ext(&ctx.headers, &ctx.aligns, &rows, ctx.double, totals.as_ref());
    if ctx.n_flag {
        session.listing.blank();
        session.listing.write_line(&format!("N = {}", n_obs));
    }
}

/// BY: verify sortedness then render one section per contiguous group, plus
/// the grand-total line when SUM spans several groups.
pub(crate) fn render_by_groups(
    session: &mut Session,
    ctx: &RenderCtx<'_>,
    ds: &crate::dataset::SasDataset,
    by_cols: &[common::ByCol],
    n_obs: usize,
    display_name: &str,
) -> Result<()> {
    let by_values: Vec<Vec<Value>> = by_cols
        .iter()
        .map(|bc| common::decode_column(ds, bc.col_idx))
        .collect::<Result<_>>()?;
    let descending: Vec<bool> = by_cols.iter().map(|b| b.descending).collect();
    let by_names: Vec<String> = by_cols.iter().map(|b| b.name.clone()).collect();
    let groups = common::by_groups(&by_values, &descending, n_obs, &by_names, display_name)?;

    let multi = groups.len() > 1;
    for (gi, (key, rows_idx)) in groups.iter().enumerate() {
        if gi > 0 {
            session.listing.blank();
        }
        // Standard BY heading line: "var1=val1 var2=val2".
        let heading = by_cols
            .iter()
            .zip(key)
            .map(|(bc, v)| format!("{}={}", bc.name, by_value_str(v)))
            .collect::<Vec<_>>()
            .join(" ");
        session.listing.write_line(&heading);
        session.listing.blank();

        let rows: Vec<Vec<String>> = rows_idx.iter().map(|&r| ctx.build_row(r)).collect();
        let totals = ctx.build_totals(rows_idx);
        session
            .listing
            .write_table_ext(&ctx.headers, &ctx.aligns, &rows, ctx.double, totals.as_ref());
        if ctx.n_flag {
            session.listing.blank();
            session.listing.write_line(&format!("N = {}", rows_idx.len()));
        }
    }

    // Grand total across all observations when SUM + more than one group.
    // SAS renders this aligned under the columns; to avoid replicating the
    // whole-report column-width computation across heterogeneous BY groups,
    // we emit it as an explicit labelled line "var=total ..." (documented
    // simplification — values are SAS-exact, the placement is textual).
    if !ctx.sum_indices.is_empty() && multi {
        let all: Vec<usize> = (0..n_obs).collect();
        let parts: Vec<String> = ctx
            .sum_indices
            .iter()
            .enumerate()
            .map(|(k, &si)| {
                let mut acc = 0.0_f64;
                let mut any = false;
                for &r in &all {
                    if let Value::Num(f) = ctx.sum_values[k][r] {
                        acc += f;
                        any = true;
                    }
                }
                let cell = if any { format_best(acc, 12) } else { ".".to_string() };
                format!("{}={}", ds.vars[si].name, cell)
            })
            .collect();
        session.listing.blank();
        session
            .listing
            .write_line(&format!("Grand total: {}", parts.join(" ")));
    }
    Ok(())
}

/// Resolve a list of variable names to dataset column indices, erroring with
/// the SAS "Variable XXXX not found." message on the first unknown name.
pub(crate) fn resolve_names(ds: &crate::dataset::SasDataset, names: &[String]) -> Result<Vec<usize>> {
    let mut idxs = Vec::with_capacity(names.len());
    for vname in names {
        match ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(vname))
        {
            Some(i) => idxs.push(i),
            None => {
                return Err(SasError::runtime(format!(
                    "Variable {} not found.",
                    vname.to_uppercase()
                )));
            }
        }
    }
    Ok(idxs)
}

/// Render a BY-key value for the BY heading line ("var=value").
pub(crate) fn by_value_str(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Missing(k) => k.display(),
        Value::Char(s) => s.trim_end().to_string(),
    }
}
