use super::*;

/// Header text used both as report header and a stat label in the listing.
pub fn stat_header(stat: &str) -> &'static str {
    match stat {
        "n" => "N",
        "nmiss" => "NMiss",
        "mean" => "Mean",
        "std" | "stddev" => "Std Dev",
        "min" => "Minimum",
        "max" => "Maximum",
        "sum" => "Sum",
        "range" => "Range",
        "stderr" => "Std Error",
        "cv" => "CV",
        "median" => "Median",
        // CI stats: alpha-dependent labels are produced by
        // `stat_report_headers`; these are generic fallbacks.
        "lclm" => "Lower CL for Mean",
        "uclm" => "Upper CL for Mean",
        "clm" => "CL for Mean",
        _ => "Stat",
    }
}

/// Render a single computed stat value into a listing cell.
pub(super) fn fmt_stat_cell(stat: &str, v: &Value) -> String {
    match v {
        Value::Num(f) => {
            if stat == "n" || stat == "nmiss" {
                format!("{}", *f as i64)
            } else {
                format_best(*f, 12)
            }
        }
        Value::Missing(k) => k.display(),
        Value::Char(s) => s.clone(),
    }
}

/// Emit the SAS BY heading line into the listing: `var1=val1 var2=val2`.
pub(super) fn emit_by_heading(session: &mut Session, by_names: &[String], by_key: &[Value]) {
    let parts: Vec<String> = by_names
        .iter()
        .zip(by_key)
        .map(|(name, v)| format!("{}={}", name, class_cell(v)))
        .collect();
    session.listing.write_line(&parts.join(" "));
    session.listing.blank();
}

/// Emit one MEANS report table for the rows in `group_rows` (the full row set
/// when no BY is active). Does NOT emit the procedure title (caller does that
/// once). CLASS grouping is applied within `group_rows` only.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_report_group(
    session: &mut Session,
    ds: &SasDataset,
    class_cols: &[usize],
    class_values: &[Vec<Value>],
    var_cols: &[usize],
    var_values: &[Vec<Value>],
    weight_values: Option<&[Value]>,
    report_stats: &[String],
    alpha: f64,
    group_rows: &[usize],
) {
    let mut headers: Vec<String> = Vec::new();
    let mut aligns: Vec<Align> = Vec::new();

    // Leading CLASS columns (only when CLASS present).
    for &ci in class_cols {
        headers.push(ds.vars[ci].name.clone());
        aligns.push(match ds.vars[ci].ty {
            VarType::Num => Align::Right,
            VarType::Char => Align::Left,
        });
    }
    headers.push("Variable".to_string());
    aligns.push(Align::Left);
    // CLM expands to two columns; LCLM/UCLM to one CL column each; all others
    // to a single column. Header text reflects the confidence level.
    for s in report_stats {
        for h in stat_report_headers(s, alpha) {
            headers.push(h);
            aligns.push(Align::Right);
        }
    }

    // Append the per-stat cells for one analysis variable to `row`, choosing
    // the weighted or unweighted path. CLM yields two cells (lower, upper).
    let push_cells = |row: &mut Vec<String>, vi: usize, grp_rows: &[usize]| match weight_values {
        Some(wv) => {
            let (pairs, nmiss) = partition_weighted(&var_values[vi], wv, grp_rows);
            for s in report_stats {
                for cell in stat_report_cells(s, &|st| compute_weighted(st, &pairs, nmiss, alpha)) {
                    row.push(cell);
                }
            }
        }
        None => {
            let (xs, nmiss) = partition_numeric(&var_values[vi], grp_rows);
            for s in report_stats {
                for cell in stat_report_cells(s, &|st| compute(st, &xs, nmiss, alpha)) {
                    row.push(cell);
                }
            }
        }
    };

    let mut rows: Vec<Vec<String>> = Vec::new();

    if class_cols.is_empty() {
        // One section over the group's rows: one row per analysis variable.
        for (vi, vname_idx) in var_cols.iter().enumerate() {
            let mut row = vec![ds.vars[*vname_idx].name.clone()];
            push_cells(&mut row, vi, group_rows);
            rows.push(row);
        }
    } else {
        // CLASS grouping restricted to this BY group's rows.
        let cv_refs: Vec<&Vec<Value>> = class_values.iter().collect();
        let groups = group_by_keys_subset(&cv_refs, group_rows);
        for (key, grp_rows) in &groups {
            for (vi, vname_idx) in var_cols.iter().enumerate() {
                let mut row: Vec<String> = Vec::new();
                for kv in key {
                    row.push(class_cell(kv));
                }
                row.push(ds.vars[*vname_idx].name.clone());
                push_cells(&mut row, vi, grp_rows);
                rows.push(row);
            }
        }
    }

    session.listing.write_table(&headers, &aligns, &rows);
}

/// Emit one MEANS report subtable for a single `_TYPE_` mask `ty` (M33.3,
/// PRINTALLTYPES / WAYS / TYPES). Only the CLASS variables ACTIVE in `ty` head
/// the table; rows are grouped by those active variables within `group_rows`.
/// `ty`=0 → no CLASS columns (the overall section). The combined default-path
/// table is still produced by `emit_report_group`; this is the per-type path.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_report_type(
    session: &mut Session,
    ds: &SasDataset,
    class_cols: &[usize],
    class_values: &[Vec<Value>],
    var_cols: &[usize],
    var_values: &[Vec<Value>],
    weight_values: Option<&[Value]>,
    report_stats: &[String],
    alpha: f64,
    group_rows: &[usize],
    ty: u64,
) {
    let k = class_cols.len();
    // Active CLASS positions for this _TYPE_: bit (k-1-i) set ⇔ class i active.
    let active: Vec<usize> = (0..k).filter(|&i| (ty >> (k - 1 - i)) & 1 == 1).collect();

    let mut headers: Vec<String> = Vec::new();
    let mut aligns: Vec<Align> = Vec::new();
    for &i in &active {
        let ci = class_cols[i];
        headers.push(ds.vars[ci].name.clone());
        aligns.push(match ds.vars[ci].ty {
            VarType::Num => Align::Right,
            VarType::Char => Align::Left,
        });
    }
    headers.push("Variable".to_string());
    aligns.push(Align::Left);
    for s in report_stats {
        for h in stat_report_headers(s, alpha) {
            headers.push(h);
            aligns.push(Align::Right);
        }
    }

    let push_cells = |row: &mut Vec<String>, vi: usize, grp_rows: &[usize]| match weight_values {
        Some(wv) => {
            let (pairs, nmiss) = partition_weighted(&var_values[vi], wv, grp_rows);
            for s in report_stats {
                for cell in stat_report_cells(s, &|st| compute_weighted(st, &pairs, nmiss, alpha)) {
                    row.push(cell);
                }
            }
        }
        None => {
            let (xs, nmiss) = partition_numeric(&var_values[vi], grp_rows);
            for s in report_stats {
                for cell in stat_report_cells(s, &|st| compute(st, &xs, nmiss, alpha)) {
                    row.push(cell);
                }
            }
        }
    };

    let mut rows: Vec<Vec<String>> = Vec::new();
    if active.is_empty() {
        // Overall (_TYPE_=0): one row per analysis variable over all group rows.
        for (vi, vname_idx) in var_cols.iter().enumerate() {
            let mut row = vec![ds.vars[*vname_idx].name.clone()];
            push_cells(&mut row, vi, group_rows);
            rows.push(row);
        }
    } else {
        let active_refs: Vec<&Vec<Value>> = active.iter().map(|&i| &class_values[i]).collect();
        let groups = group_by_keys_subset(&active_refs, group_rows);
        for (key, grp_rows) in &groups {
            for (vi, vname_idx) in var_cols.iter().enumerate() {
                let mut row: Vec<String> = Vec::new();
                for kv in key {
                    row.push(class_cell(kv));
                }
                row.push(ds.vars[*vname_idx].name.clone());
                push_cells(&mut row, vi, grp_rows);
                rows.push(row);
            }
        }
    }

    session.listing.write_table(&headers, &aligns, &rows);
}

/// Report column header(s) for a stat. Most stats map to one header; the
/// confidence-limit stats produce alpha-dependent labels and CLM produces two.
pub(super) fn stat_report_headers(stat: &str, alpha: f64) -> Vec<String> {
    let pct = cl_percent_label(alpha);
    match stat {
        "lclm" => vec![format!("Lower {pct}% CL for Mean")],
        "uclm" => vec![format!("Upper {pct}% CL for Mean")],
        "clm" => vec![
            format!("Lower {pct}% CL for Mean"),
            format!("Upper {pct}% CL for Mean"),
        ],
        _ => match percentile_header(stat) {
            Some(h) => vec![h],
            None => vec![stat_header(stat).to_string()],
        },
    }
}

/// SAS report header for a percentile keyword (None for non-percentile stats).
/// SAS prints percentiles as "Nth Pctl" (e.g. "25th Pctl"), `MEDIAN`/`P50` as
/// "Median", and `QRANGE` as "Quartile Range". `Q1`/`Q3` alias `P25`/`P75`.
pub(super) fn percentile_header(stat: &str) -> Option<String> {
    match stat {
        "qrange" => Some("Quartile Range".to_string()),
        "median" | "p50" => Some("Median".to_string()),
        _ => {
            let p = percentile_fraction(stat)?;
            // Whole-number percentile, e.g. 0.25 → 25.
            let pct = (p * 100.0).round() as i64;
            let suffix = match (pct % 10, pct % 100) {
                (1, n) if n != 11 => "st",
                (2, n) if n != 12 => "nd",
                (3, n) if n != 13 => "rd",
                _ => "th",
            };
            Some(format!("{pct}{suffix} Pctl"))
        }
    }
}

/// Report cell(s) for a stat, computing values via `f` (the unweighted or
/// weighted `compute*` closure). CLM emits two cells (LCLM then UCLM).
pub(super) fn stat_report_cells(stat: &str, f: &dyn Fn(&str) -> Value) -> Vec<String> {
    match stat {
        "clm" => vec![
            fmt_stat_cell("lclm", &f("lclm")),
            fmt_stat_cell("uclm", &f("uclm")),
        ],
        _ => vec![fmt_stat_cell(stat, &f(stat))],
    }
}

/// Format the confidence percentage for a CL header from alpha, e.g.
/// 0.05 → "95", 0.10 → "90", 0.01 → "99". Whole percents print without a
/// decimal; otherwise the trailing zeros are trimmed (matches SAS labels).
pub(super) fn cl_percent_label(alpha: f64) -> String {
    let pct = 100.0 * (1.0 - alpha);
    // Round to a sensible precision to avoid FP noise like 94.99999999.
    let rounded = (pct * 1e6).round() / 1e6;
    if (rounded - rounded.round()).abs() < 1e-9 {
        format!("{}", rounded.round() as i64)
    } else {
        format!("{rounded}")
    }
}

/// Render a class-value cell in the listing.
pub(super) fn class_cell(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Missing(k) => k.display(),
        Value::Char(s) => s.clone(),
    }
}
