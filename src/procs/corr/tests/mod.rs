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

fn parse_corr(src: &str) -> Result<CorrAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "corr"
    parse(&mut ts)
}

// ───────────── M21.5: Spearman / Kendall / WEIGHT / OUT= ─────────────

fn base_ast(table: &str) -> CorrAst {
    CorrAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: table.into() }),
        nosimple: true,
        noprob: false,
        nocorr: false,
        pearson: false,
        spearman: false,
        kendall: false,
        hoeffding: false,
        var: vec![],
        with: vec![],
        partial: vec![],
        weight: None,
        outp: None,
        outs: None,
        outk: None,
    }
}

fn vnum(vals: &[f64]) -> Vec<Value> {
    vals.iter().map(|v| Value::Num(*v)).collect()
}

// ───────────── M34.1: Hoeffding D + weighted Spearman / Kendall ─────────

/// Replicate `xs` according to integer weights `ws` (oracle helper).
fn replicate(xs: &[f64], ws: &[usize]) -> Vec<f64> {
    let mut out = Vec::new();
    for (&x, &w) in xs.iter().zip(ws) {
        for _ in 0..w {
            out.push(x);
        }
    }
    out
}

mod parse;
mod execute1;
mod execute2;
