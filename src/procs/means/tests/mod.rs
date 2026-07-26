use super::*;
use crate::session::Session;
use crate::source::SourceFile;

fn parse_means(src: &str) -> Result<MeansAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "means"
    parse(&mut ts)
}

// ───────────────────────────── execute tests ───────────────────────────

fn read_num_col(session: &Session, table: &str, col: &str) -> Vec<Value> {
    let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
    let idx = ds.vars.iter().position(|m| m.name == col).unwrap();
    decode_column(&ds, idx).unwrap()
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
