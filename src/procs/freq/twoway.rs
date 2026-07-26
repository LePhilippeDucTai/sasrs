use super::*;

/// Two-way crosstab for v1*v2, over `rows` with optional weights.
pub(super) fn two_way(
    session: &mut Session,
    ds: &SasDataset,
    req: &TableRequest,
    rows: &[usize],
    weights: Option<&[Value]>,
) -> Result<()> {
    let row_idx = find_var(ds, &req.vars[0])?;
    let col_idx = find_var(ds, &req.vars[1])?;
    let row_col = decode_column(ds, row_idx)?;
    let col_col = decode_column(ds, col_idx)?;
    let row_name = ds.vars[row_idx].name.clone();
    let col_name = ds.vars[col_idx].name.clone();

    let keep = |v: &Value| req.missing || !v.is_missing();
    let row_vals = distinct_axis(&row_col, rows, req.missing, weights);
    let col_vals = distinct_axis(&col_col, rows, req.missing, weights);

    let nr = row_vals.len();
    let nc = col_vals.len();

    // freq[r][c] = sum of weights (or counts) for the cell.
    let mut freq = vec![vec![0.0_f64; nc]; nr];
    for &i in rows {
        let Some(w) = obs_weight(weights, i) else {
            continue;
        };
        let rv = &row_col[i];
        let cv = &col_col[i];
        if !keep(rv) || !keep(cv) {
            continue;
        }
        let r = row_vals
            .iter()
            .position(|x| x.sas_cmp(rv) == Ordering::Equal);
        let c = col_vals
            .iter()
            .position(|x| x.sas_cmp(cv) == Ordering::Equal);
        if let (Some(r), Some(c)) = (r, c) {
            freq[r][c] += w;
        }
    }

    render_two_way(
        session, req, &row_name, &col_name, &row_vals, &col_vals, &freq,
    );
    Ok(())
}

/// Render a two-way crosstab from a computed weighted frequency matrix:
/// grid layout (default) or LIST layout (`/LIST`), followed by any requested
/// statistic blocks. Shared by `two_way` and the n-way stratified renderer.
pub(super) fn render_two_way(
    session: &mut Session,
    req: &TableRequest,
    row_name: &str,
    col_name: &str,
    row_vals: &[Value],
    col_vals: &[Value],
    freq: &[Vec<f64>],
) {
    let nr = row_vals.len();
    let nc = col_vals.len();

    let row_tot: Vec<f64> = (0..nr).map(|r| freq[r].iter().sum()).collect();
    let col_tot: Vec<f64> = (0..nc).map(|c| (0..nr).map(|r| freq[r][c]).sum()).collect();
    let grand: f64 = row_tot.iter().sum();

    // LIST layout: one row per non-empty cell, suppressing the grid and the
    // row/col percentages (SAS LIST shows Frequency / Percent / Cumulative).
    if req.list {
        render_two_way_list(
            session, req, row_name, col_name, row_vals, col_vals, freq, grand,
        );
        emit_two_way_stats(
            session, req, row_name, col_name, freq, &row_tot, &col_tot, grand,
        );
        return;
    }

    // Which stacked per-cell lines to show. Display options drop a line:
    //   NOFREQ    -> Frequency, NOPERCENT -> Percent,
    //   NOROW     -> Row Pct,   NOCOL     -> Col Pct.
    // Default (no options) keeps all four, exactly as before.
    let show_freq = !req.nofreq;
    let show_pct = !req.nopercent;
    let show_rowp = !req.norow;
    let show_colp = !req.nocol;

    // Legend reflecting the lines actually shown.
    let mut legend_parts: Vec<&str> = Vec::new();
    if show_freq {
        legend_parts.push("Frequency");
    }
    if show_pct {
        legend_parts.push("Percent");
    }
    if show_rowp {
        legend_parts.push("Row Pct");
    }
    if show_colp {
        legend_parts.push("Col Pct");
    }

    session
        .listing
        .write_line(&format!("Table of {row_name} by {col_name}"));
    session.listing.blank();
    if !legend_parts.is_empty() {
        session
            .listing
            .write_line(&format!("Cell contents: {}", legend_parts.join(" / ")));
        session.listing.blank();
    }

    // Header: row-var name, one column per col value, then Total.
    let mut headers = vec![row_name.to_string()];
    for cv in col_vals {
        headers.push(category_label(cv));
    }
    headers.push("Total".to_string());
    let mut aligns = vec![Align::Left];
    for _ in 0..nc {
        aligns.push(Align::Right);
    }
    aligns.push(Align::Right);

    // Each logical row -> 4 physical rows (Frequency, Percent, Row Pct,
    // Col Pct). The first physical row carries the row-value label.
    let mut rows: Vec<Vec<String>> = Vec::new();
    let pct_of = |num: f64, den: f64| -> String {
        if den > 0.0 {
            fmt_pct(100.0 * num / den)
        } else {
            fmt_pct(0.0)
        }
    };

    // The row-value label rides on the first physical line that is shown.
    let label_on_freq = show_freq;
    let label_on_pct = !show_freq && show_pct;
    let label_on_rowp = !show_freq && !show_pct && show_rowp;
    let label_on_colp = !show_freq && !show_pct && !show_rowp && show_colp;

    for r in 0..nr {
        let mut line_freq = vec![if label_on_freq {
            category_label(&row_vals[r])
        } else {
            String::new()
        }];
        let mut line_pct = vec![if label_on_pct {
            category_label(&row_vals[r])
        } else {
            String::new()
        }];
        let mut line_rowp = vec![if label_on_rowp {
            category_label(&row_vals[r])
        } else {
            String::new()
        }];
        let mut line_colp = vec![if label_on_colp {
            category_label(&row_vals[r])
        } else {
            String::new()
        }];
        for c in 0..nc {
            let f = freq[r][c];
            line_freq.push(fmt_freq(f));
            line_pct.push(pct_of(f, grand));
            line_rowp.push(pct_of(f, row_tot[r]));
            line_colp.push(pct_of(f, col_tot[c]));
        }
        // Row total margin: Frequency + Percent only.
        line_freq.push(fmt_freq(row_tot[r]));
        line_pct.push(pct_of(row_tot[r], grand));
        line_rowp.push(String::new());
        line_colp.push(String::new());
        if show_freq {
            rows.push(line_freq);
        }
        if show_pct {
            rows.push(line_pct);
        }
        if show_rowp {
            rows.push(line_rowp);
        }
        if show_colp {
            rows.push(line_colp);
        }
    }

    // Total row (column totals + grand total): Frequency + Percent only.
    let mut tot_freq = vec!["Total".to_string()];
    let mut tot_pct = vec![String::new()];
    for &tot in col_tot.iter().take(nc) {
        tot_freq.push(fmt_freq(tot));
        tot_pct.push(pct_of(tot, grand));
    }
    tot_freq.push(fmt_freq(grand));
    tot_pct.push(pct_of(grand, grand));
    if show_freq {
        rows.push(tot_freq);
    }
    if show_pct {
        // When the Frequency line is suppressed the "Total" label needs to
        // land on the percent line so the margin row stays identifiable.
        if !show_freq {
            tot_pct[0] = "Total".to_string();
        }
        rows.push(tot_pct);
    }

    session.listing.write_table(&headers, &aligns, &rows);

    emit_two_way_stats(
        session, req, row_name, col_name, freq, &row_tot, &col_tot, grand,
    );
}

/// Print all requested statistic blocks for a two-way table. CHISQ uses the
/// exact (possibly weighted) frequencies; the integer-count tests
/// (Fisher/MEASURES/AGREE/TREND) operate on a rounded copy.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_two_way_stats(
    session: &mut Session,
    req: &TableRequest,
    row_name: &str,
    col_name: &str,
    freq: &[Vec<f64>],
    row_tot: &[f64],
    col_tot: &[f64],
    grand: f64,
) {
    if req.chisq {
        chisq_block(session, row_name, col_name, freq, row_tot, col_tot, grand);
    }
    if req.fisher || req.trend || req.measures || req.agree {
        let ifreq = round_matrix(freq);
        let irow: Vec<usize> = ifreq.iter().map(|r| r.iter().sum()).collect();
        let icol: Vec<usize> = (0..col_tot.len())
            .map(|c| (0..ifreq.len()).map(|r| ifreq[r][c]).sum())
            .collect();
        let igrand: usize = irow.iter().sum();
        if req.fisher {
            fisher_block(session, &ifreq, &irow, &icol, igrand);
        }
        if req.trend {
            trend_block(session, &ifreq, &irow, &icol, igrand);
        }
        if req.measures {
            measures_block(session, &ifreq);
        }
        if req.agree {
            agree_block(session, &ifreq, &irow, &icol, igrand);
        }
    }
}

/// Render a two-way table in LIST layout: one row per non-empty cell, with
/// columns (row var, col var, Frequency, Percent, Cumulative Frequency,
/// Cumulative Percent). Cells are walked in sas_cmp order (row-major). LIST
/// suppresses the grid and the Row/Col percentages.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_two_way_list(
    session: &mut Session,
    req: &TableRequest,
    row_name: &str,
    col_name: &str,
    row_vals: &[Value],
    col_vals: &[Value],
    freq: &[Vec<f64>],
    grand: f64,
) {
    session
        .listing
        .write_line(&format!("Table of {row_name} by {col_name}"));
    session.listing.blank();

    let headers = vec![
        row_name.to_string(),
        col_name.to_string(),
        "Frequency".to_string(),
        "Percent".to_string(),
        "Cumulative Frequency".to_string(),
        "Cumulative Percent".to_string(),
    ];
    let aligns = vec![
        Align::Left,
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut cum = 0.0_f64;
    for r in 0..row_vals.len() {
        for c in 0..col_vals.len() {
            let f = freq[r][c];
            if f == 0.0 {
                continue; // LIST prints only non-empty cells.
            }
            cum += f;
            let pct = if grand > 0.0 { 100.0 * f / grand } else { 0.0 };
            let cum_pct = if grand > 0.0 {
                100.0 * cum / grand
            } else {
                0.0
            };
            rows.push(vec![
                category_label(&row_vals[r]),
                category_label(&col_vals[c]),
                fmt_freq(f),
                fmt_pct(pct),
                fmt_freq(cum),
                fmt_pct(cum_pct),
            ]);
        }
    }

    session.listing.write_table(&headers, &aligns, &rows);
}

/// n-way (≥3 variables) crosstab. SAS prints this as a series of two-way
/// tables of the LAST two variables, stratified by the distinct combinations
/// of the leading variable(s). Each stratum is preceded by a header line
/// naming the controlling values, then rendered with the existing two-way
/// layout (grid or LIST) and statistics.
pub(super) fn n_way(
    session: &mut Session,
    ds: &SasDataset,
    req: &TableRequest,
    rows: &[usize],
    weights: Option<&[Value]>,
) -> Result<()> {
    let k = req.vars.len();
    // Resolve all columns once.
    let cols: Vec<Vec<Value>> = req
        .vars
        .iter()
        .map(|v| find_var(ds, v).and_then(|i| decode_column(ds, i)))
        .collect::<Result<_>>()?;
    let names: Vec<String> = req
        .vars
        .iter()
        .map(|v| {
            find_var(ds, v)
                .map(|i| ds.vars[i].name.clone())
                .unwrap_or_else(|_| v.clone())
        })
        .collect();

    // The leading vars (all but the last two) define the strata.
    let lead = k - 2;
    let row_col = &cols[k - 2];
    let col_col = &cols[k - 1];
    let row_name = &names[k - 2];
    let col_name = &names[k - 1];

    let keep = |v: &Value| req.missing || !v.is_missing();

    // Distinct stratum keys (tuple of leading values) in sas_cmp order, only
    // over rows that pass the keep filter on the stratum vars and have a usable
    // weight.
    let lead_cols: Vec<&Vec<Value>> = (0..lead).map(|j| &cols[j]).collect();
    let mut stratum_rows: Vec<usize> = Vec::new();
    for &i in rows {
        if obs_weight(weights, i).is_none() {
            continue;
        }
        if (0..lead).all(|j| keep(&cols[j][i])) {
            stratum_rows.push(i);
        }
    }
    let strata = common::group_by_keys(&lead_cols, ds.n_obs());
    // group_by_keys walks all rows; restrict each stratum to our `rows` subset.
    let row_set: std::collections::HashSet<usize> = stratum_rows.iter().copied().collect();

    for (key, all_grp_rows) in &strata {
        let grp_rows: Vec<usize> = all_grp_rows
            .iter()
            .copied()
            .filter(|i| row_set.contains(i))
            .collect();
        if grp_rows.is_empty() {
            continue;
        }
        // Stratum header: lead1=val1 lead2=val2 ...
        let header: Vec<String> = (0..lead)
            .map(|j| format!("{}={}", names[j], category_label(&key[j])))
            .collect();
        session
            .listing
            .write_line(&format!("Controlling for {}", header.join(" ")));
        session.listing.blank();

        // Build the two-way frequency matrix for this stratum.
        let row_vals = distinct_axis(row_col, &grp_rows, req.missing, weights);
        let col_vals = distinct_axis(col_col, &grp_rows, req.missing, weights);
        let nr = row_vals.len();
        let nc = col_vals.len();
        let mut freq = vec![vec![0.0_f64; nc]; nr];
        for &i in &grp_rows {
            let Some(w) = obs_weight(weights, i) else {
                continue;
            };
            let rv = &row_col[i];
            let cv = &col_col[i];
            if !keep(rv) || !keep(cv) {
                continue;
            }
            let r = row_vals
                .iter()
                .position(|x| x.sas_cmp(rv) == Ordering::Equal);
            let c = col_vals
                .iter()
                .position(|x| x.sas_cmp(cv) == Ordering::Equal);
            if let (Some(r), Some(c)) = (r, c) {
                freq[r][c] += w;
            }
        }

        render_two_way(
            session, req, row_name, col_name, &row_vals, &col_vals, &freq,
        );
        session.listing.blank();
    }

    Ok(())
}
