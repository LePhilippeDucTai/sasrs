
// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone)]
pub struct ImlProgram {
    pub stmts: Vec<ImlStmt>,
}

#[derive(Debug, Clone)]
pub enum ImlStmt {
    Assign { var: String, expr: ImlExpr },
    Print { items: Vec<ImlPrintItem> },
    If { cond: ImlExpr, then_body: Vec<ImlStmt>, else_body: Vec<ImlStmt> },
    DoLoop { var: String, from: ImlExpr, to: ImlExpr, by: Option<ImlExpr>, body: Vec<ImlStmt> },
    DoWhile { cond: ImlExpr, body: Vec<ImlStmt> },
    DoUntil { cond: ImlExpr, body: Vec<ImlStmt> },
    /// CALL routine(args). Les arguments de sortie (CALL QR/SVDCD/EIGEN) sont
    /// des lvalues (noms de matrices à affecter) — résolus à l'exécution.
    Call { func: String, args: Vec<ImlExpr> },
    /// `CREATE ds FROM mat [COLNAME=cn];`
    Create { ds: String, from: String, colname: Option<ImlExpr> },
    /// `APPEND FROM mat;`
    Append { from: String },
    /// `CLOSE ds;`
    Close { ds: String },
    /// `USE ds;`
    Use { ds: String },
    /// `READ ALL VAR {vars} INTO mat;`
    ReadAll { vars: Vec<String>, into: String },
    /// Statements I/O non encore implémentés (erreur propre à l'exécution).
    UnsupportedIo { msg: String },
}

#[derive(Debug, Clone)]
pub enum ImlPrintItem {
    Var(String),
    StringLiteral(String),
}

#[derive(Debug, Clone)]
pub enum ImlExpr {
    Literal(Vec<Vec<f64>>),
    /// Littéral de liste de chaînes : `{"x" "y"}`. Utilisé pour COLNAME=.
    StrList(Vec<String>),
    Var(String),
    BinOp { op: ImlOp, left: Box<ImlExpr>, right: Box<ImlExpr> },
    Unary { op: UnaryOp, expr: Box<ImlExpr> },
    Transpose(Box<ImlExpr>),
    Subscript { mat: Box<ImlExpr>, row: ImlIndex, col: ImlIndex },
    FnCall { name: String, args: Vec<ImlExpr> },
}

#[derive(Debug, Clone)]
pub enum ImlIndex {
    All,
    Scalar(Box<ImlExpr>),
    /// `a:b` — intervalle inclusif 1-basé (M34.10).
    Range(Box<ImlExpr>, Box<ImlExpr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImlOp {
    Add, Sub, Mul, Hadamard, Div, Kronecker,
    Eq, Ne, Lt, Le, Gt, Ge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
}
