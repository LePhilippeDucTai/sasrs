use super::*;

/// One-way frequency table for a single variable, over `rows` (one BY group
/// or all rows). `weights` carries the WEIGHT column when present.
pub(super) fn one_way(
    session: &mut Session,
    ds: &SasDataset,
    req: &TableRequest,
    rows: &[usize],
    weights: Option<&[Value]>,
) -> Result<()> {
    let col_idx = find_var(ds, &req.vars[0])?;
    let col = decode_column(ds, col_idx)?;
    let var_name = ds.vars[col_idx].name.clone();

    let (cats, n_missing) = tally(&col, rows, req.missing, weights);

    // Denominator: the sum of the category frequencies already gives the
    // right value — with MISSING the missing categories are included in
    // `cats`, otherwise they are excluded (so denom = non-missing count).
    let denom: f64 = cats.iter().map(|c| c.freq).sum();

    // Listing table. Display options suppress whole columns:
    //   NOFREQ    -> drop Frequency
    //   NOPERCENT -> drop Percent (and Cumulative Percent)
    //   NOCUM     -> drop Cumulative Frequency and Cumulative Percent
    // The default (no options) keeps all five columns exactly as before.
    let show_freq = !req.nofreq;
    let show_pct = !req.nopercent;
    let show_cum_freq = !req.nocum;
    let show_cum_pct = !req.nocum && !req.nopercent;

    let mut headers = vec![var_name.clone()];
    let mut aligns = vec![if ds.vars[col_idx].ty == VarType::Num {
        Align::Right
    } else {
        Align::Left
    }];
    if show_freq {
        headers.push("Frequency".to_string());
        aligns.push(Align::Right);
    }
    if show_pct {
        headers.push("Percent".to_string());
        aligns.push(Align::Right);
    }
    if show_cum_freq {
        headers.push("Cumulative Frequency".to_string());
        aligns.push(Align::Right);
    }
    if show_cum_pct {
        headers.push("Cumulative Percent".to_string());
        aligns.push(Align::Right);
    }

    let mut out_rows: Vec<Vec<String>> = Vec::with_capacity(cats.len());
    let mut cum_freq = 0.0_f64;
    for c in &cats {
        cum_freq += c.freq;
        let pct = if denom > 0.0 {
            100.0 * c.freq / denom
        } else {
            0.0
        };
        let cum_pct = if denom > 0.0 {
            100.0 * cum_freq / denom
        } else {
            0.0
        };
        let mut row = vec![category_label(&c.value)];
        if show_freq {
            row.push(fmt_freq(c.freq));
        }
        if show_pct {
            row.push(fmt_pct(pct));
        }
        if show_cum_freq {
            row.push(fmt_freq(cum_freq));
        }
        if show_cum_pct {
            row.push(fmt_pct(cum_pct));
        }
        out_rows.push(row);
    }

    session.listing.write_table(&headers, &aligns, &out_rows);

    // Frequency Missing line (only when missings are excluded).
    if !req.missing && n_missing > 0.0 {
        session.listing.blank();
        session
            .listing
            .write_line(&format!("Frequency Missing = {}", fmt_freq(n_missing)));
    }

    // CHISQ one-way: goodness-of-fit against equal proportions.
    if req.chisq {
        chisq_one_way_block(session, &cats);
    }

    // OUT= dataset (one-way only).
    if let Some(out) = &req.out {
        write_one_way_out(session, ds, col_idx, &cats, denom, out)?;
    }

    Ok(())
}

/// One-way goodness-of-fit chi-square test against equal proportions
/// (TESTP= defaulting to 1/k per category). Statistic Σ(obs-exp)²/exp with
/// exp = N/k, DF = k-1. Degenerate cases (k < 2 or N = 0) are skipped with a
/// graceful note.
pub(super) fn chisq_one_way_block(session: &mut Session, cats: &[Category]) {
    let k = cats.len();
    let n: f64 = cats.iter().map(|c| c.freq).sum();

    session.listing.blank();
    if k < 2 || n <= 0.0 {
        session
            .listing
            .write_line("Chi-Square Test for Equal Proportions is not computable for this table.");
        return;
    }

    let exp = n / k as f64;
    let mut chisq = 0.0_f64;
    for c in cats {
        let d = c.freq - exp;
        chisq += d * d / exp;
    }
    let df = (k - 1) as f64;
    let p = chisq_sf(chisq, df);

    session
        .listing
        .write_line("Chi-Square Test for Equal Proportions");
    session.listing.blank();
    let headers = vec!["Statistic".to_string(), "Value".to_string()];
    let aligns = vec![Align::Left, Align::Right];
    let rows = vec![
        vec!["Chi-Square".to_string(), format!("{chisq:.4}")],
        vec!["DF".to_string(), format!("{}", k - 1)],
        vec!["Pr > ChiSq".to_string(), fmt_chisq_p(p)],
    ];
    session.listing.write_table(&headers, &aligns, &rows);
}

/// Build and write the OUT= dataset for a one-way table: columns <var>,
/// COUNT, PERCENT.
pub(super) fn write_one_way_out(
    session: &mut Session,
    ds: &SasDataset,
    col_idx: usize,
    cats: &[Category],
    denom: f64,
    out: &DatasetRef,
) -> Result<()> {
    let meta = &ds.vars[col_idx];
    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();

    // Category column (same type/meta as the input variable).
    let cat_series = match meta.ty {
        VarType::Num => {
            let vals: Vec<Option<f64>> =
                cats.iter().map(|c| value_to_num(&c.value)).collect();
            Series::new(meta.name.as_str().into(), vals)
        }
        VarType::Char => {
            let vals: Vec<Option<String>> = cats
                .iter()
                .map(|c| match &c.value {
                    Value::Char(s) if s.trim_end().is_empty() => None,
                    Value::Char(s) => Some(s.trim_end().to_string()),
                    _ => None,
                })
                .collect();
            Series::new(meta.name.as_str().into(), vals)
        }
    };
    columns.push(cat_series.into());
    vars.push(meta.clone());

    // COUNT.
    let count_vals: Vec<Option<f64>> = cats.iter().map(|c| Some(c.freq)).collect();
    columns.push(Series::new("COUNT".into(), count_vals).into());
    vars.push(num_var_meta("COUNT"));

    // PERCENT.
    let pct_vals: Vec<Option<f64>> = cats
        .iter()
        .map(|c| {
            Some(if denom > 0.0 {
                100.0 * c.freq / denom
            } else {
                0.0
            })
        })
        .collect();
    columns.push(Series::new("PERCENT".into(), pct_vals).into());
    vars.push(num_var_meta("PERCENT"));

    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars };

    let out_libref = out.libref_or_work();
    let out_table = out.name.to_uppercase();
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
