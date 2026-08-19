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

/// Parse et exécute un `proc format; ...; run;` dans `session` — peuple
/// `session.format_catalog` pour les tests qui doivent voir un `VALUE`
/// format utilisateur résolu à la compilation d'une étape DATA (M43.1,
/// `Compiler::put_width`). Miroir du pattern `execute_round_trip_parse_and_execute`
/// dans `procs::format::tests::execute`.
fn define_format(session: &mut Session, src: &str) {
    let file = SourceFile::new(src);
    let mut ts = StatementStream::new(&file).unwrap();
    assert!(ts.next().is_kw("proc"));
    assert!(ts.next().is_kw("format"));
    let ast = crate::procs::format::parse(&mut ts).unwrap();
    crate::procs::format::execute(&ast, session).unwrap();
}

mod array;
mod set;
