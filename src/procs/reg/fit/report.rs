use super::*;

/// MQ5.2 — the per-model report header block (procedure / BY / model /
/// dependent headings and the observation counts).
pub(super) fn print_report_header(
    n_read: usize,
    n_used: f64,
    model_label: &str,
    by_heading: Option<&str>,
    dep_name: &str,
    session: &mut Session,
) {
    session.listing.page_header();
    centered(session, "The REG Procedure");
    if let Some(h) = by_heading {
        centered(session, h);
    }
    centered(session, model_label);
    centered(session, &format!("Dependent Variable: {}", dep_name));
    session.listing.blank();

    session.listing.write_line(&format!(
        "               Number of Observations Read         {}",
        n_read
    ));
    session.listing.write_line(&format!(
        "               Number of Observations Used         {}",
        n_used as usize
    ));
    session.listing.blank();
    session.listing.blank();
}

/// MQ5.2 — the printed Analysis of Variance table.
pub(super) fn print_anova_table(stats: &AnovaStats, sse: f64, session: &mut Session) {
    let &AnovaStats {
        ssm,
        sst,
        model_df,
        error_df,
        total_df,
        total_label,
        msm,
        mse,
        f_stat,
        p_f,
        ..
    } = stats;
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
    let anova_rows: Vec<Vec<String>> = vec![
        vec![
            "Model".into(),
            format!("{}", model_df as usize),
            fmt5(ssm),
            fmt5(msm),
            fmt2(f_stat),
            fmt_p(Some(p_f)),
        ],
        vec![
            "Error".into(),
            format!("{}", error_df as usize),
            fmt5(sse),
            fmt5(mse),
            "".into(),
            "".into(),
        ],
        vec![
            total_label.into(),
            format!("{}", total_df as usize),
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
}

/// MQ5.2 — the printed fit-statistics block (Root MSE / R-Square / …).
pub(super) fn print_fit_stats(stats: &AnovaStats, press_stat: Option<f64>, session: &mut Session) {
    let &AnovaStats {
        y_mean,
        r2,
        adj_r2,
        root_mse,
        cv,
        ..
    } = stats;
    // Fit statistics (written manually)
    session.listing.write_line(&format!(
        "Root MSE             {}    R-Square     {}",
        fmt5(root_mse),
        fmt_fit4(r2)
    ));
    session.listing.write_line(&format!(
        "Dependent Mean       {}    Adj R-Sq     {}",
        fmt5(y_mean),
        fmt_fit4(adj_r2)
    ));
    session
        .listing
        .write_line(&format!("Coeff Var            {}", fmt5(cv)));
    // PRESS statistic (M36.5): printed among the fit statistics when MODEL PRESS
    // is requested. This is independent of MODEL R, which prints its own
    // "Predicted Residual SS (PRESS)" line in the residual-analysis summary
    // block; both may appear and report the same value.
    if let Some(press) = press_stat {
        session
            .listing
            .write_line(&format!("PRESS                {}", fmt5(press)));
    }
    session.listing.blank();
    session.listing.blank();
}
