use super::*;

/// Render (or defer) a single UNIVARIATE graphical statement, when
/// `ods_graphics.enabled` is true (the caller checks this). In the default
/// build (no `graphics` feature) this only emits a deferral NOTE; under
/// `--features graphics` it materializes a PNG/SVG via the M29.1 infrastructure.
///
/// All five plot kinds (HISTOGRAM/QQPLOT/PROBPLOT/CDFPLOT/PPPLOT) are wired to
/// the image infrastructure identically (M33.2): in the default build this
/// emits the shared "image deferred" NOTE; under `--features graphics` each
/// renders an image.
pub(super) fn render_plot(
    session: &mut Session,
    plot: &UnivariatePlot,
    var_cols: &[usize],
    var_values: &[Vec<Value>],
    ds: &SasDataset,
) {
    #[cfg(not(feature = "graphics"))]
    {
        let _ = (plot, var_cols, var_values, ds);
        session
            .log
            .note("ODS GRAPHICS: image deferred (compile with --features graphics).");
    }

    #[cfg(feature = "graphics")]
    {
        plot_graphics::render(session, plot, var_cols, var_values, ds);
    }
}

/// Collect the non-missing numeric values of a plot's target variable, in the
/// PROC's VAR-list order. When the plot has an explicit variable, only that
/// variable's values are returned (empty if it is not part of the analysis
/// list); when it has none, the FIRST analysis variable is used (SAS plots all
/// analysis variables — v1 renders only the first, matching SGPLOT's "first
/// plot" convention).
#[cfg(feature = "graphics")]
pub(super) fn plot_values(
    plot: &UnivariatePlot,
    var_cols: &[usize],
    var_values: &[Vec<Value>],
    ds: &SasDataset,
) -> (String, Vec<f64>) {
    let vi = match &plot.var {
        Some(name) => var_cols
            .iter()
            .position(|&ci| ds.vars[ci].name.eq_ignore_ascii_case(name)),
        None => {
            if var_cols.is_empty() {
                None
            } else {
                Some(0)
            }
        }
    };
    match vi {
        Some(i) => {
            let label = ds.vars[var_cols[i]].name.clone();
            let xs: Vec<f64> = var_values[i]
                .iter()
                .filter_map(|v| value_to_num(v))
                .filter(|f| !f.is_nan())
                .collect();
            (label, xs)
        }
        None => (
            plot.var.clone().unwrap_or_default(),
            Vec::new(),
        ),
    }
}
