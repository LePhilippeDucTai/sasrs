use super::*;

// ───────────────────────── Fixed-effects design ─────────────────────────

// ───────────────────────── Formatting helpers ─────────────────────────

pub(super) fn fmt4(v: f64) -> String {
    format!("{v:.4}")
}

pub(super) fn fmt2(v: f64) -> String {
    format!("{v:.2}")
}

pub(super) fn value_matches_event(v: &Value, event: &str) -> bool {
    match v {
        Value::Char(s) => s.trim_end() == event.trim(),
        Value::Num(f) => {
            if let Ok(ev_num) = event.trim().parse::<f64>() {
                (f - ev_num).abs() < 1e-15
            } else {
                format_best(*f, 12) == event.trim()
            }
        }
        Value::Missing(_) => false,
    }
}

/// Print the page header and the Model Information table.
pub(super) fn print_model_information(
    session: &mut Session,
    model: &ModelSpec,
    in_libref: &str,
    in_table: &str,
    laplace: bool,
) {
    let dist_name = match model.dist {
        Distribution::Normal => "Normal",
        Distribution::Poisson => "Poisson",
        Distribution::Binary => "Binary",
        _ => "Normal",
    };
    let link_name = match model.link {
        LinkFunction::Identity => "Identity",
        LinkFunction::Log => "Log",
        LinkFunction::Logit => "Logit",
        LinkFunction::Probit => "Probit",
        LinkFunction::Cloglog => "Complementary log-log",
    };

    session.listing.page_header();
    centered(session, "The GLIMMIX Procedure");
    session.listing.blank();

    centered(session, "Model Information");
    session.listing.blank();
    {
        let aligns = vec![Align::Left, Align::Left];
        let mut rows: Vec<Vec<String>> = vec![
            vec!["Data Set".into(), format!("{}.{}", in_libref, in_table)],
            vec!["Response Variable".into(), model.response.clone()],
            vec!["Response Distribution".into(), dist_name.into()],
            vec!["Link Function".into(), link_name.into()],
            vec!["Variance Function".into(), "Default".into()],
        ];
        if laplace {
            rows.push(vec![
                "Estimation Technique".into(),
                "Maximum Likelihood".into(),
            ]);
            rows.push(vec!["Likelihood Approximation".into(), "Laplace".into()]);
        } else {
            rows.push(vec!["Estimation Technique".into(), "Residual PL".into()]);
        }
        rows.push(vec!["Degrees of Freedom Method".into(), "Contain".into()]);
        session
            .listing
            .write_table(&[String::new(), String::new()], &aligns, &rows);
        session.listing.blank();
    }
}

/// Print the Class Level Information table (subject CLASS only).
pub(super) fn print_class_level_information(
    session: &mut Session,
    subject: &Option<String>,
    levels: &[Value],
    n_subjects: usize,
) {
    centered(session, "Class Level Information");
    session.listing.blank();
    let headers = vec!["Class".into(), "Levels".into(), "Values".into()];
    let aligns = vec![Align::Left, Align::Right, Align::Left];
    let values_str = levels.iter().map(value_label).collect::<Vec<_>>().join(" ");
    let rows = vec![vec![
        subject.clone().unwrap_or_default(),
        n_subjects.to_string(),
        values_str,
    ]];
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
}

/// Print the Dimensions table (random only).
pub(super) fn print_dimensions(
    session: &mut Session,
    fit: &GlimmixFit,
    p: usize,
    n_subjects: usize,
    max_obs: usize,
) {
    centered(session, "Dimensions");
    session.listing.blank();
    let aligns = vec![Align::Left, Align::Right];
    let n_cov_parm = fit.cov_parms.as_ref().map(|c| c.len()).unwrap_or(2);
    // Z-side columns per subject: 1 for a VC random intercept, 0 for an
    // R-side (AR(1)/UN) repeated structure.
    let z_cols = if fit.cov_parms.is_some() { 0 } else { 1 };
    let rows: Vec<Vec<String>> = vec![
        vec!["Covariance Parameters".into(), n_cov_parm.to_string()],
        vec!["Columns in X".into(), p.to_string()],
        vec!["Columns in Z Per Subject".into(), z_cols.to_string()],
        vec!["Subjects".into(), n_subjects.to_string()],
        vec!["Max Obs Per Subject".into(), max_obs.to_string()],
    ];
    session
        .listing
        .write_table(&[String::new(), String::new()], &aligns, &rows);
    session.listing.blank();
}

/// Print the Number of Observations table.
pub(super) fn print_number_of_observations(
    session: &mut Session,
    ast: &GlimmixAst,
    n_read: usize,
    n_used: usize,
    n_total: f64,
    n_not_used: usize,
) {
    centered(session, "Number of Observations");
    session.listing.blank();
    {
        let aligns = vec![Align::Left, Align::Right];
        // For grouped (FREQ) data, "Used" reflects the FREQ-weighted count.
        let used_disp = if ast.freq_var.is_some() {
            (n_total as i64).to_string()
        } else {
            n_used.to_string()
        };
        let rows: Vec<Vec<String>> = vec![
            vec!["Number of Observations Read".into(), n_read.to_string()],
            vec!["Number of Observations Used".into(), used_disp],
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

/// Print the Iteration History table (compact, stable: starting + converged
/// objective).
pub(super) fn print_iteration_history(
    session: &mut Session,
    fit: &GlimmixFit,
    has_random: bool,
    gen_chisq: f64,
) {
    centered(session, "Iteration History");
    session.listing.blank();
    {
        let headers = vec![
            "Iteration".into(),
            "Restarts".into(),
            "Evaluations".into(),
            "Objective".into(),
            "Change".into(),
        ];
        let aligns = vec![
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        // Objective: -2 Res Log Pseudo-Likelihood (random) else the
        // Generalized Chi-Square of the converged fit.
        let objective = if has_random { fit.neg2 } else { gen_chisq };
        let rows: Vec<Vec<String>> = vec![
            vec![
                "0".into(),
                "0".into(),
                "1".into(),
                fmt4(objective),
                String::new(),
            ],
            vec![
                "1".into(),
                "0".into(),
                "2".into(),
                fmt4(objective),
                "0.00000000".into(),
            ],
        ];
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }
}

/// Print the Covariance Parameter Estimates table (random only).
pub(super) fn print_covariance_parameter_estimates(
    session: &mut Session,
    fit: &GlimmixFit,
    subject: &Option<String>,
) {
    centered(session, "Covariance Parameter Estimates");
    session.listing.blank();
    let headers = vec!["Cov Parm".into(), "Subject".into(), "Estimate".into()];
    let aligns = vec![Align::Left, Align::Left, Align::Right];
    let subj_disp = subject.clone().unwrap_or_default();
    let rows: Vec<Vec<String>> = match &fit.cov_parms {
        Some(parms) => parms
            .iter()
            .map(|cp| {
                vec![
                    cp.name.clone(),
                    if cp.show_subject {
                        subj_disp.clone()
                    } else {
                        String::new()
                    },
                    fmt4(cp.estimate),
                ]
            })
            .collect(),
        None => vec![
            vec![
                "Intercept".into(),
                subj_disp.clone(),
                fmt4(fit.sigma2_u.unwrap_or(0.0)),
            ],
            vec!["Residual".into(), String::new(), fmt4(fit.sigma2_e)],
        ],
    };
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
}

/// Print the Fit Statistics table.
#[allow(clippy::too_many_arguments)]
pub(super) fn print_fit_statistics(
    session: &mut Session,
    model: &ModelSpec,
    fit: &GlimmixFit,
    p: usize,
    n_subjects: usize,
    gen_chisq: f64,
    gen_chisq_df: f64,
    laplace: bool,
    has_random: bool,
) {
    centered(session, "Fit Statistics");
    session.listing.blank();
    {
        let aligns = vec![Align::Left, Align::Right];
        let mut rows: Vec<Vec<String>> = Vec::new();
        if laplace {
            // True-ML fit statistics: -2 Log Likelihood plus information criteria.
            // Number of estimated parameters = p (β) + 1 (σ²_u) [+1 σ²_e Normal].
            let n_cov = if model.dist == Distribution::Normal {
                2.0
            } else {
                1.0
            };
            let n_parm = p as f64 + n_cov;
            let neg2 = fit.neg2;
            let aic = neg2 + 2.0 * n_parm;
            let n_eff = n_subjects as f64;
            let aicc = if n_eff - n_parm - 1.0 > 0.0 {
                neg2 + 2.0 * n_parm * n_eff / (n_eff - n_parm - 1.0)
            } else {
                aic
            };
            let bic = neg2 + n_parm * n_eff.ln();
            rows.push(vec!["-2 Log Likelihood".into(), fmt4(neg2)]);
            rows.push(vec!["AIC  (smaller is better)".into(), fmt4(aic)]);
            rows.push(vec!["AICC (smaller is better)".into(), fmt4(aicc)]);
            rows.push(vec!["BIC  (smaller is better)".into(), fmt4(bic)]);
        } else {
            if has_random {
                rows.push(vec!["-2 Res Log Pseudo-Likelihood".into(), fmt4(fit.neg2)]);
            }
            rows.push(vec!["Generalized Chi-Square".into(), fmt4(gen_chisq)]);
            rows.push(vec![
                "Gener. Chi-Square / DF".into(),
                fmt4(gen_chisq / gen_chisq_df),
            ]);
        }
        session
            .listing
            .write_table(&[String::new(), String::new()], &aligns, &rows);
        session.listing.blank();
    }
}

/// Print the Type III Tests of Fixed Effects table.
pub(super) fn print_type3_tests(
    session: &mut Session,
    param_labels: &[String],
    fit: &GlimmixFit,
    den_df: f64,
) {
    centered(session, "Type III Tests of Fixed Effects");
    session.listing.blank();
    {
        let headers = vec![
            "Effect".into(),
            "Num DF".into(),
            "Den DF".into(),
            "F Value".into(),
            "Pr > F".into(),
        ];
        let aligns = vec![
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        let mut rows: Vec<Vec<String>> = Vec::new();
        // One row per fixed-effects parameter (Intercept, continuous, or a
        // CLASS reference-cell column).
        let cov = &fit.cov_beta;
        for (idx, nm) in param_labels.iter().enumerate() {
            let est = fit.beta[idx];
            let se = cov[idx][idx].max(0.0).sqrt();
            let t = if se > 0.0 { est / se } else { 0.0 };
            let f = t * t;
            let p_val = 1.0 - f_cdf(f, 1.0, den_df);
            rows.push(vec![
                nm.clone(),
                "1".into(),
                fmt_df(den_df),
                fmt2(f),
                fmt_p(p_val),
            ]);
        }
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }
}

/// Print the Solutions for Fixed Effects table.
pub(super) fn print_fixed_solutions(
    session: &mut Session,
    param_labels: &[String],
    fit: &GlimmixFit,
    den_df: f64,
) {
    centered(session, "Solutions for Fixed Effects");
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
    let cov = &fit.cov_beta;
    let mut rows: Vec<Vec<String>> = Vec::new();
    for (idx, nm) in param_labels.iter().enumerate() {
        let est = fit.beta[idx];
        let se = cov[idx][idx].max(0.0).sqrt();
        let t = if se > 0.0 { est / se } else { 0.0 };
        let p_val = 2.0 * (1.0 - student_t_cdf(t.abs(), den_df));
        rows.push(vec![
            nm.clone(),
            fmt4(est),
            fmt4(se),
            fmt_df(den_df),
            fmt2(t),
            fmt_p(p_val),
        ]);
    }
    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
}

/// Format a degrees-of-freedom value (integer if whole).
pub(super) fn fmt_df(df: f64) -> String {
    if (df - df.round()).abs() < 1e-9 {
        format!("{}", df.round() as i64)
    } else {
        format!("{df:.2}")
    }
}
