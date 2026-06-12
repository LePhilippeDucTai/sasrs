//! AST for parsed SAS blocks. One `Block` = one executable unit (a global
//! statement, a DATA step, or a PROC step). Each PROC owns its own AST
//! struct, registered in `procs::registry`.

use crate::token::Span;
use crate::value::MissingKind;

/// `lib.table` reference; libref defaults to WORK when absent.
#[derive(Debug, Clone, PartialEq)]
pub struct DatasetRef {
    pub libref: Option<String>,
    pub name: String,
}

impl DatasetRef {
    /// Display form "WORK.A" used in log NOTEs.
    pub fn display(&self) -> String {
        format!(
            "{}.{}",
            self.libref.as_deref().unwrap_or("WORK").to_uppercase(),
            self.name.to_uppercase()
        )
    }

    pub fn libref_or_work(&self) -> String {
        self.libref.as_deref().unwrap_or("WORK").to_uppercase()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Minus,
    Plus,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Power,
    Concat,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    Str(String),
    Missing(MissingKind),
    Var(String),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// `x in (1, 2, 3)`
    In {
        expr: Box<Expr>,
        list: Vec<Expr>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
}

/// Spec d'une variable dans un statement LENGTH : `$ n` (char) ou `n` (num).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LengthSpec {
    pub char: bool,
    pub len: usize,
}

/// DATA step statements (M1 subset + M2 : RETAIN, sum statement, LENGTH ;
/// M2+ ajoutera DO iterative, ARRAY, MERGE, BY...).
#[derive(Debug, Clone, PartialEq)]
pub enum DsStmt {
    /// `set lib.a;` — M1: single dataset, no options.
    Set(DatasetRef),
    Assign {
        var: String,
        expr: Expr,
    },
    If {
        cond: Expr,
        then_branch: Box<DsStmt>,
        else_branch: Option<Box<DsStmt>>,
    },
    /// Subsetting `if expr;`
    SubsettingIf(Expr),
    /// Non-iterative `do; ... end;`
    Block(Vec<DsStmt>),
    /// DO itératif / conditionnel (M2) : `do i = e1 [to e2] [by e3]
    /// [while(c)] [until(c)]; ... end;`, `do while(c); ... end;`,
    /// `do until(c); ... end;`. `index` porte le nom de la variable
    /// d'index et son expression de départ (from). Les listes de valeurs
    /// (`do i = 1, 5, 9;`) ne sont pas encore implémentées (erreur de
    /// parsing propre).
    DoLoop {
        index: Option<(String, Expr)>,
        to: Option<Expr>,
        by: Option<Expr>,
        while_: Option<Expr>,
        until: Option<Expr>,
        body: Vec<DsStmt>,
    },
    /// `delete;` — termine l'itération courante sans output implicite
    /// (même effet qu'un subsetting IF faux).
    Delete,
    Output,
    Keep(Vec<String>),
    Drop(Vec<String>),
    Stop,
    /// `retain v1 v2;` / `retain v 100;` / `retain a 1 b 'x' c;` /
    /// `retain;` (liste vide = toutes les variables du PDV). La valeur
    /// initiale optionnelle est un LITTÉRAL (`Expr::Num`, `Expr::Str` ou
    /// `Expr::Missing` — un `-5` est replié en `Num(-5.0)` par le parser).
    Retain(Vec<(String, Option<Expr>)>),
    /// Sum statement `var + expr;` (ex. `total + x;`). PAS de forme `-`.
    Sum { var: String, expr: Expr },
    /// `length v1 v2 $ 20 v3 5;`
    Length(Vec<(String, LengthSpec)>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DataStepAst {
    pub outputs: Vec<DatasetRef>,
    pub stmts: Vec<DsStmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GlobalStmt {
    Libname {
        libref: String,
        path: String,
    },
    LibnameClear {
        libref: String,
    },
    Title {
        n: u8,
        text: Option<String>,
    },
    /// Parsed OPTIONS name=value / flag list; unknown options warn.
    Options(Vec<(String, Option<String>)>),
}
