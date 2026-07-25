use super::*;

/// MQ5.2 — the XPX / I / COVB / CORRB printed-matrices section of one
/// response's report.
pub(super) fn print_matrix_options(rf: &RespFit, session: &mut Session) {
    let &RespFit {
        model,
        dep_name,
        x_mat,
        y_vec,
        sel_reg_names,
        fit,
        intercept,
        n,
        p_eff,
        weighting,
        ..
    } = rf;
    // --- Printed matrices (M36.8): XPX / I / COVB / CORRB. Each is gated on its
    // MODEL option (and !noprint), computed from the existing fit (no refit), so
    // a MODEL without any of these options is byte-identical to before. MSE uses
    // the same error df as the ANOVA table (Σf_i − p_eff with FREQ active).
    if (model.xpx || model.inv || model.covb || model.corrb) && !model.noprint {
        let n_used: f64 = weighting.as_ref().map(|w| w.total_n).unwrap_or(n as f64);
        let error_df = n_used - p_eff as f64;
        let mse = if error_df > 0.0 { fit.sse / error_df } else { f64::NAN };
        if model.xpx {
            let xpx = build_xpx(&x_mat, &y_vec);
            print_xpx(&xpx, &sel_reg_names, dep_name, intercept, session);
        }
        if model.inv {
            print_inverse(
                &fit.xtx_inv,
                &fit.beta,
                fit.sse,
                &sel_reg_names,
                dep_name,
                intercept,
                session,
            );
        }
        if model.covb || model.corrb {
            print_estimate_matrices(
                model,
                &fit.xtx_inv,
                mse,
                &sel_reg_names,
                intercept,
                session,
            );
        }
    }
}

/// MQ5.2 — the collinearity / SPEC / DW / ACOV diagnostics and the CLM/CLI /
/// R / INFLUENCE observation-statistics sections of one response's report.
pub(super) fn print_diagnostic_options(rf: &RespFit, session: &mut Session) {
    let &RespFit {
        model,
        dep_name,
        x_mat,
        y_vec,
        sel_cols,
        sel_reg_names,
        fit,
        intercept,
        n,
        p_eff,
        weighting,
        id_first,
    } = rf;
    // --- Collinearity / specification / autocorrelation diagnostics (M36.4).
    // All gated on the corresponding flags (and !noprint), so a MODEL without
    // any of these options is byte-identical to before.
    if (model.collin || model.collinoint) && !model.noprint {
        if model.collin {
            if let Ok(c) = compute_collin(&x_mat, &sel_reg_names, intercept, false) {
                print_collin(&c, false, session);
            }
        }
        if model.collinoint {
            if let Ok(c) = compute_collin(&x_mat, &sel_reg_names, intercept, true) {
                print_collin(&c, true, session);
            }
        }
    }
    if model.spec && !model.noprint {
        print_spec_test(&sel_cols, &fit.resid, session);
    }
    if model.dw && !model.noprint {
        let dwr = durbin_watson(&fit.resid, &x_mat, &fit.xtx_inv, model.dwprob);
        print_durbin_watson(&dwr, session);
    }
    if model.acov && !model.noprint {
        let cov = acov_hc0(&x_mat, &fit.resid, &fit.xtx_inv);
        print_acov(
            &cov,
            &fit.beta,
            &sel_reg_names,
            intercept,
            (n - p_eff) as f64,
            session,
        );
    }

    // --- Output Statistics (M36.2): per-observation CLM / CLI limits. Driven
    // off the (unrestricted) OLS fit, gated on the CLM/CLI model options.
    if (model.clm || model.cli) && !model.noprint {
        print_output_statistics(
            model, dep_name, &x_mat, &y_vec, &fit, n, p_eff, weighting, id_first,
            session,
        );
    }

    // --- Residual / influence diagnostics (M36.3): MODEL R and INFLUENCE.
    // Computed lazily once off the OLS fit, shared by both listings.
    if (model.r || model.influence) && !model.noprint {
        let infl = compute_influence_stats(&x_mat, &y_vec, &fit, n, p_eff, weighting);
        if model.r {
            print_r_statistics(model, &infl, id_first, weighting, session);
        }
        if model.influence {
            print_influence_statistics(&infl, &sel_reg_names, intercept, id_first, session);
        }
    }
}

/// MQ5.2 — the TEST section of one response's report: operates on the model
/// as fitted (restricted if present).
pub(super) fn run_test_section(
    entry: &RegModelEntry,
    rf: &RespFit,
    restricted: Option<&Restricted>,
    session: &mut Session,
) -> Result<()> {
    let &RespFit {
        dep_name,
        x_mat,
        sel_reg_names,
        fit,
        intercept,
        n,
        ..
    } = rf;
    if !entry.tests.is_empty() {
        let (t_beta, t_xtx, t_sse, t_dfe) = match &restricted {
            Some(r) => (&r.beta_r, &fit.xtx_inv, r.sse_r, r.df_r),
            None => (
                &fit.beta,
                &fit.xtx_inv,
                fit.sse,
                (n - x_mat[0].len()) as f64,
            ),
        };
        run_tests(
            &entry.tests,
            &sel_reg_names,
            intercept,
            dep_name,
            t_beta,
            t_xtx,
            t_sse,
            t_dfe,
            x_mat[0].len(),
            session,
        )?;
    }
    Ok(())
}

/// MQ5.2 — the diagnostics / PLOTS rendering section of one response's
/// report (M29.3 deferral NOTE, M36.11 request set + PLOT statements).
pub(super) fn render_model_plots(ast: &RegAst, rf: &RespFit, session: &mut Session) {
    let &RespFit {
        model,
        dep_name,
        x_mat,
        y_vec,
        sel_reg_names,
        fit,
        intercept,
        n,
        p_eff,
        weighting,
        ..
    } = rf;
    // --- Diagnostics (M29.3) ---
    if ast.plots_requested {
        session.log.note("PLOTS options deferred in PROC REG.");
    }
    // M36.11 — the automatic residuals-vs-predicted diagnostic fires when ODS
    // graphics is on, UNLESS PLOTS=NONE was requested (which suppresses it).
    if session.ods_graphics.enabled && !ast.plot_requests.none {
        let y_hat = fit.y_hat.clone();
        let resid = fit.resid.clone();
        reg_diagnostic_plot(session, &y_hat, &resid);
    }
    // M36.11 — explicit PLOTS=(…) request set and traditional PLOT statements.
    // Under the default build each requested plot is a clean deferral NOTE;
    // under `--features graphics` each is rendered. NONE suppresses everything.
    if ast.plot_requests.any() {
        render_plot_requests(
            &ast.plot_requests,
            &x_mat,
            &y_vec,
            &fit,
            n,
            p_eff,
            model.alpha,
            &sel_reg_names,
            intercept,
            weighting,
            session,
        );
    }
    if !ast.plot_statements.is_empty() {
        render_plot_statements(
            &ast.plot_statements,
            dep_name,
            &x_mat,
            &fit,
            &sel_reg_names,
            intercept,
            session,
        );
    }
}
