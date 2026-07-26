use super::*;
use crate::session::Session;
use crate::source::SourceFile;

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
