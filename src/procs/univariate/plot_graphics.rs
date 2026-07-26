use super::*;
use crate::graphics::render::{DrawingSpec, PlotType, draw_to_file};

pub fn render(
    session: &mut Session,
    plot: &UnivariatePlot,
    var_cols: &[usize],
    var_values: &[Vec<Value>],
    ds: &SasDataset,
) {
    let (label, xs) = plot_values(plot, var_cols, var_values, ds);

    let spec = match plot.kind {
        UnivariatePlotKind::Histogram => DrawingSpec {
            title: "The UNIVARIATE Procedure".to_string(),
            x_label: label,
            y_label: "Percent".to_string(),
            plot_type: PlotType::Histogram { bins: 10 },
            data: xs.iter().map(|&v| (v, 0.0)).collect(),
            x_categorical: vec![],
        },
        UnivariatePlotKind::QqPlot => {
            // Empirical quantiles (sorted data) vs theoretical normal
            // quantiles qnorm((i-0.375)/(n+0.25)) for i = 1..n.
            let mut sorted = xs.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            let n = sorted.len();
            let nf = n as f64;
            let data: Vec<(f64, f64)> = sorted
                .iter()
                .enumerate()
                .map(|(idx, &emp)| {
                    let i = idx as f64 + 1.0;
                    let theo = phi_inv((i - 0.375) / (nf + 0.25));
                    (theo, emp)
                })
                .collect();
            DrawingSpec {
                title: "The UNIVARIATE Procedure".to_string(),
                x_label: "Normal Quantiles".to_string(),
                y_label: label,
                plot_type: PlotType::Scatter,
                data,
                x_categorical: vec![],
            }
        }
        UnivariatePlotKind::ProbPlot => {
            // Normal probability plot: sorted data (y) vs theoretical
            // normal quantiles phi_inv((i-0.375)/(n+0.25)) (x), a QQ-style
            // scatter on a probability x-axis.
            let mut sorted = xs.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            let n = sorted.len();
            let nf = n as f64;
            let data: Vec<(f64, f64)> = sorted
                .iter()
                .enumerate()
                .map(|(idx, &emp)| {
                    let i = idx as f64 + 1.0;
                    let theo = phi_inv((i - 0.375) / (nf + 0.25));
                    (theo, emp)
                })
                .collect();
            DrawingSpec {
                title: "The UNIVARIATE Procedure".to_string(),
                x_label: "Normal Percentiles".to_string(),
                y_label: label,
                plot_type: PlotType::Scatter,
                data,
                x_categorical: vec![],
            }
        }
        UnivariatePlotKind::CdfPlot => {
            // Empirical CDF: sorted data (x) vs F_n(x) = i/n (y).
            let mut sorted = xs.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            let n = sorted.len();
            let nf = n as f64;
            let data: Vec<(f64, f64)> = sorted
                .iter()
                .enumerate()
                .map(|(idx, &v)| (v, (idx as f64 + 1.0) / nf * 100.0))
                .collect();
            DrawingSpec {
                title: "The UNIVARIATE Procedure".to_string(),
                x_label: label,
                y_label: "Cumulative Percent".to_string(),
                plot_type: PlotType::Scatter,
                data,
                x_categorical: vec![],
            }
        }
        UnivariatePlotKind::PpPlot => {
            // Probability-probability plot: empirical CDF (i-0.5)/n (x) vs
            // theoretical normal CDF Phi((x_(i)-mean)/std) (y).
            let mut sorted = xs.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            let n = sorted.len();
            let nf = n as f64;
            let mean = if n > 0 {
                sorted.iter().sum::<f64>() / nf
            } else {
                0.0
            };
            let std = sample_std(&sorted).unwrap_or(1.0);
            let std = if std > 0.0 { std } else { 1.0 };
            let data: Vec<(f64, f64)> = sorted
                .iter()
                .enumerate()
                .map(|(idx, &v)| {
                    let emp = (idx as f64 + 0.5) / nf;
                    let theo = probnorm((v - mean) / std);
                    (emp, theo)
                })
                .collect();
            DrawingSpec {
                title: "The UNIVARIATE Procedure".to_string(),
                x_label: "Empirical Cumulative Probability".to_string(),
                y_label: "Normal Cumulative Probability".to_string(),
                plot_type: PlotType::Scatter,
                data,
                x_categorical: vec![],
            }
        }
    };

    session.graphics_image_count += 1;
    let stem = session
        .ods_graphics
        .file_stem
        .clone()
        .unwrap_or_else(|| "univar".to_string());
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
