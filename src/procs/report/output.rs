use super::*;

/// Write the report's detail/group/break rows (excluding the RBREAK grand
/// total) as an OUT= data set. One output column per COLUMN entry; the SAS type
/// follows the input variable (ANALYSIS/GROUP/ORDER numeric vars stay numeric,
/// char DISPLAY vars stay char). COMPUTED columns are numeric.
pub(super) fn write_out_dataset(
    session: &mut Session,
    out_ref: &DatasetRef,
    plan: &[ColPlan],
    ds: &crate::dataset::SasDataset,
    rows: &[RowOut],
) -> Result<()> {
    use crate::dataset::{SasDataset, VarMeta};

    // OUT= captures the report body rows (detail, group, and BREAK sub-totals);
    // the RBREAK grand total is a presentation-only line and is excluded.
    let body: Vec<&RowOut> = rows.iter().filter(|r| r.kind != RowKind::Rbreak).collect();

    let mut columns: Vec<Column> = Vec::with_capacity(plan.len());
    let mut vars: Vec<VarMeta> = Vec::with_capacity(plan.len());

    for (ci, c) in plan.iter().enumerate() {
        // Decide the output type for this column.
        let is_char = match &c.usage {
            Usage::Analysis(_) | Usage::Computed => false,
            _ => c.idx != usize::MAX && ds.vars[c.idx].ty == VarType::Char,
        };
        let name = if c.idx == usize::MAX {
            c.header.clone()
        } else {
            ds.vars[c.idx].name.clone()
        };
        if is_char {
            let vals: Vec<Option<String>> =
                body.iter().map(|r| value_to_char_cell(&r.vals[ci])).collect();
            let len = vals
                .iter()
                .flatten()
                .map(|s| s.len())
                .max()
                .unwrap_or(8)
                .max(1);
            columns.push(Series::new(name.as_str().into(), vals).into());
            vars.push(VarMeta {
                name,
                ty: VarType::Char,
                length: len,
                format: None,
                label: None,
            });
        } else {
            let vals: Vec<Option<f64>> =
                body.iter().map(|r| value_to_num(&r.vals[ci])).collect();
            columns.push(Series::new(name.as_str().into(), vals).into());
            vars.push(VarMeta {
                name,
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            });
        }
    }

    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars };

    let out_libref = out_ref.libref_or_work();
    let out_table = out_ref.name.to_uppercase();
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
