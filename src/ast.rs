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

/// Options de NIVEAU STATEMENT du `SET` (M16.4), placées APRÈS la liste des
/// datasets : `set a b end=eof nobs=n point=p;`. À distinguer des options de
/// DATASET (`DatasetOptions`, entre parenthèses après chaque référence).
///
/// - `end` : nom d'une variable temporaire automatique (jamais écrite en
///   sortie, comme FIRST./LAST.) mise à 0 pendant l'itération et à 1 lorsque
///   la DERNIÈRE observation du DERNIER dataset a été lue.
/// - `nobs` : nom d'une variable numérique affectée AVANT la boucle au nombre
///   total d'observations (somme sur tous les datasets du SET).
/// - `point` : nom d'une variable numérique d'INDEX (1-based). Sa présence
///   DÉSACTIVE la boucle implicite et l'output implicite : à chaque exécution
///   du SET, l'observation à l'index courant est lue. Index missing/invalide/
///   hors bornes → erreur runtime.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SetOptions {
    pub end: Option<String>,
    pub nobs: Option<String>,
    pub point: Option<String>,
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
    /// Référence d'array indexée `arr{i}` / `arr[i]` / `arr{i, j}` (M2/M16.2).
    /// La forme à parenthèses `arr(i)` reste un `Call` : l'ambiguïté avec un
    /// appel de fonction est résolue à l'ÉVALUATION (l'array masque la
    /// fonction, comme SAS). `indices` porte un ou plusieurs indices (un par
    /// dimension de l'array, ou un seul = interprétation linéaire row-major).
    Index {
        name: String,
        indices: Vec<Expr>,
    },
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

/// Source d'un statement INFILE (M14) : un fichier sur disque (chemin
/// littéral) ou les lignes inline d'un bloc DATALINES/CARDS.
#[derive(Debug, Clone, PartialEq)]
pub enum InfileSource {
    /// `infile 'chemin';` — lecture d'un fichier texte.
    Path(String),
    /// `infile datalines;` / `infile cards;` — la source est le bloc
    /// DATALINES inline de l'étape.
    Datalines,
}

/// Options d'un statement INFILE (M14). Tous les champs sont optionnels ;
/// `None`/`false` = option absente.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InfileOptions {
    /// `DELIMITER=`/`DLM=` : caractère(s) séparateur(s) de la lecture en
    /// liste. `None` = défaut (l'espace). Une chaîne peut porter plusieurs
    /// délimiteurs (chacun de ses caractères en est un).
    pub delimiter: Option<String>,
    /// `DSD` : délimiteur-sensible — deux délimiteurs consécutifs encadrent
    /// une valeur manquante, les guillemets protègent les délimiteurs, le
    /// délimiteur par défaut devient la virgule.
    pub dsd: bool,
    /// `FIRSTOBS=` : numéro (1-based) de la première ligne lue.
    pub firstobs: Option<usize>,
    /// `OBS=` : numéro (1-based) de la dernière ligne lue.
    pub obs: Option<usize>,
    /// `MISSOVER` : un INPUT qui dépasse la fin de ligne laisse les
    /// variables restantes à missing (pas de passage à la ligne suivante).
    pub missover: bool,
    /// `TRUNCOVER` : comme MISSOVER, mais une valeur partielle en fin de
    /// ligne est tout de même lue.
    pub truncover: bool,
    /// `STOPOVER` : un INPUT qui dépasse la fin de ligne est une erreur qui
    /// arrête l'étape.
    pub stopover: bool,
    /// `LRECL=` : longueur d'enregistrement (parsée et conservée ;
    /// no-op fonctionnel — toutes les lignes sont lues entières).
    pub lrecl: Option<usize>,
}

/// Un item du statement INPUT (M14). L'ordre des items dans la liste
/// reflète l'ordre textuel ; chaque item est consommé séquentiellement.
#[derive(Debug, Clone, PartialEq)]
pub enum InputItem {
    /// Une variable à lire. Le MODE de lecture dépend des champs :
    /// - `cols = Some((a, b))` : mode COLONNE — colonnes 1-based a..=b.
    /// - `informat = Some(tok)` : mode FORMATÉ — l'informat est appliqué au
    ///   champ. Avec `list_modifier = true` (`:`), la largeur sert seulement
    ///   d'informat sur un jeton délimité (mode liste).
    /// - sinon : mode LISTE — jeton délimité par espaces/délimiteurs.
    Var {
        name: String,
        /// `$` : variable caractère.
        is_char: bool,
        /// Colonnes 1-based inclusives `a-b` (mode colonne).
        cols: Option<(usize, usize)>,
        /// Token d'informat (`date9.`, `8.2`, `$char10.`...).
        informat: Option<String>,
        /// `:` modificateur — informat appliqué en mode liste (jeton
        /// délimité, pas colonnes fixes).
        list_modifier: bool,
    },
    /// `@n` : pointeur de colonne absolu (place le curseur en colonne n).
    ColumnPointer(usize),
    /// `+n` : avance le curseur de n colonnes.
    SkipColumns(usize),
    /// `/` : passe à la ligne d'entrée suivante.
    NextLine,
    /// `@` final : maintient l'enregistrement pour le prochain INPUT de la
    /// MÊME itération (line hold simple).
    HoldLine,
    /// `@@` final : maintient l'enregistrement à TRAVERS les itérations
    /// (plusieurs « observations » par ligne).
    HoldLineDouble,
}

/// Destination d'un statement FILE (M14.2) : la sortie courante des PUT.
/// Par défaut (aucun FILE), un PUT écrit dans le LOG (comportement SAS).
#[derive(Debug, Clone, PartialEq)]
pub enum PutDest {
    /// `file 'chemin';` — un fichier texte externe (créé/tronqué à la
    /// première écriture de l'étape).
    Path(String),
    /// `file log;` — le journal SAS (destination par défaut).
    Log,
    /// `file print;` — la sortie « listing » (PROC PRINT-like).
    Print,
}

/// Un item du statement PUT (M14.2). Miroir de sortie d'`InputItem` :
/// l'ordre reflète l'ordre textuel, chaque item est rendu séquentiellement
/// dans la ligne de sortie courante.
#[derive(Debug, Clone, PartialEq)]
pub enum PutItem {
    /// Une variable écrite avec son format d'affichage (ou BESTw./$w. par
    /// défaut). `format = Some(tok)` applique un format explicite
    /// (`put x 8.2;`, `put d date9.;`).
    Var {
        name: String,
        format: Option<String>,
    },
    /// `put name=;` — écrit `name=VALEUR` (forme nommée).
    NamedVar(String),
    /// `put 'texte';` — une chaîne littérale écrite verbatim.
    Literal(String),
    /// `@n` : pointeur de colonne absolu (place le curseur en colonne n,
    /// 1-based).
    ColumnPointer(usize),
    /// `+n` : avance le curseur de n colonnes.
    SkipColumns(usize),
    /// `/` : passe à la ligne de sortie suivante (saut de ligne dans le
    /// même PUT).
    NextLine,
    /// `@` final : maintient la ligne de sortie (supprime le relâchement
    /// automatique ; le prochain PUT continue la même ligne physique).
    HoldLine,
    /// `@@` final : maintient la ligne de sortie à TRAVERS les itérations.
    HoldLineDouble,
    /// `put _all_;` — écrit `nom=valeur` pour chaque variable du PDV.
    All,
}

/// DATA step statements (M1 subset + M2 : RETAIN, sum statement, LENGTH ;
/// M2+ ajoutera DO iterative, ARRAY, MERGE, BY... ; M14 : INFILE/INPUT/
/// DATALINES).
#[derive(Debug, Clone, PartialEq)]
pub enum DsStmt {
    /// `set lib.a [lib.b ...];` — un ou plusieurs datasets, chacun avec
    /// ses options `(keep=... drop=... rename=(...) where=(...))`. Sans
    /// BY, plusieurs datasets = CONCATÉNATION (a en entier puis b) ; avec
    /// BY = INTERCLASSEMENT (M3). Les options de niveau statement (M16.4 :
    /// `end=`/`nobs=`/`point=`) sont portées par `options`.
    Set {
        specs: Vec<DatasetSpec>,
        options: SetOptions,
    },
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
    /// `do i = 1, 3, 5;` / `do c = 'a', 'b';` / `do i = 1 to 5, 10, 20 to 30 by 5;`
    /// (M16.3) — DO sur une LISTE de valeurs (valeurs explicites et/ou
    /// sous-listes `from to by`, dans n'importe quel ordre). L'index prend
    /// successivement chaque valeur de la liste développée (les ranges sont
    /// énumérés à l'exécution) ; le corps s'exécute une fois par valeur.
    /// `index` est le nom de la variable de contrôle.
    DoList {
        index: String,
        items: Vec<DoListItem>,
        body: Vec<DsStmt>,
    },
    /// `do over arr; ... end;` (M16.3) — itère implicitement sur les éléments
    /// d'un array dans l'ordre row-major. À chaque tour, une référence NUE au
    /// nom de l'array (`arr`, sans indice) désigne l'élément courant (en
    /// lecture comme en écriture) ; l'accès indexé `arr{i}` reste statique.
    /// `array` est le nom de l'array (validé à la compilation).
    DoOver {
        array: String,
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
    /// `array arr{3} x y z;` (M2) / `array arr{2,3} v1-v6;` (M16.2,
    /// multi-dimensionnel). `dims: None` = `{*}` (taille déduite de la
    /// liste, 1-D) ; sinon `Some(vec![3])` (1-D) ou `Some(vec![2,3])`
    /// (2-D, etc.) — chaque borne supérieure, borne inférieure = 1.
    /// `char_len: Some(n)` = array caractère (`$ n`, défaut 8) ; `vars`
    /// vide = éléments auto-nommés arr1..arrN (expansés à la compilation),
    /// SAUF si `special`/`temporary`. Les plages numérotées `x1-x3` sont
    /// DÉJÀ expansées par le parser. `initial`: valeurs initiales
    /// `(1, 2, 3)` en ordre row-major (vide = aucune). `temporary`:
    /// `_TEMPORARY_` — slots hors-PDV (jamais en sortie). `special`:
    /// `_NUMERIC_`/`_CHARACTER_`/`_ALL_` comme liste de variables.
    Array {
        name: String,
        dims: Option<Vec<usize>>,
        char_len: Option<usize>,
        vars: Vec<String>,
        initial: Vec<Expr>,
        temporary: bool,
        special: Option<ArraySpecial>,
    },
    /// `arr{i} = expr;` / `arr[i] = expr;` / `arr(i) = expr;` /
    /// `arr{i,j} = expr;` — assignation à un élément d'array. Pour la forme
    /// à parenthèses, le nom est validé array à la COMPILATION. `indices`
    /// porte un ou plusieurs indices (un par dimension, ou un seul linéaire).
    AssignIndexed {
        array: String,
        indices: Vec<Expr>,
        expr: Expr,
    },
    /// `call <name>(args);` — appel d'une CALL routine (M11.5). Pour v1,
    /// seule `CALL SYMPUT` est exécutée (pont DATA step → table macro) ;
    /// les autres routines parsent mais produisent une erreur runtime
    /// « not yet implemented ». Le nom est conservé tel quel (résolu en
    /// MAJUSCULES à l'exécution). Ce statement est parsé dans les DEUX
    /// builds (aucun test/fixture existant n'emploie `call`).
    CallRoutine { name: String, args: Vec<Expr> },
    /// `infile <source> [options];` (M14) — déclare la source de lecture
    /// texte de l'étape et ses options. Un seul INFILE par étape (un second
    /// → erreur de compilation).
    Infile {
        source: InfileSource,
        options: InfileOptions,
    },
    /// `input <items>;` (M14) — spécifie comment découper chaque
    /// enregistrement lu en variables du PDV. La source est l'INFILE
    /// courant, ou le bloc DATALINES inline si aucun INFILE n'a été déclaré.
    Input(Vec<InputItem>),
    /// `datalines;` / `cards;` (M14) — le bloc verbatim capturé par le lexer.
    /// Toujours le DERNIER statement exécutable de l'étape. Les lignes sont
    /// la source inline des INPUT de l'étape.
    Datalines(Vec<String>),
    /// `file <dest>;` (M14.2) — fixe la destination courante des PUT
    /// (fichier externe, LOG ou listing). Déclaratif à l'exécution : un FILE
    /// change la destination des PUT qui suivent.
    File { dest: PutDest },
    /// `put <items>;` (M14.2) — écrit une ligne de texte vers la destination
    /// courante (le LOG par défaut). Un PUT sans item relâche la ligne
    /// maintenue / écrit une ligne vide.
    Put(Vec<PutItem>),
    /// `select [(expr)]; when (...) stmt; ... otherwise stmt; end;` (M16.1).
    /// Deux formes :
    /// - **Sélecteur** : `selector = Some(expr)`. L'expression sélectrice est
    ///   évaluée UNE fois, puis chaque clause WHEN porte une liste de valeurs
    ///   (`WhenClause::values`) ; la clause s'applique si le sélecteur est
    ///   égal (sémantique `=` de SAS, via `sas_cmp`) à l'une d'elles.
    /// - **Booléen** : `selector = None`. Chaque clause WHEN porte UNE
    ///   condition (un seul élément dans `values`) évaluée en contexte
    ///   booléen ; la première vraie s'applique.
    ///
    /// Exécution : la PREMIÈRE clause qui correspond exécute son corps (UN
    /// statement, possiblement un `do; ... end;`) puis le SELECT se termine
    /// (pas de fall-through). Si aucune clause ne correspond, `otherwise`
    /// s'exécute s'il est présent, sinon erreur runtime (fidèle à SAS).
    Select {
        selector: Option<Expr>,
        whens: Vec<WhenClause>,
        otherwise: Option<Box<DsStmt>>,
    },
}

/// Une clause `when (v1, v2, ...) stmt;` d'un SELECT (M16.1). `values` porte
/// la liste de valeurs (forme sélecteur) ou l'unique condition (forme
/// booléenne). `body` est le statement exécuté quand la clause correspond.
#[derive(Debug, Clone, PartialEq)]
pub struct WhenClause {
    pub values: Vec<Expr>,
    pub body: Box<DsStmt>,
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
