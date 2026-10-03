use super::*;

/// Emit the standard BY-group heading line (`var=val var2=val2`), matching the
/// MEANS/UNIVARIATE rendering.
pub(super) fn emit_by_heading(session: &mut Session, by_names: &[String], by_key: &[Value]) {
    let parts: Vec<String> = by_names
        .iter()
        .zip(by_key)
        .map(|(name, v)| format!("{}={}", name, category_label(v)))
        .collect();
    session.listing.write_line(&parts.join(" "));
    session.listing.blank();
}

/// J02-P1 (issue #16) — statistics selected by the OUTPUT statement, computed
/// for one table request. Pearson chi-square, DF and its upper-tail p-value;
/// `None` per value when the table is degenerate (SAS stores missing).
pub(super) struct OutputChisq {
    pub pchi: Option<f64>,
    pub pchi_df: Option<f64>,
    pub p_pchi: Option<f64>,
}

/// Compute the OUTPUT-statement CHISQ statistics for a one-way request
/// (goodness-of-fit against equal proportions — same statistic as the
/// `chisq_one_way_block` listing block).
fn output_chisq_one_way(cats: &[Category]) -> OutputChisq {
    let k = cats.len();
    let n: f64 = cats.iter().map(|c| c.freq).sum();
    if k < 2 || n <= 0.0 {
        return OutputChisq {
            pchi: None,
            pchi_df: None,
            p_pchi: None,
        };
    }
    let exp = n / k as f64;
    let mut chisq = 0.0_f64;
    for c in cats {
        let d = c.freq - exp;
        chisq += d * d / exp;
    }
    let df = (k - 1) as f64;
    let p = chisq_sf(chisq, df);
    OutputChisq {
        pchi: Some(chisq),
        pchi_df: Some(df),
        p_pchi: Some(p),
    }
}

/// J02-P1 (issue #16) / J02-P2 (issue #17) — build and write the dataset
/// named by `output out=<ds> chisq;` and/or `... fisher;` for the LAST
/// TABLES request, following the SAS FREQ naming convention sanctioned by
/// decision a43c8c14: `_PCHI_` (Pearson chi-square), `_PCHI_DF_` (DF),
/// `P_PCHI` (p-value), puis `XP2_FISH` (p exacte bilatérale de Fisher) et
/// `LXP2_FISH` (logarithme népérien de XP2_FISH), one observation. Log
/// NOTE mirroring the OUT= one-way path.
///
/// Documented divergence: with BY processing SAS writes one observation per
/// BY group (with the BY columns); sasrs writes the statistics of the LAST
/// BY group only, like the existing one-way OUT= path (overwrite).
pub(super) fn write_stats_output(
    session: &mut Session,
    ds: &SasDataset,
    req: &TableRequest,
    rows: &[usize],
    weights: Option<&[Value]>,
    oreq: &FreqOutput,
) -> Result<()> {
    // Column names per decision a43c8c14 (SAS/corpus convention); order is
    // fixed (CHISQ block then FISHER block) whatever the keyword order.
    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();
    let push_num =
        |name: &str, v: Option<f64>, columns: &mut Vec<Column>, vars: &mut Vec<VarMeta>| {
            columns.push(Series::new(name.into(), vec![v]).into());
            vars.push(num_var_meta(name));
        };

    match req.vars.len() {
        1 => {
            if oreq.fisher {
                return Err(SasError::runtime(
                    "The FISHER statistic of the PROC FREQ OUTPUT statement requires a two-way table request in sasrs.",
                ));
            }
            let col_idx = find_var(ds, &req.vars[0])?;
            let col = decode_column(ds, col_idx)?;
            let (cats, _) = tally(&col, rows, req.missing, weights);
            let stats = output_chisq_one_way(&cats);
            push_num("_PCHI_", stats.pchi, &mut columns, &mut vars);
            push_num("_PCHI_DF_", stats.pchi_df, &mut columns, &mut vars);
            push_num("P_PCHI", stats.p_pchi, &mut columns, &mut vars);
        }
        2 => {
            // Same frequency-matrix construction as two_way (weighted cells,
            // sas_cmp axis ordering), then the shared computations.
            let row_idx = find_var(ds, &req.vars[0])?;
            let col_idx = find_var(ds, &req.vars[1])?;
            let row_col = decode_column(ds, row_idx)?;
            let col_col = decode_column(ds, col_idx)?;
            let keep = |v: &Value| req.missing || !v.is_missing();
            let row_vals = distinct_axis(&row_col, rows, req.missing, weights);
            let col_vals = distinct_axis(&col_col, rows, req.missing, weights);
            let nr = row_vals.len();
            let nc = col_vals.len();
            let mut freq = vec![vec![0.0_f64; nc]; nr];
            for &i in rows {
                let Some(w) = obs_weight(weights, i) else {
                    continue;
                };
                if !keep(&row_col[i]) || !keep(&col_col[i]) {
                    continue;
                }
                let r = row_vals
                    .iter()
                    .position(|x| x.sas_cmp(&row_col[i]) == Ordering::Equal);
                let c = col_vals
                    .iter()
                    .position(|x| x.sas_cmp(&col_col[i]) == Ordering::Equal);
                if let (Some(r), Some(c)) = (r, c) {
                    freq[r][c] += w;
                }
            }
            let row_tot: Vec<f64> = (0..nr).map(|r| freq[r].iter().sum()).collect();
            let col_tot: Vec<f64> = (0..nc).map(|c| (0..nr).map(|r| freq[r][c]).sum()).collect();
            let grand: f64 = row_tot.iter().sum();

            if oreq.chisq {
                let res = two_way_chisq_compute(&freq, &row_tot, &col_tot, grand);
                push_num(
                    "_PCHI_",
                    res.computable.then_some(res.pearson),
                    &mut columns,
                    &mut vars,
                );
                push_num(
                    "_PCHI_DF_",
                    res.computable.then_some(res.df),
                    &mut columns,
                    &mut vars,
                );
                push_num(
                    "P_PCHI",
                    res.computable.then_some(res.p_pearson),
                    &mut columns,
                    &mut vars,
                );
            }

            // J02-P2 (issue #17) — OUTPUT FISHER : test exact sur comptages
            // entiers (les poids éventuels sont arrondis comme pour le
            // listing), p exacte bilatérale + ln(p).
            if oreq.fisher {
                let ifreq = round_matrix(&freq);
                let irow: Vec<usize> = ifreq.iter().map(|r| r.iter().sum()).collect();
                let icol: Vec<usize> = (0..nc)
                    .map(|c| (0..nr).map(|r| ifreq[r][c]).sum())
                    .collect();
                let igrand: usize = irow.iter().sum();
                let computable = igrand > 0 && nr >= 2 && nc >= 2;
                let (xp2, lxp2) = if computable {
                    let p = fisher_exact_p_two(&ifreq, &irow, &icol, igrand);
                    (Some(p), Some(p.ln()))
                } else {
                    (None, None)
                };
                push_num("XP2_FISH", xp2, &mut columns, &mut vars);
                push_num("LXP2_FISH", lxp2, &mut columns, &mut vars);
            }
        }
        _ => {
            return Err(SasError::runtime(
                "The OUTPUT statement of PROC FREQ is not supported for n-way table requests in sasrs.",
            ));
        }
    }

    let out_ds = SasDataset {
        df: DataFrame::new(columns)?,
        vars,
    };

    let out_libref = oreq.out.libref_or_work();
    let out_table = oreq.out.name.to_uppercase();
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
