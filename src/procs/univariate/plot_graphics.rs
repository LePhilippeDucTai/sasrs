use super::*;
use crate::graphics::render::{
    Decorations, DrawingSpec, Overlay, PlotType, SeriesColor, draw_to_file_ext,
};

/// Number of points sampled along a fitted normal density curve.
const NORMAL_CURVE_POINTS: usize = 100;

/// (μ̂, σ̂) fitted to the values actually plotted — sample mean and sample
/// standard deviation (VARDEF=DF), the same estimators as the listing's
/// "Fitted Normal Distribution" table.
///
/// The image path plots raw values: like the rest of the M29.3/M33.2 rendering
/// it ignores WEIGHT and BY (`plot_values` does too), so with a WEIGHT
/// statement the drawn curve is the UNWEIGHTED fit while the listing table
/// reports the weighted μ̂/σ̂. Without WEIGHT and BY — the usual case — the two
/// coincide exactly.
fn normal_fit(xs: &[f64]) -> Option<(f64, f64)> {
    if xs.len() < 2 {
        return None;
    }
    let mean = xs.iter().sum::<f64>() / xs.len() as f64;
    Some((mean, sample_std(xs)?))
}

/// M45.2 — overlay of the fitted normal distribution requested by `/ NORMAL`,
/// or `None` when the plot did not ask for one (or cannot carry one).
///
/// - **HISTOGRAM**: the normal density rescaled to the histogram's *Percent*
///   axis. With `bins` classes spanning `[min, max]` the class width is
///   `h = (max-min)/bins`, and the expected percent of observations in a class
///   centred on `x` is `100 · h · φ((x-μ)/σ) / σ`. Sampled on
///   [`NORMAL_CURVE_POINTS`] points so the curve is smooth at any image size.
/// - **QQPLOT / PROBPLOT**: the reference line `y = μ + σ·x`, drawn across the
///   theoretical-quantile range actually plotted (two points suffice).
/// - **CDFPLOT / PPPLOT**: no overlay (documented M45.2 limit) — the listing
///   still carries the fitted-parameters table.
///
/// `xs` are the plotted values, `data` the points of the base plot.
pub(super) fn normal_overlay(
    plot: &UnivariatePlot,
    xs: &[f64],
    data: &[(f64, f64)],
    mu: f64,
    sigma: f64,
) -> Option<Overlay> {
    let fittable = sigma > 0.0; // false for NaN too — no distribution to fit
    if !plot.normal || !fittable || xs.len() < 2 {
        return None;
    }
    let points = match plot.kind {
        UnivariatePlotKind::Histogram => {
            let (lo, hi) = xs.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |a, &v| {
                (a.0.min(v), a.1.max(v))
            });
            let PlotType::Histogram { bins } = plot_kind_histogram_bins() else {
                return None;
            };
            let spread = hi > lo; // false for an empty or constant sample
            if !spread || bins == 0 {
                return None;
            }
            let h = (hi - lo) / bins as f64;
            let step = (hi - lo) / (NORMAL_CURVE_POINTS - 1) as f64;
            (0..NORMAL_CURVE_POINTS)
                .map(|i| {
                    let x = lo + step * i as f64;
                    let z = (x - mu) / sigma;
                    let density =
                        (-0.5 * z * z).exp() / (sigma * (2.0 * std::f64::consts::PI).sqrt());
                    (x, 100.0 * h * density)
                })
                .collect()
        }
        UnivariatePlotKind::QqPlot | UnivariatePlotKind::ProbPlot => {
            // The x axis carries the theoretical normal quantiles of `data`.
            let (lo, hi) = data
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |a, &(x, _)| {
                    (a.0.min(x), a.1.max(x))
                });
            if !hi.is_finite() || !lo.is_finite() {
                return None;
            }
            vec![(lo, mu + sigma * lo), (hi, mu + sigma * hi)]
        }
        UnivariatePlotKind::CdfPlot | UnivariatePlotKind::PpPlot => return None,
    };
    Some(Overlay {
        data: points,
        color: SeriesColor::Red,
        line: true,
        marker: false,
    })
}

/// The single place the HISTOGRAM class count is chosen, shared by the base
/// plot and its `/ NORMAL` overlay so the curve is scaled to the very bins
/// that are drawn.
fn plot_kind_histogram_bins() -> PlotType {
    PlotType::Histogram { bins: 10 }
}

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
            plot_type: plot_kind_histogram_bins(),
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

    // M45.2 — `/ NORMAL` : ajuste la loi sur les valeurs tracées (μ̂, σ̂ =
    // moyenne et écart-type d'échantillon, VARDEF=DF) et superpose la courbe.
    let deco = match normal_fit(&xs) {
        Some((mu, sigma)) => Decorations {
            overlays: normal_overlay(plot, &xs, &spec.data, mu, sigma)
                .into_iter()
                .collect(),
            ..Decorations::default()
        },
        None => Decorations::default(),
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

    match draw_to_file_ext(
        &spec,
        &deco,
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
