use super::*;

/// Print the page header and the Model Information table (ordinal model).
pub(super) fn print_ordinal_model_information(
    session: &mut Session,
    in_libref: &str,
    in_table: &str,
    resp_name: &str,
    k: usize,
) {
    session.listing.page_header();
    centered(session, "The LOGISTIC Procedure");
    session.listing.blank();

    centered(session, "Model Information");
    session.listing.blank();
    let ds_display = format!("{}.{}", in_libref, in_table);
    let info_rows: Vec<Vec<String>> = vec![
        vec!["Data Set".into(), ds_display],
        vec!["Response Variable".into(), resp_name.to_string()],
        vec!["Number of Response Levels".into(), k.to_string()],
        vec!["Model".into(), "cumulative logit".into()],
        vec!["Optimization Technique".into(), "Newton-Raphson".into()],
    ];
    session.listing.write_table(
        &["".into(), "".into()],
        &[Align::Left, Align::Left],
        &info_rows,
    );
    session.listing.blank();
}

/// Print the Response Profile table (ordinal model).
pub(super) fn print_ordinal_response_profile(
    session: &mut Session,
    resp_name: &str,
    cat_vec: &[usize],
    freq_vec: &[f64],
    ordered_levels: &[&Value],
    k: usize,
) {
    let n_obs = cat_vec.len();

    centered(session, "Response Profile");
    session.listing.blank();
    let mut freq_by_cat = vec![0.0_f64; k];
    for i in 0..n_obs {
        freq_by_cat[cat_vec[i] - 1] += freq_vec[i];
    }
    let rp_headers = vec![
        "Ordered Value".into(),
        resp_name.to_string(),
        "Total Frequency".into(),
    ];
    let rp_aligns = vec![Align::Right, Align::Left, Align::Right];
    let rp_rows: Vec<Vec<String>> = (0..k)
        .map(|j| {
            vec![
                (j + 1).to_string(),
                value_label(ordered_levels[j]),
                (freq_by_cat[j] as i64).to_string(),
            ]
        })
        .collect();
    session.listing.write_table(&rp_headers, &rp_aligns, &rp_rows);
    session.listing.blank();
}

/// Print the Analysis of Maximum Likelihood Estimates table (ordinal model).
pub(super) fn print_ordinal_parameter_estimates(
    session: &mut Session,
    design: &Design,
    theta: &[f64],
    se: &[f64],
    wald: &[f64],
    wald_p: &[f64],
    n_int: usize,
) {
    let n_par = theta.len();

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
    let mut amle_rows: Vec<Vec<String>> = Vec::with_capacity(n_par);
    for j in 0..n_int {
        amle_rows.push(vec![
            format!("Intercept {}", j + 1),
            "1".into(),
            fmt4(theta[j]),
            fmt4(se[j]),
            fmt4(wald[j]),
            fmt_p_opt(wald_p[j]),
        ]);
    }
    for (m, label) in design.col_labels.iter().enumerate() {
        let j = n_int + m;
        amle_rows.push(vec![
            label.clone(),
            "1".into(),
            fmt4(theta[j]),
            fmt4(se[j]),
            fmt4(wald[j]),
            fmt_p_opt(wald_p[j]),
        ]);
    }
    session
        .listing
        .write_table(&amle_headers, &amle_aligns, &amle_rows);
    session.listing.blank();
}

/// Print the Odds Ratio Estimates table (ordinal model; shared slopes).
pub(super) fn print_ordinal_odds_ratios(
    session: &mut Session,
    design: &Design,
    theta: &[f64],
    se: &[f64],
    n_int: usize,
) {
    centered(session, "Odds Ratio Estimates");
    session.listing.blank();
    let ore_headers: Vec<String> = vec![
        "Effect".into(),
        "Point Estimate".into(),
        "Lower".into(),
        "Upper".into(),
    ];
    let ore_aligns =
        vec![Align::Left, Align::Right, Align::Right, Align::Right];
    let mut col = n_int;
    let mut ore_rows: Vec<Vec<String>> = Vec::new();
    for eff in &design.effects {
        if eff.is_class {
            for lv in &eff.levels {
                ore_rows.push(vec![
                    format!("{} {} vs {}", eff.name, value_label(lv), eff.ref_label),
                    fmt4(theta[col].exp()),
                    fmt4((theta[col] - 1.96 * se[col]).exp()),
                    fmt4((theta[col] + 1.96 * se[col]).exp()),
                ]);
                col += 1;
            }
        } else {
            ore_rows.push(vec![
                eff.name.clone(),
                fmt4(theta[col].exp()),
                fmt4((theta[col] - 1.96 * se[col]).exp()),
                fmt4((theta[col] + 1.96 * se[col]).exp()),
            ]);
            col += 1;
        }
    }
    session
        .listing
        .write_table(&ore_headers, &ore_aligns, &ore_rows);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_ordinal(
    ast: &LogisticAst,
    session: &mut Session,
    model: &LogisticModel,
    ds: &SasDataset,
    in_libref: &str,
    in_table: &str,
    resp_name: &str,
    resp_col: &[Value],
    pred_cols: &[Vec<Value>],
    freq_col: &Option<Vec<Value>>,
    levels: &[Value],
    design: &Design,
    n_read: usize,
) -> Result<()> {
    let k = levels.len(); // number of response levels (>2)
    let n_int = k - 1; // number of intercepts
    let nb_cols = design.n_cols();
    let n_par = n_int + nb_cols;

    // Order of categories: DESCENDING reverses the sas_cmp ascending order.
    // `cat_of[i]` = ordered category index (1..=k) for row i's response.
    let ordered_levels: Vec<&Value> = if model.descending {
        levels.iter().rev().collect()
    } else {
        levels.iter().collect()
    };

    // ── Listwise deletion + design build ──────────────────────────────────
    let (cat_vec, x_mat, freq_vec, complete_mask) = build_ordinal_matrices(
        design,
        pred_cols,
        resp_col,
        freq_col,
        &ordered_levels,
        n_read,
        nb_cols,
    );

    let n_obs = cat_vec.len();
    let n_total: f64 = freq_vec.iter().sum();

    session.log.note(&format!(
        "There were {} observations read from the data set {}.{}.",
        n_read, in_libref, in_table
    ));
    session
        .log
        .note(&format!("There were {} observations used.", n_total as i64));

    if n_obs <= n_par {
        return Err(SasError::runtime(
            "Not enough observations for ordinal logistic regression",
        ));
    }

    // ── Newton-Raphson on the cumulative-logit log-likelihood ─────────────
    let OrdinalFit {
        theta,
        converged,
        se,
        wald,
        wald_p,
    } = fit_ordinal(session, &cat_vec, &x_mat, &freq_vec, n_total, n_int, nb_cols, k);
    let sigma = |z: f64| 1.0 / (1.0 + (-z).exp());

    // ── Listing ───────────────────────────────────────────────────────────
    if !model.noprint {
        print_ordinal_model_information(session, in_libref, in_table, resp_name, k);
        write_class_level_info(session, design);
        print_ordinal_response_profile(
            session,
            resp_name,
            &cat_vec,
            &freq_vec,
            &ordered_levels,
            k,
        );

        session
            .log
            .note("PROC LOGISTIC: Score Test for the Proportional Odds Assumption is deferred.");

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

        print_ordinal_parameter_estimates(session, design, &theta, &se, &wald, &wald_p, n_int);
        if nb_cols > 0 {
            print_ordinal_odds_ratios(session, design, &theta, &se, n_int);
        }
    }

    // ── OUTPUT: predicted = P(Y = lowest ordered category) = P(Y ≤ 1) ──────
    let predicted: Vec<f64> = x_mat
        .iter()
        .map(|xi| {
            let xb: f64 = xi.iter().zip(theta[n_int..].iter()).map(|(x, b)| x * b).sum();
            sigma(theta[0] + xb)
        })
        .collect();
    let xbeta: Vec<f64> = x_mat
        .iter()
        .map(|xi| xi.iter().zip(theta[n_int..].iter()).map(|(x, b)| x * b).sum())
        .collect();
    write_outputs(
        &ast.outputs,
        ds,
        &complete_mask,
        &predicted,
        &xbeta,
        session,
    )?;

    Ok(())
}
