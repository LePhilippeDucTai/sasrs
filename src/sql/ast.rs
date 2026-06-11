//! AST du dialecte SQL de SAS (jalon M6).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! Couvre : CREATE TABLE ... AS SELECT, SELECT (vers listing), DROP
//! TABLE, INSERT INTO (VALUES / SELECT), DELETE FROM, DESCRIBE TABLE.
//! Spécificités SAS : `CALCULATED alias` dans select/where/group/having ;
//! options de dataset sur les refs (`from a(keep=x where=(...))`) — M6
//! limite aux refs nues d'abord ; `SELECT ... INTO :macrovar` réservé
//! (ERROR propre tant que le macro-processeur n'existe pas).

#![allow(dead_code)]

use crate::ast::{DatasetRef, Expr};

pub struct SqlProgram {
    pub stmts: Vec<SqlStmt>,
}

pub enum SqlStmt {
    Select(SelectStmt),
    CreateTableAs { table: DatasetRef, query: SelectStmt },
    DropTable(Vec<DatasetRef>),
    InsertValues { table: DatasetRef, columns: Vec<String>, rows: Vec<Vec<Expr>> },
    InsertSelect { table: DatasetRef, query: SelectStmt },
    DeleteFrom { table: DatasetRef, where_: Option<SqlExpr> },
    Describe(DatasetRef),
}

pub struct SelectStmt {
    pub distinct: bool,
    pub items: Vec<SelectItem>,
    pub from: Vec<FromItem>,
    pub joins: Vec<Join>,
    pub where_: Option<SqlExpr>,
    /// Entiers positionnels OU expressions.
    pub group_by: Vec<SqlExpr>,
    pub having: Option<SqlExpr>,
    pub order_by: Vec<(SqlExpr, bool /* desc */)>,
    pub set_op: Option<(SetOp, bool /* all */, Box<SelectStmt>)>,
}

pub enum SetOp {
    Union,
    Except,
    Intersect,
}

pub struct SelectItem {
    pub expr: SqlExpr,
    pub alias: Option<String>,
}

pub struct FromItem {
    pub table: DatasetRef,
    pub alias: Option<String>,
}

pub struct Join {
    pub kind: JoinKind,
    pub table: FromItem,
    pub on: Option<SqlExpr>,
}

pub enum JoinKind {
    Inner,
    Left,
    Right,
    Full,
    Cross,
}

/// Expression SQL : réutilise Expr (mêmes littéraux/opérateurs/fonctions)
/// plus les nœuds spécifiques SQL.
pub enum SqlExpr {
    Base(Expr),
    /// `alias.colonne`
    Qualified { table: String, column: String },
    /// `CALCULATED alias` — référence à un alias du select-list.
    Calculated(String),
    /// COUNT(*) / COUNT(DISTINCT x) / SUM / AVG / MIN / MAX ...
    Aggregate { func: String, distinct: bool, arg: Option<Box<SqlExpr>>, star: bool },
    /// `x BETWEEN a AND b`, `x IS [NOT] NULL/MISSING`, `x LIKE 'p%'`
    Between { expr: Box<SqlExpr>, low: Box<SqlExpr>, high: Box<SqlExpr>, negated: bool },
    IsNull { expr: Box<SqlExpr>, negated: bool },
    Like { expr: Box<SqlExpr>, pattern: String, negated: bool },
    Binary { op: crate::ast::BinaryOp, left: Box<SqlExpr>, right: Box<SqlExpr> },
    Unary { op: crate::ast::UnaryOp, expr: Box<SqlExpr> },
}
