use super::*;

/// Print the Analysis of Variance, fit statistics, and Type I / Type III SS
/// tables for the multiway path.
pub(super) fn print_multiway_anova_and_fit(
    session: &mut Session,
    dep_var: &str,
    model: &AnovaModel,
    fit: &MultiwayFit,
) {
    let &MultiwayFit {
        y_bar,
        sst,
        sse_full,
        ssm,
        df_model,
        df_error,
        df_total,
        msm,
        mse,
        f_model,
        p_model,
        r2,
        root_mse,
        cv,
        ..
    } = fit;
    let term_dfs = &fit.term_dfs;
    let type1 = &fit.type1;
    let type3 = &fit.type3;
    let n_terms = model.terms.len();

    // ANOVA table (Model / Error / Corrected Total).
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
    let f_str = if f_model.is_nan() {
        ".".to_string()
    } else {
        fmt2(f_model)
    };
    let anova_rows: Vec<Vec<String>> = vec![
        vec![
            "Model".into(),
            format!("{}", df_model as usize),
            fmt5(ssm),
            if msm.is_nan() { ".".into() } else { fmt5(msm) },
            f_str,
            fmt_p(p_model),
        ],
        vec![
            "Error".into(),
            format!("{}", df_error as usize),
            fmt5(sse_full),
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

    // Fit statistics.
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

    // Term labels with `*` join.
    let term_labels: Vec<String> = model.terms.iter().map(|t| t.join("*")).collect();

    // Type I SS and Type III SS tables, one row per term.
    for (ss_label, ss_vals) in [("Type I SS", &type1), ("Type III SS", &type3)] {
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
        let mut t_rows: Vec<Vec<String>> = Vec::new();
        for t in 0..n_terms {
            let df = term_dfs[t] as f64;
            let ss = ss_vals[t];
            let ms = if df > 0.0 { ss / df } else { f64::NAN };
            let f = if mse > 0.0 && !mse.is_nan() && !ms.is_nan() {
                ms / mse
            } else {
                f64::NAN
            };
            let p = if f.is_nan() || df <= 0.0 || df_error <= 0.0 {
                None
            } else {
                Some((1.0 - f_cdf(f, df, df_error)).clamp(0.0, 1.0))
            };
            t_rows.push(vec![
                term_labels[t].clone(),
                format!("{}", term_dfs[t]),
                fmt5(ss),
                if ms.is_nan() { ".".into() } else { fmt5(ms) },
                if f.is_nan() { ".".into() } else { fmt2(f) },
                fmt_p(p),
            ]);
        }
        session.listing.write_table(&t_headers, &t_aligns, &t_rows);
        session.listing.blank();
        session.listing.blank();
    }
}

/// Print the MEANS tables: main-effect marginal cell means for each requested
/// CLASS var.
pub(super) fn print_multiway_means(
    session: &mut Session,
    ast: &AnovaAst,
    fit: &MultiwayFit,
    class_cols: &std::collections::HashMap<String, (String, Vec<Value>)>,
    used_classes: &[String],
) {
    let y = &fit.y;
    let n = y.len();
    let var_levels = &fit.var_levels;
    let var_codes = &fit.var_codes;

    for mvar in &ast.means_vars {
        let up = mvar.to_uppercase();
        if !used_classes.contains(&up) {
            continue;
        }
        let levels = &var_levels[&up];
        let codes = &var_codes[&up];
        let disp = &class_cols[&up].0;

        centered(session, &format!("Level of {}", disp));
        session.listing.blank();

        let means_headers: Vec<String> =
            vec![disp.clone(), "N".into(), "Mean".into(), "Std Dev".into()];
        let means_aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right];
        let mut means_rows: Vec<Vec<String>> = Vec::new();
        for (li, level) in levels.iter().enumerate() {
            let vals: Vec<f64> = (0..n).filter(|&i| codes[i] == li).map(|i| y[i]).collect();
            let ni = vals.len();
            let mean_i = if ni > 0 {
                vals.iter().sum::<f64>() / ni as f64
            } else {
                f64::NAN
            };
            let std_i = sample_std(&vals);
            means_rows.push(vec![
                value_label(level),
                format!("{}", ni),
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
}
