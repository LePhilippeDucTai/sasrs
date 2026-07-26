use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::source::SourceFile;
use crate::value::VarType;
use polars::df;

fn make_session() -> Session {
    Session::new(None, std::env::temp_dir(), true).unwrap()
}

fn parse_plot(src: &str) -> Result<PlotAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // proc
    ts.next(); // plot
    parse(&mut ts)
}

fn write_xy(session: &mut Session, table: &str) {
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
        "y" => [10.0_f64, 20.0, 15.0, 30.0, 25.0, 40.0]
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
    session.last_dataset = Some(format!("WORK.{table}"));
}

// ── Parse tests ──────────────────────────────────────────────────────

#[test]
fn parse_simple_plot() {
    let ast = parse_plot("proc plot data=a; plot y*x; run;").unwrap();
    assert_eq!(ast.plots.len(), 1);
    match &ast.plots[0] {
        PlotStmt::Plot {
            y_vars,
            x_var,
            group_var,
            symbol,
        } => {
            assert_eq!(y_vars, &vec!["y".to_string()]);
            assert_eq!(x_var, "x");
            assert!(group_var.is_none());
            assert!(symbol.is_none());
        }
    }
}

#[test]
fn parse_plot_with_symbol() {
    let ast = parse_plot("proc plot data=a; plot y*x='*'; run;").unwrap();
    match &ast.plots[0] {
        PlotStmt::Plot { symbol, .. } => assert_eq!(*symbol, Some('*')),
    }
}

#[test]
fn parse_plot_multiple_y() {
    let ast = parse_plot("proc plot data=a; plot (y1 y2)*x; run;").unwrap();
    match &ast.plots[0] {
        PlotStmt::Plot { y_vars, x_var, .. } => {
            assert_eq!(y_vars, &vec!["y1".to_string(), "y2".to_string()]);
            assert_eq!(x_var, "x");
        }
    }
}

#[test]
fn parse_plot_with_group() {
    let ast = parse_plot("proc plot data=a; plot y*x=group; run;").unwrap();
    match &ast.plots[0] {
        PlotStmt::Plot { group_var, .. } => {
            assert_eq!(group_var.as_deref(), Some("group"));
        }
    }
}

// ── ASCII render tests (default behavior, ODS off) ────────────────────

#[test]
fn render_ascii_contains_title() {
    let mut session = make_session();
    write_xy(&mut session, "XY");
    let ast = parse_plot("proc plot data=work.xy; plot y*x; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(listing.contains("Plot of Y*X"), "listing: {listing}");
}

#[test]
fn render_ascii_contains_min_max() {
    let mut session = make_session();
    write_xy(&mut session, "XY");
    let ast = parse_plot("proc plot data=work.xy; plot y*x; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    // x ranges 1..6, y ranges 10..40 — extremes must appear as tick labels.
    assert!(listing.contains('1'), "listing: {listing}");
    assert!(listing.contains('6'), "listing: {listing}");
    assert!(listing.contains("10"), "listing: {listing}");
    assert!(listing.contains("40"), "listing: {listing}");
}

// ── ODS delegation tests ──────────────────────────────────────────────

#[cfg(not(feature = "graphics"))]
#[test]
fn execute_with_ods_on_no_feature_defers() {
    let mut session = make_session();
    session.ods_graphics.enabled = true;
    write_xy(&mut session, "XY");
    let ast = parse_plot("proc plot data=work.xy; plot y*x; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("image deferred"), "log: {log}");
}

#[cfg(feature = "graphics")]
#[test]
fn execute_with_ods_on_feature_writes_image() {
    let mut session = make_session();
    session.ods_graphics.enabled = true;
    session.ods_graphics.output_dir = std::env::temp_dir();
    session.ods_graphics.file_stem = Some("plottest_img".into());
    write_xy(&mut session, "XY");
    let ast = parse_plot("proc plot data=work.xy; plot y*x='*'; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("written"), "log: {log}");
    let p = std::env::temp_dir().join("plottest_img_1.png");
    assert!(p.exists(), "image not created: {p:?}");
    assert!(p.metadata().unwrap().len() > 0);
    let _ = std::fs::remove_file(&p);
}
