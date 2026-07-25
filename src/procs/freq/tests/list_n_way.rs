use super::super::*;
use super::*;
use crate::dataset::SasDataset;
use polars::df;

/// /LIST layout: one row per non-empty cell with Frequency / Percent /
/// Cumulative columns; no grid Row/Col Pct.
/// Cells (a,1)=1, (a,2)=1, (b,1)=2 ; grand=4.
///   (a,1): 1 / 25.00 / cum 1 / 25.00
///   (a,2): 1 / 25.00 / cum 2 / 50.00
///   (b,1): 2 / 50.00 / cum 4 / 100.00
#[test]
fn list_layout_rows() {
    let mut session = make_session();
    let df = df![
        "r" => ["a", "a", "b", "b"],
        "c" => [1.0_f64, 2.0, 1.0, 1.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![char_meta("r"), num_meta("c")] };
    write_dataset(&mut session, "T", ds);

    let mut req = tr(&["r", "c"], false, None);
    req.list = true;
    let ast = fast(
        DatasetRef { libref: Some("WORK".into()), name: "T".into() },
        vec![req],
    );
    execute(&ast, &mut session).unwrap();
    let l = session.listing.into_string();
    // LIST: header columns, no "Row Pct"/"Col Pct".
    assert!(l.contains("Cumulative Frequency"), "{l}");
    assert!(!l.contains("Row Pct"), "LIST suppresses Row Pct:\n{l}");
    assert!(!l.contains("Col Pct"), "LIST suppresses Col Pct:\n{l}");
    // Cumulative percent reaches 100.00.
    assert!(l.contains("100.00"), "{l}");
    assert!(l.contains("50.00"), "{l}");
}

/// n-way (3-way) stratified rendering: one two-way table per leading value.
/// s = [A,A,B,B]; r = [x,x,y,y]; c = [1,2,1,2]. Each stratum has 2 cells.
#[test]
fn n_way_stratified() {
    let mut session = make_session();
    let df = df![
        "s" => ["A", "A", "B", "B"],
        "r" => ["x", "x", "y", "y"],
        "c" => [1.0_f64, 2.0, 1.0, 2.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("s"), char_meta("r"), num_meta("c")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = fast(
        DatasetRef { libref: Some("WORK".into()), name: "T".into() },
        vec![tr(&["s", "r", "c"], false, None)],
    );
    execute(&ast, &mut session).unwrap();
    let l = session.listing.into_string();
    assert!(l.contains("Controlling for s=A"), "stratum A header:\n{l}");
    assert!(l.contains("Controlling for s=B"), "stratum B header:\n{l}");
    assert!(l.contains("Table of r by c"), "{l}");
}
