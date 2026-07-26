use super::*;

// ───────────────────── One-way execute helpers ─────────────────────

/// Print the listing page header, the Class Level Information table and the
/// observation count for the one-way path.
pub(super) fn print_class_level_info_oneway(
    session: &mut Session,
    ast: &AnovaAst,
    ds: &crate::dataset::SasDataset,
    n_obs: usize,
) -> Result<()> {
    session.listing.page_header();
    centered(session, "The ANOVA Procedure");
    session.listing.blank();

    centered(session, "Class Level Information");
    session.listing.blank();

    let cli_headers: Vec<String> = vec!["Class".into(), "Levels".into(), "Values".into()];
    let cli_aligns = vec![Align::Left, Align::Right, Align::Left];
    let mut cli_rows: Vec<Vec<String>> = Vec::new();

    // Decode CLASS columns and collect distinct values
    let mut class_col_data: Vec<(String, Vec<Value>)> = Vec::new();
    for class_var in &ast.class_vars {
        let col_idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(class_var))
            .unwrap(); // already validated above
        let col = decode_column(ds, col_idx)?;

        // Collect distinct non-missing values, sorted by sas_cmp
        let levels = crate::procs::lincom::class_levels(col.iter().take(n_obs));

        let values_str: Vec<String> = levels
            .iter()
            .map(|v| match v {
                Value::Char(s) => s.trim_end().to_string(),
                Value::Num(f) => format!("{f}"),
                Value::Missing(k) => k.display(),
            })
            .collect();

        cli_rows.push(vec![
            ds.vars[col_idx].name.clone(),
            format!("{}", levels.len()),
            values_str.join(" "),
        ]);

        class_col_data.push((ds.vars[col_idx].name.clone(), col));
    }

    session
        .listing
        .write_table(&cli_headers, &cli_aligns, &cli_rows);
    session.listing.blank();

    // Number of Observations
    session.listing.write_line(&format!(
        "               Number of Observations Read     {}",
        n_obs
    ));
    session.listing.blank();
    session.listing.blank();

    Ok(())
}

/// One-way ANOVA statistics for a single dependent variable.
pub(super) struct OneWayStats {
    pub(super) levels: Vec<Value>,
    pub(super) groups: Vec<Vec<f64>>,
    pub(super) y_bar: f64,
    pub(super) ssm: f64,
    pub(super) sse: f64,
    pub(super) sst: f64,
    pub(super) df_model: f64,
    pub(super) df_error: f64,
    pub(super) df_total: f64,
    pub(super) msm: f64,
    pub(super) mse: f64,
    pub(super) f_stat: f64,
    pub(super) p_f: Option<f64>,
    pub(super) r2: f64,
    pub(super) root_mse: f64,
    pub(super) cv: f64,
}

/// Resolve the dependent/CLASS columns, apply listwise deletion, group by
/// CLASS level and compute the one-way ANOVA statistics.
pub(super) fn compute_oneway_stats(
    session: &mut Session,
    ds: &crate::dataset::SasDataset,
    dep_var: &str,
    eff: &str,
    n_obs: usize,
) -> Result<OneWayStats> {
    // Find dependent column
    let dep_idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(dep_var))
        .ok_or_else(|| {
            SasError::runtime(format!("Variable {} not found.", dep_var.to_uppercase()))
        })?;
    if ds.vars[dep_idx].ty != VarType::Num {
        return Err(SasError::runtime(format!(
            "Dependent variable {} must be numeric.",
            dep_var.to_uppercase()
        )));
    }
    let dep_col = decode_column(ds, dep_idx)?;

    // Find the CLASS column for this effect
    let class_col_idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(eff))
        .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", eff.to_uppercase())))?;
    let class_col = decode_column(ds, class_col_idx)?;

    // Listwise deletion: keep rows where both dep_var and class_var are non-missing
    let mut usable_rows: Vec<usize> = Vec::new();
    for i in 0..n_obs {
        let dep_ok = matches!(value_to_num(&dep_col[i]), Some(v) if !v.is_nan());
        let cls_ok = !class_col[i].is_missing();
        if dep_ok && cls_ok {
            usable_rows.push(i);
        }
    }
    let n = usable_rows.len();

    // Group by CLASS levels (usable rows are non-missing by construction).
    let levels = crate::procs::lincom::class_levels(usable_rows.iter().map(|&r| &class_col[r]));
    let k = levels.len();

    // Collect values per group
    let mut groups: Vec<Vec<f64>> = vec![Vec::new(); k];
    for &r in &usable_rows {
        let v = &class_col[r];
        let yi = value_to_num(&dep_col[r]).unwrap();
        let gi = levels
            .iter()
            .position(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
            .unwrap();
        groups[gi].push(yi);
    }

    // Compute statistics
    let y_bar = if n > 0 {
        groups.iter().flat_map(|g| g.iter()).sum::<f64>() / n as f64
    } else {
        f64::NAN
    };

    let mut ssm = 0.0_f64;
    let mut sse = 0.0_f64;
    let mut group_means: Vec<f64> = Vec::with_capacity(k);
    for g in &groups {
        let ni = g.len();
        let y_bar_i = if ni > 0 {
            g.iter().sum::<f64>() / ni as f64
        } else {
            f64::NAN
        };
        group_means.push(y_bar_i);
        ssm += ni as f64 * (y_bar_i - y_bar).powi(2);
        sse += g.iter().map(|&y| (y - y_bar_i).powi(2)).sum::<f64>();
    }
    let sst = ssm + sse;

    let df_model = (k as f64 - 1.0).max(0.0);
    let df_error = (n as f64 - k as f64).max(0.0);
    let df_total = (n as f64 - 1.0).max(0.0);

    let msm = if df_model > 0.0 {
        ssm / df_model
    } else {
        f64::NAN
    };
    let mse = if df_error > 0.0 {
        sse / df_error
    } else {
        f64::NAN
    };
    let f_stat = if mse > 0.0 && !mse.is_nan() {
        msm / mse
    } else {
        f64::NAN
    };
    let p_f = if f_stat.is_nan() {
        None
    } else {
        Some((1.0 - f_cdf(f_stat, df_model, df_error)).clamp(0.0, 1.0))
    };

    let r2 = if sst > 0.0 { ssm / sst } else { f64::NAN };
    let root_mse = if !mse.is_nan() { mse.sqrt() } else { f64::NAN };
    let cv = if y_bar.abs() > 1e-15 && !root_mse.is_nan() {
        root_mse / y_bar.abs() * 100.0
    } else {
        f64::NAN
    };

    session
        .log
        .note(&format!("There were {} observations used.", n));

    Ok(OneWayStats {
        levels,
        groups,
        y_bar,
        ssm,
        sse,
        sst,
        df_model,
        df_error,
        df_total,
        msm,
        mse,
        f_stat,
        p_f,
        r2,
        root_mse,
        cv,
    })
}

/// Print the Analysis of Variance table, the fit statistics table, and the
/// Type I / Type III SS tables (identical for one-way).
pub(super) fn print_oneway_anova_and_fit(
    session: &mut Session,
    dep_var: &str,
    eff: &str,
    stats: &OneWayStats,
) {
    let &OneWayStats {
        y_bar,
        ssm,
        sse,
        sst,
        df_model,
        df_error,
        df_total,
        msm,
        mse,
        f_stat,
        p_f,
        r2,
        root_mse,
        cv,
        ..
    } = stats;

    // ANOVA table
    centered(session, "Analysis of Variance");
    session.listing.blank();

    let anova_headers: Vec<String> = vec![
        "Source".into(),
        "DF".into(),
        "Sum of Squares".into(),
        "Mean Square".into(),
        "F Value".into(),
        "Pr > F".into(),
    ];
    let anova_aligns = vec![
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    let f_str = if f_stat.is_nan() {
        ".".to_string()
    } else {
        fmt2(f_stat)
    };
    let anova_rows: Vec<Vec<String>> = vec![
        vec![
            "Model".into(),
            format!("{}", df_model as usize),
            fmt5(ssm),
            if msm.is_nan() { ".".into() } else { fmt5(msm) },
            f_str,
            fmt_p(p_f),
        ],
        vec![
            "Error".into(),
            format!("{}", df_error as usize),
            fmt5(sse),
            if mse.is_nan() { ".".into() } else { fmt5(mse) },
            "".into(),
            "".into(),
        ],
        vec![
            "Corrected Total".into(),
            format!("{}", df_total as usize),
            fmt5(sst),
            "".into(),
            "".into(),
            "".into(),
        ],
    ];
    session
        .listing
        .write_table(&anova_headers, &anova_aligns, &anova_rows);
    session.listing.blank();
    session.listing.blank();

    // Fit statistics table
    let dep_mean_header = format!("{} Mean", dep_var);
    let fit_headers: Vec<String> = vec![
        "R-Square".into(),
        "Coeff Var".into(),
        "Root MSE".into(),
        dep_mean_header,
    ];
    let fit_aligns = vec![Align::Right, Align::Right, Align::Right, Align::Right];
    let fit_rows: Vec<Vec<String>> = vec![vec![fmt6(r2), fmt6(cv), fmt6(root_mse), fmt6(y_bar)]];
    session
        .listing
        .write_table(&fit_headers, &fit_aligns, &fit_rows);
    session.listing.blank();
    session.listing.blank();

    // Type I SS and Type III SS (identical for one-way)
    for (ss_label, _is_type3) in [("Type I SS", false), ("Type III SS", true)] {
        let t_headers: Vec<String> = vec![
            "Source".into(),
            "DF".into(),
            ss_label.into(),
            "Mean Square".into(),
            "F Value".into(),
            "Pr > F".into(),
        ];
        let t_aligns = vec![
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        let f_str2 = if f_stat.is_nan() {
            ".".to_string()
        } else {
            fmt2(f_stat)
        };
        let t_rows: Vec<Vec<String>> = vec![vec![
            eff.to_string(),
            format!("{}", df_model as usize),
            if msm.is_nan() { ".".into() } else { fmt5(ssm) },
            if msm.is_nan() { ".".into() } else { fmt5(msm) },
            f_str2,
            fmt_p(p_f),
        ]];
        session.listing.write_table(&t_headers, &t_aligns, &t_rows);
        session.listing.blank();
        session.listing.blank();
    }
}

/// Print the MEANS section (Level of ... table) for the one-way path.
pub(super) fn print_oneway_means(session: &mut Session, eff: &str, stats: &OneWayStats) {
    let levels = &stats.levels;
    let groups = &stats.groups;

    centered(session, &format!("Level of {}", eff));
    session.listing.blank();

    let means_headers: Vec<String> =
        vec![eff.to_string(), "N".into(), "Mean".into(), "Std Dev".into()];
    let means_aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right];
    let mut means_rows: Vec<Vec<String>> = Vec::new();

    for (gi, level) in levels.iter().enumerate() {
        let level_label = match level {
            Value::Char(s) => s.trim_end().to_string(),
            Value::Num(f) => format!("{f}"),
            Value::Missing(k) => k.display(),
        };
        let n_i = groups[gi].len();
        let mean_i = if n_i > 0 {
            groups[gi].iter().sum::<f64>() / n_i as f64
        } else {
            f64::NAN
        };
        let std_i = sample_std(&groups[gi]);
        means_rows.push(vec![
            level_label,
            format!("{}", n_i),
            fmt6(mean_i),
            match std_i {
                Some(s) => fmt6(s),
                None => ".".to_string(),
            },
        ]);
    }

    session
        .listing
        .write_table(&means_headers, &means_aligns, &means_rows);
    session.listing.blank();
    session.listing.blank();
}
