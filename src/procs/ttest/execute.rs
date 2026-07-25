use super::*;

/// Mode 1 — one-sample t tests on every VAR variable.
pub(super) fn execute_one_sample(
    ast: &TTestAst,
    session: &mut Session,
    ds: &SasDataset,
    var_cols: &[usize],
    all_rows: &[usize],
    alpha: f64,
) -> Result<()> {
    let h0 = ast.proc_options.h0;
    let sides = ast.proc_options.sides;
    let show_ci = ast.proc_options.ci_explicit;
    centered(session, "One-Sample t Tests");
    session.listing.blank();

    let cl = cl_pct(alpha);
    let mut headers: Vec<String> = vec![
        "Variable".into(),
        "N".into(),
        "Mean".into(),
        "Std Dev".into(),
        "Std Err".into(),
        "Minimum".into(),
        "Maximum".into(),
    ];
    let mut aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right, Align::Right, Align::Right, Align::Right];
    if show_ci {
        // Confidence-limit columns, gated by an explicit CI= request so the
        // default listing stays byte-identical.
        headers.push(format!("{cl} CL Mean L"));
        headers.push(format!("{cl} CL Mean U"));
        headers.push(format!("{cl} CL Std L"));
        headers.push(format!("{cl} CL Std U"));
        aligns.extend([Align::Right, Align::Right, Align::Right, Align::Right]);
    }
    headers.push("t Value".into());
    headers.push(p_header(sides).into());
    aligns.push(Align::Right);
    aligns.push(Align::Right);

    let mut rows: Vec<Vec<String>> = Vec::with_capacity(var_cols.len());
    let mut ods_rows: Vec<(String, OneSampleResult)> = Vec::new();
    for &c in var_cols {
        let col = decode_column(ds, c)?;
        let (xs, _nmiss) = partition_numeric(&col, all_rows);
        let r = one_sample(&xs, h0, alpha, sides);
        let mut row = vec![
            ds.vars[c].name.clone(),
            format!("{}", r.n),
            fmt4(if r.n > 0 { Some(r.mean) } else { None }),
            fmt4(r.std),
            fmt4(r.se),
            fmt4(if r.n > 0 { Some(r.min) } else { None }),
            fmt4(if r.n > 0 { Some(r.max) } else { None }),
        ];
        if show_ci {
            row.push(fmt4(r.mean_lcl));
            row.push(fmt4(r.mean_ucl));
            row.push(fmt4(r.std_lcl));
            row.push(fmt4(r.std_ucl));
        }
        row.push(fmt4(r.t));
        row.push(fmt_p(r.p));
        rows.push(row);
        ods_rows.push((ds.vars[c].name.clone(), r));
    }
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();

    // ODS OUTPUT TTest + OUT= dataset.
    maybe_write_one_sample_output(ast, session, &ods_rows)?;
    Ok(())
}

/// Mode 2 — two-sample t tests defined by a CLASS variable with 2 levels.
pub(super) fn execute_two_sample(
    ast: &TTestAst,
    session: &mut Session,
    ds: &SasDataset,
    var_cols: &[usize],
    class_name: &str,
    all_rows: &[usize],
    alpha: f64,
) -> Result<()> {
    let sides = ast.proc_options.sides;
    let show_ci = ast.proc_options.ci_explicit;
    let class_idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(class_name))
        .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", class_name.to_uppercase())))?;
    let class_vals = decode_column(ds, class_idx)?;

    // Collect distinct non-missing class levels (sas_cmp comparison + order).
    let mut levels: Vec<Value> = Vec::new();
    for &r in all_rows {
        let v = &class_vals[r];
        if v.is_missing() {
            continue;
        }
        if !levels.iter().any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal) {
            levels.push(v.clone());
        }
    }
    levels.sort_by(|a, b| a.sas_cmp(b));
    if levels.len() != 2 {
        return Err(SasError::runtime(format!(
            "The CLASS variable {} must have exactly 2 levels.",
            class_name.to_uppercase()
        )));
    }

    let level_label = |v: &Value| -> String {
        match v {
            Value::Char(s) => s.trim_end().to_string(),
            Value::Num(f) => format!("{f}"),
            Value::Missing(_) => ".".to_string(),
        }
    };
    let label_a = level_label(&levels[0]);
    let label_b = level_label(&levels[1]);

    centered(session, "Two-Sample t Tests");
    session.listing.blank();

    let cl = cl_pct(alpha);
    let mut headers: Vec<String> = vec![
        "Variable".into(),
        "Method".into(),
    ];
    let mut aligns = vec![Align::Left, Align::Left];
    if show_ci {
        headers.push("Mean Diff".into());
        headers.push(format!("{cl} CL Diff L"));
        headers.push(format!("{cl} CL Diff U"));
        aligns.extend([Align::Right, Align::Right, Align::Right]);
    }
    headers.push("DF".into());
    headers.push("t Value".into());
    headers.push(p_header(sides).into());
    aligns.extend([Align::Right, Align::Right, Align::Right]);

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut feq_rows: Vec<Vec<String>> = Vec::new();
    let mut ods: Vec<(String, TwoSampleResult)> = Vec::new();

    for &c in var_cols {
        let col = decode_column(ds, c)?;
        let mut a: Vec<f64> = Vec::new();
        let mut b: Vec<f64> = Vec::new();
        for &r in all_rows {
            let lv = &class_vals[r];
            if lv.is_missing() {
                continue;
            }
            let group = if lv.sas_cmp(&levels[0]) == std::cmp::Ordering::Equal {
                Some(&mut a)
            } else if lv.sas_cmp(&levels[1]) == std::cmp::Ordering::Equal {
                Some(&mut b)
            } else {
                None
            };
            if let Some(g) = group {
                if let Some(x) = crate::missing::value_to_num(&col[r]) {
                    if !x.is_nan() {
                        g.push(x);
                    }
                }
            }
        }
        let res = two_sample(&a, &b, alpha, sides);
        let vname = ds.vars[c].name.clone();

        let diff = if res.diff.is_nan() { None } else { Some(res.diff) };
        let (pt, pdf, pp) = match res.pooled {
            Some((t, df, p)) => (Some(t), Some(df), Some(p)),
            None => (None, None, None),
        };
        let mut pooled_row = vec![vname.clone(), "Pooled".into()];
        if show_ci {
            pooled_row.push(fmt4(diff));
            pooled_row.push(fmt4(res.pooled_cl.map(|(l, _)| l)));
            pooled_row.push(fmt4(res.pooled_cl.map(|(_, u)| u)));
        }
        pooled_row.push(fmt4(pdf));
        pooled_row.push(fmt4(pt));
        pooled_row.push(fmt_p(pp));
        rows.push(pooled_row);

        let (st, sdf, sp) = match res.satterthwaite {
            Some((t, df, p)) => (Some(t), Some(df), Some(p)),
            None => (None, None, None),
        };
        let mut satt_row = vec![vname.clone(), "Satterthwaite".into()];
        if show_ci {
            satt_row.push(fmt4(diff));
            satt_row.push(fmt4(res.satt_cl.map(|(l, _)| l)));
            satt_row.push(fmt4(res.satt_cl.map(|(_, u)| u)));
        }
        satt_row.push(fmt4(sdf));
        satt_row.push(fmt4(st));
        satt_row.push(fmt_p(sp));
        rows.push(satt_row);

        if let Some((f, df1, df2, p)) = res.f_test {
            feq_rows.push(vec![
                vname.clone(),
                format!("{}", df1 as usize),
                format!("{}", df2 as usize),
                fmt4(Some(f)),
                fmt_p(Some(p)),
            ]);
        } else {
            feq_rows.push(vec![vname.clone(), ".".into(), ".".into(), ".".into(), ".".into()]);
        }
        ods.push((vname, res));
    }

    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();

    // Equality of Variances section.
    centered(session, "Equality of Variances");
    session.listing.blank();
    let feq_headers: Vec<String> = vec![
        "Variable".into(),
        "Num DF".into(),
        "Den DF".into(),
        "F Value".into(),
        "Pr > F".into(),
    ];
    let feq_aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right, Align::Right];
    session.listing.write_table(&feq_headers, &feq_aligns, &feq_rows);
    session.listing.blank();

    maybe_write_two_sample_output(ast, session, &ods, &label_a, &label_b)?;
    Ok(())
}

/// Mode 3 — paired t tests on each (x, y) difference.
pub(super) fn execute_paired(
    ast: &TTestAst,
    session: &mut Session,
    ds: &SasDataset,
    all_rows: &[usize],
    alpha: f64,
    find_col: &dyn Fn(&str) -> Result<usize>,
) -> Result<()> {
    let sides = ast.proc_options.sides;
    let show_ci = ast.proc_options.ci_explicit;
    centered(session, "Paired t Tests");
    session.listing.blank();

    let cl = cl_pct(alpha);
    let mut headers: Vec<String> = vec![
        "Variable".into(),
        "N Pairs".into(),
        "Mean".into(),
        "Std Dev".into(),
        "Std Err".into(),
    ];
    let mut aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right, Align::Right];
    if show_ci {
        headers.push(format!("{cl} CL Mean L"));
        headers.push(format!("{cl} CL Mean U"));
        aligns.extend([Align::Right, Align::Right]);
    }
    headers.push("DF".into());
    headers.push("t Value".into());
    headers.push(p_header(sides).into());
    aligns.extend([Align::Right, Align::Right, Align::Right]);

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut ods: Vec<(String, OneSampleResult)> = Vec::new();

    for (xn, yn) in &ast.paired_vars {
        let xi = find_col(xn)?;
        let yi = find_col(yn)?;
        if ds.vars[xi].ty != VarType::Num || ds.vars[yi].ty != VarType::Num {
            return Err(SasError::runtime(format!(
                "PAIRED variables {} and {} must be numeric.",
                xn.to_uppercase(),
                yn.to_uppercase()
            )));
        }
        let xc = decode_column(ds, xi)?;
        let yc = decode_column(ds, yi)?;
        let mut diffs: Vec<f64> = Vec::new();
        for &r in all_rows {
            match (
                crate::missing::value_to_num(&xc[r]),
                crate::missing::value_to_num(&yc[r]),
            ) {
                (Some(x), Some(y)) if !x.is_nan() && !y.is_nan() => diffs.push(x - y),
                _ => {}
            }
        }
        let res = one_sample(&diffs, 0.0, alpha, sides);
        let label = format!("{}-{}", ds.vars[xi].name, ds.vars[yi].name);
        let mut row = vec![
            label.clone(),
            format!("{}", res.n),
            fmt4(if res.n > 0 { Some(res.mean) } else { None }),
            fmt4(res.std),
            fmt4(res.se),
        ];
        if show_ci {
            row.push(fmt4(res.mean_lcl));
            row.push(fmt4(res.mean_ucl));
        }
        row.push(fmt4(if res.n >= 1 { Some(res.df) } else { None }));
        row.push(fmt4(res.t));
        row.push(fmt_p(res.p));
        rows.push(row);
        ods.push((label, res));
    }

    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();

    maybe_write_paired_output(ast, session, &ods)?;
    Ok(())
}
