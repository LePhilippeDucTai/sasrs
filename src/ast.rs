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

/// Options de dataset `(keep=... drop=... rename=(...) where=(...))` (M2).
/// `keep`/`drop` : `None` = option absente (≠ liste vide). `rename` :
/// paires (ancien, nouveau). `where_` : expression filtrante (valide en
/// entrée SET seulement ; en sortie DATA → erreur de compilation).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DatasetOptions {
    pub keep: Option<Vec<String>>,
    pub drop: Option<Vec<String>>,
    pub rename: Vec<(String, String)>,
    pub where_: Option<Expr>,
    /// `in=nom` (M3) : variable automatique temporaire 0/1 indiquant si le
    /// dataset a participé au groupe de clé BY courant d'un MERGE. Valide
    /// uniquement en INPUT de MERGE ; en sortie DATA → erreur de
    /// compilation. Jamais écrite en sortie (comme FIRST./LAST.).
    pub in_: Option<String>,
}

/// Référence de dataset accompagnée de ses options : `lib.a(keep=x y)`.
#[derive(Debug, Clone, PartialEq)]
pub struct DatasetSpec {
    pub dref: DatasetRef,
    pub options: DatasetOptions,
}

impl DatasetSpec {
    /// Spec sans options (helper pour les constructions simples / tests).
    pub fn plain(dref: DatasetRef) -> Self {
        DatasetSpec {
            dref,
            options: DatasetOptions::default(),
        }
    }

    /// Display form "WORK.A" (délégué à `DatasetRef`).
    pub fn display(&self) -> String {
        self.dref.display()
    }

    pub fn libref_or_work(&self) -> String {
        self.dref.libref_or_work()
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
    /// Référence d'array indexée `arr{i}` / `arr[i]` (M2). La forme à
    /// parenthèses `arr(i)` reste un `Call` : l'ambiguïté avec un appel de
    /// fonction est résolue à l'ÉVALUATION (l'array masque la fonction,
    /// comme SAS).
    Index {
        name: String,
        index: Box<Expr>,
    },
}

/// Spec d'une variable dans un statement LENGTH : `$ n` (char) ou `n` (num).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LengthSpec {
    pub char: bool,
    pub len: usize,
}

/// Un item du statement ATTRIB : un groupe de variables et les attributs
/// déclarés. `format`/`label` sont optionnels ; `length` est conservé pour
/// compatibilité mais non appliqué en M4 (voir parser).
#[derive(Debug, Clone, PartialEq)]
pub struct AttribItem {
    pub vars: Vec<String>,
    pub format: Option<String>,
    pub label: Option<String>,
    pub length: Option<LengthSpec>,
}

/// DATA step statements (M1 subset + M2 : RETAIN, sum statement, LENGTH ;
/// M2+ ajoutera DO iterative, ARRAY, MERGE, BY...).
#[derive(Debug, Clone, PartialEq)]
pub enum DsStmt {
    /// `set lib.a [lib.b ...];` — un ou plusieurs datasets, chacun avec
    /// ses options `(keep=... drop=... rename=(...) where=(...))`. Sans
    /// BY, plusieurs datasets = CONCATÉNATION (a en entier puis b) ; avec
    /// BY = INTERCLASSEMENT (M3).
    Set(Vec<DatasetSpec>),
    /// `by [descending] v1 [descending] v2 ...;` — clés d'interclassement
    /// du SET (M3). Chaque paire = (nom, descending). Le statement est
    /// purement déclaratif : la sémantique (tri, FIRST./LAST.) est résolue
    /// à la compilation/exécution.
    By(Vec<(String, bool)>),
    /// `merge ds1[(in=a)] ds2[(in=b)] ...;` (M3) — match-merge SAS par BY.
    /// Comme SET, chaque dataset porte ses options `(keep=/drop=/rename=/
    /// where=/in=)`. Une étape ne peut avoir qu'UN SET ou MERGE. Les
    /// datasets doivent être triés par BY ; la sémantique (persistance du
    /// côté court, IN=, FIRST./LAST.) est résolue à la compilation/exécution.
    Merge(Vec<DatasetSpec>),
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
    /// `output;` (liste vide = TOUTES les sorties du DATA) ou
    /// `output a [b...];` (sorties ciblées — `output a b;` écrit dans a ET
    /// b). Seul le nom (lib.table) compte ici, sans options ; chaque nom
    /// doit correspondre à une sortie du statement DATA (vérifié à la
    /// compilation).
    Output(Vec<DatasetRef>),
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
    /// `format weight height 8.2 name $char10.;` (M4) — chaque groupe est
    /// une liste de variables suivie d'un token de format. Déclaratif :
    /// associe un format aux variables (appliqué à la finalisation du PDV /
    /// par PROC PRINT) ; aucun effet à l'exécution.
    Format(Vec<(Vec<String>, String)>),
    /// `label weight='Body Weight' name='Pupil';` (M4) — paires
    /// (variable, libellé). Déclaratif.
    Label(Vec<(String, String)>),
    /// `attrib weight format=8.2 label='Body Weight';` (M4) — un item par
    /// groupe de variables, portant format=/label=/length= (length=
    /// optionnel). Déclaratif.
    Attrib(Vec<AttribItem>),
    /// `array arr{3} x y z;` (M2, 1-D). `size: None` = `{*}` (taille
    /// déduite de la liste) ; `char_len: Some(n)` = array caractère
    /// (`$ n`, défaut 8) ; `vars` vide = éléments auto-nommés arr1..arrN
    /// (expansés à la compilation). Les plages numérotées `x1-x3` sont
    /// DÉJÀ expansées par le parser.
    Array {
        name: String,
        size: Option<usize>,
        char_len: Option<usize>,
        vars: Vec<String>,
    },
    /// `arr{i} = expr;` / `arr[i] = expr;` / `arr(i) = expr;` —
    /// assignation à un élément d'array. Pour la forme à parenthèses, le
    /// nom est validé array à la COMPILATION.
    AssignIndexed {
        array: String,
        index: Expr,
        expr: Expr,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DataStepAst {
    pub outputs: Vec<DatasetSpec>,
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
