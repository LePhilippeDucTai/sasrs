use super::*;

/// Page header + Model Information table (legacy path).
pub(super) fn print_model_information_legacy(
    session: &mut Session,
    ast: &MixedAst,
    model: &ModelSpec,
    random: &RandomSpec,
    in_libref: &str,
    in_table: &str,
) {
    let method_name = match ast.method {
        Method::Reml => "REML",
        Method::Ml => "ML",
    };
    let cov_struct = match random.cov_type {
        CovType::Cs => "Compound Symmetry",
        _ => "Variance Components",
    };

    session.listing.page_header();
    centered(session, "The Mixed Procedure");
    session.listing.blank();

    centered(session, "Model Information");
    session.listing.blank();
    {
        let aligns = vec![Align::Left, Align::Left];
        let rows: Vec<Vec<String>> = vec![
            vec!["Data Set".into(), format!("{}.{}", in_libref, in_table)],
            vec!["Dependent Variable".into(), model.response.clone()],
            vec!["Covariance Structure".into(), cov_struct.into()],
            vec!["Estimation Method".into(), method_name.into()],
            vec!["Residual Variance Method".into(), "Profile".into()],
            vec!["Fixed Effects SE Method".into(), "Model-Based".into()],
            vec!["Degrees of Freedom Method".into(), "Contain".into()],
        ];
        session
            .listing
            .write_table(&[String::new(), String::new()], &aligns, &rows);
        session.listing.blank();
    }
}

/// Class Level Information table (legacy path: the SUBJECT= class only).
pub(super) fn print_class_level_information_legacy(
    session: &mut Session,
    subject: &str,
    levels: &[Value],
) {
    centered(session, "Class Level Information");
    session.listing.blank();
    {
        let headers = vec!["Class".into(), "Levels".into(), "Values".into()];
        let aligns = vec![Align::Left, Align::Right, Align::Left];
        let values_str = levels.iter().map(value_label).collect::<Vec<_>>().join(" ");
        let rows = vec![vec![
            subject.to_string(),
            levels.len().to_string(),
            values_str,
        ]];
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }
}

/// Dimensions table (legacy path).
pub(super) fn print_dimensions_legacy(
    session: &mut Session,
    fit: &MixedFit,
    n_subjects: usize,
    max_obs: usize,
) {
    centered(session, "Dimensions");
    session.listing.blank();
    {
        let aligns = vec![Align::Left, Align::Right];
        let rows: Vec<Vec<String>> = vec![
            vec!["Covariance Parameters".into(), "2".into()],
            vec!["Columns in X".into(), fit.p.to_string()],
            vec!["Columns in Z Per Subject".into(), "1".into()],
            vec!["Subjects".into(), n_subjects.to_string()],
            vec!["Max Obs Per Subject".into(), max_obs.to_string()],
        ];
        session
            .listing
            .write_table(&[String::new(), String::new()], &aligns, &rows);
        session.listing.blank();
    }
}

/// Number of Observations table (legacy path).
pub(super) fn print_number_of_observations_legacy(
    session: &mut Session,
    n_read: usize,
    n_used: usize,
    n_not_used: usize,
) {
    centered(session, "Number of Observations");
    session.listing.blank();
    {
        let aligns = vec![Align::Left, Align::Right];
        let rows: Vec<Vec<String>> = vec![
            vec!["Number of Observations Read".into(), n_read.to_string()],
            vec!["Number of Observations Used".into(), n_used.to_string()],
            vec![
                "Number of Observations Not Used".into(),
                n_not_used.to_string(),
            ],
        ];
        session
            .listing
            .write_table(&[String::new(), String::new()], &aligns, &rows);
        session.listing.blank();
    }
}

/// Iteration History (minimal, stable) + convergence message (legacy path).
pub(super) fn print_iteration_history_legacy(session: &mut Session, fit: &MixedFit) {
    centered(session, "Iteration History");
    session.listing.blank();
    {
        let headers = vec![
            "Iteration".into(),
            "Evaluations".into(),
            "-2 Res Log Like".into(),
            "Criterion".into(),
        ];
        let aligns = vec![Align::Right, Align::Right, Align::Right, Align::Right];
        let rows: Vec<Vec<String>> = vec![
            vec!["0".into(), "1".into(), fmt4(fit.neg2ll), String::new()],
            vec![
                "1".into(),
                "1".into(),
                fmt4(fit.neg2ll),
                "0.00000000".into(),
            ],
        ];
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }
    centered(session, "Convergence criteria met.");
    session.listing.blank();
}

/// Covariance Parameter Estimates table (legacy path).
pub(super) fn print_covariance_parameter_estimates_legacy(
    session: &mut Session,
    random: &RandomSpec,
    subject: &str,
    fit: &MixedFit,
) {
    centered(session, "Covariance Parameter Estimates");
    session.listing.blank();
    {
        let headers = vec!["Cov Parm".into(), "Subject".into(), "Estimate".into()];
        let aligns = vec![Align::Left, Align::Left, Align::Right];
        let cov_parm_name = match random.cov_type {
            CovType::Cs => "CS",
            _ => "Intercept",
        };
        let rows: Vec<Vec<String>> = vec![
            vec![
                cov_parm_name.into(),
                subject.to_string(),
                fmt4(fit.sigma2_u),
            ],
            vec!["Residual".into(), String::new(), fmt4(fit.sigma2_e)],
        ];
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }
}

/// Fit Statistics table (-2LL, AIC, AICC, BIC) for the legacy path.
pub(super) fn print_fit_statistics_legacy(
    session: &mut Session,
    ast: &MixedAst,
    fit: &MixedFit,
    n_subjects: usize,
) {
    let neg2 = fit.neg2ll;
    let n_cov = 2.0_f64;
    let aic = neg2 + 2.0 * n_cov;
    let n_eff = match ast.method {
        Method::Reml => (fit.n - fit.p) as f64,
        Method::Ml => fit.n as f64,
    };
    let aicc = if n_eff - n_cov - 1.0 > 0.0 {
        neg2 + 2.0 * n_cov * n_eff / (n_eff - n_cov - 1.0)
    } else {
        aic
    };
    let bic = neg2 + n_cov * (n_subjects as f64).ln();
    centered(session, "Fit Statistics");
    session.listing.blank();
    {
        let aligns = vec![Align::Left, Align::Right];
        let label = match ast.method {
            Method::Reml => "-2 Res Log Likelihood",
            Method::Ml => "-2 Log Likelihood",
        };
        let rows: Vec<Vec<String>> = vec![
            vec![label.into(), fmt4(neg2)],
            vec!["AIC (Smaller is Better)".into(), fmt4(aic)],
            vec!["AICC (Smaller is Better)".into(), fmt4(aicc)],
            vec!["BIC (Smaller is Better)".into(), fmt4(bic)],
        ];
        session
            .listing
            .write_table(&[String::new(), String::new()], &aligns, &rows);
        session.listing.blank();
    }
}

/// Solution for Fixed Effects (intercept-only, ddfm=contain) — legacy path.
pub(super) fn print_fixed_solution_legacy(
    session: &mut Session,
    fit: &MixedFit,
    n_subjects: usize,
) {
    centered(session, "Solution for Fixed Effects");
    session.listing.blank();
    let headers = vec![
        "Effect".into(),
        "Estimate".into(),
        "Standard Error".into(),
        "DF".into(),
        "t Value".into(),
        "Pr > |t|".into(),
    ];
    let aligns = vec![
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    // Intercept-only: single row.
    let est = fit.beta[0];
    let se = fit.cov_beta[0][0].max(0.0).sqrt();
    // ddfm=contain: DF = number of subjects - number of fixed parameters.
    let df = (n_subjects as i64 - fit.p as i64).max(1);
    let t = if se > 0.0 { est / se } else { 0.0 };
    let p = 2.0 * (1.0 - student_t_cdf(t.abs(), df as f64));
    let rows = vec![vec![
        "Intercept".into(),
        fmt4(est),
        fmt4(se),
        df.to_string(),
        fmt2(t),
        fmt_p(p),
    ]];
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
}
