use super::*;

/// Print the listing page header and the Model Information table.
pub(super) fn print_model_information(
    session: &mut Session,
    in_libref: &str,
    in_table: &str,
    resp_name: &str,
    dist: &Distribution,
    lf: &LinkFunction,
    n_total: f64,
) {
    session.listing.page_header();
    centered(session, "The GENMOD Procedure");
    session.listing.blank();

    centered(session, "Model Information");
    session.listing.blank();

    let ds_display = format!("{}.{}", in_libref, in_table);
    let dist_name = match dist {
        Distribution::Poisson => "Poisson",
        Distribution::Binomial => "Binomial",
        Distribution::Normal => "Normal",
        Distribution::Gamma => "Gamma",
    };
    let link_name = match lf {
        LinkFunction::Log => "Log",
        LinkFunction::Logit => "Logit",
        LinkFunction::Identity => "Identity",
        LinkFunction::Reciprocal => "Reciprocal",
    };

    let info_headers: Vec<String> = vec!["".into(), "".into()];
    let info_aligns = vec![Align::Left, Align::Left];
    let info_rows: Vec<Vec<String>> = vec![
        vec!["Data Set".into(), ds_display],
        vec!["Response Variable".into(), resp_name.to_string()],
        vec!["Distribution".into(), dist_name.into()],
        vec!["Link Function".into(), link_name.into()],
        vec!["Dependent Variable".into(), resp_name.to_string()],
        vec!["Observations Used".into(), (n_total as i64).to_string()],
    ];
    session
        .listing
        .write_table(&info_headers, &info_aligns, &info_rows);
    session.listing.blank();
}

/// Print the Class Level Information table (CLASS variables present).
pub(super) fn print_class_level_information(session: &mut Session, design_terms: &[DesignTerm]) {
    centered(session, "Class Level Information");
    session.listing.blank();

    let cli_headers: Vec<String> =
        vec!["Class".into(), "Levels".into(), "Values".into()];
    let cli_aligns = vec![Align::Left, Align::Right, Align::Left];
    let mut cli_rows: Vec<Vec<String>> = Vec::new();
    for term in design_terms {
        if let DesignTerm::Class { name, levels, .. } = term {
            let values_str: Vec<String> =
                levels.iter().map(class_level_label).collect();
            cli_rows.push(vec![
                name.clone(),
                format!("{}", levels.len()),
                values_str.join(" "),
            ]);
        }
    }
    session
        .listing
        .write_table(&cli_headers, &cli_aligns, &cli_rows);
    session.listing.blank();
}

/// Print the Response Profile table (Binomial only).
pub(super) fn print_response_profile(
    session: &mut Session,
    resp_name: &str,
    binomial_event_label: &Option<String>,
    binomial_nonevent_label: &Option<String>,
    binomial_n_event_total: f64,
    binomial_n_nonevent_total: f64,
) {
    centered(session, "Response Profile");
    session.listing.blank();

    let el = binomial_event_label.as_deref().unwrap_or("");
    let nel = binomial_nonevent_label.as_deref().unwrap_or("");

    let rp_headers: Vec<String> = vec![
        "Ordered Value".into(),
        resp_name.to_string(),
        "Total Frequency".into(),
    ];
    let rp_aligns = vec![Align::Right, Align::Left, Align::Right];
    let rp_rows: Vec<Vec<String>> = vec![
        vec![
            "1".into(),
            el.to_string(),
            (binomial_n_event_total as i64).to_string(),
        ],
        vec![
            "2".into(),
            nel.to_string(),
            (binomial_n_nonevent_total as i64).to_string(),
        ],
    ];
    session
        .listing
        .write_table(&rp_headers, &rp_aligns, &rp_rows);
    session.listing.blank();
    session.listing.write_line(&format!(
        "PROC GENMOD is modeling the probability that {}={}.",
        resp_name, el
    ));
    session.listing.blank();
}

/// Print the Model Convergence Status block.
pub(super) fn print_convergence_status(session: &mut Session) {
    centered(session, "Model Convergence Status");
    session.listing.blank();
    session
        .listing
        .write_line("     Convergence criterion (GCONV=1E-8) satisfied.");
    session.listing.blank();
}

/// Print the Criteria For Assessing Goodness Of Fit table.
pub(super) fn print_gof(session: &mut Session, crit: &FitCriteria) {
    let &FitCriteria {
        log_lik,
        deviance,
        pearson,
        scaled_deviance,
        scaled_pearson,
        df_gof,
        aic,
        aicc,
        bic,
    } = crit;

    centered(session, "Criteria For Assessing Goodness Of Fit");
    session.listing.blank();

    let gof_headers: Vec<String> = vec![
        "Criterion".into(),
        "DF".into(),
        "Value".into(),
        "Value/DF".into(),
    ];
    let gof_aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right];

    let df_str = df_gof.to_string();
    let blank = "".to_string();

    let gof_rows: Vec<Vec<String>> = vec![
        vec![
            "Deviance".into(),
            df_str.clone(),
            fmt4(deviance),
            fmt4(deviance / df_gof as f64),
        ],
        vec![
            "Scaled Deviance".into(),
            df_str.clone(),
            fmt4(scaled_deviance),
            fmt4(scaled_deviance / df_gof as f64),
        ],
        vec![
            "Pearson Chi-Square".into(),
            df_str.clone(),
            fmt4(pearson),
            fmt4(pearson / df_gof as f64),
        ],
        vec![
            "Scaled Pearson X2".into(),
            df_str.clone(),
            fmt4(scaled_pearson),
            fmt4(scaled_pearson / df_gof as f64),
        ],
        vec!["Log Likelihood".into(), blank.clone(), fmt4(log_lik), blank.clone()],
        vec!["Full Log Likelihood".into(), blank.clone(), fmt4(log_lik), blank.clone()],
        vec!["AIC (smaller is better)".into(), blank.clone(), fmt4(aic), blank.clone()],
        vec!["AICC (smaller is better)".into(), blank.clone(), fmt4(aicc), blank.clone()],
        vec!["BIC (smaller is better)".into(), blank.clone(), fmt4(bic), blank.clone()],
    ];

    session
        .listing
        .write_table(&gof_headers, &gof_aligns, &gof_rows);
    session.listing.blank();
}

/// Print the Analysis Of Maximum Likelihood Parameter Estimates table.
#[allow(clippy::too_many_arguments)]
pub(super) fn print_parameter_estimates(
    session: &mut Session,
    design_terms: &[DesignTerm],
    beta: &[f64],
    se_beta: &[f64],
    wald_chi2: &[f64],
    wald_p: &[f64],
    dist: &Distribution,
    scale: &ScaleInfo,
) {
    let p_param = beta.len();
    let scale_est = scale.scale_est;
    let scale_df = scale.scale_df;
    let se_scale = scale.se_scale;

    centered(session, "Analysis Of Maximum Likelihood Parameter Estimates");
    session.listing.blank();

    let amle_headers: Vec<String> = vec![
        "Parameter".into(),
        "DF".into(),
        "Estimate".into(),
        "Standard Error".into(),
        "Wald 95% Confidence Limits Lower".into(),
        "Wald 95% Confidence Limits Upper".into(),
        "Wald Chi-Square".into(),
        "Pr > ChiSq".into(),
    ];
    let amle_aligns = vec![
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];

    let mut amle_rows: Vec<Vec<String>> = Vec::with_capacity(p_param + 1);

    // Intercept row.
    {
        let ci_lower = beta[0] - 1.96 * se_beta[0];
        let ci_upper = beta[0] + 1.96 * se_beta[0];
        amle_rows.push(vec![
            "Intercept".to_string(),
            "1".into(),
            fmt4(beta[0]),
            fmt4(se_beta[0]),
            fmt4(ci_lower),
            fmt4(ci_upper),
            fmt4(wald_chi2[0]),
            fmt_p_opt(wald_p[0]),
        ]);
    }

    // Predictor / CLASS rows. `col` walks the β vector starting after the
    // intercept; CLASS factors emit one row per non-reference level (label
    // "classvar level") followed by a reference-level row (estimate 0, DF 0,
    // ref = last level). Continuous predictors emit a single row labelled by
    // the variable name (byte-identical to the pre-CLASS layout).
    let mut col = 1usize;
    for term in design_terms {
        match term {
            DesignTerm::Continuous { name, .. } => {
                let j = col;
                let ci_lower = beta[j] - 1.96 * se_beta[j];
                let ci_upper = beta[j] + 1.96 * se_beta[j];
                amle_rows.push(vec![
                    name.clone(),
                    "1".into(),
                    fmt4(beta[j]),
                    fmt4(se_beta[j]),
                    fmt4(ci_lower),
                    fmt4(ci_upper),
                    fmt4(wald_chi2[j]),
                    fmt_p_opt(wald_p[j]),
                ]);
                col += 1;
            }
            DesignTerm::Class { name, levels, .. } => {
                let nref = levels.len() - 1;
                for li in 0..nref {
                    let j = col + li;
                    let lbl = format!("{} {}", name, class_level_label(&levels[li]));
                    let ci_lower = beta[j] - 1.96 * se_beta[j];
                    let ci_upper = beta[j] + 1.96 * se_beta[j];
                    amle_rows.push(vec![
                        lbl,
                        "1".into(),
                        fmt4(beta[j]),
                        fmt4(se_beta[j]),
                        fmt4(ci_lower),
                        fmt4(ci_upper),
                        fmt4(wald_chi2[j]),
                        fmt_p_opt(wald_p[j]),
                    ]);
                }
                // Reference level row (last level): estimate 0, DF 0.
                let ref_lbl =
                    format!("{} {}", name, class_level_label(&levels[nref]));
                amle_rows.push(vec![
                    ref_lbl,
                    "0".into(),
                    fmt4(0.0),
                    fmt4(0.0),
                    fmt4(0.0),
                    fmt4(0.0),
                    ".".into(),
                    ".".into(),
                ]);
                col += nref;
            }
        }
    }

    // Scale row. Normal: Scale=σ̂ with a Wald CI from se_scale. Gamma: Scale
    // is the 1/φ estimate (Pearson-based); we report the estimate and its CI
    // from se_scale (≈ Scale/√(2·df)). Poisson/Binomial: fixed at 1.
    let (scale_ci_lower, scale_ci_upper) =
        if *dist == Distribution::Normal || *dist == Distribution::Gamma {
            (
                fmt4((scale_est - 1.96 * se_scale).max(0.0)),
                fmt4(scale_est + 1.96 * se_scale),
            )
        } else {
            (fmt4(1.0), fmt4(1.0))
        };

    amle_rows.push(vec![
        "Scale".into(),
        scale_df.to_string(),
        fmt4(scale_est),
        fmt4(se_scale),
        scale_ci_lower,
        scale_ci_upper,
        ".".into(), // no Wald for scale row
        ".".into(),
    ]);

    session
        .listing
        .write_table(&amle_headers, &amle_aligns, &amle_rows);
}
