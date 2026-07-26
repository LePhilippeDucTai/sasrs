use super::*;

/// Generate (or defer) the automatic residuals-vs-predicted diagnostic plot
/// after a MODEL statement, when `ods_graphics.enabled` is true (the caller
/// checks this). Default build: a deferral NOTE; `--features graphics`: a
/// `reg_{N}` scatter image (x = predicted, y = residual).
pub(crate) fn reg_diagnostic_plot(session: &mut Session, y_hat: &[f64], resid: &[f64]) {
    #[cfg(not(feature = "graphics"))]
    {
        let _ = (y_hat, resid);
        session
            .log
            .note("REG diagnostics: image deferred (compile with --features graphics).");
    }

    #[cfg(feature = "graphics")]
    {
        use crate::graphics::render::{DrawingSpec, PlotType, draw_to_file};

        let data: Vec<(f64, f64)> = y_hat
            .iter()
            .zip(resid.iter())
            .filter(|(p, r)| p.is_finite() && r.is_finite())
            .map(|(p, r)| (*p, *r))
            .collect();
        let spec = DrawingSpec {
            title: "The REG Procedure".to_string(),
            x_label: "Predicted Value".to_string(),
            y_label: "Residual".to_string(),
            plot_type: PlotType::Scatter,
            data,
            x_categorical: vec![],
        };

        session.graphics_image_count += 1;
        let stem = session
            .ods_graphics
            .file_stem
            .clone()
            .unwrap_or_else(|| "reg".to_string());
        let fmt = session.ods_graphics.image_format;
        let name = format!(
            "{}_{}.{}",
            stem,
            session.graphics_image_count,
            fmt.extension()
        );
        let path = session.ods_graphics.output_dir.join(&name);
        match draw_to_file(
            &spec,
            &path,
            session.ods_graphics.width,
            session.ods_graphics.height,
            fmt,
        ) {
            Ok((w, h)) => {
                session
                    .log
                    .note(&format!("Output '{}' ({}x{}) written.", name, w, h));
            }
            Err(e) => {
                session
                    .log
                    .note(&format!("WARNING: could not write image {}: {}", name, e));
            }
        }
    }
}

/// M36.11 — render (or defer) the explicit `PLOTS=(…)` diagnostic request set
/// after a MODEL fit. Default build: one plural-invariant deferral NOTE listing
/// the requested plot count. `--features graphics`: each requested plot family
/// (DIAGNOSTICS, RESIDUALS, FIT) is rendered as a separate `reg_{N}` image
/// (panel components are emitted unpacked). `ALL` expands to all three families.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_plot_requests(
    req: &PlotRequests,
    x_mat: &[Vec<f64>],
    y: &[f64],
    fit: &OlsFit,
    n: usize,
    p_eff: usize,
    alpha: f64,
    sel_reg_names: &[String],
    intercept: bool,
    weighting: Option<&Weighting>,
    session: &mut Session,
) {
    let want_diag = req.all || req.diagnostics;
    let want_resid = req.all || req.residuals;
    let want_fit = req.all || req.fit;

    #[cfg(not(feature = "graphics"))]
    {
        let _ = (
            x_mat,
            y,
            fit,
            n,
            p_eff,
            alpha,
            sel_reg_names,
            intercept,
            weighting,
        );
        let count = [want_diag, want_resid, want_fit]
            .iter()
            .filter(|b| **b)
            .count();
        session.log.note(&format!(
            "REG PLOTS= request: {} plot(s) deferred (compile with --features graphics).",
            count
        ));
    }

    #[cfg(feature = "graphics")]
    {
        use crate::graphics::render::{Decorations, DrawingSpec, Overlay, PlotType, SeriesColor};

        let obs = compute_obs_stats(x_mat, y, fit, n, p_eff, alpha, weighting);
        let infl = compute_influence_stats(x_mat, y, fit, n, p_eff, weighting);

        // Helper: residual-vs-predicted scatter (the core diagnostics panel cell).
        if want_diag {
            // (1) Residual vs predicted.
            let data: Vec<(f64, f64)> = fit
                .y_hat
                .iter()
                .zip(fit.resid.iter())
                .filter(|(p, r)| p.is_finite() && r.is_finite())
                .map(|(p, r)| (*p, *r))
                .collect();
            render_reg_image(
                session,
                &DrawingSpec {
                    title: "Fit Diagnostics — Residual by Predicted".to_string(),
                    x_label: "Predicted Value".to_string(),
                    y_label: "Residual".to_string(),
                    plot_type: PlotType::Scatter,
                    data,
                    x_categorical: vec![],
                },
                &Decorations::default(),
            );

            // (2) RStudent vs predicted.
            let data: Vec<(f64, f64)> = infl
                .iter()
                .filter(|s| s.y_hat.is_finite() && s.rstudent.is_finite())
                .map(|s| (s.y_hat, s.rstudent))
                .collect();
            render_reg_image(
                session,
                &DrawingSpec {
                    title: "Fit Diagnostics — RStudent by Predicted".to_string(),
                    x_label: "Predicted Value".to_string(),
                    y_label: "RStudent".to_string(),
                    plot_type: PlotType::Scatter,
                    data,
                    x_categorical: vec![],
                },
                &Decorations::default(),
            );

            // (3) Cook's D vs leverage.
            let data: Vec<(f64, f64)> = infl
                .iter()
                .filter(|s| s.h.is_finite() && s.cookd.is_finite())
                .map(|s| (s.h, s.cookd))
                .collect();
            render_reg_image(
                session,
                &DrawingSpec {
                    title: "Fit Diagnostics — Cook's D by Leverage".to_string(),
                    x_label: "Leverage".to_string(),
                    y_label: "Cook's D".to_string(),
                    plot_type: PlotType::Scatter,
                    data,
                    x_categorical: vec![],
                },
                &Decorations::default(),
            );

            // (4) Normal QQ-plot of residuals: sorted residuals vs normal scores.
            let mut rs: Vec<f64> = fit
                .resid
                .iter()
                .copied()
                .filter(|r| r.is_finite())
                .collect();
            rs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let m = rs.len();
            let qq: Vec<(f64, f64)> = rs
                .iter()
                .enumerate()
                .map(|(i, &r)| {
                    let p = (i as f64 + 0.5) / m as f64;
                    (normal_quantile(p), r)
                })
                .collect();
            render_reg_image(
                session,
                &DrawingSpec {
                    title: "Fit Diagnostics — Normal Q-Q".to_string(),
                    x_label: "Quantile".to_string(),
                    y_label: "Residual".to_string(),
                    plot_type: PlotType::Scatter,
                    data: qq,
                    x_categorical: vec![],
                },
                &Decorations::default(),
            );
        }

        // RESIDUALS — residual vs each regressor column.
        if want_resid {
            let off = usize::from(intercept);
            for (j, name) in sel_reg_names.iter().enumerate() {
                let col = off + j;
                let data: Vec<(f64, f64)> = (0..n)
                    .filter_map(|i| {
                        let xv = x_mat[i][col];
                        let rv = fit.resid[i];
                        (xv.is_finite() && rv.is_finite()).then_some((xv, rv))
                    })
                    .collect();
                render_reg_image(
                    session,
                    &DrawingSpec {
                        title: format!("Residual by {}", name),
                        x_label: name.clone(),
                        y_label: "Residual".to_string(),
                        plot_type: PlotType::Scatter,
                        data,
                        x_categorical: vec![],
                    },
                    &Decorations::default(),
                );
            }
        }

        // FIT — single-regressor fit plot with the regression line and the
        // CLM (mean) / CLI (individual) confidence bands. Only meaningful when
        // there is exactly one regressor (plus the intercept).
        if want_fit {
            let off = usize::from(intercept);
            if sel_reg_names.len() == 1 {
                let col = off; // the single regressor column
                let mut pts: Vec<(f64, ObsStat)> = (0..n)
                    .filter(|&i| x_mat[i][col].is_finite())
                    .map(|i| (x_mat[i][col], obs[i].clone()))
                    .collect();
                pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

                let data: Vec<(f64, f64)> = pts.iter().map(|(x, o)| (*x, o.y)).collect();
                let fit_line: Vec<(f64, f64)> = pts.iter().map(|(x, o)| (*x, o.y_hat)).collect();
                let clm_lo: Vec<(f64, f64)> = pts.iter().map(|(x, o)| (*x, o.lclm)).collect();
                let clm_hi: Vec<(f64, f64)> = pts.iter().map(|(x, o)| (*x, o.uclm)).collect();
                let cli_lo: Vec<(f64, f64)> = pts.iter().map(|(x, o)| (*x, o.lcl)).collect();
                let cli_hi: Vec<(f64, f64)> = pts.iter().map(|(x, o)| (*x, o.ucl)).collect();
                let deco = Decorations {
                    overlays: vec![
                        Overlay {
                            data: fit_line,
                            color: SeriesColor::Blue,
                            line: true,
                            marker: false,
                        },
                        Overlay {
                            data: clm_lo,
                            color: SeriesColor::Green,
                            line: true,
                            marker: false,
                        },
                        Overlay {
                            data: clm_hi,
                            color: SeriesColor::Green,
                            line: true,
                            marker: false,
                        },
                        Overlay {
                            data: cli_lo,
                            color: SeriesColor::Orange,
                            line: true,
                            marker: false,
                        },
                        Overlay {
                            data: cli_hi,
                            color: SeriesColor::Orange,
                            line: true,
                            marker: false,
                        },
                    ],
                    x_range: None,
                    y_range: None,
                };
                render_reg_image(
                    session,
                    &DrawingSpec {
                        title: format!("Fit Plot by {}", sel_reg_names[0]),
                        x_label: sel_reg_names[0].clone(),
                        y_label: "Response".to_string(),
                        plot_type: PlotType::Scatter,
                        data,
                        x_categorical: vec![],
                    },
                    &deco,
                );
            } else {
                // Multi-regressor fit plot is not meaningful; fall back to a
                // response-vs-predicted scatter (the SAS panel's analogue).
                let data: Vec<(f64, f64)> = (0..n)
                    .filter(|&i| fit.y_hat[i].is_finite() && y[i].is_finite())
                    .map(|i| (fit.y_hat[i], y[i]))
                    .collect();
                render_reg_image(
                    session,
                    &DrawingSpec {
                        title: "Fit Plot — Observed by Predicted".to_string(),
                        x_label: "Predicted Value".to_string(),
                        y_label: "Response".to_string(),
                        plot_type: PlotType::Scatter,
                        data,
                        x_categorical: vec![],
                    },
                    &Decorations::default(),
                );
            }
        }
    }
}

/// M36.11 — render (or defer) the traditional `PLOT y*x …;` scatters after a
/// MODEL fit. Default build: one plural-invariant deferral NOTE per statement
/// listing the pair count. `--features graphics`: each (y,x) pair is rendered as
/// a separate `reg_{N}` scatter, resolving the `PREDICTED.`/`RESIDUAL.` special
/// variables from the fit and plain names from the design matrix.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_plot_statements(
    pairs: &[PlotPair],
    dep_name: &str,
    x_mat: &[Vec<f64>],
    fit: &OlsFit,
    sel_reg_names: &[String],
    intercept: bool,
    session: &mut Session,
) {
    #[cfg(not(feature = "graphics"))]
    {
        let _ = (dep_name, x_mat, fit, sel_reg_names, intercept);
        session.log.note(&format!(
            "REG PLOT statement: {} scatter(s) deferred (compile with --features graphics).",
            pairs.len()
        ));
    }

    #[cfg(feature = "graphics")]
    {
        use crate::graphics::render::{Decorations, DrawingSpec, PlotType};

        // Resolve a PlotVar to a per-observation value series + an axis label.
        let resolve = |v: &PlotVar| -> Option<(Vec<f64>, String)> {
            match v {
                PlotVar::Predicted => Some((fit.y_hat.clone(), "Predicted Value".to_string())),
                PlotVar::Residual => Some((fit.resid.clone(), "Residual".to_string())),
                PlotVar::Named(name) => {
                    let up = name.to_ascii_uppercase();
                    if up == dep_name.to_ascii_uppercase() {
                        // The dependent: reconstruct y = ŷ + resid.
                        let yv: Vec<f64> = fit
                            .y_hat
                            .iter()
                            .zip(fit.resid.iter())
                            .map(|(p, r)| p + r)
                            .collect();
                        return Some((yv, name.clone()));
                    }
                    // A regressor column.
                    let off = usize::from(intercept);
                    sel_reg_names
                        .iter()
                        .position(|r| r.eq_ignore_ascii_case(&up))
                        .map(|j| {
                            let col = off + j;
                            let vals: Vec<f64> = (0..x_mat.len()).map(|i| x_mat[i][col]).collect();
                            (vals, name.clone())
                        })
                }
            }
        };

        for pair in pairs {
            let (Some((ys, ylab)), Some((xs, xlab))) = (resolve(&pair.y), resolve(&pair.x)) else {
                session.log.note(
                    "REG PLOT statement: a variable could not be resolved; that plot is skipped.",
                );
                continue;
            };
            let data: Vec<(f64, f64)> = xs
                .iter()
                .zip(ys.iter())
                .filter(|(x, y)| x.is_finite() && y.is_finite())
                .map(|(x, y)| (*x, *y))
                .collect();
            render_reg_image(
                session,
                &DrawingSpec {
                    title: format!("Plot of {} by {}", ylab, xlab),
                    x_label: xlab,
                    y_label: ylab,
                    plot_type: PlotType::Scatter,
                    data,
                    x_categorical: vec![],
                },
                &Decorations::default(),
            );
        }
    }
}

/// M36.11 (graphics only) — render one `DrawingSpec`+`Decorations` into the next
/// `reg_{N}.{fmt}` image in `ods_graphics.output_dir`, mirroring the naming /
/// counter / NOTE convention of `reg_diagnostic_plot`.
#[cfg(feature = "graphics")]
pub(crate) fn render_reg_image(
    session: &mut Session,
    spec: &crate::graphics::render::DrawingSpec,
    deco: &crate::graphics::render::Decorations,
) {
    use crate::graphics::render::draw_to_file_ext;

    session.graphics_image_count += 1;
    let stem = session
        .ods_graphics
        .file_stem
        .clone()
        .unwrap_or_else(|| "reg".to_string());
    let fmt = session.ods_graphics.image_format;
    let name = format!(
        "{}_{}.{}",
        stem,
        session.graphics_image_count,
        fmt.extension()
    );
    let path = session.ods_graphics.output_dir.join(&name);
    match draw_to_file_ext(
        spec,
        deco,
        &path,
        session.ods_graphics.width,
        session.ods_graphics.height,
        fmt,
    ) {
        Ok((w, h)) => {
            session
                .log
                .note(&format!("Output '{}' ({}x{}) written.", name, w, h));
        }
        Err(e) => {
            session
                .log
                .note(&format!("WARNING: could not write image {}: {}", name, e));
        }
    }
}

/// M36.11 (graphics only) — approximate standard-normal quantile (inverse CDF)
/// via the Beasley-Springer/Moro algorithm, for the QQ-plot's theoretical
/// scores. Accuracy is ample for plotting; not used on any printed path.
#[cfg(feature = "graphics")]
pub(crate) fn normal_quantile(p: f64) -> f64 {
    // Clamp away from the open-interval endpoints.
    let p = p.clamp(1e-9, 1.0 - 1e-9);
    // Rational approximation (Acklam) — relative error < 1.15e-9.
    const A: [f64; 6] = [
        -3.969683028665376e+01,
        2.209460984245205e+02,
        -2.759285104469687e+02,
        1.383577518672690e+02,
        -3.066479806614716e+01,
        2.506628277459239e+00,
    ];
    const B: [f64; 5] = [
        -5.447609879822406e+01,
        1.615858368580409e+02,
        -1.556989798598866e+02,
        6.680131188771972e+01,
        -1.328068155288572e+01,
    ];
    const C: [f64; 6] = [
        -7.784894002430293e-03,
        -3.223964580411365e-01,
        -2.400758277161838e+00,
        -2.549732539343734e+00,
        4.374664141464968e+00,
        2.938163982698783e+00,
    ];
    const D: [f64; 4] = [
        7.784695709041462e-03,
        3.224671290700398e-01,
        2.445134137142996e+00,
        3.754408661907416e+00,
    ];
    let plow = 0.02425;
    let phigh = 1.0 - plow;
    if p < plow {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= phigh {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    }
}
