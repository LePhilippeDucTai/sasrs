use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::parser::StatementStream;
use crate::source::SourceFile;
use polars::df;
use std::path::PathBuf;

fn session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

/// Écrit un petit dataset (age num avec un missing, name char) dans WORK.
fn write_class(session: &Session, table: &str) {
    let df = df!(
        "Age" => [Some(14.0), None, Some(13.0)],
        "Name" => ["Alfred", "Alice", "Barbara"],
    )
    .unwrap();
    let vars = vec![
        VarMeta {
            name: "Age".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
        VarMeta {
            name: "Name".into(),
            ty: VarType::Char,
            length: 7,
            format: None,
            label: None,
        },
    ];
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(table, &SasDataset { df, vars })
        .unwrap();
}

fn parse_step(src: &str) -> DataStepAst {
    let file = SourceFile::new(src);
    let mut ts = StatementStream::new(&file).unwrap();
    assert!(ts.next().is_kw("data"));
    crate::parser::datastep::parse_data_step(&mut ts).unwrap()
}

fn compile_src(src: &str, session: &mut Session) -> Result<StepProgram> {
    compile(&parse_step(src), session)
}

// ── SET multi-datasets + BY + FIRST./LAST. (M3) ──────────────────────

/// Petit dataset numérique (Age, Weight) pour les unions de variables.
fn write_weights(session: &Session, table: &str) {
    let df = df!(
        "Age" => [11.0, 12.0],
        "Weight" => [50.0, 60.0],
    )
    .unwrap();
    let vars = vec![
        VarMeta {
            name: "Age".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
        VarMeta {
            name: "Weight".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
    ];
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(table, &SasDataset { df, vars })
        .unwrap();
}

mod array;
mod set;
