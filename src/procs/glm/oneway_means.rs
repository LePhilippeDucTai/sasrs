use super::*;

/// Print the Least Squares Means table.
pub(super) fn print_oneway_lsmeans(
    session: &mut Session,
    dep_var: &str,
    eff: &str,
    stats: &OneWayStats,
) {
    let df_error = stats.df_error;
    let mse = stats.mse;
    let levels = &stats.levels;
    let groups = &stats.groups;
    let group_means = &stats.group_means;
    let level_label = level_label_value;

    centered(session, "Least Squares Means");
    session.listing.blank();

    let lsm_headers: Vec<String> = vec![
        eff.to_string(),
        format!("{} LSMEAN", dep_var),
        "Standard Error".into(),
        "Pr > |t|".into(),
    ];
    let lsm_aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right];
    let mut lsm_rows: Vec<Vec<String>> = Vec::new();

    for (gi, level) in levels.iter().enumerate() {
        let lbl = level_label(level);
        let n_i = groups[gi].len();
        let lsmean_i = group_means[gi];
        let se_lsm = if n_i > 0 && !mse.is_nan() {
            (mse / n_i as f64).sqrt()
        } else {
            f64::NAN
        };
        let t_lsm = if se_lsm > 0.0 {
            lsmean_i / se_lsm
        } else {
            f64::NAN
        };
        let p_lsm = if t_lsm.is_nan() {
            None
        } else {
            Some(2.0 * (1.0 - student_t_cdf(t_lsm.abs(), df_error)))
        };
        lsm_rows.push(vec![lbl, fmt6(lsmean_i), fmt6(se_lsm), fmt_p(p_lsm)]);
    }

    session
        .listing
        .write_table(&lsm_headers, &lsm_aligns, &lsm_rows);
    session.listing.blank();
    session.listing.blank();
}

/// Print the Contrasts table for the CONTRAST statements matching `eff`.
pub(super) fn print_oneway_contrasts(
    session: &mut Session,
    ast: &GlmAst,
    eff: &str,
    stats: &OneWayStats,
) -> Result<()> {
    let k = stats.k;
    let df_error = stats.df_error;
    let mse = stats.mse;
    let groups = &stats.groups;
    let group_means = &stats.group_means;

    let relevant_contrasts: Vec<&GlmContrast> = ast
        .contrasts
        .iter()
        .filter(|c| c.effect.eq_ignore_ascii_case(eff))
        .collect();

    if !relevant_contrasts.is_empty() {
        centered(session, "Contrasts");
        session.listing.blank();

        let con_headers: Vec<String> = vec![
            "Contrast".into(),
            "DF".into(),
            "Contrast SS".into(),
            "Mean Square".into(),
            "F Value".into(),
            "Pr > F".into(),
        ];
        let con_aligns = vec![
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        let mut con_rows: Vec<Vec<String>> = Vec::new();

        for contrast in &relevant_contrasts {
            let c = &contrast.coefficients;
            if c.len() != k {
                return Err(SasError::runtime(format!(
                    "Contrast '{}' coefficients mismatch: expected {k} coefficients, got {}.",
                    contrast.label,
                    c.len()
                )));
            }
            // Estimate = Σ c_i × ȳ_i
            let estimate: f64 = c
                .iter()
                .zip(group_means.iter())
                .map(|(ci, yi)| ci * yi)
                .sum();
            // SE² = MSE × Σ (c_i²/n_i)
            let sum_c2_over_n: f64 = c
                .iter()
                .zip(groups.iter())
                .map(|(ci, g)| {
                    let ni = g.len();
                    if ni > 0 { ci * ci / ni as f64 } else { 0.0 }
                })
                .sum();
            let se_sq = if !mse.is_nan() {
                mse * sum_c2_over_n
            } else {
                f64::NAN
            };
            // F = Estimate² / se_sq
            let f_con = if se_sq > 0.0 {
                estimate * estimate / se_sq
            } else {
                f64::NAN
            };
            let p_con = if f_con.is_nan() {
                None
            } else {
                Some((1.0 - f_cdf(f_con, 1.0, df_error)).clamp(0.0, 1.0))
            };
            // Contrast SS = F × MSE = Estimate² / Σ(c_i²/n_i)
            let css = if sum_c2_over_n > 0.0 {
                estimate * estimate / sum_c2_over_n
            } else {
                f64::NAN
            };

            con_rows.push(vec![
                contrast.label.clone(),
                "1".into(),
                if css.is_nan() { ".".into() } else { fmt5(css) },
                if css.is_nan() { ".".into() } else { fmt5(css) },
                if f_con.is_nan() {
                    ".".into()
                } else {
                    fmt2(f_con)
                },
                fmt_p(p_con),
            ]);
        }

        session
            .listing
            .write_table(&con_headers, &con_aligns, &con_rows);
        session.listing.blank();
        session.listing.blank();
    }

    Ok(())
}

/// Print the Estimates table for the ESTIMATE statements matching `eff`.
pub(super) fn print_oneway_estimates(
    session: &mut Session,
    ast: &GlmAst,
    eff: &str,
    stats: &OneWayStats,
) {
    let df_error = stats.df_error;
    let mse = stats.mse;
    let groups = &stats.groups;
    let group_means = &stats.group_means;

    let relevant_estimates: Vec<&GlmEstimate> = ast
        .estimates
        .iter()
        .filter(|e| e.effect.eq_ignore_ascii_case(eff))
        .collect();

    if !relevant_estimates.is_empty() {
        centered(session, "Estimates");
        session.listing.blank();

        let est_headers: Vec<String> = vec![
            "Parameter".into(),
            "Estimate".into(),
            "Standard Error".into(),
            "t Value".into(),
            "Pr > |t|".into(),
        ];
        let est_aligns = vec![
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        let mut est_rows: Vec<Vec<String>> = Vec::new();

        for est in &relevant_estimates {
            let c = &est.coefficients;
            // Estimate = Σ c_i × ȳ_i
            let estimate: f64 = c
                .iter()
                .zip(group_means.iter())
                .map(|(ci, yi)| ci * yi)
                .sum();
            // SE² = MSE × Σ (c_i²/n_i)
            let sum_c2_over_n: f64 = c
                .iter()
                .zip(groups.iter())
                .map(|(ci, g)| {
                    let ni = g.len();
                    if ni > 0 { ci * ci / ni as f64 } else { 0.0 }
                })
                .sum();
            let se = if !mse.is_nan() && sum_c2_over_n > 0.0 {
                (mse * sum_c2_over_n).sqrt()
            } else {
                f64::NAN
            };
            let t_val = if se > 0.0 { estimate / se } else { f64::NAN };
            let p_val = if t_val.is_nan() {
                None
            } else {
                Some(2.0 * (1.0 - student_t_cdf(t_val.abs(), df_error)))
            };

            est_rows.push(vec![
                est.label.clone(),
                fmt6(estimate),
                fmt6(se),
                fmt2(t_val),
                fmt_p(p_val),
            ]);
        }

        session
            .listing
            .write_table(&est_headers, &est_aligns, &est_rows);
        session.listing.blank();
        session.listing.blank();
    }
}

/// Print the MEANS section (Level of ... table).
pub(super) fn print_oneway_means(session: &mut Session, eff: &str, stats: &OneWayStats) {
    let levels = &stats.levels;
    let groups = &stats.groups;
    let group_means = &stats.group_means;
    let level_label = level_label_value;

    centered(session, &format!("Level of {}", eff));
    session.listing.blank();

    let means_headers: Vec<String> =
        vec![eff.to_string(), "N".into(), "Mean".into(), "Std Dev".into()];
    let means_aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right];
    let mut means_rows: Vec<Vec<String>> = Vec::new();

    for (gi, level) in levels.iter().enumerate() {
        let lbl = level_label(level);
        let n_i = groups[gi].len();
        let mean_i = group_means[gi];
        let std_i = sample_std(&groups[gi]);
        means_rows.push(vec![
            lbl,
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
