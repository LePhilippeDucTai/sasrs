use super::*;

mod cases;

pub(super) use cases::*;

/// Run a single MODEL statement: resolve columns, do listwise deletion, then
/// dispatch to the default/NOINT path or the SELECTION path. Writes any OUTPUT
/// dataset associated with the model.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_model(
    ast: &RegAst,
    entry: &RegModelEntry,
    ds: &SasDataset,
    in_libref: &str,
    in_table: &str,
    rows: &[usize],
    weight_col: Option<&[crate::value::Value]>,
    freq_col: Option<&[crate::value::Value]>,
    id_cols: &[(String, Vec<crate::value::Value>)],
    model_label: &str,
    // M36.7: BY-group heading line, threaded down so it can be emitted inside
    // the per-model header block (after "The REG Procedure"). `None` when no BY.
    by_heading: Option<&str>,
    // M36.8: OUTEST= accumulator. `Some` when a PROC OUTEST= was requested; this
    // model pushes its PARMS/cov/se data here for the per-PROC writer. `None`
    // keeps the OUTEST-free path byte-identical.
    outest_accum: Option<&mut Vec<OutEstEntry>>,
    session: &mut Session,
) -> Result<()> {
    let _ = (in_libref, in_table);
    let model = &entry.model;
    // M36.10 run-group editing (ADD/DELETE) — see `effective_regressors`.
    let regressors_owned: Option<Vec<String>> = effective_regressors(entry);
    let regressors: &[String] = match &regressors_owned {
        Some(v) => v.as_slice(),
        None => &model.regressors,
    };
    let p = regressors.len();
    let n_read = rows.len();

    // --- Resolve + decode the regressor columns (shared across responses) ---
    let reg_cols = decode_regressor_columns(ds, regressors)?;

    // M36.10 multi-response MODEL (`model y1 y2 = x;`): SAS PROC REG prints a
    // SEPARATE univariate regression analysis for EACH dependent, in MODEL
    // order, then the MTEST tables once. We loop the existing univariate
    // fit+print path once per response. With a single response (`dependents`
    // has one element) the loop body runs exactly once and the emitted output —
    // headers, spacing, NOTE lines, OUTEST rows, OUTPUT datasets — is
    // byte-identical to the prior single-response path.
    //
    // Per-response OUTPUT/diagnostics: each response's OUTPUT dataset, R /
    // INFLUENCE / CLM / CLI listings, OUTEST row, and printed matrices are
    // produced from that response's own fit, exactly as a univariate run would.
    // OUTSSCP / SIMPLE / CORR are PROC-level displays driven off the FIRST
    // model's variables and are emitted once in the caller, unchanged.
    let mut outest_accum = outest_accum;
    // MTEST runs ONCE after all per-response blocks. It needs the selected design
    // (intercept + chosen regressors). With no SELECTION this is identical for
    // every response; with SELECTION the prior code used the (sole) response's
    // choice, so we capture the FIRST response's selection here for parity.
    let mut mtest_inputs: Option<(Vec<usize>, Vec<String>, bool)> = None;
    for (resp_i, dep_name) in model.dependents.iter().enumerate() {
        let dep_name: &str = dep_name.as_str();
        let dep_idx = find_col(ds, dep_name)?;
        if ds.vars[dep_idx].ty != VarType::Num {
            return Err(SasError::runtime(format!(
                "Dependent variable {} must be numeric.",
                dep_name.to_uppercase()
            )));
        }
        let dep_col = decode_column(ds, dep_idx)?;

        // --- M36.7 WEIGHT/FREQ bookkeeping + listwise deletion — see
        // `gather_complete_cases`. When no WEIGHT and no FREQ are present,
        // `weighting` stays inactive and the whole analysis is byte-identical to
        // the prior OLS path.
        let CaseData {
            xcols,
            y_vec,
            complete_mask,
            weighting,
            id_used,
        } = gather_complete_cases(ds, rows, weight_col, freq_col, id_cols, &dep_col, &reg_cols);
        let id_first: Option<&[String]> = if id_cols.is_empty() {
            None
        } else {
            Some(&id_used)
        };

        let n = y_vec.len();
        session
            .log
            .note(&format!("There were {} observations used.", n));

        let intercept = !model.noint;

        // --- SELECTION path: choose the final regressor subset, then fit/print it.
        let selected: Vec<usize> = if let Some(sel) = model.selection {
            match run_selection(&sel, &xcols, &y_vec, regressors, intercept, model, session) {
                Some(s) => s,
                None => {
                    // Nothing entered (FORWARD/STEPWISE) — fit the intercept-only
                    // model (or note for NOINT) and finish, no OUTPUT. With multiple
                    // responses, move on to the next dependent (`continue`); with a
                    // single response this ends the only iteration just as the prior
                    // `return Ok(())` did, so the output is unchanged.
                    fit_and_print_empty(
                        model,
                        dep_name,
                        &FitReportOptions {
                            n_read,
                            n,
                            model_label,
                            by_heading,
                            ..Default::default()
                        },
                        session,
                    );
                    continue;
                }
            }
        } else {
            (0..p).collect()
        };

        // Build the final design matrix over the selected columns.
        let sel_p = selected.len();
        let p_eff = sel_p + intercept as usize;

        if n <= p_eff {
            return Err(SasError::runtime("Not enough observations for regression"));
        }

        let mut x_mat: Vec<Vec<f64>> = Vec::with_capacity(n);
        for i in 0..n {
            let mut row = Vec::with_capacity(p_eff);
            if intercept {
                row.push(1.0);
            }
            for &c in &selected {
                row.push(xcols[c][i]);
            }
            x_mat.push(row);
        }

        let sel_reg_names: Vec<String> = selected.iter().map(|&c| regressors[c].clone()).collect();

        // Capture the first response's selected design for the post-loop MTEST.
        if resp_i == 0 {
            mtest_inputs = Some((selected.clone(), sel_reg_names.clone(), intercept));
        }

        // Weighted-least-squares fit when WEIGHT/FREQ is active; plain OLS
        // otherwise (byte-identical default path).
        let fit_result = match &weighting {
            Some(w) => weighted_ols_fit(&x_mat, &y_vec, &w.wf),
            None => ols_fit(&x_mat, &y_vec),
        };
        let fit = match fit_result {
            Ok(f) => f,
            Err(e) => {
                session.log.error(&format!("Regression failed: {}", e));
                return Err(e);
            }
        };

        // --- RESTRICT (M36.1): re-estimate under the linear constraints. The model
        // is printed as the restricted fit; TEST then operates on that fit. When
        // there are no RESTRICT statements this stays None and the OLS path is
        // byte-identical to before.
        let restricted = if entry.restricts.is_empty() {
            None
        } else {
            compute_restricted(
                &entry.restricts,
                &sel_reg_names,
                intercept,
                &x_mat,
                &y_vec,
                &fit,
                n,
            )?
        };

        // --- VIF / TOL (M36.4): per-regressor tolerance & variance inflation,
        // computed from the selected regressor columns (no intercept). Only built
        // when requested so the default path stays allocation-free / byte-identical.
        let sel_cols: Vec<Vec<f64>> = selected.iter().map(|&c| xcols[c].clone()).collect();
        let tolvif = if (model.vif || model.tol) && !model.noprint {
            Some(vif_tol(&sel_cols))
        } else {
            None
        };

        // --- Partial-SS / correlation statistics (M36.5). Computed on the OLS fit
        // (not the restricted fit). Only built when any SS1/SS2/STB/PCORR/SCORR/SEQB
        // option is requested, so the default / RESTRICT paths stay byte-identical.
        let want_seq = model.ss1
            || model.ss2
            || model.stb
            || model.pcorr1
            || model.pcorr2
            || model.scorr1
            || model.scorr2
            || model.seqb;
        // SST consistent with fit_and_print: corrected total (intercept) or
        // uncorrected total Σy² (NOINT).
        let sst_seq = if intercept {
            let ybar = y_vec.iter().sum::<f64>() / n as f64;
            y_vec.iter().map(|v| (v - ybar) * (v - ybar)).sum()
        } else {
            y_vec.iter().map(|v| v * v).sum()
        };
        let seqstats = if want_seq && !model.noprint {
            Some(compute_seq_stats(
                model, &x_mat, &y_vec, &fit, sst_seq, intercept,
            ))
        } else {
            None
        };

        // PRESS statistic (M36.5) — see `compute_press_stat`.
        let press_stat = compute_press_stat(model, &x_mat, &fit, weighting.as_ref());

        // --- RIDGE= / PCOMIT= (M36.9): when requested, SAS replaces the ordinary
        // parameter-estimates analysis with the ridge / incomplete-principal-
        // component results. Entirely gated on the new PROC options so the default
        // path is byte-identical. Ridge/IPC operate on the standardized model and
        // require an intercept (SAS centers); NOINT ⇒ clean NOTE + skip.
        let want_ridge = !ast.data_options.ridge.is_empty();
        let want_ipc = !ast.data_options.pcomit.is_empty();
        let ridge_ipc_active = want_ridge || want_ipc;
        // Back-transformed ridge/IPC rows to attach to this model's OUTEST entry.
        let mut ridge_ipc_rows: Vec<RidgeIpcRow> = Vec::new();
        // Report context + optional statistics shared by both printers (the
        // ridge/IPC one only reads the header context).
        let fit_opts = FitReportOptions {
            n_read,
            n,
            intercept,
            model_label,
            restricted: restricted.as_ref(),
            tolvif: tolvif.as_ref(),
            seqstats: seqstats.as_ref(),
            press_stat,
            weighting: weighting.as_ref(),
            by_heading,
        };
        if ridge_ipc_active {
            if !intercept {
                session.log.note(
                    "RIDGE/PCOMIT regression requires an intercept; the NOINT model is skipped.",
                );
            } else {
                ridge_ipc_rows = fit_and_print_ridge_ipc(
                    ast,
                    model,
                    dep_name,
                    &sel_reg_names,
                    &sel_cols,
                    &y_vec,
                    &fit_opts,
                    session,
                );
            }
        } else {
            fit_and_print(model, dep_name, &sel_reg_names, &fit, &fit_opts, session);
        }

        // --- OUTEST= (M36.8): record this fit for the per-PROC parameter-estimates
        // dataset. Built from the existing fit (no refit). `_MODEL_` uses the bare
        // "MODELn" label (the model_label is "Model: MODELn").
        if let Some(accum) = outest_accum.as_deref_mut() {
            let n_used: f64 = weighting.as_ref().map(|w| w.total_n).unwrap_or(n as f64);
            let bare_label = model_label
                .strip_prefix("Model: ")
                .unwrap_or(model_label)
                .to_string();
            let mut entry = build_outest_entry(
                &bare_label,
                dep_name,
                &sel_reg_names,
                &fit,
                intercept,
                n_used,
                model.alpha,
            );
            // M36.9: attach back-transformed ridge / IPC rows so the per-PROC writer
            // emits the `_TYPE_`=RIDGE/RIDGEVIF/IPC rows alongside the PARMS row.
            entry.ridge_ipc = ridge_ipc_rows;
            accum.push(entry);
        }

        // MQ5.2 — shared per-response context for the post-fit option sections.
        let rf = RespFit {
            model,
            dep_name,
            x_mat: &x_mat,
            y_vec: &y_vec,
            sel_cols: &sel_cols,
            sel_reg_names: &sel_reg_names,
            fit: &fit,
            intercept,
            n,
            p_eff,
            weighting: weighting.as_ref(),
            id_first,
        };
        // --- Printed matrices (M36.8): XPX / I / COVB / CORRB — see
        // `print_matrix_options`.
        print_matrix_options(&rf, session);

        // --- Diagnostics / observation statistics (M36.2-M36.4) — see
        // `print_diagnostic_options`.
        print_diagnostic_options(&rf, session);

        // --- TEST (M36.1): operate on the model as fitted (restricted if present).
        run_test_section(entry, &rf, restricted.as_ref(), session)?;

        // --- OUTPUT dataset(s) for this model (complete cases only) ---
        write_outputs(
            entry,
            ds,
            &complete_mask,
            n,
            &fit,
            &x_mat,
            p_eff,
            model.alpha,
            &sel_reg_names,
            intercept,
            weighting.as_ref(),
            session,
        )?;

        // --- Diagnostics / PLOTS rendering (M29.3, M36.11) — see
        // `render_model_plots`.
        render_model_plots(ast, &rf, session);
    } // end per-response loop

    // --- MTEST (M36.10): multivariate hypothesis tests across all responses.
    // Self-contained: gathers a fresh multivariate response/design matrix with
    // listwise deletion across every response + the selected regressors, fits
    // the multivariate coefficient matrix, and prints the four MANOVA statistics
    // per MTEST. Printed ONCE, after every per-response univariate block. Only
    // entered when MTEST statements are present, so the single-response /
    // no-MTEST path is byte-identical.
    let run_mtest = !entry.mtests.is_empty() && !model.noprint;
    if let Some((selected, sel_reg_names, intercept)) = mtest_inputs.as_ref().filter(|_| run_mtest)
    {
        run_mtests(
            &entry.mtests,
            model,
            ds,
            rows,
            selected,
            sel_reg_names,
            *intercept,
            session,
        )?;
    }

    Ok(())
}
