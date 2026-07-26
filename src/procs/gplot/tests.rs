use super::*;
use crate::source::SourceFile;

fn parse_gplot(src: &str) -> Result<GplotAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // proc
    ts.next(); // gplot
    parse(&mut ts)
}

#[allow(dead_code)]
fn make_session() -> Session {
    Session::new(None, std::env::temp_dir(), true).unwrap()
}

// ── Parse tests ──────────────────────────────────────────────────────

#[test]
fn parse_plot_simple() {
    let ast = parse_gplot("proc gplot data=a; plot y*x; run;").unwrap();
    assert_eq!(ast.plots.len(), 1);
    match &ast.plots[0] {
        GplotStmt::Plot {
            y_vars,
            x_var,
            group_var,
        } => {
            assert_eq!(y_vars, &vec!["y".to_string()]);
            assert_eq!(x_var, "x");
            assert!(group_var.is_none());
        }
    }
}

#[test]
fn parse_plot_with_group() {
    let ast = parse_gplot("proc gplot data=a; plot y*x=group; run;").unwrap();
    match &ast.plots[0] {
        GplotStmt::Plot {
            y_vars,
            x_var,
            group_var,
        } => {
            assert_eq!(y_vars, &vec!["y".to_string()]);
            assert_eq!(x_var, "x");
            assert_eq!(group_var.as_deref(), Some("group"));
        }
    }
}

#[test]
fn parse_plot_multiple_y() {
    let ast = parse_gplot("proc gplot data=a; plot (y1 y2)*x; run;").unwrap();
    match &ast.plots[0] {
        GplotStmt::Plot { y_vars, x_var, .. } => {
            assert_eq!(y_vars, &vec!["y1".to_string(), "y2".to_string()]);
            assert_eq!(x_var, "x");
        }
    }
}

#[test]
fn parse_symbol_axis_ignored() {
    // SYMBOL/AXIS dans le bloc sont parsés sans erreur et ignorés.
    let ast = parse_gplot(
        "proc gplot data=a; symbol1 color=blue value=dot; axis1 label=('T'); plot y*x; run;",
    )
    .unwrap();
    assert_eq!(ast.plots.len(), 1);
}

// ── Execute tests (default build) ────────────────────────────────────

#[test]
fn execute_without_ods_on_notes_not_enabled() {
    let mut session = make_session();
    let ast = parse_gplot("proc gplot data=a; plot y*x; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(
        log.contains("ODS GRAPHICS is not enabled") && log.contains("PROC GPLOT"),
        "log: {log}"
    );
}

#[cfg(not(feature = "graphics"))]
#[test]
fn execute_with_ods_on_no_feature_defers() {
    let mut session = make_session();
    session.ods_graphics.enabled = true;
    let ast = parse_gplot("proc gplot data=a; plot y*x; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("image deferred"), "log: {log}");
}

// ── Execute tests (feature graphics) ─────────────────────────────────

#[cfg(feature = "graphics")]
fn write_xy(session: &mut Session, table: &str) {
    use crate::dataset::{SasDataset, VarMeta};
    use crate::value::VarType;
    use polars::df;
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0],
        "y" => [2.0_f64, 4.0, 3.0, 5.0, 6.0]
    ]
    .unwrap();
    let vars = vec![
        VarMeta {
            name: "x".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
        VarMeta {
            name: "y".into(),
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
    let mut session = make_session();
    session.ods_graphics.enabled = true;
    session.ods_graphics.output_dir = std::env::temp_dir();
    session.ods_graphics.file_stem = Some("gplottest_single".into());
    write_xy(&mut session, "XY");
    let ast = parse_gplot("proc gplot data=work.xy; plot y*x; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("written"), "log: {log}");
    let p = std::env::temp_dir().join("gplottest_single_1.png");
    assert!(p.exists(), "image not created: {p:?}");
    assert!(p.metadata().unwrap().len() > 0);
    let _ = std::fs::remove_file(&p);
}

// ── M34.11 : multi-séries + SYMBOL/AXIS ──────────────────────────────

#[cfg(feature = "graphics")]
fn write_multi(session: &mut Session, table: &str) {
    use crate::dataset::{SasDataset, VarMeta};
    use crate::value::VarType;
    use polars::df;
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0],
        "y1" => [1.0_f64, 2.0, 3.0, 4.0],
        "y2" => [4.0_f64, 3.0, 2.0, 1.0],
        "g" => ["a", "b", "a", "b"]
    ]
    .unwrap();
    let mk = |n: &str, t: VarType, l: usize| VarMeta {
        name: n.into(),
        ty: t,
        length: l,
        format: None,
        label: None,
    };
    let vars = vec![
        mk("x", VarType::Num, 8),
        mk("y1", VarType::Num, 8),
        mk("y2", VarType::Num, 8),
        mk("g", VarType::Char, 1),
    ];
    let ds = SasDataset { df, vars };
    session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
}

#[cfg(feature = "graphics")]
#[test]
fn build_series_two_y_vars_makes_two_series() {
    use crate::dataset::{SasDataset, VarMeta};
    use crate::value::VarType;
    use polars::df;
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0],
        "y1" => [1.0_f64, 2.0, 3.0],
        "y2" => [3.0_f64, 2.0, 1.0]
    ]
    .unwrap();
    let mk = |n: &str| VarMeta {
        name: n.into(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    };
    let ds = SasDataset {
        df,
        vars: vec![mk("x"), mk("y1"), mk("y2")],
    };
    let ast = parse_gplot("proc gplot data=work.m; plot (y1 y2)*x; run;").unwrap();
    let series = graphics_impl::build_series(&ds, &ast.plots[0], &ast.symbols).unwrap();
    assert_eq!(series.len(), 2, "expected one series per Y var");
    assert_eq!(series[0].0.len(), 3);
    assert_eq!(series[1].0.len(), 3);
}

#[cfg(feature = "graphics")]
#[test]
fn build_series_group_makes_one_series_per_level() {
    use crate::dataset::{SasDataset, VarMeta};
    use crate::value::VarType;
    use polars::df;
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0],
        "y" => [1.0_f64, 2.0, 3.0, 4.0],
        "g" => ["a", "b", "a", "b"]
    ]
    .unwrap();
    let mk = |n: &str, t: VarType| VarMeta {
        name: n.into(),
        ty: t,
        length: 8,
        format: None,
        label: None,
    };
    let ds = SasDataset {
        df,
        vars: vec![
            mk("x", VarType::Num),
            mk("y", VarType::Num),
            mk("g", VarType::Char),
        ],
    };
    let ast = parse_gplot("proc gplot data=work.m; plot y*x=g; run;").unwrap();
    let series = graphics_impl::build_series(&ds, &ast.plots[0], &ast.symbols).unwrap();
    assert_eq!(series.len(), 2, "expected one series per group level");
    // 2 niveaux (a, b) avec 2 points chacun.
    assert_eq!(series[0].0.len(), 2);
    assert_eq!(series[1].0.len(), 2);
}

#[cfg(feature = "graphics")]
#[test]
fn symbol_interpol_join_makes_line() {
    use crate::graphics::render::SeriesColor;
    let sym = SymbolDef {
        interpol: Some("join".into()),
        value: None,
        color: Some("red".into()),
    };
    let (line, _marker) = graphics_impl::line_marker(Some(&sym));
    assert!(line, "INTERPOL=JOIN should yield a line");
    assert_eq!(
        graphics_impl::color_from_name("red"),
        Some(SeriesColor::Red)
    );
    // Sans SYMBOL : marqueurs, pas de ligne.
    let (line0, marker0) = graphics_impl::line_marker(None);
    assert!(!line0 && marker0);
}

#[cfg(feature = "graphics")]
#[test]
fn execute_multi_series_writes_image() {
    let mut session = make_session();
    session.ods_graphics.enabled = true;
    session.ods_graphics.output_dir = std::env::temp_dir();
    session.ods_graphics.file_stem = Some("gplottest_multi".into());
    write_multi(&mut session, "M");
    let ast = parse_gplot(
        "proc gplot data=work.m; symbol1 interpol=join color=blue; symbol2 interpol=join color=red; axis1 order=(0 to 5) label=('X axis'); plot (y1 y2)*x; run;",
    )
    .unwrap();
    assert_eq!(ast.symbols.len(), 2);
    assert_eq!(ast.axes.len(), 1);
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("written"), "log: {log}");
    let p = std::env::temp_dir().join("gplottest_multi_1.png");
    assert!(p.exists(), "image not created: {p:?}");
    assert!(p.metadata().unwrap().len() > 0);
    let _ = std::fs::remove_file(&p);
}

#[test]
fn parse_symbol_axis_captured() {
    let ast = parse_gplot(
        "proc gplot data=a; symbol1 i=join v=dot c=blue; axis1 order=(0 to 100 by 10) label=('Time'); plot y*x; run;",
    )
    .unwrap();
    assert_eq!(ast.symbols.len(), 1);
    assert_eq!(ast.symbols[0].interpol.as_deref(), Some("join"));
    assert_eq!(ast.symbols[0].value.as_deref(), Some("dot"));
    assert_eq!(ast.symbols[0].color.as_deref(), Some("blue"));
    assert_eq!(ast.axes.len(), 1);
    assert_eq!(ast.axes[0].order_min, Some(0.0));
    assert_eq!(ast.axes[0].order_max, Some(100.0));
    assert_eq!(ast.axes[0].label.as_deref(), Some("Time"));
}
