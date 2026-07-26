use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::session::Session;
use crate::source::SourceFile;
use crate::value::VarType;
use polars::prelude::*;
use std::path::PathBuf;

fn make_session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

fn parse_print_src(src: &str) -> Result<PrintAst> {
    let full = format!("proc print {}; run;", src);
    let source = SourceFile::new(&full);
    let mut ts = crate::parser::StatementStream::new(&source).unwrap();
    // consume "proc"
    ts.next();
    // consume "print"
    ts.next();
    parse(&mut ts)
}

fn parse_print_with_var(src: &str) -> Result<PrintAst> {
    let source = SourceFile::new(src);
    let mut ts = crate::parser::StatementStream::new(&source).unwrap();
    ts.next(); // proc
    ts.next(); // print
    parse(&mut ts)
}

// --- execution tests ---

fn write_test_dataset(session: &mut Session) {
    // Build a small DataFrame with one numeric and one char column
    let df = df![
        "name" => ["Alice", "Bob", "Carol"],
        "age"  => [30.0_f64, 25.0, 40.0]
    ]
    .unwrap();

    let vars = vec![
        VarMeta {
            name: "name".to_string(),
            ty: VarType::Char,
            length: 5,
            format: None,
            label: None,
        },
        VarMeta {
            name: "age".to_string(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
    ];
    let ds = SasDataset { df, vars };
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("MYDATA", &ds)
        .unwrap();
    session.last_dataset = Some("WORK.MYDATA".to_string());
}

// ── M4 : formats appliqués + option LABEL ─────────────────────────────

fn write_formatted_dataset(session: &mut Session) {
    let df = df![
        "name"   => ["Alice", "Bob"],
        "weight" => [112.0_f64, 98.0]
    ]
    .unwrap();
    let vars = vec![
        VarMeta {
            name: "name".to_string(),
            ty: VarType::Char,
            length: 5,
            format: None,
            label: Some("Pupil Name".to_string()),
        },
        VarMeta {
            name: "weight".to_string(),
            ty: VarType::Num,
            length: 8,
            format: Some("dollar8.".to_string()),
            label: Some("Body Weight".to_string()),
        },
    ];
    let ds = SasDataset { df, vars };
    session.libs.get("WORK").unwrap().write("FMT", &ds).unwrap();
}

// ── M33.6 : BY / ID / SUM / DOUBLE / N ────────────────────────────────────

/// Build a small dataset sorted by `grp`: two groups A (2 rows) / B (1 row),
/// with a numeric `v` to sum. Sums: A → 3+4=7, B → 5; grand → 12.
fn write_grouped(session: &mut Session) {
    let df = df![
        "grp" => ["A", "A", "B"],
        "v"   => [3.0_f64, 4.0, 5.0]
    ]
    .unwrap();
    let vars = vec![
        VarMeta {
            name: "grp".into(),
            ty: VarType::Char,
            length: 1,
            format: None,
            label: None,
        },
        VarMeta {
            name: "v".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
    ];
    let ds = SasDataset { df, vars };
    session.libs.get("WORK").unwrap().write("G", &ds).unwrap();
    session.last_dataset = Some("WORK.G".to_string());
}

fn base_ast() -> PrintAst {
    PrintAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "G".into(),
        }),
        vars: None,
        noobs: false,
        label: false,
        double: false,
        n: false,
        by: vec![],
        id: vec![],
        sum: vec![],
    }
}

mod parse;
mod user;
