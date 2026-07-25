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

fn parse_report(src: &str) -> Result<ReportAst> {
    let source = SourceFile::new(src);
    let mut ts = crate::parser::StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "report"
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
        length: 8,
        format: None,
        label: None,
    }
}

fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
    session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
    session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
}

/// Defaults for the advanced (M21.4) ReportAst fields, used with Rust's
/// struct-update syntax (`..report_defaults()`) in the execute tests.
fn report_defaults() -> ReportAst {
    ReportAst {
        data: None,
        noheader: false,
        columns: None,
        defines: vec![],
        where_: None,
        out: None,
        breaks: vec![],
        rbreak: None,
        computes: vec![],
    }
}

fn work_ref(name: &str) -> DatasetRef {
    DatasetRef {
        libref: Some("WORK".into()),
        name: name.into(),
    }
}

/// Parse a standalone expression (e.g. a WHERE condition) for tests. The
/// SourceFile is owned within this scope; the returned Expr is owned.
fn parse_test_expr(src: &str) -> Expr {
    let source = crate::source::SourceFile::new(src);
    let mut ts = crate::parser::StatementStream::new(&source).unwrap();
    crate::parser::expr::parse_expr(&mut ts).unwrap()
}

// ─────────────────── M21.4 advanced feature tests ───────────────────

/// sashelp.class-like sex/age fixture: M/F with weights.
fn class_like(session: &mut Session) {
    // 3 F (ages 11,12,13 → sum 36) and 2 M (ages 14,15 → sum 29).
    let df = df![
        "sex" => ["F", "F", "F", "M", "M"],
        "age" => [11.0_f64, 12.0, 13.0, 14.0, 15.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("sex"), num_meta("age")],
    };
    write_dataset(session, "C", ds);
}

// ─────────────────── M33.5 deferred-option tests ───────────────────

/// Build a DEFINE with optional format/width/spacing (M33.5 test helper).
fn def(
    var: &str,
    usage: Usage,
    label: Option<&str>,
    format: Option<&str>,
    width: Option<usize>,
    spacing: Option<usize>,
) -> Define {
    Define {
        var: var.into(),
        usage,
        order: OrderDir::Ascending,
        label: label.map(|s| s.to_string()),
        format: format.map(|s| s.to_string()),
        width,
        spacing,
    }
}

mod parse;
mod default;
mod rbreak;
