use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::session::Session;
use crate::source::SourceFile;
use crate::value::VarType;
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

fn char_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Char,
        length: 4,
        format: None,
        label: None,
    }
}

fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
    session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
    session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
}

fn read_num_col(session: &Session, table: &str, col: &str) -> Vec<Value> {
    let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
    let idx = ds.vars.iter().position(|m| m.name == col).unwrap();
    decode_column(&ds, idx).unwrap()
}

fn parse_rank(src: &str) -> Result<RankAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "rank"
    parse(&mut ts)
}

fn dref(table: &str) -> DatasetRef {
    DatasetRef {
        libref: Some("WORK".into()),
        name: table.into(),
    }
}

// ───────────── ranking core tests ─────────────

fn nums(xs: &[f64]) -> Vec<Value> {
    xs.iter().map(|v| Value::Num(*v)).collect()
}

// ───────────── method core tests (hand-verified) ─────────────

fn approx_eq(out: &[Value], exp: &[f64]) {
    assert_eq!(out.len(), exp.len());
    for (o, e) in out.iter().zip(exp) {
        match o {
            Value::Num(v) => assert!((v - e).abs() < 1e-9, "got {v}, want {e}"),
            _ => panic!("missing where {e} expected"),
        }
    }
}

mod execute_method;
mod parse_rank;
