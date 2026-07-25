use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::session::Session;
use crate::source::SourceFile;
use crate::value::VarType;
use polars::df;
use std::path::PathBuf;

fn make_session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

fn num_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}

fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
    session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
    session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
}

fn parse_distance(src: &str) -> Result<DistanceAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "distance"
    parse(&mut ts)
}

#[test]
fn parse_minimal() {
    let ast = parse_distance("proc distance data=a out=b; var x y; run;").unwrap();
    assert_eq!(ast.data.as_ref().unwrap().name, "a");
    assert_eq!(ast.out.as_ref().unwrap().name, "b");
    assert_eq!(ast.method, DistMethod::Euclid);
    assert_eq!(ast.var, vec!["x", "y"]);
}

#[test]
fn parse_methods() {
    assert_eq!(
        parse_distance("proc distance method=cityblock; var x; run;").unwrap().method,
        DistMethod::CityBlock
    );
    assert_eq!(
        parse_distance("proc distance method=L2; var x; run;").unwrap().method,
        DistMethod::Euclid
    );
    assert_eq!(
        parse_distance("proc distance method=chebychev; var x; run;").unwrap().method,
        DistMethod::Chebychev
    );
    assert_eq!(
        parse_distance("proc distance method=cosine; var x; run;").unwrap().method,
        DistMethod::Cosine
    );
}

/// Oracle: 3 points in 3-space — x=(0,1,0), y=(0,0,1), z=(1,0,0).
/// All pairwise Euclidean distances = sqrt(2). 3×3 matrix, zero diagonal.
#[test]
fn euclid_three_points_oracle() {
    let x = [0.0, 0.0, 1.0];
    let y = [1.0, 0.0, 0.0];
    let z = [0.0, 1.0, 0.0];
    let s2 = 2.0_f64.sqrt();
    assert!((distance(DistMethod::Euclid, &x, &y) - s2).abs() < 1e-12);
    assert!((distance(DistMethod::Euclid, &x, &z) - s2).abs() < 1e-12);
    assert!((distance(DistMethod::Euclid, &y, &z) - s2).abs() < 1e-12);
    assert!(distance(DistMethod::Euclid, &x, &x).abs() < 1e-12);
}

#[test]
fn cityblock_and_chebychev_oracle() {
    let a = [1.0, 2.0, 3.0];
    let b = [4.0, 0.0, 3.0];
    // L1 = |1-4|+|2-0|+|3-3| = 3+2+0 = 5
    assert!((distance(DistMethod::CityBlock, &a, &b) - 5.0).abs() < 1e-12);
    // Linf = max(3,2,0) = 3
    assert!((distance(DistMethod::Chebychev, &a, &b) - 3.0).abs() < 1e-12);
}

/// 1D fixture: x=(1,2,3,7,8,9), out= dataset stores the 6×6 matrix.
#[test]
fn execute_writes_out_dataset() {
    let mut session = make_session();
    let df = df!["x" => [1.0_f64, 2.0, 3.0, 7.0, 8.0, 9.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "PTS", ds);

    let ast = DistanceAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "PTS".into() }),
        out: Some(DatasetRef { libref: Some("WORK".into()), name: "DIST".into() }),
        method: DistMethod::Euclid,
        var: vec!["x".into()],
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("DIST").unwrap();
    assert_eq!(out.n_obs(), 6);
    // _TYPE_, _NAME_, Col1..Col6 = 8 variables.
    assert_eq!(out.vars.len(), 8);
    // Col6 distance for Row1 (x=1) is |1-9| = 8.
    let col6 = out.df.column("Col6").unwrap().f64().unwrap();
    assert_eq!(col6.get(0), Some(8.0));
    // Diagonal: Row3 vs Col3 = 0.
    let col3 = out.df.column("Col3").unwrap().f64().unwrap();
    assert_eq!(col3.get(2), Some(0.0));
}

#[test]
fn execute_no_out_emits_note() {
    let mut session = make_session();
    let df = df!["x" => [1.0_f64, 2.0, 3.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "PTS", ds);

    let ast = DistanceAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "PTS".into() }),
        out: None,
        method: DistMethod::Euclid,
        var: vec!["x".into()],
    };
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(
        log.contains("No output dataset specified for PROC DISTANCE"),
        "{log}"
    );
}
