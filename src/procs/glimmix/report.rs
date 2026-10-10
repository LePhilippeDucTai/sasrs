use super::*;

// ───────────────────────── Fixed-effects design ─────────────────────────

// ───────────────────────── Formatting helpers ─────────────────────────

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
    has_random: bool,
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
        } else if has_random {
            rows.push(vec!["Estimation Technique".into(), "Residual PL".into()]);
        } else {
            // J02-P3 — GLM mode (no RANDOM statement) used to be labelled
            // « Residual PL », METHOD=LAPLACE included. SAS/STAT 9.4, The
            // GLIMMIX Procedure, « GLM Mode or GLMM Mode » / « Default
            // Estimation Techniques »: METHOD= has no effect in GLM mode, the
            // model is fit by restricted maximum likelihood for normal data
            // and by maximum likelihood otherwise (here the IRLS fit).
            let technique = if model.dist == Distribution::Normal {
                "Restricted Maximum Likelihood"
            } else {
                "Maximum Likelihood"
            };
            rows.push(vec!["Estimation Technique".into(), technique.into()]);
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

/// Print the convergence status (J02-P3): the line names the criterion the
/// solver actually tested (it used to read « GCONV=1E-8 » for every fit, a
/// gradient criterion never tested) and is asserted only when that criterion
/// was met (J02-P6); on failure SAS GLIMMIX reports « Did not converge. » in
/// the log (SAS/STAT User's Guide, The GLIMMIX Procedure, Convergence Status).
///
/// The former « Iteration History » table was synthetic (two rows repeating
/// the final objective, an invented 0.00000000 change and invented evaluation
/// counts): it is removed until a real iteration history (roadmap-avancee
/// J08-P2/J08-P3); no value is invented.
pub(super) fn print_convergence_status(session: &mut Session, fit: &GlimmixFit) {
    if fit.converged {
        centered(session, &fit.criterion.satisfied_text());
    } else {
        centered(session, "Convergence criterion was not satisfied.");
        session
            .log
            .warning("Did not converge. The estimates from PROC GLIMMIX may not be reliable.");
    }
    session.listing.blank();
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

/// Columns of the fixed-effects design that belong to one MODEL effect.
pub(super) struct EffectColumns {
    pub(super) name: String,
    pub(super) cols: Vec<usize>,
}

/// Design columns of each MODEL effect, in the order of `build_design`: the
/// intercept first (unless NOINT), then one column per continuous effect and
/// L−1 reference-coded columns per CLASS effect with L levels among the
/// observations used. `kept_fixed` holds the MODEL effects in MODEL order.
pub(super) fn effect_columns(
    model: &ModelSpec,
    class_vars: &[String],
    kept_fixed: &[(String, Vec<Value>)],
    p: usize,
) -> Result<Vec<EffectColumns>> {
    let mut next = usize::from(!model.noint);
    let mut effects = Vec::with_capacity(model.fixed.len());
    for (name, (_, col)) in model.fixed.iter().zip(kept_fixed) {
        let n_cols = if class_vars.iter().any(|c| c.eq_ignore_ascii_case(name)) {
            crate::procs::lincom::class_levels(col)
                .len()
                .saturating_sub(1)
        } else {
            1
        };
        effects.push(EffectColumns {
            name: name.clone(),
            cols: (next..next + n_cols).collect(),
        });
        next += n_cols;
    }
    if next != p {
        return Err(SasError::runtime(format!(
            "PROC GLIMMIX: internal error, {next} design columns attributed to the MODEL \
             effects instead of {p}."
        )));
    }
    Ok(effects)
}

/// Print the Type III Tests of Fixed Effects table: one row per MODEL effect,
/// without an Intercept row (SAS default; the MODEL INTERCEPT option adds it).
///
/// J02-P3 — the table used to list every parameter (Intercept and each
/// reference-coded CLASS column included) with F = t². A single-parameter
/// effect is tested by the Wald F = t² on 1 numerator DF; an effect with
/// several parameters needs the multi-DF Wald test of roadmap-avancee J08-P2,
/// so the table is withheld with a NOTE rather than misreported. An
/// intercept-only model has no effect to test and no table.
pub(super) fn print_type3_tests(
    session: &mut Session,
    effects: &[EffectColumns],
    fit: &GlimmixFit,
    den_df: f64,
) {
    if effects.is_empty() {
        return;
    }
    let multi: Vec<&str> = effects
        .iter()
        .filter(|e| e.cols.len() > 1)
        .map(|e| e.name.as_str())
        .collect();
    if !multi.is_empty() {
        session.log.note(&format!(
            "Type III tests of effects with more than one parameter ({}) are not \
             implemented in PROC GLIMMIX; the Type III Tests of Fixed Effects table is \
             not displayed (planned: roadmap-avancee J08-P2).",
            multi.join(", ")
        ));
        return;
    }
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
        let cov = &fit.cov_beta;
        let rows: Vec<Vec<String>> = effects
            .iter()
            .map(|eff| match eff.cols.first() {
                Some(&idx) => {
                    let est = fit.beta[idx];
                    let se = cov[idx][idx].max(0.0).sqrt();
                    let t = if se > 0.0 { est / se } else { 0.0 };
                    let f = t * t;
                    let p_val = 1.0 - f_cdf(f, 1.0, den_df);
                    vec![
                        eff.name.clone(),
                        "1".into(),
                        fmt_df(den_df),
                        fmt2(f),
                        fmt_p(p_val),
                    ]
                }
                // A CLASS effect with a single level has no parameter.
                None => vec![
                    eff.name.clone(),
                    "0".into(),
                    fmt_df(den_df),
                    ".".into(),
                    ".".into(),
                ],
            })
            .collect();
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
