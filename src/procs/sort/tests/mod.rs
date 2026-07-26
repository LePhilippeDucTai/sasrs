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

fn parse_sort(src: &str) -> Result<SortAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "sort"
    parse(&mut ts)
}

// --- execute tests ---

fn write_num_dataset(session: &mut Session, table: &str, name: &str, xs: Vec<Option<f64>>) {
    let df = df![name => xs].unwrap();
    let vars = vec![VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }];
    let ds = SasDataset { df, vars };
    session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
    session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
}

fn read_num_col(session: &Session, table: &str, col: &str) -> Vec<Value> {
    let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
    let idx = ds.vars.iter().position(|m| m.name == col).unwrap();
    decode_column(&ds, idx).unwrap()
}

mod execute;
mod parse;
