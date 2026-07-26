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

fn parse_freq(src: &str) -> Result<FreqAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "freq"
    parse(&mut ts)
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

/// Build a TableRequest with all display/chisq options off (defaults).
fn tr(vars: &[&str], missing: bool, out: Option<DatasetRef>) -> TableRequest {
    TableRequest {
        vars: vars.iter().map(|s| s.to_string()).collect(),
        missing,
        out,
        nofreq: false,
        nopercent: false,
        norow: false,
        nocol: false,
        nocum: false,
        chisq: false,
        fisher: false,
        agree: false,
        measures: false,
        trend: false,
        list: false,
    }
}

/// Build a FreqAst with no WEIGHT/BY (test convenience).
fn fast(data: DatasetRef, tables: Vec<TableRequest>) -> FreqAst {
    FreqAst {
        data: Some(data),
        tables,
        weight: None,
        by: Vec::new(),
    }
}

fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
    session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
    session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
}

fn read_col(session: &Session, table: &str, col: &str) -> Vec<Value> {
    let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
    let idx = ds.vars.iter().position(|m| m.name == col).unwrap();
    decode_column(&ds, idx).unwrap()
}

// ───────────────────────── display-option tests ─────────────────────────

fn one_way_listing(opts: impl Fn(&mut TableRequest)) -> String {
    let mut session = make_session();
    let df = df!["x" => [Some(1.0_f64), Some(1.0), Some(2.0)]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);
    let mut req = tr(&["x"], false, None);
    opts(&mut req);
    let ast = FreqAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        tables: vec![req],
        weight: None,
        by: Vec::new(),
    };
    execute(&ast, &mut session).unwrap();
    session.listing.take_string()
}

fn crosstab_listing(opts: impl Fn(&mut TableRequest)) -> String {
    let mut session = make_session();
    let df = df![
        "r" => ["a", "a", "b", "b"],
        "c" => [1.0_f64, 2.0, 1.0, 1.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("r"), num_meta("c")],
    };
    write_dataset(&mut session, "T", ds);
    let mut req = tr(&["r", "c"], false, None);
    opts(&mut req);
    let ast = FreqAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        tables: vec![req],
        weight: None,
        by: Vec::new(),
    };
    execute(&ast, &mut session).unwrap();
    session.listing.take_string()
}

// ───────────────────── M21.2 advanced statistics ─────────────────────

/// Render the listing produced by a block fn for assertions.
fn run_block<F: FnOnce(&mut Session)>(f: F) -> String {
    let mut session = make_session();
    f(&mut session);
    session.listing.take_string()
}

fn margins(freq: &[Vec<usize>]) -> (Vec<usize>, Vec<usize>, usize) {
    let nr = freq.len();
    let nc = freq[0].len();
    let row_tot: Vec<usize> = (0..nr).map(|r| freq[r].iter().sum()).collect();
    let col_tot: Vec<usize> = (0..nc).map(|c| (0..nr).map(|r| freq[r][c]).sum()).collect();
    let grand: usize = row_tot.iter().sum();
    (row_tot, col_tot, grand)
}

mod crosstab;
mod list_n_way;
mod parse;
