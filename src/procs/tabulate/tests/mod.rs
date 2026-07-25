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
    VarMeta { name: name.into(), ty: VarType::Num, length: 8, format: None, label: None }
}

fn char_meta(name: &str) -> VarMeta {
    VarMeta { name: name.into(), ty: VarType::Char, length: 8, format: None, label: None }
}

fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
    session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
    session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
}

fn parse_src(src: &str) -> Result<TabulateAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "tabulate"
    parse(&mut ts)
}

/// Parse + execute through a session, returning the listing string.
fn run(mut session: Session, src: &str) -> Result<String> {
    let ast = parse_src(src)?;
    execute(&ast, &mut session)?;
    Ok(session.listing.into_string())
}

// ─────────────── M21.4: page dimension ───────────────

/// Build the classic sashelp.class-like fixture (subset of rows is fine).
fn class_fixture(session: &mut Session) {
    let df = df![
        "sex"    => ["M", "F", "M", "F", "M"],
        "age"    => [14.0_f64, 13.0, 12.0, 13.0, 14.0],
        "height" => [69.0_f64, 56.5, 57.3, 65.3, 62.5],
        "weight" => [112.5_f64, 84.0, 83.0, 98.0, 84.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![
            char_meta("sex"),
            num_meta("age"),
            num_meta("height"),
            num_meta("weight"),
        ],
    };
    write_dataset(session, "C", ds);
}

mod parse;
mod no_output;
