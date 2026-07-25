use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::parser::StatementStream;
use crate::session::Session;
use crate::source::SourceFile;
use crate::sql::ast::{SelectStmt, SqlStmt};
use crate::sql::parser::parse_sql_program;
use crate::value::VarType;
use polars::df;
use std::path::PathBuf;

fn make_session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

fn first_select(src: &str) -> SelectStmt {
    let file = SourceFile::new(src);
    let mut ts = StatementStream::new(&file).unwrap();
    let prog = parse_sql_program(&mut ts).unwrap();
    match prog.stmts.into_iter().next().unwrap() {
        SqlStmt::Select(s) => s,
        other => panic!("expected SELECT, got {other:?}"),
    }
}

fn run(src: &str, session: &mut Session) -> DataFrame {
    let sel = first_select(src);
    lower_select(&sel, session).unwrap().collect().unwrap()
}

/// Écrit une table dans WORK.
fn write_table(session: &mut Session, name: &str, df: DataFrame, vars: Vec<VarMeta>) {
    let ds = SasDataset { df, vars };
    session.libs.get("WORK").unwrap().write(name, &ds).unwrap();
}

fn num(name: &str) -> VarMeta {
    VarMeta {
        name: name.into(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}

fn chr(name: &str, len: usize) -> VarMeta {
    VarMeta {
        name: name.into(),
        ty: VarType::Char,
        length: len,
        format: None,
        label: None,
    }
}

fn write_people(session: &mut Session) {
    let df = df![
        "name" => ["Al", "Bo", "Cy", "Di"],
        "sex"  => ["M", "M", "F", "F"],
        "age"  => [10.0_f64, 14.0, 13.0, 11.0],
        "height" => [50.0_f64, 60.0, 55.0, 52.0],
    ]
    .unwrap();
    write_table(
        session,
        "T",
        df,
        vec![chr("name", 8), chr("sex", 1), num("age"), num("height")],
    );
}

// ---- M20.1 : LIKE complet (regex maison SAS) -------------------------

/// Récupère les valeurs d'une colonne char (triées) pour comparaison.
fn sorted_strs(df: &DataFrame, col: &str) -> Vec<String> {
    let mut v: Vec<String> = df
        .column(col)
        .unwrap()
        .str()
        .unwrap()
        .iter()
        .map(|o| o.unwrap().to_string())
        .collect();
    v.sort();
    v
}

fn write_words(session: &mut Session) {
    let df = df![
        "w" => ["abc", "abx", "xbc", "axc", "abcd", "ABC", "a_c"],
    ]
    .unwrap();
    write_table(session, "W", df, vec![chr("w", 8)]);
}

// ---- M20.1 : EXCEPT / INTERSECT ALL (multiplicité exacte) ------------

/// Tables avec dupliqués pour tester la multiplicité.
fn write_multi(session: &mut Session) {
    // A : 1 apparaît 3×, 2 apparaît 1×, 3 apparaît 2×.
    let a = df!["x" => [1.0_f64, 1.0, 1.0, 2.0, 3.0, 3.0]].unwrap();
    // B : 1 apparaît 1×, 3 apparaît 1×, 4 apparaît 1×.
    let b = df!["x" => [1.0_f64, 3.0, 4.0]].unwrap();
    write_table(session, "A", a, vec![num("x")]);
    write_table(session, "B", b, vec![num("x")]);
}

fn nums(df: &DataFrame, col: &str) -> Vec<f64> {
    let mut v: Vec<f64> = df
        .column(col)
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v
}

// ---- M20.2 : sous-requêtes (non-corrélées + corrélées) ---------------

/// Erreur d'abaissement (collect inclus) d'une requête SQL.
fn run_err(src: &str, session: &mut Session) -> String {
    let sel = first_select(src);
    match lower_select(&sel, session).and_then(|lf| Ok(lf.collect()?)) {
        Ok(_) => panic!("expected an error for {src:?}"),
        Err(e) => e.to_string(),
    }
}

// ------------------------------------------------------------------------
// M20.3 — dictionary tables (DICTIONARY.TABLES/COLUMNS/MACROS, sashelp.v*)
// ------------------------------------------------------------------------

/// Valeurs string d'une colonne (dans l'ordre des lignes), nulls → "".
fn strs(df: &DataFrame, col: &str) -> Vec<String> {
    df.column(col)
        .unwrap()
        .str()
        .unwrap()
        .into_iter()
        .map(|o| o.unwrap_or("").to_string())
        .collect()
}

mod where_filter;
mod scalar;
