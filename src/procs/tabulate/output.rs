use super::*;

/// One row of the OUT= cell dataset.
pub(super) struct OutCell {
    /// Per-CLASS-variable cell value (level value when active, else missing).
    pub(super) class_cells: Vec<Value>,
    /// `_TYPE_` 0/1 pattern over the CLASS variables (1 = active in this cell).
    pub(super) type_pattern: String,
    pub(super) page_no: f64,
    /// (stat-column name, value) for each computed statistic in the cell.
    pub(super) stats: Vec<(String, Value)>,
}

/// Build and write the OUT= cell dataset (M33.4). See the file header for the
/// chosen naming convention. One observation per rendered cell.
#[allow(clippy::too_many_arguments)]
pub(super) fn write_out_dataset(
    session: &mut Session,
    ds: &SasDataset,
    class_cols: &[(String, usize)],
    var_values: &[(usize, Vec<Value>)],
    class_values: &[(usize, Vec<Value>)],
    page_cells: &[Option<Cell>],
    row_cells: &[Cell],
    col_cells: &[Cell],
    n_obs: usize,
    out: &DatasetRef,
) -> Result<()> {
    let mut out_rows: Vec<OutCell> = Vec::new();

    for (page_idx, page) in page_cells.iter().enumerate() {
        let page_atoms: &[Atom] = match page {
            Some(pc) => &pc.atoms,
            None => &[],
        };
        for rc in row_cells {
            for cc in col_cells {
                let merged: Vec<Atom> = page_atoms
                    .iter()
                    .chain(rc.atoms.iter())
                    .chain(cc.atoms.iter())
                    .cloned()
                    .collect();
                let res = compute_cell_value(&merged, var_values, class_values, n_obs)?;

                // CLASS cell values + _TYPE_ pattern: a CLASS var is "active"
                // when a ClassLevel atom binds it in this cell.
                let mut class_cells: Vec<Value> = Vec::with_capacity(class_cols.len());
                let mut pattern = String::with_capacity(class_cols.len());
                for (_, ci) in class_cols {
                    let bound = merged.iter().find_map(|a| match a {
                        Atom::ClassLevel { col, level, .. } if col == ci => Some(level.clone()),
                        _ => None,
                    });
                    match bound {
                        Some(level) => {
                            class_cells.push(level);
                            pattern.push('1');
                        }
                        None => {
                            let missing = match ds.vars[*ci].ty {
                                VarType::Num => Value::missing(),
                                VarType::Char => Value::Char(String::new()),
                            };
                            class_cells.push(missing);
                            pattern.push('0');
                        }
                    }
                }

                // The cell's analysis VAR (if any) for stat-column naming.
                let var_name = merged.iter().find_map(|a| match a {
                    Atom::Var { col, .. } => Some(ds.vars[*col].name.clone()),
                    _ => None,
                });
                let stat_label = tab_stat_header(&res.stat);
                let colname = match &var_name {
                    Some(v) => format!("{v}_{stat_label}"),
                    None => stat_label.to_string(),
                };

                out_rows.push(OutCell {
                    class_cells,
                    type_pattern: pattern,
                    page_no: (page_idx + 1) as f64,
                    stats: vec![(colname, res.value)],
                });
            }
        }
    }

    // Build the DataFrame column-by-column.
    let n_rows = out_rows.len();
    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();

    // CLASS columns (copy input VarMeta; encode per-row values).
    for (ci, (_, col_idx)) in class_cols.iter().enumerate() {
        let meta = &ds.vars[*col_idx];
        let series = match meta.ty {
            VarType::Num => {
                let vals: Vec<Option<f64>> =
                    out_rows.iter().map(|r| value_to_num(&r.class_cells[ci])).collect();
                Series::new(meta.name.as_str().into(), vals)
            }
            VarType::Char => {
                let vals: Vec<Option<String>> = out_rows
                    .iter()
                    .map(|r| match &r.class_cells[ci] {
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

    // _TYPE_ (char 0/1 pattern).
    let type_len = class_cols.len().max(1);
    let type_vals: Vec<Option<String>> =
        out_rows.iter().map(|r| Some(r.type_pattern.clone())).collect();
    columns.push(Series::new("_TYPE_".into(), type_vals).into());
    vars.push(VarMeta {
        name: "_TYPE_".to_string(),
        ty: VarType::Char,
        length: type_len,
        format: None,
        label: None,
    });

    // _PAGE_ and _TABLE_.
    let page_vals: Vec<Option<f64>> = out_rows.iter().map(|r| Some(r.page_no)).collect();
    columns.push(Series::new("_PAGE_".into(), page_vals).into());
    vars.push(out_num_meta("_PAGE_"));
    let table_vals: Vec<Option<f64>> = out_rows.iter().map(|_| Some(1.0)).collect();
    columns.push(Series::new("_TABLE_".into(), table_vals).into());
    vars.push(out_num_meta("_TABLE_"));

    // One column per distinct stat-column name, in first-seen order. A row that
    // does not produce a given stat column gets a missing value there.
    let mut stat_names: Vec<String> = Vec::new();
    for r in &out_rows {
        for (name, _) in &r.stats {
            if !stat_names.iter().any(|n| n == name) {
                stat_names.push(name.clone());
            }
        }
    }
    for name in &stat_names {
        let vals: Vec<Option<f64>> = out_rows
            .iter()
            .map(|r| {
                r.stats
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, v)| value_to_num(v))
                    .unwrap_or(None)
            })
            .collect();
        columns.push(Series::new(name.as_str().into(), vals).into());
        vars.push(out_num_meta(name));
    }

    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars };

    let out_libref = out.libref_or_work();
    let out_table = out.name.to_uppercase();
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

pub(super) fn out_num_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}

/// Best-effort name of the page dimension for the page-label line: the first
/// CLASS variable that appears in the page expression, else "Page".
pub(super) fn page_dim_name(ast: &TabulateAst, ds: &SasDataset) -> String {
    if let Some(p) = &ast.page {
        if let Some(name) = first_class_name(p, ds) {
            return name;
        }
    }
    "Page".to_string()
}

pub(super) fn first_class_name(dim: &DimExpr, ds: &SasDataset) -> Option<String> {
    for term in &dim.terms {
        for factor in &term.factors {
            match factor {
                Factor::Name { name, .. } => {
                    if let Some(m) = ds.vars.iter().find(|m| m.name.eq_ignore_ascii_case(name)) {
                        return Some(m.name.clone());
                    }
                }
                Factor::Group(inner) => {
                    if let Some(n) = first_class_name(inner, ds) {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}
