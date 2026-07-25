use super::*;

// ───────────────────────── formatting ─────────────────────────

/// Format a correlation r to 5 decimals, SAS-style. Missing → ".".
pub(super) fn fmt_r(r: Option<f64>) -> String {
    match r {
        Some(v) => format!("{v:.5}"),
        None => ".".to_string(),
    }
}

/// Format a two-sided p-value SAS-style: `<.0001`, else 4 decimals. None
/// (undefined, e.g. on an exact-1 diagonal) → empty cell.
// Divergence volontaire avec `common::fmt_p` : CORR affiche une cellule
// vide (pas `.`) pour une p-value manquante.
pub(super) fn fmt_p(p: Option<f64>) -> String {
    match p {
        None => String::new(),
        Some(v) => {
            if v < 0.0001 {
                "<.0001".to_string()
            } else {
                format!("{v:.4}")
            }
        }
    }
}

pub(super) fn emit_simple_statistics(
    session: &mut Session,
    ds: &crate::dataset::SasDataset,
    analysis_cols: &[usize],
    decoded: &std::collections::HashMap<usize, Vec<Value>>,
    n_obs: usize,
) {
    centered(session, "Simple Statistics");
    session.listing.blank();

    let headers: Vec<String> = vec![
        "Variable".into(),
        "N".into(),
        "Mean".into(),
        "Std Dev".into(),
        "Sum".into(),
        "Minimum".into(),
        "Maximum".into(),
    ];
    let aligns = vec![
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];

    let all_rows: Vec<usize> = (0..n_obs).collect();
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(analysis_cols.len());
    for &c in analysis_cols {
        let col = &decoded[&c];
        let (xs, _nmiss) = partition_numeric(col, &all_rows);
        let n = xs.len();
        let mean = if n > 0 {
            Some(xs.iter().sum::<f64>() / n as f64)
        } else {
            None
        };
        let sum = xs.iter().sum::<f64>();
        let std = sample_std(&xs);
        let min = xs.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

        let cell_opt = |v: Option<f64>| -> String {
            match v {
                Some(f) => format_best(f, 12),
                None => ".".to_string(),
            }
        };

        rows.push(vec![
            ds.vars[c].name.clone(),
            format!("{n}"),
            cell_opt(mean),
            cell_opt(std),
            format_best(sum, 12),
            cell_opt(if n > 0 { Some(min) } else { None }),
            cell_opt(if n > 0 { Some(max) } else { None }),
        ]);
    }

    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
}

pub(super) fn emit_correlations(
    session: &mut Session,
    ds: &crate::dataset::SasDataset,
    row_cols: &[usize],
    col_cols: &[usize],
    heading: &str,
    prob_line: &str,
    cells: &[Vec<Cell>],
    noprob: bool,
) {
    centered(session, heading);
    if !noprob {
        centered(session, prob_line);
    }
    session.listing.blank();

    let nr = row_cols.len();
    let nc = col_cols.len();
    let rmat: Vec<Vec<Option<f64>>> =
        cells.iter().map(|row| row.iter().map(|c| c.r).collect()).collect();
    let pmat: Vec<Vec<Option<f64>>> =
        cells.iter().map(|row| row.iter().map(|c| c.p).collect()).collect();
    let nmat: Vec<Vec<usize>> =
        cells.iter().map(|row| row.iter().map(|c| c.n).collect()).collect();

    // Decide whether to print the per-cell N line: only when pairwise N
    // differs across the matrix (SAS prints N only when observations vary).
    let max_n = nmat.iter().flatten().copied().max().unwrap_or(0);
    let any_n_differs = nmat.iter().flatten().any(|&n| n != max_n);

    // Build the table. Column 0 is the row-variable label; each subsequent
    // column is one VAR. Each matrix row expands into up to 3 table rows:
    // r, prob (unless noprob), and N (only when any_n_differs).
    let mut headers: Vec<String> = Vec::with_capacity(nc + 1);
    headers.push(String::new());
    let mut aligns: Vec<Align> = Vec::with_capacity(nc + 1);
    aligns.push(Align::Left);
    for &cc in col_cols {
        headers.push(ds.vars[cc].name.clone());
        aligns.push(Align::Right);
    }

    let mut rows: Vec<Vec<String>> = Vec::new();
    for i in 0..nr {
        // r line, labelled with the row variable.
        let mut rline = vec![ds.vars[row_cols[i]].name.clone()];
        for j in 0..nc {
            rline.push(fmt_r(rmat[i][j]));
        }
        rows.push(rline);

        if !noprob {
            let mut pline = vec![String::new()];
            for j in 0..nc {
                pline.push(fmt_p(pmat[i][j]));
            }
            rows.push(pline);
        }

        if any_n_differs {
            let mut nline = vec![String::new()];
            for j in 0..nc {
                nline.push(format!("{}", nmat[i][j]));
            }
            rows.push(nline);
        }
    }

    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
}
