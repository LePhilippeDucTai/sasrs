use super::*;
use crate::source::SourceFile;
use crate::testkit::*;

fn parse_gchart(src: &str) -> Result<GchartAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // proc
    ts.next(); // gchart
    parse(&mut ts)
}

#[allow(dead_code)]
// ── Parse tests ──────────────────────────────────────────────────────
#[test]
fn parse_vbar_freq() {
    let ast = parse_gchart("proc gchart data=a; vbar category; run;").unwrap();
    assert_eq!(ast.charts.len(), 1);
    match &ast.charts[0] {
        GchartStmt::VBar {
            category,
            sumvar,
            chart_type,
        } => {
            assert_eq!(category, "category");
            assert!(sumvar.is_none());
            assert_eq!(*chart_type, ChartType::Freq);
        }
        other => panic!("expected VBar, got {other:?}"),
    }
}

#[test]
fn parse_vbar_sumvar_implies_sum() {
    let ast = parse_gchart("proc gchart data=a; vbar category / sumvar=count; run;").unwrap();
    match &ast.charts[0] {
        GchartStmt::VBar {
            sumvar, chart_type, ..
        } => {
            assert_eq!(sumvar.as_deref(), Some("count"));
            assert_eq!(*chart_type, ChartType::Sum);
        }
        other => panic!("expected VBar, got {other:?}"),
    }
}

#[test]
fn parse_vbar_type_mean() {
    let ast = parse_gchart("proc gchart data=a; vbar category / type=mean; run;").unwrap();
    match &ast.charts[0] {
        GchartStmt::VBar { chart_type, .. } => assert_eq!(*chart_type, ChartType::Mean),
        other => panic!("expected VBar, got {other:?}"),
    }
}

#[test]
fn parse_hbar() {
    let ast = parse_gchart("proc gchart data=a; hbar category; run;").unwrap();
    match &ast.charts[0] {
        GchartStmt::HBar { category, .. } => assert_eq!(category, "category"),
        other => panic!("expected HBar, got {other:?}"),
    }
}

#[test]
fn parse_pie() {
    let ast = parse_gchart("proc gchart data=a; pie category; run;").unwrap();
    assert!(matches!(ast.charts[0], GchartStmt::Pie { .. }));
}

#[test]
fn parse_pie_sumvar_implies_sum() {
    let ast = parse_gchart("proc gchart data=a; pie region / sumvar=sales; run;").unwrap();
    match &ast.charts[0] {
        GchartStmt::Pie {
            category,
            sumvar,
            chart_type,
        } => {
            assert_eq!(category, "region");
            assert_eq!(sumvar.as_deref(), Some("sales"));
            assert_eq!(*chart_type, ChartType::Sum);
        }
        other => panic!("expected Pie, got {other:?}"),
    }
}

// ── Execute tests (default build) ────────────────────────────────────

#[test]
fn execute_without_ods_on_notes_not_enabled() {
    let mut session = make_session_in_temp();
    let ast = parse_gchart("proc gchart data=a; vbar category; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(
        log.contains("ODS GRAPHICS is not enabled") && log.contains("PROC GCHART"),
        "log: {log}"
    );
}

#[cfg(not(feature = "graphics"))]
#[test]
fn execute_pie_defers() {
    let mut session = make_session_in_temp();
    session.ods_graphics.enabled = true;
    let ast = parse_gchart("proc gchart data=a; pie category; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(
        log.contains("PIE chart deferred in PROC GCHART."),
        "log: {log}"
    );
}

#[cfg(not(feature = "graphics"))]
#[test]
fn execute_vbar_with_ods_on_no_feature_defers() {
    let mut session = make_session_in_temp();
    session.ods_graphics.enabled = true;
    let ast = parse_gchart("proc gchart data=a; vbar category; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("image deferred"), "log: {log}");
}

// ── Execute tests (feature graphics) ─────────────────────────────────

#[cfg(feature = "graphics")]
fn write_cats(session: &mut Session, table: &str) {
    use crate::dataset::{SasDataset, VarMeta};
    use crate::value::VarType;
    use polars::df;
    let df = df![
        "category" => ["A", "B", "C", "D"],
        "count" => [10.0_f64, 25.0, 15.0, 30.0]
    ]
    .unwrap();
    let vars = vec![
        VarMeta {
            name: "category".into(),
            ty: VarType::Char,
            length: 1,
            format: None,
            label: None,
        },
        VarMeta {
            name: "count".into(),
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
fn execute_vbar_with_graphics_writes_image() {
    let mut session = make_session_in_temp();
    session.ods_graphics.enabled = true;
    session.ods_graphics.output_dir = std::env::temp_dir();
    session.ods_graphics.file_stem = Some("gcharttest_single".into());
    write_cats(&mut session, "CATS");
    let ast =
        parse_gchart("proc gchart data=work.cats; vbar category / sumvar=count; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("written"), "log: {log}");
    let p = std::env::temp_dir().join("gcharttest_single_1.png");
    assert!(p.exists(), "image not created: {p:?}");
    assert!(p.metadata().unwrap().len() > 0);
    let _ = std::fs::remove_file(&p);
}

#[cfg(feature = "graphics")]
#[test]
fn execute_pie_with_graphics_writes_image() {
    let mut session = make_session_in_temp();
    session.ods_graphics.enabled = true;
    session.ods_graphics.output_dir = std::env::temp_dir();
    session.ods_graphics.file_stem = Some("gcharttest_pie".into());
    write_cats(&mut session, "CATS");
    let ast =
        parse_gchart("proc gchart data=work.cats; pie category / sumvar=count; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("written"), "log: {log}");
    assert!(
        !log.contains("PIE chart deferred"),
        "should not defer: {log}"
    );
    let p = std::env::temp_dir().join("gcharttest_pie_1.png");
    assert!(p.exists(), "pie image not created: {p:?}");
    assert!(p.metadata().unwrap().len() > 0);
    let _ = std::fs::remove_file(&p);
}

#[cfg(feature = "graphics")]
#[test]
fn pie_aggregate_proportional_to_totals() {
    use crate::dataset::{SasDataset, VarMeta};
    use crate::graphics::render::pie_angles;
    use crate::value::VarType;
    use polars::df;
    // FREQ : A x3, B x1 → angles 3:1.
    let df = df!["category" => ["A", "A", "A", "B"]].unwrap();
    let vars = vec![VarMeta {
        name: "category".into(),
        ty: VarType::Char,
        length: 1,
        format: None,
        label: None,
    }];
    let ds = SasDataset { df, vars };
    let agg = graphics_impl::aggregate(&ds, "category", &None, ChartType::Freq).unwrap();
    let vals: Vec<f64> = agg.iter().map(|(_, v)| *v).collect();
    let angles = pie_angles(&vals);
    let total: f64 = angles.iter().map(|(s, e)| e - s).sum();
    assert!((total - std::f64::consts::TAU).abs() < 1e-9);
    // A (3) doit occuper 3x l'arc de B (1).
    let span_a = angles[0].1 - angles[0].0;
    let span_b = angles[1].1 - angles[1].0;
    assert!(
        (span_a - 3.0 * span_b).abs() < 1e-9,
        "a={span_a} b={span_b}"
    );
}
