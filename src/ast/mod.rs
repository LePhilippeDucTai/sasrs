//! AST for parsed SAS blocks. One `Block` = one executable unit (a global
//! statement, a DATA step, or a PROC step). Each PROC owns its own AST
//! struct, registered in `procs::registry`.

use crate::token::Span;
use crate::value::MissingKind;

mod dataset;
mod expr;
mod global;
mod io;

pub use dataset::DatasetOptions;
pub use dataset::DatasetRef;
pub use dataset::DatasetSpec;
pub use dataset::SetOptions;
pub use expr::ArraySpecial;
pub use expr::BinaryOp;
pub use expr::DoListItem;
pub use expr::Expr;
pub use expr::HashArg;
pub use expr::HashMethodCall;
pub use expr::UnaryOp;
pub use global::GlobalStmt;
pub use global::OdsAction;
pub use global::OdsGraphicsStmt;
pub use global::OdsGraphicsToggle;
pub use io::InfileOptions;
pub use io::InfileSource;
pub use io::InputItem;
pub use io::PutDest;
pub use io::PutItem;

/// Spec d'une variable dans un statement LENGTH : `$ n` (char) ou `n` (num).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LengthSpec {
    pub char: bool,
    pub len: usize,
}

/// Un item du statement ATTRIB : un groupe de variables et les attributs
/// déclarés. `format`/`informat`/`label` sont optionnels ; `length` est
/// conservé pour compatibilité mais non appliqué en M4 (voir parser).
#[derive(Debug, Clone, PartialEq)]
pub struct AttribItem {
    pub vars: Vec<String>,
    pub format: Option<String>,
    pub informat: Option<String>,
    pub label: Option<String>,
    pub length: Option<LengthSpec>,
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
    /// `end=`/`nobs=`/`point=`) sont portées par `options`. `specs` vide =
    /// `set;` nu (M40.2) : re-référence `_LAST_` (résolu à la compilation).
    Set {
        specs: Vec<DatasetSpec>,
        options: SetOptions,
        /// N° de SITE DE LECTURE (M40.2) : chaque statement SET de l'étape
        /// est un site indépendant (curseur, END=, comptes de lignes
        /// propres). Posé par la compilation (`stamp_set_sites`, pré-ordre
        /// textuel) ; le parser laisse 0. Relie le `DsStmt::Set` exécuté à
        /// son `InputData` (site 0 = `StepProgram::input`, sites suivants =
        /// `StepProgram::extra_inputs[site-1]`).
        site: usize,
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
    /// `where expr;` standalone (M40.3) — filtre PRÉ-CHARGEMENT appliqué à
    /// TOUS les datasets lus par SET/MERGE : même effet qu'un `WHERE=()`
    /// posé sur chaque dataset d'entrée (les obs rejetées n'entrent jamais
    /// au PDV, `_N_` ne compte que les retenues, FIRST./LAST. sur le flux
    /// filtré). Un `WHERE=()` de dataset REMPLACE le statement pour CE
    /// dataset (règle SAS : l'option gagne, pas de cumul). Plusieurs WHERE
    /// statements : le dernier gagne (NOTE « WHERE clause has been
    /// replaced. »). Résolu en fin de compilation (`build_input`) — le
    /// statement lui-même est un marqueur no-op à l'exécution.
    Where(Expr),
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
    Sum {
        var: String,
        expr: Expr,
    },
    /// `length v1 v2 $ 20 v3 5;`
    Length(Vec<(String, LengthSpec)>),
    /// `format weight height 8.2 name $char10.;` (M4) — chaque groupe est
    /// une liste de variables suivie d'un token de format. Déclaratif :
    /// associe un format aux variables (appliqué à la finalisation du PDV /
    /// par PROC PRINT) ; aucun effet à l'exécution.
    Format(Vec<(Vec<String>, String)>),
    /// `informat d date9. name $10.;` (M40.3) — même grammaire que FORMAT :
    /// chaque groupe est une liste de variables suivie d'un token
    /// d'informat. Déclaratif : associe un informat PAR DÉFAUT aux
    /// variables, utilisé par l'INPUT en mode liste quand l'item n'a pas
    /// d'informat explicite (lecture « modified list input », comme `:inf.`).
    /// La fonction INPUT(), elle, porte son informat en argument et n'est
    /// pas concernée.
    Informat(Vec<(Vec<String>, String)>),
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
    CallRoutine {
        name: String,
        args: Vec<Expr>,
    },
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
    File {
        dest: PutDest,
    },
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
    /// `update master[(where=(...))] transaction key=k1 k2;` (M16.5) — fusion
    /// maître/transaction. Le maître est lu séquentiellement ; pour chaque obs
    /// maître, la transaction correspondante (par clé `key_vars`) est
    /// superposée (seules les valeurs NON MANQUANTES écrasent ; les variables
    /// clé ne sont jamais écrasées). L'obs maître mise à jour est sortie. Un
    /// statement BY optionnel restreint la fusion aux groupes BY. Seules les
    /// options `(where=(...))` du maître sont portées par `master_where` ;
    /// `key_vars` est la liste (non vide) des variables de clé.
    Update {
        master: DatasetRef,
        master_where: Option<Expr>,
        transaction: DatasetRef,
        key_vars: Vec<String>,
    },
    /// `modify dataset key=k1 k2;` (M16.5) — modification EN PLACE. Le dataset
    /// est lu, ses variables peuvent être modifiées par assignation, puis il
    /// est RÉÉCRIT (même table) avec les valeurs modifiées. Pas d'output
    /// implicite (les valeurs modifiées par MODIFY sont finales). Supporte
    /// `point=`/`nobs=` comme SET pour l'accès direct ; OUTPUT n'est pas
    /// autorisé (→ erreur). `key_vars` peut être vide (lecture séquentielle).
    Modify {
        dataset: DatasetRef,
        key_vars: Vec<String>,
        point: Option<String>,
        nobs: Option<String>,
    },
    /// `label_name: <statement>` (M16.6) — un statement étiqueté. Le nom
    /// d'étiquette est une cible compile-time pour `GOTO`/`LINK`. `stmt` est le
    /// statement réellement exécuté (un seul ; pour plusieurs, utiliser
    /// `do; ... end;`). Les étiquettes sont lexicalement portées par l'étape
    /// DATA et résolues à la compilation.
    Labeled {
        name: String,
        stmt: Box<DsStmt>,
    },
    /// `goto label;` / `go to label;` (M16.6) — saut INCONDITIONNEL vers le
    /// statement étiqueté `label` (au niveau supérieur de l'étape). Termine les
    /// boucles DO englobantes. Étiquette inconnue → erreur de compilation.
    Goto(String),
    /// `link label;` (M16.6) — appel de sous-routine : exécute le code à partir
    /// du statement étiqueté `label` jusqu'au prochain `RETURN` (ou la fin de
    /// l'étape), puis reprend juste après le `LINK`. Imbrication autorisée
    /// (pile d'adresses de retour). Étiquette inconnue → erreur de compilation.
    Link(String),
    /// `return;` (M16.6) — retour de la sous-routine `LINK` courante (dépile
    /// l'adresse de retour). Sans `LINK` actif, RETURN termine l'itération
    /// courante (output implicite puis itération suivante), comme en SAS.
    Return,
    /// `declare hash h(opt:val, ...);` / `dcl hash h();` (M17.1) — crée un
    /// objet hash nommé `name`. Les options sont des paires `clé:valeur`
    /// (`ordered:'yes'`, `duplicate:'replace'`, `multidata:'yes'`,
    /// `dataset:'lib.table'`), séparées par des virgules ; chaque valeur est
    /// un littéral chaîne ou numérique normalisé en `String`. L'objet est
    /// défini ensuite par les méthodes `defineKey`/`defineData`/`defineDone`
    /// (M17.1) puis manipulé par find/add/etc. (M17.2).
    DeclareHash {
        name: String,
        options: Vec<(String, String)>,
    },
    /// `h.method(args);` (M17.1/M17.2) — appel d'une méthode d'un objet hash
    /// en FORME STATEMENT (code retour ignoré). `object` est le nom de l'objet
    /// hash (résolu en MAJUSCULES) ; `method` le nom de la méthode (résolue
    /// insensible à la casse) ; `args` ses arguments (positionnels ou nommés).
    /// La forme expression (`rc = h.find();`) passe par `Expr::HashMethod`.
    /// Boxé (partage `HashMethodCall` avec la forme expression).
    HashMethod(Box<HashMethodCall>),
    /// `declare hiter hi('h');` / `dcl hiter hi('h');` (M17.2) — déclare un
    /// itérateur lié à l'objet hash nommé dans la chaîne `hash_name`. Les
    /// méthodes `first`/`next`/`last`/`prev` parcourent l'objet (ordre `ordered:`
    /// ou ordre d'insertion) et copient la clé+les données de l'entrée courante
    /// dans le PDV.
    DeclareHiter {
        name: String,
        hash_name: String,
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
