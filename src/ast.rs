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

/// Source d'un statement INFILE (M14.1).
#[derive(Debug, Clone, PartialEq)]
pub enum InfileSource {
    /// `infile 'chemin' ...;` — fichier texte externe (chemin littéral).
    Path(String),
    /// `infile datalines ...;` — relit les lignes du bloc DATALINES de
    /// l'étape (équivalent au INPUT direct depuis DATALINES, mais permet de
    /// poser des options DSD/DLM=).
    Datalines,
}

/// Options du statement INFILE (M14.1). Toutes optionnelles.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InfileOptions {
    /// `DLM=`/`DELIMITER=` : caractère(s) délimiteur(s). `None` = défaut
    /// (blanc en list input, ou `,` sous DSD).
    pub delimiter: Option<String>,
    /// `DSD` : champs façon CSV (délimiteur défaut `,`, quotes gérées,
    /// délimiteurs consécutifs = valeur manquante).
    pub dsd: bool,
    /// `FIRSTOBS=` : 1re ligne (1-based) à lire ; défaut 1.
    pub firstobs: Option<usize>,
    /// `OBS=` : numéro de la dernière ligne à lire (1-based).
    pub obs: Option<usize>,
    /// `MISSOVER` : fin de ligne prématurée → variables restantes missing,
    /// sans passer à la ligne suivante.
    pub missover: bool,
    /// `TRUNCOVER` : comme MISSOVER mais la dernière variable reçoit ce qui
    /// reste (même partiel) avant la fin de ligne.
    pub truncover: bool,
    /// `STOPOVER` : fin de ligne prématurée → erreur, l'étape s'arrête.
    pub stopover: bool,
    /// `LRECL=` : accepté puis ignoré (longueur d'enregistrement).
    pub lrecl: Option<usize>,
}

/// Style d'un item d'INPUT (M14.1).
#[derive(Debug, Clone, PartialEq)]
pub enum InputItem {
    /// Une variable à lire. Le style de lecture dépend des champs présents :
    /// list (rien), column (`col_range`), ou formatted (`informat`,
    /// éventuellement précédé d'un pointeur `@col`).
    Var {
        name: String,
        /// Variable caractère (`$`).
        is_char: bool,
        /// Column input : plage de colonnes 1-based inclusive `(début, fin)`.
        col_range: Option<(usize, usize)>,
        /// Formatted/list-with-informat : informat (token, ex. "5.2",
        /// "$10.", "COMMA8.").
        informat: Option<String>,
    },
    /// `@n` : positionne le pointeur de colonne à la colonne n (1-based).
    PointerCol(usize),
    /// `+n` : avance le pointeur de colonne de n.
    PointerSkip(usize),
    /// `/` : passe à la ligne d'entrée suivante.
    NextLine,
}

/// Destination d'un statement FILE / PUT (M14.2).
#[derive(Debug, Clone, PartialEq)]
pub enum PutDest {
    /// `file 'chemin';` — fichier physique (chemin littéral).
    Path(String),
    /// `file log;` / `put` par défaut — le journal SAS.
    Log,
    /// `file print;` — le listing (sortie « print »).
    Print,
}

/// Style d'un item de PUT (M14.2). Miroir de `InputItem`.
#[derive(Debug, Clone, PartialEq)]
pub enum PutItem {
    /// Une variable à écrire. Le style d'écriture dépend des champs :
    /// list (rien), column (`@col` posé en amont via `PointerCol`),
    /// formatted (`format`).
    Var {
        name: String,
        /// Format d'écriture éventuel (token, ex. "5.2", "$10.", "DATE9.").
        format: Option<String>,
    },
    /// Named output `var=` : écrit `NOM=valeur` (forme `put name=;`).
    NamedVar(String),
    /// Littéral entre quotes : écrit verbatim.
    Literal(String),
    /// `@n` : positionne le pointeur de colonne de sortie (1-based).
    PointerCol(usize),
    /// `+n` : avance le pointeur de colonne de sortie de n.
    PointerSkip(usize),
    /// `/` : passe à la ligne de sortie suivante (dans le même PUT).
    NextLine,
    /// `_all_` : écrit `var=valeur` pour toutes les variables du PDV.
    All,
    /// `@` en fin de PUT : maintien de ligne dans la MÊME itération.
    HoldLine,
    /// `@@` en fin de PUT : maintien de ligne à travers les itérations.
    HoldLineAcross,
}

/// DATA step statements (M1 subset + M2 : RETAIN, sum statement, LENGTH ;
/// M14 : INFILE/INPUT/DATALINES, FILE/PUT).
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
    /// `call <name>(args);` — appel d'une CALL routine (M11.5). Pour v1,
    /// seule `CALL SYMPUT` est exécutée (pont DATA step → table macro) ;
    /// les autres routines parsent mais produisent une erreur runtime
    /// « not yet implemented ». Le nom est conservé tel quel (résolu en
    /// MAJUSCULES à l'exécution). Ce statement est parsé dans les DEUX
    /// builds (aucun test/fixture existant n'emploie `call`).
    CallRoutine { name: String, args: Vec<Expr> },
    /// `infile <source> <options>;` (M14.1) — déclare la source de lecture
    /// texte de l'étape. Une seule source active à la fois (un second INFILE
    /// remplace le premier, comme SAS).
    Infile {
        source: InfileSource,
        options: InfileOptions,
    },
    /// `input <items>;` (M14.1) — lit la ligne d'entrée courante dans le PDV
    /// selon les items (list / column / formatted). Le `@@` (hold) n'est pas
    /// couvert ; voir parser.
    Input { items: Vec<InputItem> },
    /// `datalines;`/`cards;` (M14.1) — lignes de données en ligne dans le
    /// source. Capturées brutes par le lexer ; le INPUT les consomme.
    Datalines { lines: Vec<String> },
    /// `file <dest> <options>;` (M14.2) — choisit la destination des PUT
    /// suivants (fichier physique / LOG / PRINT). Options DLM=/DSD/LRECL=
    /// acceptées (LRECL ignorée). Le dernier FILE gagne.
    File {
        dest: PutDest,
        /// DLM=/DELIMITER= : séparateur inséré entre les items de list output.
        delimiter: Option<String>,
        /// DSD : sépare par `,` et entoure de quotes les valeurs char
        /// contenant le délimiteur.
        dsd: bool,
    },
    /// `put <items>;` (M14.2) — écrit dans la destination courante (LOG par
    /// défaut) selon les items (list / formatted / named / littéraux),
    /// pointeurs `@n`/`+n`, `/`, maintien de ligne `@`/`@@`, `_all_`.
    Put { items: Vec<PutItem> },
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
        /// Optional engine keyword (e.g. `"CSV"`, `"XLSX"`). `None` = default
        /// Parquet/DirLibrary behaviour (backward-compatible).
        engine: Option<String>,
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
