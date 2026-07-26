use super::*;

/// Render a Value into a listing cell (numeric via format_best, missing → ".").
pub(super) fn fmt_cell(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Missing(k) => k.display(),
        Value::Char(s) => s.clone(),
    }
}

/// Render a Value into a listing cell, honoring an optional `format=<fmt>`
/// DEFINE option (M33.5). With no format, this is byte-identical to `fmt_cell`.
/// With a format, the value routes through the SAS format engine and the
/// leading pad is trimmed so the listing aligner controls width (mirrors
/// TABULATE's M33.4 cell formatting).
pub(super) fn fmt_cell_fmt(
    v: &Value,
    format: Option<&str>,
    catalog: &crate::formats::FormatCatalog,
) -> String {
    if let Some(spec) = format.and_then(crate::formats::FormatSpec::parse) {
        return catalog.format(v, &spec).trim_start().to_string();
    }
    fmt_cell(v)
}

// ───────────────────────── ACROSS report ─────────────────────────

/// Render an ACROSS report: GROUP/ORDER vars in rows, the distinct values of
/// the ACROSS var in columns, each cell = the statistic of the ANALYSIS var.
/// v1 supports exactly one ACROSS var and one ANALYSIS var; the two-level
/// header (across value over statistic) is flattened into a single header line
/// "value stat" (documented simplification, since the listing has no spanner).
pub(super) fn execute_across(
    ast: &ReportAst,
    session: &mut Session,
    ds: &crate::dataset::SasDataset,
    plan: &[ColPlan],
    decoded: &[Vec<Value>],
    n_obs: usize,
    display_name: &str,
) -> Result<()> {
    // Identify the across, group/order, and analysis columns.
    let across_pos = plan
        .iter()
        .position(|c| matches!(c.usage, Usage::Across))
        .expect("execute_across called without an ACROSS column");
    let group_positions: Vec<usize> = plan
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c.usage, Usage::Group | Usage::Order))
        .map(|(i, _)| i)
        .collect();
    let analysis_pos = plan
        .iter()
        .position(|c| matches!(c.usage, Usage::Analysis(_)));
    let (apos, stat) = match analysis_pos {
        Some(p) => match &plan[p].usage {
            Usage::Analysis(s) => (p, s.clone()),
            _ => unreachable!(),
        },
        None => {
            return Err(SasError::runtime(
                "PROC REPORT ACROSS in v1 requires exactly one ANALYSIS variable.",
            ));
        }
    };

    // Distinct across values (sorted via sas_cmp, honoring direction).
    let across_dir = plan[across_pos].dir;
    let mut across_vals: Vec<Value> = Vec::new();
    for v in decoded[across_pos].iter().take(n_obs) {
        if !across_vals.iter().any(|e| e.sas_cmp(v) == Ordering::Equal) {
            across_vals.push(v.clone());
        }
    }
    across_vals.sort_by(|a, b| {
        let c = a.sas_cmp(b);
        if across_dir == OrderDir::Descending {
            c.reverse()
        } else {
            c
        }
    });

    // Group the rows by the GROUP/ORDER key tuple.
    let key_refs: Vec<&Vec<Value>> = group_positions.iter().map(|&p| &decoded[p]).collect();
    let mut groups = group_by_keys(&key_refs, n_obs);
    let dirs: Vec<OrderDir> = group_positions.iter().map(|&p| plan[p].dir).collect();
    groups.sort_by(|(a, _), (b, _)| {
        for ((x, y), dir) in a.iter().zip(b).zip(&dirs) {
            let mut c = x.sas_cmp(y);
            if *dir == OrderDir::Descending {
                c = c.reverse();
            }
            if c != Ordering::Equal {
                return c;
            }
        }
        Ordering::Equal
    });

    // Headers: the GROUP/ORDER columns, then one column per across value.
    let mut headers: Vec<String> = group_positions
        .iter()
        .map(|&p| plan[p].header.clone())
        .collect();
    let stat_label = stat.to_uppercase();
    for av in &across_vals {
        headers.push(format!("{} {}", value_to_disp(av), stat_label));
    }

    let mut aligns: Vec<Align> = group_positions
        .iter()
        .map(|&p| match ds.vars[plan[p].idx].ty {
            VarType::Num => Align::Right,
            VarType::Char => Align::Left,
        })
        .collect();
    aligns.extend(std::iter::repeat_n(Align::Right, across_vals.len()));

    let mut rows: Vec<Vec<String>> = Vec::with_capacity(groups.len());
    for (_key, grp_rows) in &groups {
        let mut row: Vec<String> = Vec::new();
        for &gp in &group_positions {
            row.push(fmt_cell(&decoded[gp][grp_rows[0]]));
        }
        for av in &across_vals {
            // Sub-select the group rows whose across value equals `av`.
            let sub: Vec<usize> = grp_rows
                .iter()
                .copied()
                .filter(|&r| decoded[across_pos][r].sas_cmp(av) == Ordering::Equal)
                .collect();
            let (xs, nmiss) = partition_numeric(&decoded[apos], &sub);
            let v = means::compute(&stat, &xs, nmiss, 0.05);
            row.push(fmt_cell(&v));
        }
        rows.push(row);
    }

    session.listing.page_header();
    if ast.noheader {
        write_table_noheader(session, &aligns, &rows);
    } else {
        session.listing.write_table(&headers, &aligns, &rows);
    }

    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        n_obs, display_name
    ));
    // OUT= for ACROSS is deferred CLEANLY (no panic): note and skip.
    if ast.out.is_some() {
        session.log.note(
            "PROC REPORT v1 does not write an OUT= data set for ACROSS reports; OUT= ignored.",
        );
    }
    Ok(())
}

/// Render `compute after; line ...; endcomp;` free-text lines below the report.
/// LINE items are concatenated: string literals verbatim, `@<col>` pointers pad
/// to a column, and expressions are resolved over the grand-total context (with
/// `_Cn_` aliases) and rendered with an optional trailing format (M33.5).
pub(super) fn render_after_lines(
    ast: &ReportAst,
    session: &mut Session,
    plan: &[ColPlan],
    rows: &[RowOut],
    catalog: &crate::formats::FormatCatalog,
) {
    for comp in &ast.computes {
        if !comp.target.eq_ignore_ascii_case("after") {
            continue;
        }
        // Context for LINE expressions: the grand-total (RBREAK) row if present,
        // else the last row, else empty.
        let ctx_row = rows
            .iter()
            .rev()
            .find(|r| r.kind == RowKind::Rbreak)
            .or_else(|| rows.last());
        let ctx_cols: Option<Vec<(String, Vec<Value>)>> =
            ctx_row.map(|ro| compute_row_context(plan, &ro.vals));
        for st in &comp.stmts {
            if let ComputeStmt::Line(items) = st {
                let mut line = String::new();
                for item in items {
                    match item {
                        LineItem::Literal(s) => line.push_str(s),
                        LineItem::Pointer(col) => {
                            // Pad the line out to (1-based) column `col`.
                            if *col > line.len() {
                                line.push_str(&" ".repeat(*col - line.len()));
                            }
                        }
                        LineItem::Expr(e, fmt) => {
                            let v = match &ctx_cols {
                                Some(cols) => eval_row_expr(e, cols, 0),
                                None => Value::missing(),
                            };
                            match fmt.as_deref().and_then(crate::formats::FormatSpec::parse) {
                                Some(spec) => line.push_str(catalog.format(&v, &spec).trim_start()),
                                None => line.push_str(&value_to_disp(&v)),
                            }
                        }
                    }
                }
                session.listing.write_line(line.trim_end());
            }
        }
    }
}

// ───────────────────────── OUT= dataset ─────────────────────────

/// Render a Value as an optional char cell for OUT= (trailing blanks trimmed,
/// blanks/missing → null).
pub(super) fn value_to_char_cell(v: &Value) -> Option<String> {
    match v {
        Value::Char(s) => {
            let t = s.trim_end();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Value::Num(f) => Some(format_best(*f, 12).trim().to_string()),
        Value::Missing(_) => None,
    }
}

/// Render a table without a header row. We compute column widths from the
/// data cells and align each column, mirroring the listing's table layout
/// but skipping the header line entirely (NOHEADER option).
pub(super) fn write_table_noheader(session: &mut Session, aligns: &[Align], rows: &[Vec<String>]) {
    let ncol = aligns.len();
    if ncol == 0 {
        return;
    }
    let mut widths = vec![0usize; ncol];
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.len());
        }
    }
    for row in rows {
        let mut line = String::new();
        for (i, cell) in row.iter().enumerate() {
            if i > 0 {
                line.push_str("  ");
            }
            let w = widths[i];
            match aligns[i] {
                Align::Right => {
                    let pad = w.saturating_sub(cell.len());
                    line.push_str(&" ".repeat(pad));
                    line.push_str(cell);
                }
                Align::Left => {
                    line.push_str(cell);
                    let pad = w.saturating_sub(cell.len());
                    line.push_str(&" ".repeat(pad));
                }
            }
        }
        session.listing.write_line(line.trim_end());
    }
}

/// Render the report table honoring per-column `WIDTH=`/`SPACING=` DEFINE
/// options (M33.5). Used only when at least one column carries WIDTH=/SPACING=;
/// the default path stays on `ListingWriter::write_table`.
///
/// Semantics (faithful to SAS LISTING):
///   - A column's width is its `WIDTH=` if given, else the max of the header
///     and cell lengths (the auto width).
///   - Cells/header are truncated or padded to the width; numeric (Right-
///     aligned) columns right-justify, character (Left-aligned) columns
///     left-justify.
///   - `SPACING=<n>` sets the number of blank spaces BEFORE the column
///     (default 2). The leading column's spacing is rendered as left padding
///     too (SAS indents the first column by its spacing).
pub(super) fn write_table_layout(
    session: &mut Session,
    headers: &[String],
    aligns: &[Align],
    rows: &[Vec<String>],
    plan: &[ColPlan],
    noheader: bool,
) {
    let ncol = headers.len();
    if ncol == 0 {
        return;
    }

    // Resolve each column's effective width.
    let mut widths = vec![0usize; ncol];
    for i in 0..ncol {
        match plan[i].width {
            Some(w) => widths[i] = w,
            None => {
                let mut w = headers[i].len();
                for row in rows {
                    if let Some(cell) = row.get(i) {
                        w = w.max(cell.len());
                    }
                }
                widths[i] = w;
            }
        }
    }

    // Spacing before each column (default 2; the leading column's spacing is
    // emitted as left indentation).
    let spacing: Vec<usize> = plan.iter().map(|c| c.spacing.unwrap_or(2)).collect();

    let pad_cell = |cell: &str, w: usize, align: Align| -> String {
        let mut s = cell.to_string();
        if s.len() > w {
            s.truncate(w);
        }
        let pad = w.saturating_sub(s.len());
        match align {
            Align::Right => format!("{}{}", " ".repeat(pad), s),
            Align::Left => format!("{}{}", s, " ".repeat(pad)),
        }
    };

    let render = |cells: &dyn Fn(usize) -> String| -> String {
        let mut line = String::new();
        for i in 0..ncol {
            line.push_str(&" ".repeat(spacing[i]));
            let align = aligns.get(i).copied().unwrap_or(Align::Left);
            line.push_str(&pad_cell(&cells(i), widths[i], align));
        }
        line
    };

    if !noheader {
        let header_line = render(&|i| headers[i].clone());
        session.listing.write_line(header_line.trim_end());
        session.listing.blank();
    }
    for row in rows {
        let line = render(&|i| row.get(i).cloned().unwrap_or_default());
        session.listing.write_line(line.trim_end());
    }
}
