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

#[derive(Debug, Clone, PartialEq)]
pub struct SqlProgram {
    pub stmts: Vec<SqlStmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SqlStmt {
    Select(SelectStmt),
    CreateTableAs { table: DatasetRef, query: SelectStmt },
    DropTable(Vec<DatasetRef>),
    InsertValues { table: DatasetRef, columns: Vec<String>, rows: Vec<Vec<Expr>> },
    InsertSelect { table: DatasetRef, query: SelectStmt },
    DeleteFrom { table: DatasetRef, where_: Option<SqlExpr> },
    Describe(DatasetRef),
}

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
pub enum SetOp {
    Union,
    Except,
    Intersect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectItem {
    pub expr: SqlExpr,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FromItem {
    pub table: DatasetRef,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Join {
    pub kind: JoinKind,
    pub table: FromItem,
    pub on: Option<SqlExpr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JoinKind {
    Inner,
    Left,
    Right,
    Full,
    Cross,
}

/// Expression SQL : réutilise Expr (mêmes littéraux/opérateurs/fonctions)
/// plus les nœuds spécifiques SQL.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlExpr {
    Base(Expr),
    /// `*` — toutes les colonnes (uniquement valide en tête de select-list).
    Star,
    /// `alias.*` — toutes les colonnes d'une table qualifiée.
    QualifiedStar(String),
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
    /// Sous-requête scalaire `(SELECT ...)` : doit renvoyer une seule colonne /
    /// une seule ligne. Évaluée (non-corrélée) avant l'abaissement Polars.
    Subquery(Box<SelectStmt>),
    /// `expr [NOT] IN (SELECT ...)` : la sous-requête fournit une colonne de
    /// valeurs. Non-corrélée → évaluée puis transformée en liste.
    InSubquery { expr: Box<SqlExpr>, query: Box<SelectStmt>, negated: bool },
    /// `[NOT] EXISTS (SELECT ...)` : vrai si la sous-requête renvoie ≥ 1 ligne.
    /// Non-corrélée → évaluée puis réduite à un booléen constant.
    Exists { query: Box<SelectStmt>, negated: bool },
}
