use super::*;
use crate::source::SourceFile;

fn parse(src: &str) -> Result<SqlProgram> {
    let file = SourceFile::new(src);
    let mut ts = StatementStream::new(&file)?;
    parse_sql_program(&mut ts)
}

fn ok(src: &str) -> SqlProgram {
    parse(src).unwrap_or_else(|e| panic!("parse of {src:?} failed: {e}"))
}

fn one(src: &str) -> SqlStmt {
    let mut prog = ok(src);
    assert_eq!(prog.stmts.len(), 1, "expected exactly one statement");
    prog.stmts.pop().unwrap()
}

fn dref(name: &str) -> crate::ast::DatasetRef {
    crate::ast::DatasetRef {
        libref: None,
        name: name.to_string(),
    }
}

fn var(s: &str) -> SqlExpr {
    SqlExpr::Base(Expr::Var(s.to_string()))
}

fn qual(t: &str, c: &str) -> SqlExpr {
    SqlExpr::Qualified {
        table: t.to_string(),
        column: c.to_string(),
    }
}

mod select;
mod exists;
