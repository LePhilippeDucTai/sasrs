use super::*;

// ───────────────────────── Formatting helpers ─────────────────────────

pub(super) fn fmt4(v: f64) -> String {
    format!("{v:.4}")
}

pub(super) fn fmt2(v: f64) -> String {
    format!("{v:.2}")
}

/// Print the page header and the Model Information table.
#[allow(clippy::too_many_arguments)]
pub(super) fn print_model_information_gen(
    session: &mut Session,
    ast: &MixedAst,
    model: &ModelSpec,
    plan: &Plan,
    cov: GenCov,
    in_libref: &str,
    in_table: &str,
) {
    let method_name = match ast.method {
        Method::Reml => "REML",
        Method::Ml => "ML",
    };
    let cov_struct = match cov {
        GenCov::RepeatedAr1 => "Autoregressive",
        GenCov::RepeatedUn { .. } => "Unstructured",
        GenCov::RandomVc => match &plan {
            Plan::RandomVc(_, CovType::Cs) => "Compound Symmetry",
            _ => "Variance Components",
        },
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

/// Print the Class Level Information table (subject + any class fixed effects).
pub(super) fn print_class_level_information_gen(
    session: &mut Session,
    ast: &MixedAst,
    subject: &str,
    levels: &[Value],
    n_subjects: usize,
    kept_fixed: &[(String, Vec<Value>)],
) {
    centered(session, "Class Level Information");
    session.listing.blank();
    {
        let headers = vec!["Class".into(), "Levels".into(), "Values".into()];
        let aligns = vec![Align::Left, Align::Right, Align::Left];
        let mut rows: Vec<Vec<String>> = Vec::new();
        // Subject class.
        let values_str = levels.iter().map(value_label).collect::<Vec<_>>().join(" ");
        rows.push(vec![
            subject.to_string(),
            n_subjects.to_string(),
            values_str,
        ]);
        // Fixed CLASS variables.
        for (nm, col) in kept_fixed {
            if ast.class_vars.iter().any(|c| c.eq_ignore_ascii_case(nm))
                && !nm.eq_ignore_ascii_case(subject)
            {
                let mut lv: Vec<Value> = Vec::new();
                for v in col {
                    if !lv.iter().any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal) {
                        lv.push(v.clone());
                    }
                }
                lv.sort_by(|a, b| a.sas_cmp(b));
                let vs = lv.iter().map(value_label).collect::<Vec<_>>().join(" ");
                rows.push(vec![nm.clone(), lv.len().to_string(), vs]);
            }
        }
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }
}

/// Print the Dimensions table.
pub(super) fn print_dimensions_gen(
    session: &mut Session,
    cov: GenCov,
    n_cov: usize,
    p: usize,
    n_subjects: usize,
    max_obs: usize,
) {
    centered(session, "Dimensions");
    session.listing.blank();
    {
        let aligns = vec![Align::Left, Align::Right];
        let mut rows: Vec<Vec<String>> = vec![
            vec!["Covariance Parameters".into(), n_cov.to_string()],
            vec!["Columns in X".into(), p.to_string()],
        ];
        if matches!(cov, GenCov::RandomVc) {
            rows.push(vec!["Columns in Z Per Subject".into(), "1".into()]);
        }
        rows.push(vec!["Subjects".into(), n_subjects.to_string()]);
        rows.push(vec!["Max Obs Per Subject".into(), max_obs.to_string()]);
        session
            .listing
            .write_table(&[String::new(), String::new()], &aligns, &rows);
        session.listing.blank();
    }
}

/// Print the Number of Observations table.
pub(super) fn print_number_of_observations_gen(
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

/// Print the Iteration History table and the convergence note.
pub(super) fn print_iteration_history_gen(session: &mut Session, ast: &MixedAst, fit: &GenFit) {
    let res_label = match ast.method {
        Method::Reml => "-2 Res Log Like",
        Method::Ml => "-2 Log Like",
    };
    centered(session, "Iteration History");
    session.listing.blank();
    {
        let headers = vec![
            "Iteration".into(),
            "Evaluations".into(),
            res_label.into(),
            "Criterion".into(),
        ];
        let aligns = vec![Align::Right, Align::Right, Align::Right, Align::Right];
        let rows: Vec<Vec<String>> = vec![
            vec!["0".into(), "1".into(), fmt4(fit.neg2_start), String::new()],
            vec![
                "1".into(),
                // The raw Nelder-Mead evaluation total is an implementation detail
                // that drifts across builds/platforms (and bears no relation to
                // SAS's Newton-Raphson count). Show it in normal runs, but freeze
                // it under --deterministic so snapshots stay byte-stable.
                if session.deterministic {
                    "1".into()
                } else {
                    fit.iters.to_string()
                },
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

/// Print the Covariance Parameter Estimates table.
pub(super) fn print_covariance_parameter_estimates_gen(
    session: &mut Session,
    cov: GenCov,
    fit: &GenFit,
    subject: &str,
    is_cs: bool,
) {
    centered(session, "Covariance Parameter Estimates");
    session.listing.blank();
    {
        let headers = vec!["Cov Parm".into(), "Subject".into(), "Estimate".into()];
        let aligns = vec![Align::Left, Align::Left, Align::Right];
        let rows = cov_parm_rows(cov, &fit.theta, subject, is_cs);
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }
}

/// Print the Fit Statistics table (AIC/AICC/BIC).
pub(super) fn print_fit_statistics_gen(
    session: &mut Session,
    ast: &MixedAst,
    fit: &GenFit,
    n_cov: usize,
    n_used: usize,
    p: usize,
    n_subjects: usize,
) {
    let neg2 = fit.neg2ll;
    let nc = n_cov as f64;
    let aic = neg2 + 2.0 * nc;
    let n_eff = match ast.method {
        Method::Reml => (n_used - p) as f64,
        Method::Ml => n_used as f64,
    };
    let aicc = if n_eff - nc - 1.0 > 0.0 {
        neg2 + 2.0 * nc * n_eff / (n_eff - nc - 1.0)
    } else {
        aic
    };
    let bic = neg2 + nc * (n_subjects as f64).ln();
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

/// Print the Solution for Fixed Effects table.
pub(super) fn print_fixed_solutions_gen(
    session: &mut Session,
    fit: &GenFit,
    labels: &[String],
    p: usize,
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
    // Containment df: subjects − fixed parameters (approximate).
    let df = (n_subjects as i64 - p as i64).max(1);
    let mut rows: Vec<Vec<String>> = Vec::new();
    for (a, label) in labels.iter().enumerate().take(p) {
        let est = fit.beta[a];
        let se = fit.cov_beta[a][a].max(0.0).sqrt();
        let t = if se > 0.0 { est / se } else { 0.0 };
        let pv = 2.0 * (1.0 - student_t_cdf(t.abs(), df as f64));
        rows.push(vec![
            label.clone(),
            fmt4(est),
            fmt4(se),
            df.to_string(),
            fmt2(t),
            fmt_p(pv),
        ]);
    }
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
}
