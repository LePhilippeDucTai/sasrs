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
    /// Données (catégorie, valeur) pour VBAR.
    pub x_categorical: Vec<(String, f64)>,
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
    match fmt {
        ImageFmt::Png => {
            let backend = BitMapBackend::new(path, (width, height));
            draw_on(backend.into_drawing_area(), spec)?;
        }
        ImageFmt::Svg => {
            let backend = SVGBackend::new(path, (width, height));
            draw_on(backend.into_drawing_area(), spec)?;
        }
    }
    Ok((width, height))
}

/// Cœur du rendu, générique sur le backend `plotters`.
fn draw_on<DB>(
    area: DrawingArea<DB, plotters::coord::Shift>,
    spec: &DrawingSpec,
) -> Result<(), SasError>
where
    DB: DrawingBackend,
{
    area.fill(&WHITE)
        .map_err(|e| SasError::runtime(format!("graphics fill error: {e}")))?;

    // Calcul des plages selon le type de tracé.
    let (x_lo, x_hi, y_lo, y_hi): (f64, f64, f64, f64) = match &spec.plot_type {
        PlotType::VBar => {
            let (y_lo, y_hi) = safe_range(spec.x_categorical.iter().map(|(_, v)| *v));
            let n = spec.x_categorical.len().max(1) as f64;
            (0.0, n, y_lo.min(0.0), y_hi)
        }
        PlotType::Histogram { .. } => {
            let (x_lo, x_hi) = safe_range(spec.data.iter().map(|(x, _)| *x));
            (x_lo, x_hi, 0.0, 1.0)
        }
        _ => {
            let (x_lo, x_hi) = safe_range(spec.data.iter().map(|(x, _)| *x));
            let (y_lo, y_hi) = safe_range(spec.data.iter().map(|(_, y)| *y));
            (x_lo, x_hi, y_lo, y_hi)
        }
    };

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
        PlotType::Histogram { .. } => {
            // M29.1 : tracé minimal — les points bruts comme marqueurs.
            // Le binning réel arrive en M29.2.
            chart
                .draw_series(
                    spec.data
                        .iter()
                        .map(|(x, _)| Circle::new((*x, 0.0), 2, RED.filled())),
                )
                .map_err(|e| SasError::runtime(format!("graphics hist error: {e}")))?;
        }
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
        let spec = DrawingSpec {
            title: "Empty".into(),
            x_label: "x".into(),
            y_label: "y".into(),
            plot_type: PlotType::Scatter,
            data: vec![],
            x_categorical: vec![],
        };
        let path = tmp_path("sasrs_test_empty.png");
        let _ = std::fs::remove_file(&path);
        let res = draw_to_file(&spec, &path, 400, 300, ImageFmt::Png);
        assert!(res.is_ok(), "draw_to_file should not fail: {res:?}");
        assert_eq!(res.unwrap(), (400, 300));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn draw_scatter_creates_nonempty_png() {
        let spec = DrawingSpec {
            title: "Scatter".into(),
            x_label: "x".into(),
            y_label: "y".into(),
            plot_type: PlotType::Scatter,
            data: vec![(1.0, 2.0), (2.0, 3.0), (3.0, 5.0)],
            x_categorical: vec![],
        };
        let path = tmp_path("sasrs_test_scatter.png");
        let _ = std::fs::remove_file(&path);
        draw_to_file(&spec, &path, 600, 400, ImageFmt::Png).unwrap();
        assert!(path.exists(), "PNG must exist");
        assert!(path.metadata().unwrap().len() > 0, "PNG must be non-empty");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn draw_svg_creates_nonempty_file() {
        let spec = DrawingSpec {
            title: "Series".into(),
            x_label: "x".into(),
            y_label: "y".into(),
            plot_type: PlotType::Series,
            data: vec![(0.0, 0.0), (1.0, 1.0)],
            x_categorical: vec![],
        };
        let path = tmp_path("sasrs_test_series.svg");
        let _ = std::fs::remove_file(&path);
        draw_to_file(&spec, &path, 500, 500, ImageFmt::Svg).unwrap();
        assert!(path.exists());
        assert!(path.metadata().unwrap().len() > 0);
        let _ = std::fs::remove_file(&path);
    }
}
