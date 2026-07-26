use super::*;

// ───────────────────── One-way execute helpers ─────────────────────

/// Print the listing page header, the Class Level Information table and the
/// observation count for the one-way path.
pub(super) fn print_class_level_info_oneway(
    session: &mut Session,
    ast: &GlmAst,
    ds: &crate::dataset::SasDataset,
    n_obs: usize,
) -> Result<()> {
    session.listing.page_header();
    centered(session, "The GLM Procedure");
    session.listing.blank();

    centered(session, "Class Level Information");
    session.listing.blank();

    let cli_headers: Vec<String> = vec!["Class".into(), "Levels".into(), "Values".into()];
    let cli_aligns = vec![Align::Left, Align::Right, Align::Left];
    let mut cli_rows: Vec<Vec<String>> = Vec::new();

    let mut class_col_data: Vec<(String, Vec<Value>)> = Vec::new();
    for class_var in &ast.class_vars {
        let col_idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(class_var))
            .unwrap();
        let col = decode_column(ds, col_idx)?;

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

    session.listing.write_line(&format!(
        "               Number of Observations Read     {}",
        n_obs
    ));
    session.listing.blank();
    session.listing.blank();

    Ok(())
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
        df_error: _,
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
    let df_error = stats.df_error;

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
            f_str.clone(),
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

/// Print the Parameter Estimates table (/SOLUTION, reference-cell coding).
pub(super) fn print_oneway_solution(session: &mut Session, eff: &str, stats: &OneWayStats) {
    let k = stats.k;
    let df_error = stats.df_error;
    let mse = stats.mse;
    let levels = &stats.levels;
    let groups = &stats.groups;
    let group_means = &stats.group_means;
    let level_label = level_label_value;

    centered(session, "Parameter Estimates");
    session.listing.blank();

    let param_headers: Vec<String> = vec![
        "Parameter".into(),
        "Estimate".into(),
        "Standard Error".into(),
        "t Value".into(),
        "Pr > |t|".into(),
    ];
    let param_aligns = vec![
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    let mut param_rows: Vec<Vec<String>> = Vec::new();

    // Reference level = last level (index k-1)
    let ref_idx = k - 1;
    let n_ref = groups[ref_idx].len();
    let y_ref = group_means[ref_idx];

    // Intercept = mean of reference level
    let intercept = y_ref;
    let se_intercept = if n_ref > 0 && !mse.is_nan() {
        (mse / n_ref as f64).sqrt()
    } else {
        f64::NAN
    };
    let t_intercept = if se_intercept > 0.0 {
        intercept / se_intercept
    } else {
        f64::NAN
    };
    let p_intercept = if t_intercept.is_nan() {
        None
    } else {
        Some(2.0 * (1.0 - student_t_cdf(t_intercept.abs(), df_error)))
    };

    param_rows.push(vec![
        "Intercept".into(),
        fmt6(intercept),
        fmt6(se_intercept),
        fmt2(t_intercept),
        fmt_p(p_intercept),
    ]);

    // Effect levels: i = 0..k-2 (all except reference)
    for i in 0..k - 1 {
        let n_i = groups[i].len();
        let y_i = group_means[i];
        let estimate_i = y_i - y_ref;
        let se_i = if n_i > 0 && n_ref > 0 && !mse.is_nan() {
            (mse * (1.0 / n_i as f64 + 1.0 / n_ref as f64)).sqrt()
        } else {
            f64::NAN
        };
        let t_i = if se_i > 0.0 {
            estimate_i / se_i
        } else {
            f64::NAN
        };
        let p_i = if t_i.is_nan() {
            None
        } else {
            Some(2.0 * (1.0 - student_t_cdf(t_i.abs(), df_error)))
        };
        let lbl_i = level_label(&levels[i]);
        param_rows.push(vec![
            format!("{} {}", eff, lbl_i),
            fmt6(estimate_i),
            fmt6(se_i),
            fmt2(t_i),
            fmt_p(p_i),
        ]);
    }

    // Reference level row: "B" in SE column, estimate "0"
    let lbl_ref = level_label(&levels[ref_idx]);
    param_rows.push(vec![
        format!("{} {}", eff, lbl_ref),
        "0".into(),
        "B".into(),
        "".into(),
        "".into(),
    ]);

    session
        .listing
        .write_table(&param_headers, &param_aligns, &param_rows);
    session.listing.blank();
    session.listing.blank();
}
