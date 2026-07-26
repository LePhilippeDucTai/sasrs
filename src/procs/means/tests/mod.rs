use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::session::Session;
use crate::source::SourceFile;
use crate::value::VarType;
use std::path::PathBuf;

fn make_session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

fn parse_means(src: &str) -> Result<MeansAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "means"
    parse(&mut ts)
}

// ───────────────────────────── execute tests ───────────────────────────

fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
    session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
    session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
}

fn read_num_col(session: &Session, table: &str, col: &str) -> Vec<Value> {
    let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
    let idx = ds.vars.iter().position(|m| m.name == col).unwrap();
    decode_column(&ds, idx).unwrap()
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

// ─────────────────────── ODS OUTPUT Summary= (M22.3) ────────────────────

fn means_ast_var_x() -> MeansAst {
    MeansAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        summary: false,
        noprint: false,
        stats: vec![],
        class: vec![],
        var: vec!["x".into()],
        by: vec![],
        weight: None,
        alpha: 0.05,
        printalltypes: false,
        ways: vec![],
        types: vec![],
        output: None,
    }
}

mod execute1;
mod execute2;
mod parse;
