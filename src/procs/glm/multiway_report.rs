use super::*;

// ───────────────────── Multiway execute helpers ─────────────────────

/// Print the listing page header, the Class Level Information table and the
/// observation count for the multiway path.
pub(super) fn print_class_level_info_multiway(
    session: &mut Session,
    class_cols: &[(String, Vec<Value>)],
    n_obs: usize,
) {
    session.listing.page_header();
    centered(session, "The GLM Procedure");
    session.listing.blank();

    centered(session, "Class Level Information");
    session.listing.blank();

    let cli_headers: Vec<String> = vec!["Class".into(), "Levels".into(), "Values".into()];
    let cli_aligns = vec![Align::Left, Align::Right, Align::Left];
    let mut cli_rows: Vec<Vec<String>> = Vec::new();
    for (name, col) in class_cols {
        let levels = crate::procs::lincom::class_levels(col.iter());
        let values_str: Vec<String> = levels.iter().map(level_label_value).collect();
        cli_rows.push(vec![
            name.clone(),
            format!("{}", levels.len()),
            values_str.join(" "),
        ]);
    }
    session.listing.write_table(&cli_headers, &cli_aligns, &cli_rows);
    session.listing.blank();
    session.listing.write_line(&format!(
        "               Number of Observations Read     {}",
        n_obs
    ));
    session.listing.blank();
    session.listing.blank();
}

/// Print the Analysis of Variance, fit statistics, and Type I / Type III SS
/// tables for the multiway path.
pub(super) fn print_multiway_anova_and_fit(session: &mut Session, dep_var: &str, fit: &MultiwayFit) {
    let &MultiwayFit {
        y_bar,
        sst,
        sse_full,
        ssm,
        df_error,
        df_model,
        df_total,
        mse,
        msm,
        f_model,
        p_model,
        r2,
        root_mse,
        cv,
        ..
    } = fit;
    let term_df = &fit.term_df;
    let term_labels = &fit.term_labels;
    let type1_ss = &fit.type1_ss;
    let type3_ss = &fit.type3_ss;

    // ANOVA table
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
            format!("{}", df_model as usize),
            fmt5(ssm),
            if msm.is_nan() { ".".into() } else { fmt5(msm) },
            if f_model.is_nan() { ".".into() } else { fmt2(f_model) },
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
    session.listing.write_table(&anova_headers, &anova_aligns, &anova_rows);
    session.listing.blank();
    session.listing.blank();

    // Fit statistics
    let dep_mean_header = format!("{} Mean", dep_var);
    let fit_headers: Vec<String> = vec![
        "R-Square".into(),
        "Coeff Var".into(),
        "Root MSE".into(),
        dep_mean_header,
    ];
    let fit_aligns = vec![Align::Right, Align::Right, Align::Right, Align::Right];
    let fit_rows: Vec<Vec<String>> = vec![vec![
        fmt6(r2),
        fmt6(cv),
        fmt6(root_mse),
        fmt6(y_bar),
    ]];
    session.listing.write_table(&fit_headers, &fit_aligns, &fit_rows);
    session.listing.blank();
    session.listing.blank();

    // Type I / Type III SS tables
    for (ss_label, ss_vec) in [("Type I SS", &type1_ss), ("Type III SS", &type3_ss)] {
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
        for (ti, &ss) in ss_vec.iter().enumerate() {
            let df = term_df[ti] as f64;
            let ms = if df > 0.0 { ss / df } else { f64::NAN };
            let f = if mse > 0.0 && !mse.is_nan() && !ms.is_nan() {
                ms / mse
            } else {
                f64::NAN
            };
            let p = if f.is_nan() {
                None
            } else {
                Some((1.0 - f_cdf(f, df, df_error)).clamp(0.0, 1.0))
            };
            t_rows.push(vec![
                term_labels[ti].clone(),
                format!("{}", term_df[ti]),
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

/// Print the Parameter Estimates table (/SOLUTION) for the multiway path.
pub(super) fn print_multiway_solution(
    session: &mut Session,
    model: &GlmModel,
    fit: &MultiwayFit,
    lincom_engine: &Option<crate::procs::lincom::LinCombEngine>,
    beta: &Option<Vec<f64>>,
    xtx_inv: &Option<Vec<Vec<f64>>>,
) {
    let col_specs = &fit.col_specs;
    let factors = &fit.factors;
    let term_factor_idxs = &fit.term_factor_idxs;
    let ncols = fit.ncols;
    let mse = fit.mse;
    let df_error = fit.df_error;

    centered(session, "Parameter Estimates");
    session.listing.blank();
    let param_headers: Vec<String> = vec![
        "Parameter".into(),
        "Estimate".into(),
        "Standard Error".into(),
        "t Value".into(),
        "Pr > |t|".into(),
    ];
    let param_aligns = vec![
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    let mut param_rows: Vec<Vec<String>> = Vec::new();

    // Column labels parallel to full_design columns: [Intercept, then term cols].
    let mut col_labels: Vec<String> = vec!["Intercept".into()];
    for (ti, specs) in col_specs.iter().enumerate() {
        let term = &model.effect_terms[ti];
        for spec in specs {
            // spec is Vec<(factor_idx, dummy_idx)>; build "fac LEVEL" pieces.
            let pieces: Vec<String> = spec
                .iter()
                .map(|&(fi, dj)| {
                    format!("{} {}", factors[fi].name, level_label_value(&factors[fi].levels[dj]))
                })
                .collect();
            let _ = term; // term name implied by factor names in pieces
            col_labels.push(pieces.join(" "));
        }
    }

    // M37.1: each parameter row is the estimable function selecting a
    // single design column (unit vector). Routed through the shared
    // engine — `estimate(unit_vec, 0)` is bit-identical to the previous
    // inline `b[col]` / `mse*inv[col][col]` arithmetic (the zero-skip and
    // unit vector collapse the quadratic form to `inv[col][col]`, and
    // signed-zero products do not perturb the dot-product sum).
    match (&lincom_engine, &beta) {
        (Some(eng), _) => {
            for (ci, lbl) in col_labels.iter().enumerate() {
                let mut l = vec![0.0; ncols];
                l[ci] = 1.0;
                let r = eng.estimate(&l, 0.0);
                param_rows.push(vec![
                    lbl.clone(),
                    fmt6(r.estimate),
                    fmt6(r.se),
                    fmt2(r.t),
                    fmt_p(r.p),
                ]);
            }
        }
        // Singular covariance (engine unavailable) but β solvable: keep the
        // previous inline behaviour exactly (SE = NaN for every column).
        (None, Some(b)) => {
            for (ci, lbl) in col_labels.iter().enumerate() {
                let est = b[ci];
                let se = match &xtx_inv {
                    Some(inv) if !mse.is_nan() && inv[ci][ci] >= 0.0 => {
                        (mse * inv[ci][ci]).sqrt()
                    }
                    _ => f64::NAN,
                };
                let t = if se > 0.0 { est / se } else { f64::NAN };
                let p = if t.is_nan() {
                    None
                } else {
                    Some(2.0 * (1.0 - student_t_cdf(t.abs(), df_error)))
                };
                param_rows.push(vec![
                    lbl.clone(),
                    fmt6(est),
                    fmt6(se),
                    fmt2(t),
                    fmt_p(p),
                ]);
            }
        }
        (None, None) => {}
    }
    // Reference-level rows (estimate 0, "B"), one per main effect's last level
    // and per interaction combination touching a reference level, mirroring
    // the one-way path's single reference row. We append the main-effect
    // reference rows for readability.
    for (fi, factor) in factors.iter().enumerate() {
        // Only emit a reference row if this factor appears as a main effect term.
        let is_main = term_factor_idxs.iter().any(|t| t.len() == 1 && t[0] == fi);
        if is_main {
            let ref_lvl = factor.levels.last();
            if let Some(rl) = ref_lvl {
                param_rows.push(vec![
                    format!("{} {}", factor.name, level_label_value(rl)),
                    "0".into(),
                    "B".into(),
                    "".into(),
                    "".into(),
                ]);
            }
        }
    }
    session.listing.write_table(&param_headers, &param_aligns, &param_rows);
    session.listing.blank();
    session.listing.blank();
}

/// Print the Least Squares Means tables (main effects only) for the multiway
/// path.
#[allow(clippy::too_many_arguments)]
pub(super) fn print_multiway_lsmeans(
    session: &mut Session,
    ast: &GlmAst,
    dep_var: &str,
    fit: &MultiwayFit,
    lincom_engine: &Option<crate::procs::lincom::LinCombEngine>,
    lincom_coding: &crate::procs::lincom::Coding,
    beta: &Option<Vec<f64>>,
    xtx_inv: &Option<Vec<Vec<f64>>>,
) {
    let factors = &fit.factors;
    let ncols = fit.ncols;
    let mse = fit.mse;
    let df_error = fit.df_error;

    for lsm_var in &ast.lsmeans_vars {
        let fi = match factors.iter().position(|f| f.name.eq_ignore_ascii_case(lsm_var)) {
            Some(i) => i,
            None => continue,
        };
        // Only meaningful for a main-effect factor.
        centered(session, "Least Squares Means");
        session.listing.blank();
        let lsm_headers: Vec<String> = vec![
            factors[fi].name.clone(),
            format!("{} LSMEAN", dep_var),
            "Standard Error".into(),
            "Pr > |t|".into(),
        ];
        let lsm_aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right];
        let mut lsm_rows: Vec<Vec<String>> = Vec::new();

        // LS mean for level L of factor fi = average over balanced (uniform)
        // levels of all OTHER factors of the predicted cell mean. With a
        // reference-cell coding and fitted β, the predicted value for a cell is
        // a linear combo of β; averaging the contrast vector over the other
        // factors' levels yields the LS-mean estimable function L·β.
        //
        // M37.1: delegated to the shared LinCombEngine, which reproduces the
        // exact estimable-function arithmetic (est = l·β, se = √(mse·l'·inv·l)
        // with the same zero-skip and accumulation order). When the engine
        // could not be built (singular fit), fall back to NaN rows in level
        // order, identical to the previous behaviour.
        match &lincom_engine {
            Some(eng) => {
                for r in eng.lsmeans(&factors[fi].name) {
                    lsm_rows.push(vec![
                        r.level_label,
                        fmt6(r.estimate),
                        fmt6(r.se),
                        fmt_p(r.p),
                    ]);
                }
            }
            // Degenerate fit (engine unavailable: X'X singular ⇒ no
            // covariance). Reproduce the original Option-aware arithmetic
            // exactly: est = l·β when β is solvable, se = NaN (no covariance).
            None => {
                for (li, level) in factors[fi].levels.iter().enumerate() {
                    let lvec = crate::procs::lincom::lsmean_coef(lincom_coding, fi, li);
                    let est = match &beta {
                        Some(b) => lvec.iter().zip(b).map(|(c, bb)| c * bb).sum::<f64>(),
                        None => f64::NAN,
                    };
                    let se = match &xtx_inv {
                        Some(inv) if !mse.is_nan() => {
                            let mut q = 0.0;
                            for a in 0..ncols {
                                if lvec[a] == 0.0 {
                                    continue;
                                }
                                for b2 in 0..ncols {
                                    q += lvec[a] * inv[a][b2] * lvec[b2];
                                }
                            }
                            if q >= 0.0 { (mse * q).sqrt() } else { f64::NAN }
                        }
                        _ => f64::NAN,
                    };
                    let t = if se > 0.0 { est / se } else { f64::NAN };
                    let p = if t.is_nan() {
                        None
                    } else {
                        Some(2.0 * (1.0 - student_t_cdf(t.abs(), df_error)))
                    };
                    lsm_rows.push(vec![
                        level_label_value(level),
                        fmt6(est),
                        fmt6(se),
                        fmt_p(p),
                    ]);
                }
            }
        }
        session.listing.write_table(&lsm_headers, &lsm_aligns, &lsm_rows);
        session.listing.blank();
        session.listing.blank();
    }
}

/// NOTE any CONTRAST / ESTIMATE statements referencing effects the multiway
/// path does not support.
pub(super) fn note_skipped_contrasts(
    session: &mut Session,
    ast: &GlmAst,
    model: &GlmModel,
    factors: &[Factor],
) {
    // --- CONTRAST / ESTIMATE: main-effect coefficient vectors only ---
    // Group means for a single main-effect factor are reconstructed via the
    // LS means above; ESTIMATE/CONTRAST referencing an interaction emit a NOTE.
    for c in &ast.contrasts {
        if model
            .effect_terms
            .iter()
            .any(|t| t.len() > 1 && t.iter().any(|v| v.eq_ignore_ascii_case(&c.effect)))
            && !factors.iter().any(|f| f.name.eq_ignore_ascii_case(&c.effect))
        {
            session.log.note(&format!(
                "CONTRAST '{}' references an effect not supported in the multiway path; skipped.",
                c.label
            ));
        }
    }
    for e in &ast.estimates {
        if !factors.iter().any(|f| f.name.eq_ignore_ascii_case(&e.effect)) {
            session.log.note(&format!(
                "ESTIMATE '{}' references an effect not supported in the multiway path; skipped.",
                e.label
            ));
        }
    }
}
