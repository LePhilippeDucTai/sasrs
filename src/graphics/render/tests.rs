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
