use super::*;

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
    /// Référence d'array indexée `arr{i}` / `arr[i]` / `arr{i, j}` (M2/M16.2).
    /// La forme à parenthèses `arr(i)` reste un `Call` : l'ambiguïté avec un
    /// appel de fonction est résolue à l'ÉVALUATION (l'array masque la
    /// fonction, comme SAS). `indices` porte un ou plusieurs indices (un par
    /// dimension de l'array, ou un seul = interprétation linéaire row-major).
    Index {
        name: String,
        indices: Vec<Expr>,
    },
    /// Appel de méthode d'objet hash en POSITION D'EXPRESSION (M17.2) :
    /// `rc = h.find();`. Renvoie le code retour numérique de la méthode (0 =
    /// succès, ≠0 = échec). Évalué par `exec::eval_checked` (qui a `&mut self`)
    /// car les méthodes hash mutent le PDV (copie des données sur `find`) et
    /// les objets hash — l'évaluateur immuable `eval()` ne peut pas les servir
    /// et renvoie un missing de garde. La forme statement (`h.find();`) reste
    /// `DsStmt::HashMethod` (le code retour est ignoré).
    /// Boxé pour garder `Expr` compact (l'appel de méthode hash est rare).
    HashMethod(Box<HashMethodCall>),
}

/// Données d'un appel de méthode d'objet hash (M17.2), partagées par
/// `Expr::HashMethod` (forme expression) et `DsStmt::HashMethod` (statement).
#[derive(Debug, Clone, PartialEq)]
pub struct HashMethodCall {
    pub object: String,
    pub method: String,
    pub args: Vec<HashArg>,
}

/// Un argument d'appel de méthode d'objet hash (M17.2). Soit positionnel
/// (`defineKey('k')`, `find()`), soit nommé (`add(key:1, data:'x')`,
/// `output(dataset:'lib.tab')`). Le nom est normalisé en minuscules.
#[derive(Debug, Clone, PartialEq)]
pub enum HashArg {
    /// Argument positionnel : une expression (souvent un littéral chaîne
    /// nommant une variable, pour defineKey/defineData).
    Positional(Expr),
    /// Argument nommé `name: expr` (`key:`, `data:`, `dataset:`).
    Named(String, Expr),
}

/// Liste spéciale d'éléments d'un statement ARRAY (M16.2). À la
/// compilation, ces mots-clés sont remplacés par l'ensemble des variables
/// correspondantes du PDV (toutes celles connues au point du statement).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArraySpecial {
    /// `_NUMERIC_` : toutes les variables numériques.
    Numeric,
    /// `_CHARACTER_` : toutes les variables caractère.
    Character,
    /// `_ALL_` : toutes les variables (de même type — SAS exige
    /// l'homogénéité ; on prend toutes les numériques OU toutes les char
    /// selon `$`, à défaut toutes les numériques).
    All,
}

/// Un élément d'une liste de valeurs de DO (M16.3). Soit une valeur unique
/// (`Value`), soit une sous-liste itérative `from to e [by k]` (`Range`),
/// énumérée à l'exécution comme un DO classique.
#[derive(Debug, Clone, PartialEq)]
pub enum DoListItem {
    /// Valeur explicite unique : `3`, `'red'`, `x+1`.
    Value(Expr),
    /// Sous-liste `from to to_ [by by_]`.
    Range {
        from: Expr,
        to: Expr,
        by: Option<Expr>,
    },
}
