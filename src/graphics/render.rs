//! M29.1 — Rendu d'images via `plotters` (compilé uniquement avec la feature
//! `graphics`).
//!
//! L'objectif de M29.1 est l'INFRASTRUCTURE, pas la qualité graphique : on
//! produit un canvas blanc, un titre, des axes libellés et les données. Le
//! critère de validation est qu'un fichier (PNG ou SVG) de taille > 0 soit
//! écrit aux dimensions demandées. PROC SGPLOT (M29.2) construira des
//! `DrawingSpec` plus riches au-dessus de cette même fonction.

use crate::error::SasError;
use crate::ods_graphics::ImageFmt;
use plotters::prelude::*;
use std::path::Path;

/// Type de tracé demandé.
#[derive(Debug, Clone, PartialEq)]
pub enum PlotType {
    /// Nuage de points (SCATTER).
    Scatter,
    /// Courbe reliant les points (SERIES).
    Series,
    /// Barres verticales par catégorie (VBAR).
    VBar,
    /// Histogramme (HISTOGRAM) avec un nombre de classes donné.
    Histogram { bins: usize },
    /// Camembert (PIE) — proportions par catégorie (M34.11).
    Pie,
}

/// Couleur logique d'une série, traduite vers une couleur `plotters`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesColor {
    Blue,
    Red,
    Green,
    Orange,
    Black,
}

#[cfg(feature = "graphics")]
impl SeriesColor {
    fn rgb(self) -> RGBColor {
        match self {
            SeriesColor::Blue => RGBColor(0, 0, 255),
            SeriesColor::Red => RGBColor(255, 0, 0),
            SeriesColor::Green => RGBColor(0, 160, 0),
            SeriesColor::Orange => RGBColor(255, 140, 0),
            SeriesColor::Black => RGBColor(0, 0, 0),
        }
    }
}

/// Palette par défaut indexée (pour multi-séries sans couleur explicite).
pub fn palette(i: usize) -> SeriesColor {
    const P: [SeriesColor; 5] = [
        SeriesColor::Blue,
        SeriesColor::Red,
        SeriesColor::Green,
        SeriesColor::Orange,
        SeriesColor::Black,
    ];
    P[i % P.len()]
}

/// Rendu d'une série superposée (overlay) : ligne, marqueurs, ou les deux.
/// Sert pour la courbe LOESS / DENSITY (ligne) et pour les séries multiples
/// GPLOT (`plot (y1 y2)*x` / `y*x=group`).
#[derive(Debug, Clone, PartialEq)]
pub struct Overlay {
    pub data: Vec<(f64, f64)>,
    pub color: SeriesColor,
    /// Tracer une ligne reliant les points.
    pub line: bool,
    /// Tracer un marqueur (cercle) sur chaque point.
    pub marker: bool,
}

/// Spécification d'un dessin, indépendante du backend.
#[derive(Debug, Clone)]
pub struct DrawingSpec {
    pub title: String,
    pub x_label: String,
    pub y_label: String,
    pub plot_type: PlotType,
    /// Données numériques (x, y) pour SCATTER / SERIES / HISTOGRAM (x = valeur).
    pub data: Vec<(f64, f64)>,
    /// Données (catégorie, valeur) pour VBAR / PIE.
    pub x_categorical: Vec<(String, f64)>,
}

impl DrawingSpec {
    /// Constructeur minimal (champs `data` / `x_categorical` vides). Pratique
    /// pour les appelants M34.11 qui passent par [`draw_to_file_ext`].
    pub fn new(
        title: impl Into<String>,
        x_label: impl Into<String>,
        y_label: impl Into<String>,
        plot_type: PlotType,
    ) -> Self {
        DrawingSpec {
            title: title.into(),
            x_label: x_label.into(),
            y_label: y_label.into(),
            plot_type,
            data: vec![],
            x_categorical: vec![],
        }
    }
}

/// Décorations additives M34.11 (overlays, bornes d'axes forcées), passées en
/// supplément du [`DrawingSpec`] à [`draw_to_file_ext`]. Conserve le struct
/// `DrawingSpec` rétro-compatible avec les appelants M29/M30.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Decorations {
    /// Séries superposées : courbes LOESS/DENSITY, séries multiples GPLOT.
    pub overlays: Vec<Overlay>,
    /// Bornes d'axe forcées (XAXIS/YAXIS VALUES= ou AXIS ORDER=). Ignoré
    /// pour VBAR/PIE.
    pub x_range: Option<(f64, f64)>,
    pub y_range: Option<(f64, f64)>,
}

/// Bornes (min, max) d'une suite de valeurs, avec une marge de sécurité pour
/// que les données ne touchent pas les bords. Renvoie une plage valide même
/// pour des données vides ou constantes (évite une panique de `plotters`).
fn safe_range(values: impl Iterator<Item = f64>) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for v in values {
        if v.is_finite() {
            if v < min {
                min = v;
            }
            if v > max {
                max = v;
            }
        }
    }
    if !min.is_finite() || !max.is_finite() {
        // Données vides : plage par défaut.
        return (0.0, 1.0);
    }
    if (max - min).abs() < f64::EPSILON {
        // Données constantes : élargir autour de la valeur.
        return (min - 1.0, max + 1.0);
    }
    let pad = (max - min) * 0.05;
    (min - pad, max + pad)
}

/// Dessine `spec` dans un fichier image à `path`, aux dimensions `width`×`height`
/// et au format `fmt`. Renvoie `(width, height)` en cas de succès.
///
/// Pour M29.1 le tracé est volontairement minimaliste. Le contrat fort est :
/// (1) un fichier non vide est écrit, (2) au format demandé, (3) aux dimensions
/// demandées. La fonction ne panique JAMAIS, y compris sur données vides.
pub fn draw_to_file(
    spec: &DrawingSpec,
    path: &Path,
    width: u32,
    height: u32,
    fmt: ImageFmt,
) -> Result<(u32, u32), SasError> {
    draw_to_file_ext(spec, &Decorations::default(), path, width, height, fmt)
}

/// Comme [`draw_to_file`] mais avec des décorations additives (overlays, bornes
/// d'axes forcées). Utilisé par PROC SGPLOT/GPLOT/GCHART (M34.11).
pub fn draw_to_file_ext(
    spec: &DrawingSpec,
    deco: &Decorations,
    path: &Path,
    width: u32,
    height: u32,
    fmt: ImageFmt,
) -> Result<(u32, u32), SasError> {
    match fmt {
        ImageFmt::Png => {
            let backend = BitMapBackend::new(path, (width, height));
            draw_on(backend.into_drawing_area(), spec, deco)?;
        }
        ImageFmt::Svg => {
            let backend = SVGBackend::new(path, (width, height));
            draw_on(backend.into_drawing_area(), spec, deco)?;
        }
    }
    Ok((width, height))
}

/// Cœur du rendu, générique sur le backend `plotters`.
fn draw_on<DB>(
    area: DrawingArea<DB, plotters::coord::Shift>,
    spec: &DrawingSpec,
    deco: &Decorations,
) -> Result<(), SasError>
where
    DB: DrawingBackend,
{
    area.fill(&WHITE)
        .map_err(|e| SasError::runtime(format!("graphics fill error: {e}")))?;

    // PIE : pas d'axes cartésiens — délégué à un rendu dédié.
    if spec.plot_type == PlotType::Pie {
        return draw_pie(&area, spec);
    }

    // Calcul des plages selon le type de tracé. Les overlays (LOESS, DENSITY,
    // séries multiples) participent aux bornes pour ne pas être tronqués.
    let overlay_x = || deco.overlays.iter().flat_map(|o| o.data.iter().map(|(x, _)| *x));
    let overlay_y = || deco.overlays.iter().flat_map(|o| o.data.iter().map(|(_, y)| *y));
    let (mut x_lo, mut x_hi, mut y_lo, mut y_hi): (f64, f64, f64, f64) = match &spec.plot_type {
        PlotType::VBar => {
            let (y_lo, y_hi) = safe_range(spec.x_categorical.iter().map(|(_, v)| *v));
            let n = spec.x_categorical.len().max(1) as f64;
            (0.0, n, y_lo.min(0.0), y_hi)
        }
        PlotType::Histogram { .. } => {
            let (x_lo, x_hi) = safe_range(spec.data.iter().map(|(x, _)| *x));
            // Densité superposée éventuelle : étendre l'axe Y pour la courbe.
            let (_, y_hi) = safe_range(overlay_y());
            (x_lo, x_hi, 0.0, y_hi.max(1.0))
        }
        PlotType::Pie => unreachable!("handled above"),
        _ => {
            let (x_lo, x_hi) =
                safe_range(spec.data.iter().map(|(x, _)| *x).chain(overlay_x()));
            let (y_lo, y_hi) =
                safe_range(spec.data.iter().map(|(_, y)| *y).chain(overlay_y()));
            (x_lo, x_hi, y_lo, y_hi)
        }
    };

    // Bornes forcées (XAXIS/YAXIS VALUES= ou AXIS ORDER=).
    if let Some((lo, hi)) = deco.x_range {
        if hi > lo {
            x_lo = lo;
            x_hi = hi;
        }
    }
    if let Some((lo, hi)) = deco.y_range {
        if hi > lo {
            y_lo = lo;
            y_hi = hi;
        }
    }

    let mut chart = ChartBuilder::on(&area)
        .caption(&spec.title, ("sans-serif", 20))
        .margin(10)
        .x_label_area_size(40)
        .y_label_area_size(50)
        .build_cartesian_2d(x_lo..x_hi, y_lo..y_hi)
        .map_err(|e| SasError::runtime(format!("graphics chart error: {e}")))?;

    chart
        .configure_mesh()
        .x_desc(&spec.x_label)
        .y_desc(&spec.y_label)
        .draw()
        .map_err(|e| SasError::runtime(format!("graphics mesh error: {e}")))?;

    match &spec.plot_type {
        PlotType::Scatter => {
            chart
                .draw_series(
                    spec.data
                        .iter()
                        .map(|(x, y)| Circle::new((*x, *y), 3, BLUE.filled())),
                )
                .map_err(|e| SasError::runtime(format!("graphics series error: {e}")))?;
        }
        PlotType::Series => {
            chart
                .draw_series(LineSeries::new(spec.data.iter().copied(), &BLUE))
                .map_err(|e| SasError::runtime(format!("graphics series error: {e}")))?;
        }
        PlotType::VBar => {
            for (i, (_, v)) in spec.x_categorical.iter().enumerate() {
                let x0 = i as f64 + 0.1;
                let x1 = i as f64 + 0.9;
                chart
                    .draw_series(std::iter::once(Rectangle::new(
                        [(x0, 0.0), (x1, *v)],
                        BLUE.filled(),
                    )))
                    .map_err(|e| SasError::runtime(format!("graphics vbar error: {e}")))?;
            }
        }
        PlotType::Histogram { bins } => {
            // Binning réel (M34.11) : on dessine des rectangles count par classe.
            let xs: Vec<f64> = spec.data.iter().map(|(x, _)| *x).collect();
            if !xs.is_empty() {
                let mn = xs.iter().cloned().fold(f64::INFINITY, f64::min);
                let mx = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                let nb = (*bins).max(1);
                let w = if mx > mn { (mx - mn) / nb as f64 } else { 1.0 };
                let mut counts = vec![0u32; nb];
                for &x in &xs {
                    let mut k = ((x - mn) / w).floor() as isize;
                    if k < 0 {
                        k = 0;
                    }
                    if k as usize >= nb {
                        k = nb as isize - 1;
                    }
                    counts[k as usize] += 1;
                }
                for (i, &c) in counts.iter().enumerate() {
                    let x0 = mn + i as f64 * w;
                    let x1 = x0 + w;
                    chart
                        .draw_series(std::iter::once(Rectangle::new(
                            [(x0, 0.0), (x1, c as f64)],
                            RGBColor(100, 149, 237).filled(),
                        )))
                        .map_err(|e| SasError::runtime(format!("graphics hist error: {e}")))?;
                }
            }
        }
        PlotType::Pie => unreachable!("handled above"),
    }

    // Overlays : courbes (LOESS, DENSITY) et séries multiples.
    for ov in &deco.overlays {
        let col = ov.color.rgb();
        if ov.line {
            chart
                .draw_series(LineSeries::new(ov.data.iter().copied(), col.stroke_width(2)))
                .map_err(|e| SasError::runtime(format!("graphics overlay line error: {e}")))?;
        }
        if ov.marker {
            chart
                .draw_series(
                    ov.data
                        .iter()
                        .map(|(x, y)| Circle::new((*x, *y), 3, col.filled())),
                )
                .map_err(|e| SasError::runtime(format!("graphics overlay marker error: {e}")))?;
        }
    }

    area.present()
        .map_err(|e| SasError::runtime(format!("graphics present error: {e}")))?;
    Ok(())
}

/// Calcule les angles (début, fin) en radians de chaque part d'un camembert,
/// proportionnels aux valeurs (les valeurs négatives/non finies sont ignorées).
/// La somme des arcs vaut 2π (à epsilon près) dès qu'un total > 0 existe.
pub fn pie_angles(values: &[f64]) -> Vec<(f64, f64)> {
    let total: f64 = values.iter().filter(|v| v.is_finite() && **v > 0.0).sum();
    let mut out = Vec::with_capacity(values.len());
    if total <= 0.0 {
        return values.iter().map(|_| (0.0, 0.0)).collect();
    }
    let mut acc = 0.0;
    for &v in values {
        let frac = if v.is_finite() && v > 0.0 { v / total } else { 0.0 };
        let start = acc * std::f64::consts::TAU;
        acc += frac;
        let end = acc * std::f64::consts::TAU;
        out.push((start, end));
    }
    out
}

/// Rendu d'un camembert via tracé direct `plotters` (pas d'axes cartésiens).
fn draw_pie<DB>(
    area: &DrawingArea<DB, plotters::coord::Shift>,
    spec: &DrawingSpec,
) -> Result<(), SasError>
where
    DB: DrawingBackend,
{
    let (w, h) = area.dim_in_pixel();
    // Titre en haut.
    area.draw(&Text::new(
        spec.title.clone(),
        (10, 6),
        ("sans-serif", 20).into_font(),
    ))
    .map_err(|e| SasError::runtime(format!("graphics pie title error: {e}")))?;

    let values: Vec<f64> = spec.x_categorical.iter().map(|(_, v)| *v).collect();
    let angles = pie_angles(&values);

    let cx = w as f64 * 0.5;
    let cy = h as f64 * 0.55;
    let radius = (w.min(h) as f64) * 0.35;

    for (i, (start, end)) in angles.iter().enumerate() {
        if (end - start).abs() < f64::EPSILON {
            continue;
        }
        let col = palette(i).rgb();
        // Triangulation de la part en segments d'arc.
        let steps = ((end - start) / 0.05).ceil().max(1.0) as usize;
        let mut poly = vec![(cx as i32, cy as i32)];
        for s in 0..=steps {
            let a = start + (end - start) * s as f64 / steps as f64;
            let px = cx + radius * a.cos();
            let py = cy + radius * a.sin();
            poly.push((px as i32, py as i32));
        }
        area.draw(&Polygon::new(poly, col.filled()))
            .map_err(|e| SasError::runtime(format!("graphics pie slice error: {e}")))?;
    }

    // Légende simple : labels de catégorie sous le camembert.
    let mut ly = (cy + radius) as i32 + 20;
    for (i, (label, value)) in spec.x_categorical.iter().enumerate() {
        let col = palette(i).rgb();
        area.draw(&Rectangle::new(
            [(12, ly), (24, ly + 12)],
            col.filled(),
        ))
        .map_err(|e| SasError::runtime(format!("graphics pie legend error: {e}")))?;
        area.draw(&Text::new(
            format!("{label} ({value})"),
            (30, ly),
            ("sans-serif", 14).into_font(),
        ))
        .map_err(|e| SasError::runtime(format!("graphics pie legend text error: {e}")))?;
        ly += 18;
    }

    area.present()
        .map_err(|e| SasError::runtime(format!("graphics present error: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn draw_empty_data_does_not_panic() {
        let spec = DrawingSpec::new("Empty", "x", "y", PlotType::Scatter);
        let path = tmp_path("sasrs_test_empty.png");
        let _ = std::fs::remove_file(&path);
        let res = draw_to_file(&spec, &path, 400, 300, ImageFmt::Png);
        assert!(res.is_ok(), "draw_to_file should not fail: {res:?}");
        assert_eq!(res.unwrap(), (400, 300));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn draw_scatter_creates_nonempty_png() {
        let mut spec = DrawingSpec::new("Scatter", "x", "y", PlotType::Scatter);
        spec.data = vec![(1.0, 2.0), (2.0, 3.0), (3.0, 5.0)];
        let path = tmp_path("sasrs_test_scatter.png");
        let _ = std::fs::remove_file(&path);
        draw_to_file(&spec, &path, 600, 400, ImageFmt::Png).unwrap();
        assert!(path.exists(), "PNG must exist");
        assert!(path.metadata().unwrap().len() > 0, "PNG must be non-empty");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn draw_svg_creates_nonempty_file() {
        let mut spec = DrawingSpec::new("Series", "x", "y", PlotType::Series);
        spec.data = vec![(0.0, 0.0), (1.0, 1.0)];
        let path = tmp_path("sasrs_test_series.svg");
        let _ = std::fs::remove_file(&path);
        draw_to_file(&spec, &path, 500, 500, ImageFmt::Svg).unwrap();
        assert!(path.exists());
        assert!(path.metadata().unwrap().len() > 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn pie_angles_sum_to_tau_and_proportional() {
        let vals = [10.0, 25.0, 15.0, 30.0];
        let angles = pie_angles(&vals);
        assert_eq!(angles.len(), 4);
        // Somme des arcs = 2π.
        let total: f64 = angles.iter().map(|(s, e)| e - s).sum();
        assert!((total - std::f64::consts::TAU).abs() < 1e-9, "total={total}");
        // Part proportionnelle : 10/80 du tour pour la 1re.
        let sum: f64 = vals.iter().sum();
        let span0 = angles[0].1 - angles[0].0;
        assert!((span0 - std::f64::consts::TAU * 10.0 / sum).abs() < 1e-9);
        // Parts contiguës (fin de l'une = début de la suivante).
        for w in angles.windows(2) {
            assert!((w[0].1 - w[1].0).abs() < 1e-12);
        }
    }

    #[test]
    fn pie_angles_all_zero_no_panic() {
        let angles = pie_angles(&[0.0, 0.0]);
        assert_eq!(angles, vec![(0.0, 0.0), (0.0, 0.0)]);
    }

    #[test]
    fn draw_pie_creates_nonempty_png() {
        let mut spec = DrawingSpec::new("Pie", "cat", "freq", PlotType::Pie);
        spec.x_categorical = vec![
            ("A".into(), 10.0),
            ("B".into(), 25.0),
            ("C".into(), 15.0),
        ];
        let path = tmp_path("sasrs_test_pie.png");
        let _ = std::fs::remove_file(&path);
        let res = draw_to_file(&spec, &path, 600, 500, ImageFmt::Png).unwrap();
        assert_eq!(res, (600, 500));
        assert!(path.exists());
        assert!(path.metadata().unwrap().len() > 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn draw_with_overlay_creates_nonempty_png() {
        let mut spec = DrawingSpec::new("Scatter+curve", "x", "y", PlotType::Scatter);
        spec.data = vec![(1.0, 2.0), (2.0, 3.0), (3.0, 5.0)];
        let deco = Decorations {
            overlays: vec![Overlay {
                data: vec![(1.0, 2.0), (2.0, 3.5), (3.0, 4.8)],
                color: SeriesColor::Red,
                line: true,
                marker: false,
            }],
            x_range: None,
            y_range: None,
        };
        let path = tmp_path("sasrs_test_overlay.png");
        let _ = std::fs::remove_file(&path);
        draw_to_file_ext(&spec, &deco, &path, 500, 400, ImageFmt::Png).unwrap();
        assert!(path.metadata().unwrap().len() > 0);
        let _ = std::fs::remove_file(&path);
    }
}
