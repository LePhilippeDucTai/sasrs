use super::*;
use crate::source::SourceFile;
use crate::testkit::*;

fn parse_sgplot(src: &str) -> Result<SgplotAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // proc
    ts.next(); // sgplot
    parse(&mut ts)
}

#[allow(dead_code)]
// ── Parse tests ──────────────────────────────────────────────────────
#[test]
fn parse_scatter() {
    let ast = parse_sgplot("proc sgplot data=a; scatter x=age y=height; run;").unwrap();
    assert_eq!(ast.plot_stmts.len(), 1);
    match &ast.plot_stmts[0] {
        SgplotStmt::Scatter { x, y, group, .. } => {
            assert_eq!(x, "age");
            assert_eq!(y, "height");
            assert!(group.is_none());
        }
        other => panic!("expected Scatter, got {other:?}"),
    }
}

#[test]
fn parse_scatter_with_group_and_markerattrs() {
    let ast = parse_sgplot(
        "proc sgplot data=a; scatter x=age y=height / group=sex markerattrs=(symbol=circlefilled color=red); run;",
    )
    .unwrap();
    match &ast.plot_stmts[0] {
        SgplotStmt::Scatter {
            group, markerattrs, ..
        } => {
            assert_eq!(group.as_deref(), Some("sex"));
            let m = markerattrs.as_ref().unwrap();
            assert_eq!(m.symbol.as_deref(), Some("circlefilled"));
            assert_eq!(m.color.as_deref(), Some("red"));
        }
        other => panic!("expected Scatter, got {other:?}"),
    }
}

#[test]
fn parse_series() {
    let ast = parse_sgplot("proc sgplot data=a; series x=time y=value; run;").unwrap();
    match &ast.plot_stmts[0] {
        SgplotStmt::Series { x, y, .. } => {
            assert_eq!(x, "time");
            assert_eq!(y, "value");
        }
        other => panic!("expected Series, got {other:?}"),
    }
}

#[test]
fn parse_vbar() {
    let ast =
        parse_sgplot("proc sgplot data=a; vbar category / response=n stat=sum; run;").unwrap();
    match &ast.plot_stmts[0] {
        SgplotStmt::VBar {
            category,
            response,
            stat,
        } => {
            assert_eq!(category, "category");
            assert_eq!(response.as_deref(), Some("n"));
            assert_eq!(*stat, BarStat::Sum);
        }
        other => panic!("expected VBar, got {other:?}"),
    }
}

#[test]
fn parse_hbar_default_stat_freq() {
    let ast = parse_sgplot("proc sgplot data=a; hbar category / response=amount; run;").unwrap();
    match &ast.plot_stmts[0] {
        SgplotStmt::HBar { stat, .. } => assert_eq!(*stat, BarStat::Freq),
        other => panic!("expected HBar, got {other:?}"),
    }
}

#[test]
fn parse_histogram() {
    let ast = parse_sgplot("proc sgplot data=a; histogram height / binwidth=10; run;").unwrap();
    match &ast.plot_stmts[0] {
        SgplotStmt::Histogram { var, binwidth, .. } => {
            assert_eq!(var, "height");
            assert_eq!(*binwidth, Some(10.0));
        }
        other => panic!("expected Histogram, got {other:?}"),
    }
}

#[test]
fn parse_xaxis_yaxis() {
    let ast = parse_sgplot(
        "proc sgplot data=a; scatter x=age y=h; xaxis label='Age'; yaxis type=log; run;",
    )
    .unwrap();
    let x = ast.xaxis.as_ref().unwrap();
    assert_eq!(x.label.as_deref(), Some("Age"));
    let y = ast.yaxis.as_ref().unwrap();
    assert_eq!(y.type_, Some(AxisType::Log));
}

#[test]
fn parse_xaxis_values_range() {
    let ast =
        parse_sgplot("proc sgplot data=a; scatter x=age y=h; xaxis values=(0 to 100 by 10); run;")
            .unwrap();
    let x = ast.xaxis.as_ref().unwrap();
    assert_eq!(x.values_min, Some(0.0));
    assert_eq!(x.values_max, Some(100.0));
}

#[test]
fn parse_reg_default_degree() {
    let ast = parse_sgplot("proc sgplot data=a; reg x=age y=height; run;").unwrap();
    match &ast.plot_stmts[0] {
        SgplotStmt::Reg { degree, .. } => assert_eq!(*degree, 1),
        other => panic!("expected Reg, got {other:?}"),
    }
}

#[test]
fn parse_reg_degree2() {
    let ast = parse_sgplot("proc sgplot data=a; reg x=age y=height / degree=2; run;").unwrap();
    match &ast.plot_stmts[0] {
        SgplotStmt::Reg { degree, .. } => assert_eq!(*degree, 2),
        other => panic!("expected Reg, got {other:?}"),
    }
}

#[test]
fn parse_loess() {
    let ast = parse_sgplot("proc sgplot data=a; loess x=age y=height / smooth=0.5; run;").unwrap();
    match &ast.plot_stmts[0] {
        SgplotStmt::Loess { smooth, .. } => assert_eq!(*smooth, 0.5),
        other => panic!("expected Loess, got {other:?}"),
    }
}

#[test]
fn parse_density() {
    let ast = parse_sgplot("proc sgplot data=a; density height / kernel; run;").unwrap();
    match &ast.plot_stmts[0] {
        SgplotStmt::Density { var, kernel } => {
            assert_eq!(var, "height");
            assert!(*kernel);
        }
        other => panic!("expected Density, got {other:?}"),
    }
}

#[test]
fn parse_density_default_normal() {
    let ast = parse_sgplot("proc sgplot data=a; density height; run;").unwrap();
    match &ast.plot_stmts[0] {
        SgplotStmt::Density { kernel, .. } => assert!(!*kernel),
        other => panic!("expected Density, got {other:?}"),
    }
}

#[test]
fn parse_vbox() {
    let ast = parse_sgplot("proc sgplot data=a; vbox response / category=group; run;").unwrap();
    match &ast.plot_stmts[0] {
        SgplotStmt::VBox { category, response } => {
            assert_eq!(response, "response");
            assert_eq!(category.as_deref(), Some("group"));
        }
        other => panic!("expected VBox, got {other:?}"),
    }
}

#[test]
fn parse_by() {
    let ast = parse_sgplot("proc sgplot data=a; by sex; scatter x=age y=h; run;").unwrap();
    assert_eq!(ast.by_var.as_deref(), Some("sex"));
}

// ── Execute tests (default build) ────────────────────────────────────

#[test]
fn execute_without_ods_on_notes_not_enabled() {
    let mut session = make_session_in_temp();
    let ast = parse_sgplot("proc sgplot data=a; scatter x=age y=h; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("ODS GRAPHICS is not enabled"), "log: {log}");
}

#[cfg(not(feature = "graphics"))]
#[test]
fn execute_with_ods_on_no_feature_defers() {
    let mut session = make_session_in_temp();
    session.ods_graphics.enabled = true;
    let ast = parse_sgplot("proc sgplot data=a; scatter x=age y=h; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("image deferred"), "log: {log}");
}

#[cfg(not(feature = "graphics"))]
#[test]
fn execute_loess_defers() {
    let mut session = make_session_in_temp();
    session.ods_graphics.enabled = true;
    let ast = parse_sgplot("proc sgplot data=a; loess x=age y=h / smooth=0.5; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("LOESS plot deferred"), "log: {log}");
}

#[cfg(not(feature = "graphics"))]
#[test]
fn execute_density_defers() {
    let mut session = make_session_in_temp();
    session.ods_graphics.enabled = true;
    let ast = parse_sgplot("proc sgplot data=a; density h; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("DENSITY plot deferred"), "log: {log}");
}

#[test]
fn execute_by_defers() {
    let mut session = make_session_in_temp();
    session.ods_graphics.enabled = true;
    let ast = parse_sgplot("proc sgplot data=a; by sex; scatter x=age y=h; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("BY-group processing deferred"), "log: {log}");
}

// ── Execute tests (feature graphics) ─────────────────────────────────

#[cfg(feature = "graphics")]
fn write_heights(session: &mut Session, table: &str) {
    use crate::dataset::{SasDataset, VarMeta};
    use crate::value::VarType;
    use polars::df;
    let df = df![
        "age" => [10.0_f64, 12.0, 14.0, 16.0, 18.0],
        "height" => [140.0_f64, 150.0, 158.0, 165.0, 170.0]
    ]
    .unwrap();
    let vars = vec![
        VarMeta {
            name: "age".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
        VarMeta {
            name: "height".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
    ];
    let ds = SasDataset { df, vars };
    session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
}

#[cfg(feature = "graphics")]
#[test]
fn execute_with_graphics_writes_image() {
    let mut session = make_session_in_temp();
    session.ods_graphics.enabled = true;
    session.ods_graphics.output_dir = std::env::temp_dir();
    session.ods_graphics.file_stem = Some("sgtest_single".into());
    write_heights(&mut session, "H");
    let ast = parse_sgplot("proc sgplot data=work.h; scatter x=age y=height; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("written"), "log: {log}");
    let p = std::env::temp_dir().join("sgtest_single_1.png");
    assert!(p.exists(), "image not created: {p:?}");
    assert!(p.metadata().unwrap().len() > 0);
    let _ = std::fs::remove_file(&p);
}

#[cfg(feature = "graphics")]
#[test]
fn execute_sequential_naming() {
    let mut session = make_session_in_temp();
    session.ods_graphics.enabled = true;
    session.ods_graphics.output_dir = std::env::temp_dir();
    session.ods_graphics.file_stem = Some("sgtest_seq".into());
    write_heights(&mut session, "H");
    let ast = parse_sgplot("proc sgplot data=work.h; scatter x=age y=height; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    execute(&ast, &mut session).unwrap();
    let p1 = std::env::temp_dir().join("sgtest_seq_1.png");
    let p2 = std::env::temp_dir().join("sgtest_seq_2.png");
    assert!(p1.exists(), "first image missing");
    assert!(p2.exists(), "second image missing");
    let _ = std::fs::remove_file(&p1);
    let _ = std::fs::remove_file(&p2);
}

// ── M34.11 : LOESS / DENSITY oracles ─────────────────────────────────

#[cfg(feature = "graphics")]
#[test]
fn loess_curve_npoints_and_monotone_x() {
    use graphics_impl::loess_curve;
    let xs: Vec<f64> = (0..20).map(|i| i as f64).collect();
    let ys: Vec<f64> = xs.iter().map(|x| 2.0 * x + 1.0).collect();
    let curve = loess_curve(&xs, &ys, 0.5, 50);
    assert_eq!(curve.len(), 50, "expected 50 points");
    // x strictement croissant.
    for w in curve.windows(2) {
        assert!(w[1].0 > w[0].0, "x not monotone: {:?}", w);
    }
    // Sur données linéaires, LOESS local-linéaire reproduit la droite.
    for (x, y) in &curve {
        assert!((y - (2.0 * x + 1.0)).abs() < 1e-6, "x={x} y={y}");
    }
}

#[cfg(feature = "graphics")]
#[test]
fn loess_curve_degenerate_returns_empty() {
    use graphics_impl::loess_curve;
    assert!(loess_curve(&[1.0], &[2.0], 0.5, 10).is_empty());
    assert!(loess_curve(&[3.0, 3.0, 3.0], &[1.0, 2.0, 3.0], 0.5, 10).is_empty());
}

#[cfg(feature = "graphics")]
#[test]
fn normal_density_peaks_at_mean_and_integrates_to_one() {
    use graphics_impl::normal_density_curve;
    // Échantillon symétrique autour de 0.
    let xs: Vec<f64> = (-50..=50).map(|i| i as f64 / 10.0).collect();
    let curve = normal_density_curve(&xs, 400);
    assert_eq!(curve.len(), 400);
    // Intégrale par trapèzes ≈ 1 (la plage couvre l'essentiel de la masse).
    let mut area = 0.0;
    for w in curve.windows(2) {
        area += 0.5 * (w[0].1 + w[1].1) * (w[1].0 - w[0].0);
    }
    assert!((area - 1.0).abs() < 0.05, "area={area}");
    // pdf au mode ≈ 1/(sd*sqrt(2π)) ; vérifie que le max est près de x=mean=0.
    let (mode_x, _) =
        curve.iter().cloned().fold(
            (0.0, f64::NEG_INFINITY),
            |acc, p| if p.1 > acc.1 { p } else { acc },
        );
    assert!(mode_x.abs() < 0.5, "mode_x={mode_x}");
}

#[cfg(feature = "graphics")]
#[test]
fn kernel_density_integrates_to_one() {
    use graphics_impl::kernel_density_curve;
    let xs: Vec<f64> = (0..200)
        .map(|i| (i as f64 * 0.137).sin() * 3.0 + 5.0)
        .collect();
    let curve = kernel_density_curve(&xs, 500);
    let mut area = 0.0;
    for w in curve.windows(2) {
        area += 0.5 * (w[0].1 + w[1].1) * (w[1].0 - w[0].0);
    }
    assert!((area - 1.0).abs() < 0.05, "area={area}");
}

#[cfg(feature = "graphics")]
#[test]
fn execute_loess_writes_image() {
    let mut session = make_session_in_temp();
    session.ods_graphics.enabled = true;
    session.ods_graphics.output_dir = std::env::temp_dir();
    session.ods_graphics.file_stem = Some("sgtest_loess".into());
    write_heights(&mut session, "H");
    let ast = parse_sgplot(
        "proc sgplot data=work.h; scatter x=age y=height; loess x=age y=height / smooth=0.6; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("written"), "log: {log}");
    assert!(
        !log.contains("LOESS plot deferred"),
        "should not defer: {log}"
    );
    let p = std::env::temp_dir().join("sgtest_loess_1.png");
    assert!(p.exists(), "image not created: {p:?}");
    assert!(p.metadata().unwrap().len() > 0);
    let _ = std::fs::remove_file(&p);
}

#[cfg(feature = "graphics")]
#[test]
fn execute_density_writes_image() {
    let mut session = make_session_in_temp();
    session.ods_graphics.enabled = true;
    session.ods_graphics.output_dir = std::env::temp_dir();
    session.ods_graphics.file_stem = Some("sgtest_density".into());
    write_heights(&mut session, "H");
    let ast =
        parse_sgplot("proc sgplot data=work.h; histogram height; density height; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("written"), "log: {log}");
    assert!(
        !log.contains("DENSITY plot deferred"),
        "should not defer: {log}"
    );
    let p = std::env::temp_dir().join("sgtest_density_1.png");
    assert!(p.exists(), "image not created: {p:?}");
    let _ = std::fs::remove_file(&p);
}

#[cfg(feature = "graphics")]
#[test]
fn execute_missing_column_errors() {
    let mut session = make_session_in_temp();
    session.ods_graphics.enabled = true;
    session.ods_graphics.output_dir = std::env::temp_dir();
    write_heights(&mut session, "H");
    let ast =
        parse_sgplot("proc sgplot data=work.h; scatter x=nonexistent y=height; run;").unwrap();
    let res = execute(&ast, &mut session);
    assert!(res.is_err(), "expected error for missing column");
    let msg = res.err().unwrap().to_string();
    assert!(msg.contains("NONEXISTENT"), "msg: {msg}");
}
