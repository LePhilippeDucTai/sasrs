use super::*;

/// Build the header/stub label for an expanded cell: components joined by "*".
///
/// Header-text precedence per component (M33.4): explicit `='label'` overrides
/// everything; otherwise a VAR atom falls back to its stored VarMeta LABEL,
/// then to the raw name. A STAT atom falls back to its stat header. A CLASS
/// level always renders the level value (the flat model has no variable-name
/// slot for class levels), so its label/stored-label are accepted but do not
/// change the level text — documented simplification. Default (no label, no
/// stored label) stays byte-identical.
pub(super) fn cell_label(cell: &Cell, ds: &SasDataset) -> String {
    if cell.atoms.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = cell
        .atoms
        .iter()
        .map(|a| match a {
            Atom::ClassLevel { level, .. } => level_label(level),
            Atom::Var { col, label, .. } => match label {
                Some(l) => l.clone(),
                None => match &ds.vars[*col].label {
                    Some(l) if !l.is_empty() => l.clone(),
                    _ => ds.vars[*col].name.clone(),
                },
            },
            Atom::Stat { stat, label, .. } => match label {
                Some(l) => l.clone(),
                None => tab_stat_header(stat).to_string(),
            },
            Atom::All { label, .. } => match label {
                Some(l) => l.clone(),
                None => "All".to_string(),
            },
        })
        .collect();
    parts.join("*")
}

/// Header label for a stat keyword, extending `means::stat_header` with the
/// TABULATE-specific percentage stats (kept local to avoid touching common
/// code shared with the parallel REPORT work).
pub(super) fn tab_stat_header(stat: &str) -> &'static str {
    match stat {
        "pctn" => "PctN",
        "pctsum" => "PctSum",
        _ => stat_header(stat),
    }
}

pub(super) fn level_label(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Char(s) => s.clone(),
        Value::Missing(k) => k.display(),
    }
}

/// The computed result of one cell: the resolved statistic keyword, the raw
/// numeric `Value`, and the optional per-cell `*f=<fmt>` format.
pub(super) struct CellResult {
    pub(super) stat: String,
    pub(super) value: Value,
    pub(super) format: Option<String>,
}

/// Validate the merged cell's atoms and compute its raw numeric value.
/// Returns a missing `Value` when the cell is undefined (no qualifying rows /
/// undefined statistic). Errors cleanly for unsupported constructs (>1 VAR,
/// >1 stat). Formatting is applied separately by the caller.
pub(super) fn compute_cell_value(
    atoms: &[Atom],
    var_values: &[(usize, Vec<Value>)],
    class_values: &[(usize, Vec<Value>)],
    n_obs: usize,
) -> Result<CellResult> {
    let mut var_col: Option<usize> = None;
    let mut stat: Option<String> = None;
    // (class col, required level) constraints.
    let mut class_constraints: Vec<(usize, &Value)> = Vec::new();
    // Per-cell format: the first `*f=` carried by any atom of the cell.
    let mut cell_format: Option<String> = None;

    for a in atoms {
        if cell_format.is_none() {
            if let Some(f) = a.format() {
                cell_format = Some(f.to_string());
            }
        }
        match a {
            Atom::Var { col, .. } => {
                if var_col.is_some() {
                    return Err(SasError::runtime(
                        "PROC TABULATE: crossing two analysis variables not yet supported",
                    ));
                }
                var_col = Some(*col);
            }
            Atom::Stat { stat: s, .. } => {
                if stat.is_some() {
                    return Err(SasError::runtime(
                        "PROC TABULATE: crossing two statistics not yet supported",
                    ));
                }
                stat = Some(s.clone());
            }
            Atom::ClassLevel { col, level, .. } => {
                class_constraints.push((*col, level));
            }
            // Universal class: aggregate over every category — no constraint.
            Atom::All { .. } => {}
        }
    }

    // Select rows matching ALL class constraints (and excluding missing
    // class values — they are never equal to a non-missing required level).
    let rows: Vec<usize> = (0..n_obs)
        .filter(|&r| {
            class_constraints.iter().all(|(col, level)| {
                let vals = &class_values
                    .iter()
                    .find(|(c, _)| c == col)
                    .expect("class col decoded")
                    .1;
                vals[r].sas_cmp(level) == Ordering::Equal
            })
        })
        .collect();

    // Default statistic: SUM when a VAR is present, N otherwise (frequency).
    let stat = stat.unwrap_or_else(|| {
        if var_col.is_some() {
            "sum".to_string()
        } else {
            "n".to_string()
        }
    });

    let mk = |value: Value| CellResult {
        stat: stat.clone(),
        value,
        format: cell_format.clone(),
    };

    // Percentage statistics: numerator over the selected rows, denominator
    // over the grand total (all observations). v1 supports only the grand
    // total denominator (group denominators PCTN<...> are deferred).
    if stat == "pctn" {
        let denom = n_obs as f64;
        let value = if denom == 0.0 {
            Value::Missing(crate::value::MissingKind::Dot)
        } else {
            Value::Num(100.0 * rows.len() as f64 / denom)
        };
        return Ok(mk(value));
    }
    if stat == "pctsum" {
        let ci = var_col.ok_or_else(|| {
            SasError::runtime(
                "PROC TABULATE: PCTSUM requires an analysis variable (not yet supported)",
            )
        })?;
        let col = &var_values
            .iter()
            .find(|(c, _)| *c == ci)
            .expect("var col decoded")
            .1;
        let (xs, _) = partition_numeric(col, &rows);
        let all_rows: Vec<usize> = (0..n_obs).collect();
        let (all_xs, _) = partition_numeric(col, &all_rows);
        let denom: f64 = all_xs.iter().sum();
        let numer: f64 = xs.iter().sum();
        let value = if denom == 0.0 {
            Value::Missing(crate::value::MissingKind::Dot)
        } else {
            Value::Num(100.0 * numer / denom)
        };
        return Ok(mk(value));
    }

    // Determine the analysis values. With no VAR, only N/NMISS are meaningful
    // (frequency counts over the selected rows).
    let value: Value = match var_col {
        Some(ci) => {
            let col = &var_values
                .iter()
                .find(|(c, _)| *c == ci)
                .expect("var col decoded")
                .1;
            let (xs, nmiss) = partition_numeric(col, &rows);
            // TABULATE has no CI statistics; default alpha is unused here.
            compute(&stat, &xs, nmiss, 0.05)
        }
        None => {
            // No analysis variable: only frequency-style stats are defined.
            match stat.as_str() {
                "n" => Value::Num(rows.len() as f64),
                "nmiss" => Value::Num(0.0),
                _ => {
                    return Err(SasError::runtime(format!(
                        "PROC TABULATE: statistic {} requires an analysis variable (not yet supported)",
                        stat.to_uppercase()
                    )))
                }
            }
        }
    };

    Ok(mk(value))
}

/// Compute a cell and render it to a listing string. The effective format is
/// the per-cell `*f=` (if any) else the table-level `format=` default; with
/// neither, the rendering is byte-identical to the historical path.
pub(super) fn compute_cell(
    atoms: &[Atom],
    var_values: &[(usize, Vec<Value>)],
    class_values: &[(usize, Vec<Value>)],
    n_obs: usize,
    table_format: Option<&str>,
    catalog: &crate::formats::FormatCatalog,
) -> Result<String> {
    let res = compute_cell_value(atoms, var_values, class_values, n_obs)?;
    let fmt = res.format.as_deref().or(table_format);
    Ok(fmt_cell(&res.stat, &res.value, fmt, catalog))
}

/// Format a computed cell value for the listing. With no format spec, keeps the
/// historical rendering (integers for N/NMISS, BESTw. otherwise, "." for
/// missing). With a format, routes through the SAS format engine.
pub(super) fn fmt_cell(
    stat: &str,
    v: &Value,
    format: Option<&str>,
    catalog: &crate::formats::FormatCatalog,
) -> String {
    if let Some(f) = format.and_then(FormatSpec::parse) {
        // SAS format engine; missings render via the engine too. Trim leading
        // pad so the listing column aligner controls width (matches the
        // unformatted path, which emits unpadded tokens).
        return catalog.format(v, &f).trim_start().to_string();
    }
    match v {
        Value::Num(f) => {
            if stat == "n" || stat == "nmiss" {
                format!("{}", *f as i64)
            } else {
                format_best(*f, 12)
            }
        }
        Value::Missing(_) => ".".to_string(),
        Value::Char(s) => s.clone(),
    }
}
