use super::*;
use crate::ast::{DatasetRef, DatasetSpec, Expr};
use crate::source::SourceFile;

/// Parse une étape DATA en supposant le mot-clé `data` déjà consommé.
fn parse(src: &str) -> Result<DataStepAst> {
    let file = SourceFile::new(src);
    let mut ts = StatementStream::new(&file).unwrap();
    // Consommer le `data` de tête comme le fait next_block().
    assert!(ts.peek().is_kw("data"), "test source must start with DATA");
    ts.next();
    parse_data_step(&mut ts)
}

fn dsref(name: &str) -> DatasetRef {
    DatasetRef {
        libref: None,
        name: name.to_string(),
    }
}

/// Spec sans options.
fn dspec(name: &str) -> DatasetSpec {
    DatasetSpec::plain(dsref(name))
}

/// `DsStmt::Set` sans options de niveau statement (M16.4).
fn set_stmt(specs: Vec<DatasetSpec>) -> DsStmt {
    DsStmt::Set {
        specs,
        options: crate::ast::SetOptions::default(),
    }
}

fn var(s: &str) -> Expr {
    Expr::Var(s.to_string())
}

// ── DO itératif / conditionnel (M2) ──────────────────────────────────

/// Déstructure un DoLoop ou panique.
fn as_do_loop(
    stmt: &DsStmt,
) -> (
    &Option<(String, Expr)>,
    &Option<Expr>,
    &Option<Expr>,
    &Option<Expr>,
    &Option<Expr>,
    &Vec<DsStmt>,
) {
    let DsStmt::DoLoop {
        index,
        to,
        by,
        while_,
        until,
        body,
    } = stmt
    else {
        panic!("expected a DoLoop, got {stmt:?}");
    };
    (index, to, by, while_, until, body)
}

// ── ARRAY (M2, lot 3) ────────────────────────────────────────────────

/// Constructeur d'un `DsStmt::Array` simple pour les tests.
fn array_stmt(
    name: &str,
    dims: Option<Vec<usize>>,
    char_len: Option<usize>,
    vars: Vec<&str>,
) -> DsStmt {
    DsStmt::Array {
        name: name.to_string(),
        dims,
        char_len,
        vars: vars.into_iter().map(String::from).collect(),
        initial: vec![],
        temporary: false,
        special: None,
    }
}

// ── INFILE / INPUT / DATALINES (M14) ─────────────────────────────────

fn var_item(name: &str, is_char: bool) -> InputItem {
    InputItem::Var {
        name: name.to_string(),
        is_char,
        cols: None,
        informat: None,
        list_modifier: false,
    }
}

mod array;
mod input;
mod parse;
mod retain;
mod simple;
