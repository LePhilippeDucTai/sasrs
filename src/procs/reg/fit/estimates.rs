use super::*;

/// MQ5.2 — everything the parameter-estimates table prints for one model
/// beyond the model/regressor context: the estimates, their tests, and the
/// optional RESTRICT / VIF-TOL / partial-SS blocks.
pub(super) struct PeTableCtx<'a> {
    pub(super) beta: &'a [f64],
    pub(super) se_beta: &'a [f64],
    pub(super) t_beta: &'a [f64],
    pub(super) p_beta: &'a [f64],
    pub(super) error_df: f64,
    pub(super) p_eff: usize,
    pub(super) restricted: Option<&'a Restricted>,
    pub(super) tolvif: Option<&'a (Vec<f64>, Vec<f64>)>,
    pub(super) seqstats: Option<&'a SeqStats>,
}

/// MQ5.2 — the printed Parameter Estimates table (with the optional CLB /
/// Tolerance-VIF / partial-SS / RESTRICT columns and rows).
pub(super) fn print_parameter_estimates(
    model: &RegModel,
    reg_names: &[String],
    intercept: bool,
    pe: &PeTableCtx,
    session: &mut Session,
) {
    let &PeTableCtx {
        beta,
        se_beta,
        t_beta,
        p_beta,
        error_df,
        p_eff,
        restricted,
        tolvif,
        seqstats,
    } = pe;
    // Parameter estimates table. With RESTRICT statements a trailing Label
    // column carries the restriction expression; the unrestricted path keeps
    // the original 6-column layout byte-identical.
    let with_label = restricted.is_some();
    // CLB (M36.2): append two confidence-limit columns to the parameter table.
    let with_clb = model.clb;
    let clb_level = 100.0 * (1.0 - model.alpha);
    let t_crit = t_quantile(1.0 - model.alpha / 2.0, error_df);
    // VIF / TOL columns (M36.4). SAS orders Tolerance before Variance Inflation.
    let with_tol = model.tol && tolvif.is_some();
    let with_vif = model.vif && tolvif.is_some();
    let with_seq = seqstats.is_some();
    let (pe_headers, pe_aligns) = pe_table_columns(
        model, with_label, with_clb, clb_level, with_tol, with_vif, with_seq,
    );
    let mut pe_rows: Vec<Vec<String>> = Vec::with_capacity(p_eff);
    for j in 0..p_eff {
        let var_name = if intercept {
            if j == 0 {
                "Intercept".to_string()
            } else {
                reg_names[j - 1].clone()
            }
        } else {
            reg_names[j].clone()
        };
        let mut row = vec![
            var_name,
            "1".into(),
            fmt5(beta[j]),
            fmt5(se_beta[j]),
            fmt2(t_beta[j]),
            fmt_p(Some(p_beta[j])),
        ];
        if with_clb {
            row.push(fmt5(beta[j] - t_crit * se_beta[j]));
            row.push(fmt5(beta[j] + t_crit * se_beta[j]));
        }
        if with_tol || with_vif {
            // Map design column j to a regressor index (intercept has none).
            let reg_idx = if intercept {
                if j == 0 { None } else { Some(j - 1) }
            } else {
                Some(j)
            };
            let (tv, vv) = tolvif.expect("tolvif present when columns requested");
            if with_tol {
                // Intercept row: Tolerance blank.
                match reg_idx {
                    Some(k) => row.push(fmt5(tv[k])),
                    None => row.push(String::new()),
                }
            }
            if with_vif {
                // Intercept row: SAS prints 0 for the intercept VIF.
                match reg_idx {
                    Some(k) => row.push(if vv[k].is_finite() {
                        fmt5(vv[k])
                    } else {
                        // Perfect collinearity → SAS prints a very large value;
                        // render a sentinel `.` for non-finite.
                        ".".to_string()
                    }),
                    None => row.push(fmt5(0.0)),
                }
            }
        }
        if let Some(ss) = seqstats {
            if model.ss1 {
                row.push(fmt5(ss.ss1[j]));
            }
            if model.ss2 {
                row.push(fmt5(ss.ss2[j]));
            }
            if model.stb {
                row.push(fmt5(ss.stb[j]));
            }
            if model.pcorr1 {
                row.push(fmt5(ss.pcorr1[j]));
            }
            if model.pcorr2 {
                row.push(fmt5(ss.pcorr2[j]));
            }
            if model.scorr1 {
                row.push(fmt5(ss.scorr1[j]));
            }
            if model.scorr2 {
                row.push(fmt5(ss.scorr2[j]));
            }
            if model.seqb {
                row.push(if ss.seqb[j].is_finite() {
                    fmt5(ss.seqb[j])
                } else {
                    ".".to_string()
                });
            }
        }
        if with_label {
            row.push(String::new());
        }
        pe_rows.push(row);
    }
    // Append RESTRICT rows — see `append_restrict_rows`.
    append_restrict_rows(
        model,
        restricted,
        with_clb,
        with_tol,
        with_vif,
        with_seq,
        &mut pe_rows,
    );
    session
        .listing
        .write_table(&pe_headers, &pe_aligns, &pe_rows);
}

/// MQ5.2 — build the Parameter Estimates table's headers and alignments from
/// the requested optional columns.
pub(super) fn pe_table_columns(
    model: &RegModel,
    with_label: bool,
    with_clb: bool,
    clb_level: f64,
    with_tol: bool,
    with_vif: bool,
    with_seq: bool,
) -> (Vec<String>, Vec<Align>) {
    let mut pe_headers: Vec<String> = vec![
        "Variable".into(),
        "DF".into(),
        "Parameter Estimate".into(),
        "Standard Error".into(),
        "t Value".into(),
        "Pr > |t|".into(),
    ];
    let mut pe_aligns = vec![
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    if with_clb {
        pe_headers.push(format!("{}% Confidence Limits", fmt_level(clb_level)));
        pe_aligns.push(Align::Right);
        // The interval prints as two value columns under one spanning header;
        // emit a second (blank-titled) column to carry the upper limit.
        pe_headers.push(String::new());
        pe_aligns.push(Align::Right);
    }
    if with_tol {
        pe_headers.push("Tolerance".into());
        pe_aligns.push(Align::Right);
    }
    if with_vif {
        pe_headers.push("Variance Inflation".into());
        pe_aligns.push(Align::Right);
    }
    // M36.5 partial-SS / correlation columns. SAS appends them in this order:
    // Type I SS, Type II SS, Standardized Estimate, Squared Partial Corr Type I,
    // Squared Partial Corr Type II, Squared Semi-partial Corr Type I, Squared
    // Semi-partial Corr Type II, Sequential Parameter Estimate.
    if with_seq {
        if model.ss1 {
            pe_headers.push("Type I SS".into());
            pe_aligns.push(Align::Right);
        }
        if model.ss2 {
            pe_headers.push("Type II SS".into());
            pe_aligns.push(Align::Right);
        }
        if model.stb {
            pe_headers.push("Standardized Estimate".into());
            pe_aligns.push(Align::Right);
        }
        if model.pcorr1 {
            pe_headers.push("Squared Partial Corr Type I".into());
            pe_aligns.push(Align::Right);
        }
        if model.pcorr2 {
            pe_headers.push("Squared Partial Corr Type II".into());
            pe_aligns.push(Align::Right);
        }
        if model.scorr1 {
            pe_headers.push("Squared Semi-partial Corr Type I".into());
            pe_aligns.push(Align::Right);
        }
        if model.scorr2 {
            pe_headers.push("Squared Semi-partial Corr Type II".into());
            pe_aligns.push(Align::Right);
        }
        if model.seqb {
            pe_headers.push("Sequential Parameter Estimate".into());
            pe_aligns.push(Align::Right);
        }
    }
    if with_label {
        pe_headers.push("Label".into());
        pe_aligns.push(Align::Left);
    }
    (pe_headers, pe_aligns)
}

// Append RESTRICT rows: Variable="RESTRICT", DF=-1 (negative per SAS),
// Estimate=λ_i, with the restriction expression in the Label column.
#[allow(clippy::too_many_arguments)]
pub(super) fn append_restrict_rows(
    model: &RegModel,
    restricted: Option<&Restricted>,
    with_clb: bool,
    with_tol: bool,
    with_vif: bool,
    with_seq: bool,
    pe_rows: &mut Vec<Vec<String>>,
) {
    if let Some(r) = restricted {
        for (label, lam, se, t, pv) in &r.lambda_rows {
            let mut row = vec![
                "RESTRICT".into(),
                "-1".into(),
                fmt5(*lam),
                fmt5(*se),
                fmt2(*t),
                fmt_p(Some(*pv)),
            ];
            if with_clb {
                // SAS leaves the confidence-limit cells blank for RESTRICT rows.
                row.push(String::new());
                row.push(String::new());
            }
            if with_tol {
                row.push(String::new());
            }
            if with_vif {
                row.push(String::new());
            }
            if with_seq {
                // SAS leaves the M36.5 partial-SS / correlation cells blank for
                // RESTRICT rows.
                for present in [
                    model.ss1,
                    model.ss2,
                    model.stb,
                    model.pcorr1,
                    model.pcorr2,
                    model.scorr1,
                    model.scorr2,
                    model.seqb,
                ] {
                    if present {
                        row.push(String::new());
                    }
                }
            }
            row.push(label.clone());
            pe_rows.push(row);
        }
    }
}
