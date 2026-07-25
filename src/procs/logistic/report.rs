use super::*;

/// Print the page header and the Model Information table.
pub(super) fn print_model_information(
    session: &mut Session,
    in_libref: &str,
    in_table: &str,
    resp_name: &str,
    model_desc: &str,
) {
    session.listing.page_header();
    centered(session, "The LOGISTIC Procedure");
    session.listing.blank();

    centered(session, "Model Information");
    session.listing.blank();

    let ds_display = format!("{}.{}", in_libref, in_table);
    let info_headers: Vec<String> = vec!["".into(), "".into()];
    let info_aligns = vec![Align::Left, Align::Left];
    let info_rows: Vec<Vec<String>> = vec![
        vec!["Data Set".into(), ds_display],
        vec!["Response Variable".into(), resp_name.to_string()],
        vec!["Number of Response Levels".into(), "2".into()],
        vec!["Model".into(), model_desc.to_string()],
        vec!["Optimization Technique".into(), "Newton-Raphson".into()],
    ];
    session
        .listing
        .write_table(&info_headers, &info_aligns, &info_rows);
    session.listing.blank();
}

/// Print the Response Profile table.
pub(super) fn print_response_profile(
    session: &mut Session,
    resp_name: &str,
    event_label: &str,
    nonevent_label: &str,
    n_event_total: f64,
    n_nonevent_total: f64,
) {
    centered(session, "Response Profile");
    session.listing.blank();

    let rp_headers: Vec<String> = vec![
        "Ordered Value".into(),
        resp_name.to_string(),
        "Total Frequency".into(),
    ];
    let rp_aligns = vec![Align::Right, Align::Left, Align::Right];
    let rp_rows: Vec<Vec<String>> = vec![
        vec![
            "1".into(),
            event_label.to_string(),
            (n_event_total as i64).to_string(),
        ],
        vec![
            "2".into(),
            nonevent_label.to_string(),
            (n_nonevent_total as i64).to_string(),
        ],
    ];
    session
        .listing
        .write_table(&rp_headers, &rp_aligns, &rp_rows);
    session.listing.blank();
    session.listing.write_line(&format!(
        "PROC LOGISTIC is modeling the probability that {}={}.",
        resp_name, event_label
    ));
}

/// Print the Model Convergence Status block.
pub(super) fn print_convergence_status(session: &mut Session, converged: bool) {
    session.listing.blank();
    centered(session, "Model Convergence Status");
    session.listing.blank();
    if converged {
        session
            .listing
            .write_line("     Convergence criterion (GCONV=1E-8) satisfied.");
    } else {
        session
            .listing
            .write_line("     Iteration limit reached without convergence.");
    }
    session.listing.blank();
}

/// Print the Model Fit Statistics table.
pub(super) fn print_model_fit_statistics(
    session: &mut Session,
    aic_null: f64,
    aic: f64,
    sc_null: f64,
    sc: f64,
    neg2log_l_null: f64,
    neg2log_l: f64,
) {
    centered(session, "Model Fit Statistics");
    session.listing.blank();

    let mfs_headers: Vec<String> = vec![
        "Criterion".into(),
        "Intercept Only".into(),
        "Intercept and Covariates".into(),
    ];
    let mfs_aligns = vec![Align::Left, Align::Right, Align::Right];
    let mfs_rows: Vec<Vec<String>> = vec![
        vec!["AIC".into(), fmt4(aic_null), fmt4(aic)],
        vec!["SC".into(), fmt4(sc_null), fmt4(sc)],
        vec!["-2 Log L".into(), fmt4(neg2log_l_null), fmt4(neg2log_l)],
    ];
    session
        .listing
        .write_table(&mfs_headers, &mfs_aligns, &mfs_rows);
    session.listing.blank();
}

/// Print the Testing Global Null Hypothesis: BETA=0 table.
#[allow(clippy::too_many_arguments)]
pub(super) fn print_global_tests(
    session: &mut Session,
    nb_cols: usize,
    lr_chi2: f64,
    lr_p: f64,
    score_chi2: f64,
    score_p: f64,
    wald_chi2_global: f64,
    wald_global_p: f64,
) {
    centered(session, "Testing Global Null Hypothesis: BETA=0");
    session.listing.blank();

    let gnh_headers: Vec<String> = vec![
        "Test".into(),
        "Chi-Square".into(),
        "DF".into(),
        "Pr > ChiSq".into(),
    ];
    let gnh_aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right];
    let gnh_rows: Vec<Vec<String>> = vec![
        vec![
            "Likelihood Ratio".into(),
            fmt4(lr_chi2),
            nb_cols.to_string(),
            fmt_p_opt(lr_p),
        ],
        vec![
            "Score".into(),
            fmt4(score_chi2),
            nb_cols.to_string(),
            fmt_p_opt(score_p),
        ],
        vec![
            "Wald".into(),
            fmt4(wald_chi2_global),
            nb_cols.to_string(),
            fmt_p_opt(wald_global_p),
        ],
    ];
    session
        .listing
        .write_table(&gnh_headers, &gnh_aligns, &gnh_rows);
    session.listing.blank();
}

/// Print the Analysis of Maximum Likelihood Estimates table.
pub(super) fn print_parameter_estimates(
    session: &mut Session,
    design: &Design,
    beta: &[f64],
    se_beta: &[f64],
    wald_chi2: &[f64],
    wald_p: &[f64],
) {
    let p_param = beta.len();

    centered(session, "Analysis of Maximum Likelihood Estimates");
    session.listing.blank();

    let amle_headers: Vec<String> = vec![
        "Parameter".into(),
        "DF".into(),
        "Estimate".into(),
        "Standard Error".into(),
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
    ];
    let mut amle_rows: Vec<Vec<String>> = Vec::with_capacity(p_param);
    for j in 0..p_param {
        let param_name = if j == 0 {
            "Intercept".to_string()
        } else {
            design.col_labels[j - 1].clone()
        };
        amle_rows.push(vec![
            param_name,
            "1".into(),
            fmt4(beta[j]),
            fmt4(se_beta[j]),
            fmt4(wald_chi2[j]),
            fmt_p_opt(wald_p[j]),
        ]);
    }
    session
        .listing
        .write_table(&amle_headers, &amle_aligns, &amle_rows);
    session.listing.blank();
}

/// Print the Odds Ratio Estimates table (LINK=LOGIT only).
pub(super) fn print_odds_ratios(
    session: &mut Session,
    design: &Design,
    beta: &[f64],
    se_beta: &[f64],
) {
    let nb_cols = beta.len() - 1;

    centered(session, "Odds Ratio Estimates");
    session.listing.blank();

    let ore_headers: Vec<String> = vec![
        "Effect".into(),
        "Point Estimate".into(),
        "Lower".into(),
        "Upper".into(),
    ];
    let ore_aligns = vec![
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    let mut ore_rows: Vec<Vec<String>> = Vec::with_capacity(nb_cols);
    // Walk the design columns effect-by-effect so CLASS rows can be
    // labelled "var level vs reflevel".
    let mut col = 1usize; // beta index (skip intercept)
    for eff in &design.effects {
        if eff.is_class {
            for lv in &eff.levels {
                let or_j = beta[col].exp();
                let ci_lower = (beta[col] - 1.96 * se_beta[col]).exp();
                let ci_upper = (beta[col] + 1.96 * se_beta[col]).exp();
                ore_rows.push(vec![
                    format!(
                        "{} {} vs {}",
                        eff.name,
                        value_label(lv),
                        eff.ref_label
                    ),
                    fmt4(or_j),
                    fmt4(ci_lower),
                    fmt4(ci_upper),
                ]);
                col += 1;
            }
        } else {
            let or_j = beta[col].exp();
            let ci_lower = (beta[col] - 1.96 * se_beta[col]).exp();
            let ci_upper = (beta[col] + 1.96 * se_beta[col]).exp();
            ore_rows.push(vec![
                eff.name.clone(),
                fmt4(or_j),
                fmt4(ci_lower),
                fmt4(ci_upper),
            ]);
            col += 1;
        }
    }
    session
        .listing
        .write_table(&ore_headers, &ore_aligns, &ore_rows);
}
