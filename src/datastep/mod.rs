//! Compilation de l'étape DATA : AST → `StepProgram` exécutable.
//!
//! # Plan du fichier — voir PLAN.md  (difficulté : ÉLEVÉE — sémantique SAS)
//!
//! SAS exécute une étape DATA en deux phases ; ce module est la phase de
//! COMPILATION. Une passe sur l'AST construit :
//!
//! ## 1. Le PDV (Program Data Vector)
//! - Variables dans l'ORDRE DE PREMIÈRE RÉFÉRENCE textuelle (cet ordre
//!   définit l'ordre des colonnes en sortie !).
//! - `set lib.a` : lire le dataset (via `session.libs`, libref par défaut
//!   WORK) — ses variables entrent dans le PDV avec type/longueur/format
//!   de `VarMeta`, marquées `from_input=true` (elles ne sont PAS remises
//!   à missing à chaque itération). Logger les notes de coercition
//!   parquet (`LogWriter::forward`).
//! - Cible d'assignation : si absente du PDV, créer avec le type INFÉRÉ
//!   de l'expression (voir §3). Variable seulement RÉFÉRENCÉE (jamais
//!   assignée ni lue d'un input) : créer numérique + NOTE
//!   "Variable x is uninitialized." à l'exécution de la 1re itération.
//!
//! ## 2. Les sorties
//! Pour chaque dataset de `ast.outputs` : appliquer KEEP/DROP (statements
//! M1 ; options de dataset M2) → `kept_slots` = indices PDV dans l'ordre
//! PDV. KEEP et DROP simultanés : DROP gagne sur l'intersection (SAS
//! émet WARNING). Variables KEEP inexistantes → ERROR à la compilation
//! ("The variable x in the DROP, KEEP, or RENAME list has never been
//! referenced.").
//!
//! ## 3. Inférence de type d'une expression (compile-time, comme SAS)
//! - littéral num / opération arithmétique / comparaison / logique → Num
//! - littéral chaîne → Char(longueur du littéral)
//! - `Var` → type/longueur de la variable au PDV
//! - `||` → Char(somme des longueurs des opérandes)
//! - `Call` : table par fonction (upcase/lowcase/trim/strip/left →
//!   Char(longueur arg), substr → Char(longueur arg1), cat* → 200,
//!   put → largeur du format ; défaut → Num)
//! - La PREMIÈRE assignation fige type et longueur (redéfinir → la
//!   longueur d'origine reste, SAS tronque silencieusement).
//!
//! ## 4. has_explicit_output
//! Si AU MOINS UN `output;` apparaît dans l'étape, l'output implicite de
//! fin d'itération est désactivé (règle SAS).
//!
//! Erreur de compilation → l'exécuteur loggue ERROR + NOTE "The SAS
//! System stopped processing this step because of errors." et n'exécute
//! pas (mais la session continue).
//!
//! ## Choix d'implémentation
//! - Ordre de première référence d'une assignation : la CIBLE entre au PDV
//!   avant les variables de son expression (ordre textuel gauche→droite).
//! - Opérande numérique de `||` : contribue 12 à la longueur inférée
//!   (conversion implicite BEST12. comme SAS).
//! - `put(x, fmt)` : largeur = chiffres finaux du nom de format si
//!   disponibles, sinon 200 (le parser M1 ne sait pas encore produire un
//!   littéral de format ; best-effort documenté).
//! - Un seul statement SET par étape (le second → erreur "not yet
//!   implemented"), mais un SET peut lister PLUSIEURS datasets (M3) : le
//!   PDV reçoit l'UNION de leurs variables en ordre de première
//!   apparition ; une variable présente avec des types incompatibles →
//!   ERROR "Variable X has been defined as both character and numeric.".
//!   Le statement BY est résolu en fin de compilation (`build_input`) :
//!   chaque clé doit exister dans CHAQUE dataset du SET, et toute
//!   référence FIRST.x/LAST.x exige que x soit une clé BY. FIRST./LAST.
//!   ne créent jamais de slot PDV (comme _N_/_ERROR_).

pub mod eval;
pub mod exec;
pub mod fastpath;
pub mod functions;
pub mod pdv;

use crate::ast::{BinaryOp, DataStepAst, DatasetOptions, DatasetSpec, DoListItem, DsStmt, Expr};
use crate::error::{Result, SasError};
use crate::missing::num_to_value;
use crate::session::Session;
use crate::value::{Value, VarType};
use pdv::{Pdv, PdvVar};
use std::collections::{HashMap, HashSet};

/// Un dataset matérialisé d'un statement SET.
pub struct InputDataset {
    /// "WORK.B" pour les NOTEs du log.
    pub display: String,
    /// Colonnes décodées en `Value` (via `missing::num_to_value`), une
    /// seule passe de downcast par colonne — jamais de get_row. Seules les
    /// colonnes retenues par KEEP=/DROP= sont présentes ; une colonne
    /// renommée par RENAME= est au PDV sous son NOUVEAU nom.
    pub columns: Vec<Vec<Value>>,
    /// Slot PDV de chaque colonne (parallèle à `columns`).
    pub var_slots: Vec<usize>,
    pub n_rows: usize,
    /// `WHERE=` du SET : sans BY, évalué à l'EXÉCUTION après chargement de
    /// chaque ligne dans le PDV ; une ligne qui échoue est sautée SANS
    /// exécuter le reste de l'itération et ne compte PAS dans les
    /// observations lues (comme la NOTE SAS "There were N observations
    /// read"). Avec BY, le filtre est PRÉ-APPLIQUÉ par l'exécuteur avant
    /// l'interclassement (mêmes règles). NB : comme l'évaluation se fait
    /// sur le PDV, un WHERE= combiné à RENAME= référence les NOUVEAUX noms
    /// (divergence documentée — SAS applique WHERE= avant RENAME= en
    /// entrée).
    pub where_: Option<Expr>,
    /// Index dans `columns` de chaque variable BY (parallèle à
    /// `InputData::by`) ; vide sans BY. Chaque variable BY doit exister
    /// dans CHAQUE dataset du SET (vérifié à la compilation).
    pub by_cols: Vec<usize>,
}

/// Une clé du statement BY, résolue à la compilation.
pub struct ByVar {
    /// Nom canonique MAJUSCULE (sert les variables FIRST.x / LAST.x).
    pub name: String,
    /// Slot PDV de la variable.
    pub slot: usize,
    pub descending: bool,
}

/// Données d'entrée matérialisées du statement SET (M3 : un ou plusieurs
/// datasets, BY optionnel).
pub struct InputData {
    /// Les datasets, dans l'ordre du statement SET. Sans BY, ils sont lus
    /// en CONCATÉNATION (le premier en entier, puis le suivant) ; avec BY,
    /// en INTERCLASSEMENT par clés croissantes (cf. exec.rs).
    pub datasets: Vec<InputDataset>,
    /// Clés du BY (vide = pas de BY).
    pub by: Vec<ByVar>,
    /// MERGE (M3) : `true` = match-merge SAS par BY (au lieu de SET).
    /// L'exécuteur pré-calcule la séquence des obs de sortie groupe par
    /// groupe (cf. exec.rs).
    pub merge: bool,
    /// Variables IN= du MERGE : `(nom UPPERCASE, index dataset)`. Servies
    /// par `EvalCtx::in_flags` (jamais de slot PDV, comme FIRST./LAST.).
    pub in_flags: Vec<(String, usize)>,
    /// END= (M16.4) : nom UPPERCASE de la variable automatique temporaire
    /// (0 pendant l'itération, 1 après lecture de la DERNIÈRE obs du DERNIER
    /// dataset). Servie par `EvalCtx::end_flag`, jamais écrite en sortie.
    pub end_var: Option<String>,
    /// NOBS= (M16.4) : slot PDV de la variable numérique affectée AVANT la
    /// boucle au nombre TOTAL d'observations (somme des datasets du SET).
    pub nobs_slot: Option<usize>,
    /// POINT= (M16.4) : slot PDV de la variable d'index 1-based. Sa présence
    /// DÉSACTIVE la boucle implicite et l'output implicite : chaque SET lit
    /// l'obs à l'index courant (erreur si missing/invalide/hors bornes).
    pub point_slot: Option<usize>,
}

/// Item INPUT compilé (M14) : un item AST dont les noms de variable sont
/// résolus en slots PDV et les informats en `FormatSpec`.
#[derive(Clone)]
pub enum InputAction {
    /// Lire une variable. `slot` = slot PDV ; `is_char` = type cible ;
    /// `cols` = colonnes 1-based inclusives (mode colonne) ; `informat` =
    /// `FormatSpec` (mode formaté) ; `list_modifier` = informat appliqué en
    /// mode liste.
    Var {
        slot: usize,
        is_char: bool,
        cols: Option<(usize, usize)>,
        informat: Option<crate::formats::FormatSpec>,
        list_modifier: bool,
    },
    /// `@n` : pointeur de colonne absolu (1-based).
    ColumnPointer(usize),
    /// `+n` : avance relative du curseur.
    SkipColumns(usize),
    /// `/` : ligne d'entrée suivante.
    NextLine,
    /// `@` final : maintien de l'enregistrement pour le prochain INPUT.
    HoldLine,
    /// `@@` final : maintien à travers les itérations.
    HoldLineDouble,
}

/// Options d'exécution d'une lecture texte (M14), reprises de l'INFILE.
pub struct TextOptions {
    pub delimiter: Option<String>,
    pub dsd: bool,
    pub firstobs: usize,
    pub obs: Option<usize>,
    /// Comportement en cas de ligne trop courte : 0 = défaut (passe à la
    /// ligne suivante en mode liste), 1 = MISSOVER, 2 = TRUNCOVER, 3 =
    /// STOPOVER.
    pub short: ShortMode,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ShortMode {
    Default,
    Missover,
    Truncover,
    Stopover,
}

/// Source d'entrée texte compilée (M14) : lignes brutes + spécification
/// INPUT résolue. Parallèle à `InputData` (le chemin SET).
pub struct TextInput {
    /// "the infile 'path'" pour la NOTE du log (fichier externe seulement).
    pub display: String,
    /// Lignes brutes (DATALINES inline ou contenu du fichier).
    pub lines: Vec<String>,
    pub options: TextOptions,
    /// `true` si la source est un FICHIER externe (`infile 'path'`). Pour les
    /// données instream DATALINES/CARDS, SAS n'émet PAS de NOTE "N records
    /// were read from the infile ..." (réservée aux fichiers physiques) :
    /// l'exécuteur s'en sert pour ne l'émettre que dans le cas fichier.
    pub is_file: bool,
}

/// Une sortie : où écrire et quels slots du PDV.
pub struct OutputSpec {
    pub libref: String,
    pub table: String,
    /// "WORK.A" pour les NOTEs.
    pub display: String,
    /// Slots PDV conservés, dans l'ordre PDV (= ordre des colonnes).
    /// Combinaison (intersection) des statements KEEP/DROP et des options
    /// de dataset KEEP=/DROP= de CETTE sortie.
    pub kept_slots: Vec<usize>,
    /// Nom d'écriture de chaque slot conservé (parallèle à `kept_slots`) :
    /// le nom PDV, ou le nouveau nom si RENAME= s'applique.
    pub out_names: Vec<String>,
}

/// Définition compilée d'un array (M16.2). `slots` = slots PDV des
/// éléments dans l'ordre row-major ; `dims` = bornes supérieures de chaque
/// dimension (borne inférieure = 1, comme SAS). Un array 1-D a `dims` de
/// longueur 1 ; le produit des `dims` égale toujours `slots.len()`.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrayDef {
    pub slots: Vec<usize>,
    pub dims: Vec<usize>,
}

impl ArrayDef {
    /// Traduit un sous-script multi-dimensionnel (1-based par dimension) en
    /// index linéaire 0-based row-major. `None` si un indice est hors
    /// bornes (ou si le nombre d'indices ne correspond ni à `dims.len()`
    /// ni à 1 — accès linéaire). `indices` doit déjà être arrondi/entier.
    pub fn linear_index(&self, indices: &[i64]) -> Option<usize> {
        if indices.len() == 1 && self.dims.len() != 1 {
            // Accès linéaire sur array multi-dim : `arr{n}` → 1..=total.
            let n = indices[0];
            if n >= 1 && (n as usize) <= self.slots.len() {
                return Some(n as usize - 1);
            }
            return None;
        }
        if indices.len() != self.dims.len() {
            return None;
        }
        let mut linear: usize = 0;
        for (k, &idx) in indices.iter().enumerate() {
            let bound = self.dims[k];
            if idx < 1 || (idx as usize) > bound {
                return None;
            }
            linear = linear * bound + (idx as usize - 1);
        }
        Some(linear)
    }
}

/// Données d'entrée compilées d'un statement UPDATE (M16.5). Le maître et la
/// transaction sont matérialisés en colonnes décodées (comme `InputDataset`),
/// avec le slot PDV de chaque colonne. Les variables clé (`key_slots`) servent
/// l'appariement. `master_where` est filtré à l'exécution (sur le PDV chargé,
/// comme SET WHERE=). `by` (optionnel) restreint la fusion aux groupes BY.
pub struct UpdateData {
    /// Le maître, lu séquentiellement (pilote l'itération).
    pub master: InputDataset,
    /// La transaction, indexée par clé (recherche par `key_slots`).
    pub transaction: InputDataset,
    /// Slots PDV des variables clé (ordre du KEY=). Ces slots ne sont jamais
    /// écrasés par la transaction.
    pub key_slots: Vec<usize>,
    /// Slots PDV des variables de la transaction qui peuvent superposer le
    /// maître (toutes SAUF les clés). Une valeur transaction MANQUANTE ne
    /// superpose pas (sémantique « missing = no update »).
    pub overlay_slots: Vec<usize>,
    /// WHERE= du maître, évalué à l'exécution sur le PDV chargé.
    pub master_where: Option<Expr>,
    /// Clés BY (vide = pas de BY) — déclaratif, sert FIRST./LAST.
    pub by: Vec<ByVar>,
}

/// Données d'entrée compilées d'un statement MODIFY (M16.5). Le dataset est
/// matérialisé et RÉÉCRIT en place après l'étape. `key_slots` peut être vide
/// (lecture séquentielle). `point_slot`/`nobs_slot` reprennent la sémantique
/// d'accès direct du SET.
pub struct ModifyData {
    /// Le dataset à modifier (libref/table pour la réécriture).
    pub libref: String,
    pub table: String,
    /// "WORK.A" pour les NOTEs.
    pub display: String,
    /// Le dataset matérialisé en colonnes décodées + slots PDV.
    pub data: InputDataset,
    /// Slots PDV des variables clé (vide = lecture séquentielle).
    pub key_slots: Vec<usize>,
    /// POINT= : slot PDV de l'index 1-based (accès direct, comme SET POINT=).
    pub point_slot: Option<usize>,
    /// NOBS= : slot PDV affecté avant la boucle au nombre d'observations.
    pub nobs_slot: Option<usize>,
    /// Métadonnées de sortie (VarMeta) de CHAQUE slot PDV de `data.var_slots`,
    /// dans l'ordre, pour réécrire le dataset à l'identique (mêmes colonnes).
    pub out_vars: Vec<crate::dataset::VarMeta>,
}

pub struct StepProgram {
    pub pdv: Pdv,
    pub stmts: Vec<crate::ast::DsStmt>,
    pub input: Option<InputData>,
    /// Entrée UPDATE (M16.5), exclusive de `input`/`text_input`/`modify`.
    pub update: Option<UpdateData>,
    /// Entrée MODIFY (M16.5), exclusive de `input`/`text_input`/`update`.
    pub modify: Option<ModifyData>,
    /// Source d'entrée TEXTE (M14 : INFILE/INPUT/DATALINES), parallèle à
    /// `input` (SET). Une étape ne peut avoir QUE l'un des deux.
    pub text_input: Option<TextInput>,
    pub outputs: Vec<OutputSpec>,
    pub has_explicit_output: bool,
    /// Noms (casse de première référence, ordre PDV) des variables jamais
    /// assignées ni lues d'un input : l'exécuteur émet la NOTE
    /// "Variable x is uninitialized." à la première itération.
    pub uninitialized: Vec<String>,
    /// Valeurs initiales (slot, valeur) issues de RETAIN avec init et des
    /// sum statements (0) : l'exécuteur les applique via `pdv.set` AVANT la
    /// première itération (la troncature char s'applique donc normalement).
    /// Appliquées dans l'ordre — une entrée ultérieure pour le même slot
    /// gagne (cas `n + 1; retain n 100;` : le RETAIN l'emporte).
    pub initial_values: Vec<(usize, Value)>,
    /// Arrays : nom UPPERCASE → définition (slots + dimensions). Passé tel
    /// quel à l'EvalCtx par l'exécuteur.
    pub arrays: HashMap<String, ArrayDef>,
    /// Libellés déclarés (LABEL/ATTRIB) : nom UPPERCASE → libellé.
    /// Appliqués aux `VarMeta` de sortie par l'exécuteur.
    pub labels: HashMap<String, String>,
}

pub fn compile(ast: &DataStepAst, session: &mut Session) -> Result<StepProgram> {
    let mut c = Compiler {
        pdv: Pdv::new(),
        session,
        input_datasets: Vec::new(),
        seen_set: false,
        set_options: crate::ast::SetOptions::default(),
        seen_merge: false,
        in_flags: Vec::new(),
        by: None,
        first_last_refs: Vec::new(),
        keeps: Vec::new(),
        drops: Vec::new(),
        output_displays: ast.outputs.iter().map(|s| s.display()).collect(),
        assigned: HashSet::new(),
        has_explicit_output: false,
        retain_all: false,
        retain_pending: Vec::new(),
        retained_slots: HashSet::new(),
        initial_values: Vec::new(),
        arrays: HashMap::new(),
        labels: HashMap::new(),
        formats: HashMap::new(),
        infile: None,
        datalines: None,
        seen_input: false,
        do_over_arrays: HashSet::new(),
        update: None,
        modify: None,
    };
    for stmt in &ast.stmts {
        c.walk_stmt(stmt)?;
    }
    // FORMAT/ATTRIB format= : appliqués au PDV maintenant que TOUTES les
    // variables y sont entrées (l'ordre déclaration/référence n'importe
    // plus). Variable inconnue → ignorée (SIMPLIFICATION M4 documentée).
    let formats = std::mem::take(&mut c.formats);
    for (name, token) in &formats {
        if let Some(slot) = c.pdv.slot(name) {
            c.pdv.set_format(slot, token.clone());
        }
    }

    // RETAIN sans valeur initiale — SIMPLIFICATION M2 ASSUMÉE : en vrai
    // SAS, `retain x;` ne fige PAS le type — la variable le prend à sa
    // prochaine référence. Pour approcher ça sans bouleverser la passe
    // unique, le statement n'a fait que mémoriser le nom ; ICI (fin de
    // compilation) on applique le flag `retained` à la variable, qui doit
    // alors exister (créée par une autre référence). Sinon on la crée Num
    // + uninitialized — elle arrive donc en FIN d'ordre PDV (divergence
    // mineure assumée par rapport à l'ordre de première référence SAS).
    let pending = std::mem::take(&mut c.retain_pending);
    for name in &pending {
        let slot = match c.pdv.slot(name) {
            Some(slot) => slot,
            None => c.add_var(name, VarType::Num, 8),
        };
        c.retained_slots.insert(slot);
    }
    // `retain;` seul — SIMPLIFICATION M2 : retient TOUT le PDV (en vrai
    // SAS, seulement ce qui est connu au point du statement).
    if c.retain_all {
        c.retained_slots.extend(0..c.pdv.vars().len());
    }
    // Le PDV ne permet pas de modifier une variable existante (première
    // référence fige tout, et `pdv.rs` n'expose pas de mutateur) : on le
    // reconstruit à l'identique en appliquant les flags `retained`. Les
    // slots sont préservés (même ordre d'insertion) et aucune valeur n'a
    // encore été posée à la compilation.
    if !c.retained_slots.is_empty() {
        c.pdv = rebuild_with_retained(&c.pdv, &c.retained_slots);
    }

    let input = c.build_input()?;
    let update = c.build_update()?;
    let modify = c.build_modify()?;
    let text_input = c.build_text_input()?;
    // Une étape ne peut pas mélanger plusieurs sources d'entrée concurrentes.
    let n_sources = [
        input.is_some(),
        update.is_some(),
        modify.is_some(),
        text_input.is_some(),
    ]
    .iter()
    .filter(|b| **b)
    .count();
    if n_sources > 1 {
        return Err(SasError::runtime(
            "Mixing SET/UPDATE/MODIFY with INFILE/INPUT in the same step is not yet implemented.",
        ));
    }
    // MODIFY interdit l'OUTPUT explicite (les valeurs sont écrites en place).
    if modify.is_some() && c.has_explicit_output {
        return Err(SasError::runtime(
            "The OUTPUT statement is not allowed with the MODIFY statement.",
        ));
    }
    let outputs = c.resolve_outputs(&ast.outputs)?;
    let uninitialized = c
        .pdv
        .vars()
        .iter()
        .filter(|v| !v.from_input && !v.temporary && !c.assigned.contains(&v.name.to_uppercase()))
        .map(|v| v.name.clone())
        .collect();
    Ok(StepProgram {
        pdv: c.pdv,
        stmts: ast.stmts.clone(),
        input,
        update,
        modify,
        text_input,
        outputs,
        has_explicit_output: c.has_explicit_output,
        uninitialized,
        initial_values: c.initial_values,
        arrays: c.arrays,
        labels: c.labels,
    })
}

/// Reconstruit le PDV en marquant `retained` les slots donnés (les autres
/// attributs sont copiés tels quels ; les indices de slots sont stables).
fn rebuild_with_retained(pdv: &Pdv, retained: &HashSet<usize>) -> Pdv {
    let mut rebuilt = Pdv::new();
    for (i, v) in pdv.vars().iter().enumerate() {
        let slot = rebuilt.add_var(PdvVar {
            name: v.name.clone(),
            ty: v.ty,
            length: v.length,
            retained: v.retained || retained.contains(&i),
            from_input: v.from_input,
            format: v.format.clone(),
            temporary: v.temporary,
        });
        debug_assert_eq!(slot, i, "rebuild must preserve slot indices");
    }
    rebuilt
}

/// Évalue une valeur initiale CONSTANTE d'un statement ARRAY (`(1, 2, 'x')`)
/// à la compilation. N'accepte que des littéraux (num, chaîne, missing) et
/// `-num`. La valeur est coercée vers le type de l'array (num→char =
/// formaté BEST ; char→num = parse).
fn const_eval_initial(expr: &crate::ast::Expr, ty: VarType) -> Result<Value> {
    use crate::ast::{Expr, UnaryOp};
    let v = match expr {
        Expr::Num(n) => Value::Num(*n),
        Expr::Str(s) => Value::Char(s.clone()),
        Expr::Missing(k) => Value::Missing(*k),
        Expr::Unary {
            op: UnaryOp::Minus,
            expr,
        } => match const_eval_initial(expr, VarType::Num)? {
            Value::Num(n) => Value::Num(-n),
            other => other,
        },
        _ => {
            return Err(SasError::runtime(
                "Array initial values must be constants (numbers or quoted strings).",
            ));
        }
    };
    // Coercition vers le type déclaré de l'array.
    Ok(match (ty, &v) {
        (VarType::Char, Value::Num(n)) => Value::Char(crate::value::format_best(*n, 12)),
        (VarType::Num, Value::Char(s)) => match s.trim().parse::<f64>() {
            Ok(n) => Value::Num(n),
            Err(_) => Value::missing(),
        },
        _ => v,
    })
}

struct Compiler<'a> {
    pdv: Pdv,
    session: &'a mut Session,
    /// Datasets du SET, dans l'ordre du statement (vide = pas de SET).
    input_datasets: Vec<InputDataset>,
    /// Un statement SET a déjà été rencontré (un second → erreur).
    seen_set: bool,
    /// Options de niveau statement du SET (M16.4 : end=/nobs=/point=),
    /// résolues en `build_input` (slots PDV / variable automatique).
    set_options: crate::ast::SetOptions,
    /// Un statement MERGE a déjà été rencontré (M3). Un second SET/MERGE
    /// dans la même étape → erreur "... is not allowed after ...".
    seen_merge: bool,
    /// Variables IN= des datasets d'un MERGE : `(nom UPPERCASE, index
    /// dataset)`. Jamais de slot PDV (servies par EvalCtx, comme FIRST.).
    in_flags: Vec<(String, usize)>,
    /// Items du statement BY `(nom, descending)` ; résolus en `ByVar` en
    /// fin de compilation (un second BY écrase le premier).
    by: Option<Vec<(String, bool)>>,
    /// Noms canoniques "FIRST.X"/"LAST.X" référencés dans les expressions :
    /// validés contre les variables BY en fin de compilation. Ils ne
    /// créent JAMAIS de slot PDV (comme _N_/_ERROR_).
    first_last_refs: Vec<String>,
    keeps: Vec<String>,
    drops: Vec<String>,
    /// Displays ("WORK.A") des sorties du statement DATA, pour valider les
    /// OUTPUT ciblés.
    output_displays: Vec<String>,
    /// Noms (uppercase) ayant au moins une assignation dans l'étape.
    assigned: HashSet<String>,
    has_explicit_output: bool,
    /// `retain;` sans liste rencontré : tout le PDV sera retenu.
    retain_all: bool,
    /// Noms d'un RETAIN SANS init : flag appliqué en fin de compilation.
    retain_pending: Vec<String>,
    /// Slots à marquer `retained` en fin de compilation (RETAIN avec init,
    /// sum statements, RETAIN sans init résolus).
    retained_slots: HashSet<usize>,
    /// Valeurs initiales (slot, valeur) appliquées avant la 1re itération.
    initial_values: Vec<(usize, Value)>,
    /// Arrays déclarés : nom UPPERCASE → définition (slots + dimensions).
    arrays: HashMap<String, ArrayDef>,
    /// Libellés déclarés (LABEL/ATTRIB) : nom UPPERCASE → libellé. Une
    /// déclaration ultérieure pour la même variable écrase la précédente.
    labels: HashMap<String, String>,
    /// Formats déclarés (FORMAT/ATTRIB) : nom UPPERCASE → token de format.
    /// Appliqués au PDV en fin de compilation (indépendamment de l'ordre des
    /// statements) ; l'emportent sur le format hérité de l'input.
    formats: HashMap<String, String>,
    /// INFILE rencontré (M14) : source + options. `None` = pas d'INFILE
    /// explicite (DATALINES inline implicite si présent).
    infile: Option<(crate::ast::InfileSource, crate::ast::InfileOptions)>,
    /// Lignes du bloc DATALINES inline (M14).
    datalines: Option<Vec<String>>,
    /// Un statement INPUT a déjà été vu (un second → erreur).
    seen_input: bool,
    /// Noms d'arrays (UPPERCASE) dont un `DO OVER` est actif au point de
    /// compilation courant : une référence NUE à ce nom y désigne l'élément
    /// courant (lecture/écriture), pas une variable illégale (M16.3).
    do_over_arrays: HashSet<String>,
    /// UPDATE compilé (M16.5), résolu dans `walk_stmt` (un seul par étape).
    update: Option<PendingUpdate>,
    /// MODIFY compilé (M16.5), résolu dans `walk_stmt` (un seul par étape).
    modify: Option<PendingModify>,
}

/// État intermédiaire d'un UPDATE pendant la compilation : les datasets sont
/// matérialisés tout de suite (entrée au PDV) ; les slots clé/overlay et le BY
/// sont résolus en fin de compilation (`build_update`).
struct PendingUpdate {
    master: InputDataset,
    transaction: InputDataset,
    master_display: String,
    key_names: Vec<String>,
    master_where: Option<Expr>,
}

/// État intermédiaire d'un MODIFY pendant la compilation.
struct PendingModify {
    libref: String,
    table: String,
    display: String,
    data: InputDataset,
    out_vars: Vec<crate::dataset::VarMeta>,
    key_names: Vec<String>,
    point: Option<String>,
    nobs: Option<String>,
}

impl Compiler<'_> {
    fn walk_stmt(&mut self, stmt: &DsStmt) -> Result<()> {
        match stmt {
            DsStmt::Set { specs, options } => {
                if self.seen_set {
                    return Err(SasError::runtime(
                        "Multiple SET statements are not yet implemented.",
                    ));
                }
                if self.seen_merge {
                    return Err(SasError::runtime(
                        "A SET statement is not allowed after a MERGE statement.",
                    ));
                }
                self.seen_set = true;
                for spec in specs {
                    // `in=` n'est pas valide sur un SET (MERGE seulement).
                    if spec.options.in_.is_some() {
                        return Err(SasError::runtime(
                            "The IN= data set option is only valid on a MERGE statement.",
                        ));
                    }
                    self.compile_set(spec)?;
                }
                // Options de niveau statement (M16.4). NOBS= crée (ou réutilise)
                // une variable numérique au PDV maintenant (elle est affectée
                // AVANT la boucle ⇒ doit exister) et la marque retenue (sa
                // valeur ne doit pas être remise à missing à chaque itération) ;
                // POINT= référence une variable numérique que l'utilisateur
                // pilote (créée ici si absente, comme une variable assignée).
                // END= ne crée JAMAIS de slot (variable automatique temporaire,
                // servie par EvalCtx, jamais écrite en sortie).
                if let Some(name) = &options.nobs {
                    let slot = match self.pdv.slot(name) {
                        Some(s) => s,
                        None => self.add_var(name, VarType::Num, 8),
                    };
                    self.retained_slots.insert(slot);
                    self.assigned.insert(name.to_uppercase());
                }
                if let Some(name) = &options.point {
                    if self.pdv.slot(name).is_none() {
                        self.add_var(name, VarType::Num, 8);
                    }
                    // La variable POINT= est pilotée par l'utilisateur : on la
                    // considère "assignée" (pas de NOTE "uninitialized").
                    self.assigned.insert(name.to_uppercase());
                }
                self.set_options = options.clone();
                Ok(())
            }
            // MERGE (M3) : comme SET multi-datasets mais en match-merge par
            // BY. Chaque dataset peut porter une option `in=`. Un SET/MERGE
            // a déjà été vu → erreur.
            DsStmt::Merge(specs) => {
                if self.seen_set || self.seen_merge {
                    return Err(SasError::runtime(
                        "A MERGE statement is not allowed after a SET or MERGE statement.",
                    ));
                }
                self.seen_merge = true;
                for spec in specs {
                    // L'index du dataset dans `input_datasets` AVANT le push.
                    let ds_index = self.input_datasets.len();
                    if let Some(nm) = &spec.options.in_ {
                        // Le nom IN= ne doit PAS entrer en collision avec une
                        // variable du PDV (c'est une automatique temporaire).
                        self.in_flags.push((nm.to_uppercase(), ds_index));
                    }
                    self.compile_set(spec)?;
                }
                Ok(())
            }
            // UPDATE (M16.5) : maître + transaction, fusion par KEY=. Comme
            // SET/MERGE, exclusif (un seul SET/MERGE/UPDATE/MODIFY par étape).
            DsStmt::Update {
                master,
                master_where,
                transaction,
                key_vars,
            } => {
                if self.seen_set || self.seen_merge || self.update.is_some() || self.modify.is_some()
                {
                    return Err(SasError::runtime(
                        "Only one SET, MERGE, UPDATE, or MODIFY statement is allowed per DATA step.",
                    ));
                }
                // Le maître entre au PDV en premier (ordre de référence), puis
                // la transaction (ses variables nouvelles s'ajoutent).
                let master_ds = self.materialize_input(master, &DatasetOptions::default())?;
                let transaction_ds =
                    self.materialize_input(transaction, &DatasetOptions::default())?;
                if let Some(w) = master_where {
                    self.validate_where_vars(w, &master.display())?;
                }
                self.update = Some(PendingUpdate {
                    master: master_ds,
                    transaction: transaction_ds,
                    master_display: master.display(),
                    key_names: key_vars.clone(),
                    master_where: master_where.clone(),
                });
                Ok(())
            }
            // MODIFY (M16.5) : un dataset, modification EN PLACE.
            DsStmt::Modify {
                dataset,
                key_vars,
                point,
                nobs,
            } => {
                if self.seen_set || self.seen_merge || self.update.is_some() || self.modify.is_some()
                {
                    return Err(SasError::runtime(
                        "Only one SET, MERGE, UPDATE, or MODIFY statement is allowed per DATA step.",
                    ));
                }
                let (data, out_vars) =
                    self.materialize_input_with_meta(dataset, &DatasetOptions::default())?;
                // NOBS= : variable numérique affectée AVANT la boucle (doit
                // exister, retenue). POINT= : pilotée par l'utilisateur.
                if let Some(name) = nobs {
                    let slot = match self.pdv.slot(name) {
                        Some(s) => s,
                        None => self.add_var(name, VarType::Num, 8),
                    };
                    self.retained_slots.insert(slot);
                    self.assigned.insert(name.to_uppercase());
                }
                if let Some(name) = point {
                    if self.pdv.slot(name).is_none() {
                        self.add_var(name, VarType::Num, 8);
                    }
                    self.assigned.insert(name.to_uppercase());
                }
                self.modify = Some(PendingModify {
                    libref: dataset.libref_or_work(),
                    table: dataset.name.clone(),
                    display: dataset.display(),
                    data,
                    out_vars,
                    key_names: key_vars.clone(),
                    point: point.clone(),
                    nobs: nobs.clone(),
                });
                Ok(())
            }
            // BY : purement déclaratif ici ; résolu en fin de compilation
            // (`build_input`). Les variables BY doivent venir des inputs —
            // on ne crée donc AUCUN slot ici.
            DsStmt::By(items) => {
                self.by = Some(items.clone());
                Ok(())
            }
            DsStmt::Assign { var, expr } => {
                let upper = var.to_uppercase();
                // `arr = e;` à l'intérieur d'un `DO OVER arr` : assignation à
                // l'élément courant (résolue à l'exécution) — ne crée PAS de
                // variable. Hors DO OVER, un nom d'array nu est illégal.
                if self.arrays.contains_key(&upper) {
                    if self.do_over_arrays.contains(&upper) {
                        self.assigned.insert(upper);
                        self.walk_expr(expr)?;
                        return Ok(());
                    }
                    return Err(SasError::runtime(format!(
                        "Illegal reference to the array {var}."
                    )));
                }
                // La cible entre au PDV en premier (ordre textuel), avec le
                // type inféré AVANT création des variables de l'expression
                // (les inconnues comptent comme Num, cohérent avec SAS).
                let (ty, length) = self.infer(expr);
                self.add_var(var, ty, length);
                self.assigned.insert(var.to_uppercase());
                self.walk_expr(expr)?;
                Ok(())
            }
            DsStmt::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.walk_expr(cond)?;
                self.walk_stmt(then_branch)?;
                if let Some(e) = else_branch {
                    self.walk_stmt(e)?;
                }
                Ok(())
            }
            DsStmt::SubsettingIf(cond) => self.walk_expr(cond),
            DsStmt::Block(stmts) => {
                for s in stmts {
                    self.walk_stmt(s)?;
                }
                Ok(())
            }
            DsStmt::DoLoop {
                index,
                to,
                by,
                while_,
                until,
                body,
            } => {
                // L'index entre au PDV au point du DO (ordre de première
                // référence) : Num 8, NON retenu, et il compte comme
                // assigné (pas de NOTE "uninitialized"). Puis les bornes
                // et conditions en ordre textuel, puis le corps.
                if let Some((name, from)) = index {
                    self.add_var(name, VarType::Num, 8);
                    self.assigned.insert(name.to_uppercase());
                    self.walk_expr(from)?;
                }
                for e in [to, by, while_, until].into_iter().flatten() {
                    self.walk_expr(e)?;
                }
                for s in body {
                    self.walk_stmt(s)?;
                }
                Ok(())
            }
            // DO sur liste de valeurs (M16.3) : l'index entre au PDV (Num 8,
            // assigné — pas de NOTE "uninitialized"). Le type est déduit des
            // valeurs ? SAS : numérique sauf si TOUTES les valeurs explicites
            // sont des chaînes → caractère. On infère le type/longueur de la
            // 1re valeur (suffisant pour les cas usuels).
            DsStmt::DoList { index, items, body } => {
                let (ty, length) = do_list_index_type(items);
                self.add_var(index, ty, length);
                self.assigned.insert(index.to_uppercase());
                for item in items {
                    match item {
                        DoListItem::Value(e) => self.walk_expr(e)?,
                        DoListItem::Range { from, to, by } => {
                            self.walk_expr(from)?;
                            self.walk_expr(to)?;
                            if let Some(b) = by {
                                self.walk_expr(b)?;
                            }
                        }
                    }
                }
                for s in body {
                    self.walk_stmt(s)?;
                }
                Ok(())
            }
            // DO OVER (M16.3) : itération implicite sur un array. L'array doit
            // être déclaré ; pendant le corps, une référence nue au nom de
            // l'array désigne l'élément courant (autorisée en lecture comme en
            // écriture). On installe le nom dans `do_over_arrays` le temps de
            // walker le corps.
            DsStmt::DoOver { array, body } => {
                let upper = array.to_uppercase();
                if !self.arrays.contains_key(&upper) {
                    return Err(SasError::runtime(format!(
                        "Undeclared array referenced: {array}."
                    )));
                }
                let newly = self.do_over_arrays.insert(upper.clone());
                let mut result = Ok(());
                for s in body {
                    if let Err(e) = self.walk_stmt(s) {
                        result = Err(e);
                        break;
                    }
                }
                if newly {
                    self.do_over_arrays.remove(&upper);
                }
                result
            }
            // SELECT (M16.1) : vérifie les références de variables du
            // sélecteur, de chaque valeur/condition de WHEN, et des corps
            // (WHEN + OTHERWISE), en ordre textuel.
            DsStmt::Select {
                selector,
                whens,
                otherwise,
            } => {
                if let Some(sel) = selector {
                    self.walk_expr(sel)?;
                }
                for when in whens {
                    for v in &when.values {
                        self.walk_expr(v)?;
                    }
                    self.walk_stmt(&when.body)?;
                }
                if let Some(o) = otherwise {
                    self.walk_stmt(o)?;
                }
                Ok(())
            }
            // DELETE : purement exécutif, rien à compiler.
            DsStmt::Delete => Ok(()),
            DsStmt::Output(targets) => {
                // `has_explicit_output` dès qu'UN output (ciblé ou non)
                // apparaît. Chaque cible doit être une sortie déclarée du
                // statement DATA (comparaison par display "WORK.A").
                self.has_explicit_output = true;
                for t in targets {
                    let disp = t.display();
                    if !self.output_displays.contains(&disp) {
                        return Err(SasError::runtime(format!(
                            "Output dataset {disp} is not in the DATA statement output list."
                        )));
                    }
                }
                Ok(())
            }
            DsStmt::Keep(names) => {
                self.keeps.extend(names.iter().cloned());
                Ok(())
            }
            DsStmt::Drop(names) => {
                self.drops.extend(names.iter().cloned());
                Ok(())
            }
            DsStmt::Stop => Ok(()),
            DsStmt::Retain(items) => {
                if items.is_empty() {
                    // `retain;` seul : tout le PDV (cf. fin de compile()).
                    self.retain_all = true;
                    return Ok(());
                }
                for (name, init) in items {
                    match init {
                        // AVEC init : la variable entre au PDV ICI (ordre de
                        // première référence), type/longueur du littéral, et
                        // sa valeur initiale part dans `initial_values`. Elle
                        // compte comme initialisée (pas de NOTE
                        // "uninitialized" — comme SAS).
                        Some(expr) => {
                            let (ty, length, value) = retain_literal(expr)?;
                            let slot = self.add_var(name, ty, length);
                            self.retained_slots.insert(slot);
                            self.assigned.insert(name.to_uppercase());
                            self.initial_values.push((slot, value));
                        }
                        // SANS init : ne crée PAS la variable (le type sera
                        // figé par sa prochaine référence) — voir compile().
                        None => self.retain_pending.push(name.clone()),
                    }
                }
                Ok(())
            }
            DsStmt::Sum { var, expr } => {
                // `var + expr;` : var entre au PDV (Num, 8), retenue, valeur
                // initiale 0 — SAUF si un RETAIN avec init a déjà posé une
                // valeur pour ce slot (le RETAIN gagne, comme SAS). La cible
                // entre avant les variables de l'expression (ordre textuel).
                let slot = self.add_var(var, VarType::Num, 8);
                self.retained_slots.insert(slot);
                self.assigned.insert(var.to_uppercase());
                if !self.initial_values.iter().any(|(s, _)| *s == slot) {
                    self.initial_values.push((slot, Value::Num(0.0)));
                }
                self.walk_expr(expr)?;
                Ok(())
            }
            DsStmt::Array {
                name,
                dims,
                char_len,
                vars,
                initial,
                temporary,
                special,
            } => self.compile_array(name, dims.as_deref(), *char_len, vars, initial, *temporary, *special),
            DsStmt::AssignIndexed {
                array,
                indices,
                expr,
            } => {
                let upper = array.to_uppercase();
                let Some(def) = self.arrays.get(&upper) else {
                    return Err(SasError::runtime(format!(
                        "Undeclared array referenced: {array}."
                    )));
                };
                // Tous les éléments sont potentiellement assignés via
                // l'indice : pas de NOTE "uninitialized" pour eux.
                for slot in def.slots.clone() {
                    let n = self.pdv.vars()[slot].name.to_uppercase();
                    self.assigned.insert(n);
                }
                for index in indices {
                    self.walk_expr(index)?;
                }
                self.walk_expr(expr)?;
                Ok(())
            }
            DsStmt::Length(items) => {
                for (name, spec) in items {
                    // Plages SAS : char 1..=32767, num 3..=8.
                    let (lo, hi) = if spec.char { (1, 32767) } else { (3, 8) };
                    if spec.len < lo || spec.len > hi {
                        return Err(SasError::runtime(format!(
                            "The length {} specified for the variable {} is out of range ({}-{}).",
                            spec.len, name, lo, hi
                        )));
                    }
                    match self.pdv.slot(name) {
                        // LENGTH précède la première référence : crée la
                        // variable avec cette longueur. Pour une numérique,
                        // la longueur (3..=8) est une simple MÉTADONNÉE en
                        // M2 — le stockage reste f64 sur 8 octets.
                        None => {
                            let ty = if spec.char { VarType::Char } else { VarType::Num };
                            self.add_var(name, ty, spec.len);
                        }
                        // Déjà au PDV : la longueur est figée. SAS n'émet le
                        // WARNING que pour les variables CHAR dont la
                        // longueur demandée diffère ; num : silencieux.
                        Some(slot) => {
                            let v = &self.pdv.vars()[slot];
                            if v.ty == VarType::Char && spec.char && v.length != spec.len {
                                let name = v.name.clone();
                                self.session.log.warning(&format!(
                                    "Length of character variable {name} has already been set."
                                ));
                            }
                        }
                    }
                }
                Ok(())
            }
            // FORMAT/LABEL/ATTRIB : déclarations de compilation. Le format
            // (validé via FormatSpec::parse) et le libellé sont mémorisés
            // dans des maps appliquées en fin de compilation (l'ordre
            // déclaration/référence n'importe donc pas). Une variable
            // inconnue est ignorée (SIMPLIFICATION M4 documentée : en vrai
            // SAS la variable serait créée sur le PDV).
            DsStmt::Format(groups) => {
                for (names, token) in groups {
                    if crate::formats::FormatSpec::parse(token).is_none() {
                        return Err(SasError::runtime(format!(
                            "The format {token} is not valid."
                        )));
                    }
                    for name in names {
                        self.formats.insert(name.to_uppercase(), token.clone());
                    }
                }
                Ok(())
            }
            DsStmt::Label(pairs) => {
                for (name, label) in pairs {
                    self.labels.insert(name.to_uppercase(), label.clone());
                }
                Ok(())
            }
            DsStmt::Attrib(items) => {
                for item in items {
                    if let Some(token) = &item.format
                        && crate::formats::FormatSpec::parse(token).is_none()
                    {
                        return Err(SasError::runtime(format!(
                            "The format {token} is not valid."
                        )));
                    }
                    for name in &item.vars {
                        let upper = name.to_uppercase();
                        if let Some(token) = &item.format {
                            self.formats.insert(upper.clone(), token.clone());
                        }
                        if let Some(label) = &item.label {
                            self.labels.insert(upper.clone(), label.clone());
                        }
                        // length= : parsé mais non appliqué en M4.
                    }
                }
                Ok(())
            }
            // `call <name>(args);` (M11.5) : les arguments sont des
            // expressions rvalue ordinaires (la routine ne crée pas de
            // variable PDV — `call symput` écrit dans la table macro, pas
            // dans le PDV). On parcourt donc simplement les arguments pour
            // découvrir les variables référencées.
            DsStmt::CallRoutine { name, args } => {
                // CALL SORTN/SORTC (M15.6) acceptent un NOM D'ARRAY entier en
                // argument (`call sortn(arr)`) — ce n'est pas une référence de
                // variable illégale, mais le déballage de tous ses éléments.
                // On ne walke donc PAS un argument qui nomme un array déclaré.
                let is_sort = name.eq_ignore_ascii_case("sortn")
                    || name.eq_ignore_ascii_case("sortc");
                for a in args {
                    if is_sort
                        && let Expr::Var(n) = a
                        && self.arrays.contains_key(&n.to_uppercase())
                    {
                        continue;
                    }
                    self.walk_expr(a)?;
                }
                Ok(())
            }
            // INFILE (M14) : déclaratif. Un second INFILE écrase le premier
            // (SAS le permet — le dernier gagne). On mémorise source+options.
            DsStmt::Infile { source, options } => {
                self.infile = Some((source.clone(), options.clone()));
                Ok(())
            }
            // INPUT (M14) : les variables nommées entrent au PDV en ordre de
            // première référence (char → longueur du `$ w`/informat, défaut
            // 8 ; num → 8). Plusieurs INPUT par étape sont autorisés.
            DsStmt::Input(items) => {
                self.seen_input = true;
                for item in items {
                    if let crate::ast::InputItem::Var {
                        name,
                        is_char,
                        informat,
                        ..
                    } = item
                    {
                        let (ty, length) = input_var_type(*is_char, informat.as_deref())?;
                        self.add_var(name, ty, length);
                        // Une variable d'INPUT est « assignée » (pas de NOTE
                        // uninitialized).
                        self.assigned.insert(name.to_uppercase());
                    }
                }
                Ok(())
            }
            // DATALINES (M14) : le bloc verbatim, source inline de l'étape.
            DsStmt::Datalines(lines) => {
                self.datalines = Some(lines.clone());
                Ok(())
            }
            // FILE/PUT (M14.2) : déclaratif / interprété directement en
            // exec.rs depuis l'AST (comme les assignations). Aucune variable
            // n'entre au PDV via PUT — les variables nommées doivent déjà
            // exister (résolution de slot à l'exécution, erreur si inconnue).
            DsStmt::File { .. } | DsStmt::Put(_) => Ok(()),
        }
    }

    /// Crée les variables simplement référencées (Num par défaut), en ordre
    /// textuel gauche→droite. Les noms d'array ne créent JAMAIS de variable
    /// au PDV : `Expr::Index` ne walke que son indice (le nom doit être un
    /// array déclaré), `dim(arr)` ne crée pas `arr`, et un nom d'array nu
    /// (`Expr::Var`) est une référence illégale.
    fn walk_expr(&mut self, expr: &Expr) -> Result<()> {
        match expr {
            Expr::Num(_) | Expr::Str(_) | Expr::Missing(_) => Ok(()),
            Expr::Var(name) => {
                let upper = name.to_uppercase();
                // Variables automatiques (_N_, _ERROR_) : servies par
                // l'évaluateur depuis les champs dédiés du PDV — elles ne
                // doivent JAMAIS créer de slot (sinon elles deviendraient
                // des colonnes de sortie + NOTE "uninitialized" parasite).
                if upper == "_N_" || upper == "_ERROR_" {
                    return Ok(());
                }
                // FIRST.x / LAST.x : variables automatiques de groupe BY,
                // servies par l'évaluateur depuis les flags du Runner —
                // jamais de slot PDV (donc jamais écrites en sortie). On
                // mémorise la référence pour la valider contre le BY en
                // fin de compilation.
                if upper.starts_with("FIRST.") || upper.starts_with("LAST.") {
                    self.first_last_refs.push(upper);
                    return Ok(());
                }
                // Variable IN= d'un MERGE : automatique temporaire 0/1,
                // servie par EvalCtx — jamais de slot PDV (donc jamais
                // écrite en sortie).
                if self.in_flags.iter().any(|(n, _)| *n == upper) {
                    return Ok(());
                }
                // Variable END= du SET (M16.4) : automatique temporaire 0/1,
                // servie par EvalCtx — jamais de slot PDV (donc jamais écrite
                // en sortie). On la reconnaît au nom déclaré sur le SET.
                if self
                    .set_options
                    .end
                    .as_ref()
                    .is_some_and(|e| e.eq_ignore_ascii_case(name))
                {
                    return Ok(());
                }
                if self.arrays.contains_key(&upper) {
                    // Référence nue à un array : autorisée si un `DO OVER` est
                    // actif (élément courant, résolu à l'exécution) ; ne crée
                    // pas de variable. Sinon illégale.
                    if self.do_over_arrays.contains(&upper) {
                        return Ok(());
                    }
                    return Err(SasError::runtime(format!(
                        "Illegal reference to the array {name}."
                    )));
                }
                self.add_var(name, VarType::Num, 8);
                Ok(())
            }
            Expr::Unary { expr, .. } => self.walk_expr(expr),
            Expr::Binary { left, right, .. } => {
                self.walk_expr(left)?;
                self.walk_expr(right)
            }
            Expr::In { expr, list } => {
                self.walk_expr(expr)?;
                for e in list {
                    self.walk_expr(e)?;
                }
                Ok(())
            }
            Expr::Index { name, indices } => {
                if !self.arrays.contains_key(&name.to_uppercase()) {
                    return Err(SasError::runtime(format!(
                        "Undeclared array referenced: {name}."
                    )));
                }
                for index in indices {
                    self.walk_expr(index)?;
                }
                Ok(())
            }
            Expr::Call { name, args } => {
                // `dim(arr)`/`hbound(arr[, n])`/`lbound(arr[, n])` : le 1er
                // argument nomme un array — il ne crée PAS de variable. Les
                // autres arguments (dimension) sont walkés normalement.
                let is_dim_fn = name.eq_ignore_ascii_case("dim")
                    || name.eq_ignore_ascii_case("hbound")
                    || name.eq_ignore_ascii_case("lbound");
                if is_dim_fn
                    && !args.is_empty()
                    && let Expr::Var(n) | Expr::Index { name: n, .. } = &args[0]
                    && self.arrays.contains_key(&n.to_uppercase())
                {
                    // `dim(a{i})` : l'indice du 1er argument reste walké.
                    if let Expr::Index { indices, .. } = &args[0] {
                        for index in indices {
                            self.walk_expr(index)?;
                        }
                    }
                    for a in &args[1..] {
                        self.walk_expr(a)?;
                    }
                    return Ok(());
                }
                for a in args {
                    self.walk_expr(a)?;
                }
                Ok(())
            }
        }
    }

    /// `add_var` PDV : la première référence fige tout (le PDV ignore les
    /// ajouts suivants du même nom).
    fn add_var(&mut self, name: &str, ty: VarType, length: usize) -> usize {
        self.pdv.add_var(PdvVar {
            name: name.to_string(),
            ty,
            length,
            retained: false,
            from_input: false,
            format: None,
            temporary: false,
        })
    }

    /// Slot d'un élément d'array `_TEMPORARY_` : hors-PDV-de-sortie, retenu
    /// implicitement. Les noms internes (`*name[i]`) ne peuvent collisionner
    /// avec une variable utilisateur (`*` interdit en SAS).
    fn add_temp_var(&mut self, name: &str, ty: VarType, length: usize) -> usize {
        self.pdv.add_var(PdvVar {
            name: name.to_string(),
            ty,
            length,
            retained: true,
            from_input: false,
            format: None,
            temporary: true,
        })
    }

    /// Déclare un array (M2/M16.2). Les éléments entrent au PDV ICI (ordre
    /// de première référence). `dims` None (`{*}`) → 1-D, taille déduite de
    /// la liste ; sinon bornes supérieures explicites (le produit = nombre
    /// d'éléments). `vars` vide (et pas de liste spéciale) → éléments
    /// auto-nommés name1..nameN. `char_len` → éléments caractère.
    /// `initial` → valeurs initiales row-major (RETAIN implicite). `temp` →
    /// éléments hors-sortie, retenus. `special` → `_NUMERIC_`/`_CHARACTER_`/
    /// `_ALL_` remplacé par les variables PDV correspondantes. Le registre
    /// `arrays` associe le nom UPPERCASE à la définition (slots + dims).
    #[allow(clippy::too_many_arguments)]
    fn compile_array(
        &mut self,
        name: &str,
        dims: Option<&[usize]>,
        char_len: Option<usize>,
        vars: &[String],
        initial: &[crate::ast::Expr],
        temp: bool,
        special: Option<crate::ast::ArraySpecial>,
    ) -> Result<()> {
        use crate::ast::ArraySpecial;
        let upper = name.to_uppercase();
        if self.arrays.contains_key(&upper) {
            return Err(SasError::runtime(format!(
                "An array has already been defined with the name {name}."
            )));
        }

        let (ty, length) = match char_len {
            Some(l) => (VarType::Char, l),
            None => (VarType::Num, 8),
        };

        // Liste effective d'éléments à entrer au PDV (ou slots existants
        // pour les listes spéciales). On collecte directement des slots.
        let slots: Vec<usize> = if let Some(kind) = special {
            // `_NUMERIC_`/`_CHARACTER_`/`_ALL_` : toutes les variables
            // (NON-temporaires) connues au point du statement, du type voulu.
            let want_char = matches!(kind, ArraySpecial::Character)
                || (matches!(kind, ArraySpecial::All) && char_len.is_some());
            let want = if want_char { VarType::Char } else { VarType::Num };
            let picked: Vec<usize> = self
                .pdv
                .vars()
                .iter()
                .enumerate()
                .filter(|(_, v)| {
                    !v.temporary
                        && match kind {
                            ArraySpecial::Numeric => v.ty == VarType::Num,
                            ArraySpecial::Character => v.ty == VarType::Char,
                            ArraySpecial::All => v.ty == want,
                        }
                })
                .map(|(i, _)| i)
                .collect();
            if picked.is_empty() {
                return Err(SasError::runtime(format!(
                    "The array {name} has been defined with zero elements."
                )));
            }
            // Une dimension explicite doit correspondre au compte trouvé.
            if let Some(ds) = dims {
                let total: usize = ds.iter().product();
                if total != picked.len() {
                    return Err(SasError::runtime(format!(
                        "The number of variables in the list ({}) does not match \
                         the number of elements ({}) in the array {}.",
                        picked.len(),
                        total,
                        name
                    )));
                }
            }
            picked
        } else {
            // Liste nommée OU éléments auto-générés.
            let total = dims.map(|d| d.iter().product::<usize>());
            let names: Vec<String> = if vars.is_empty() {
                let Some(n) = total else {
                    return Err(SasError::runtime(format!(
                        "The array {name} has been defined with zero elements."
                    )));
                };
                if temp {
                    // Éléments temporaires : noms internes non collisionnables.
                    (1..=n).map(|i| format!("*{name}[{i}]")).collect()
                } else {
                    (1..=n).map(|i| format!("{name}{i}")).collect()
                }
            } else {
                if let Some(n) = total
                    && n != vars.len()
                {
                    return Err(SasError::runtime(format!(
                        "The number of variables in the list ({}) does not match \
                         the number of elements ({}) in the array {}.",
                        vars.len(),
                        n,
                        name
                    )));
                }
                vars.to_vec()
            };
            if temp {
                names
                    .iter()
                    .map(|v| self.add_temp_var(v, ty, length))
                    .collect()
            } else {
                names.iter().map(|v| self.add_var(v, ty, length)).collect()
            }
        };

        // Dimensions résolues : explicites, ou 1-D = nombre d'éléments.
        let dim_vec: Vec<usize> = match dims {
            Some(d) => d.to_vec(),
            None => vec![slots.len()],
        };

        // Valeurs initiales (row-major) : évaluées à la COMPILATION (les
        // littéraux constants suffisent) puis appliquées via `initial_values`
        // avant la 1re itération, comme RETAIN avec init. SAS marque les
        // éléments initialisés comme retenus.
        if !initial.is_empty() {
            if initial.len() > slots.len() {
                return Err(SasError::runtime(format!(
                    "Too many initial values were specified for the array {name}."
                )));
            }
            for (k, expr) in initial.iter().enumerate() {
                let v = const_eval_initial(expr, ty)?;
                self.initial_values.push((slots[k], v));
                self.retained_slots.insert(slots[k]);
                let nm = self.pdv.vars()[slots[k]].name.to_uppercase();
                self.assigned.insert(nm);
            }
        }

        self.arrays.insert(
            upper,
            ArrayDef {
                slots,
                dims: dim_vec,
            },
        );
        Ok(())
    }

    /// Type et longueur des éléments d'un array (premier slot — tous les
    /// éléments d'un array M2 partagent type et longueur déclarés ; un
    /// élément préexistant au PDV garde toutefois les siens).
    fn array_elem_type(&self, name: &str) -> (VarType, usize) {
        match self
            .arrays
            .get(&name.to_uppercase())
            .and_then(|def| def.slots.first())
        {
            Some(&slot) => {
                let v = &self.pdv.vars()[slot];
                (v.ty, v.length)
            }
            None => (VarType::Num, 8),
        }
    }

    /// Compile UN dataset d'un statement SET : lecture, options de
    /// dataset, entrée des variables au PDV (union en ordre de première
    /// apparition), matérialisation des colonnes.
    /// Matérialise un dataset (toutes ses colonnes) dans le PDV pour UPDATE/
    /// MODIFY (M16.5). Comme `compile_set` mais sans KEEP=/DROP=/RENAME= :
    /// TOUTES les variables entrent au PDV (ordre de première référence), avec
    /// downcast unique par colonne (jamais de get_row). Renvoie l'`InputDataset`
    /// matérialisé (colonnes décodées + slots PDV). `opts` réservé (where=
    /// filtré à l'exécution, non ici).
    fn materialize_input(
        &mut self,
        dref: &crate::ast::DatasetRef,
        _opts: &DatasetOptions,
    ) -> Result<InputDataset> {
        Ok(self.materialize_input_with_meta(dref, _opts)?.0)
    }

    /// Comme `materialize_input` mais renvoie aussi les `VarMeta` de CHAQUE
    /// colonne (dans l'ordre `var_slots`), nécessaires à MODIFY pour réécrire
    /// le dataset à l'identique (mêmes types/longueurs/formats/libellés).
    fn materialize_input_with_meta(
        &mut self,
        dref: &crate::ast::DatasetRef,
        _opts: &DatasetOptions,
    ) -> Result<(InputDataset, Vec<crate::dataset::VarMeta>)> {
        let libref = dref.libref_or_work();
        let provider = self.session.libs.get(&libref)?;
        if !provider.exists(&dref.name) {
            return Err(SasError::runtime(format!(
                "File {}.DATA does not exist.",
                dref.display()
            )));
        }
        let (ds, notes) = provider.read(&dref.name)?;
        for note in &notes {
            self.session.log.forward(note);
        }
        let mut columns = Vec::with_capacity(ds.vars.len());
        let mut var_slots = Vec::with_capacity(ds.vars.len());
        let mut out_vars = Vec::with_capacity(ds.vars.len());
        for (col, meta) in ds.df.get_columns().iter().zip(&ds.vars) {
            if self
                .pdv
                .slot(&meta.name)
                .is_some_and(|slot| self.pdv.vars()[slot].ty != meta.ty)
            {
                return Err(SasError::runtime(format!(
                    "Variable {} has been defined as both character and numeric.",
                    meta.name
                )));
            }
            let slot = self.pdv.add_var(PdvVar {
                name: meta.name.clone(),
                ty: meta.ty,
                length: meta.length,
                retained: false,
                from_input: true,
                format: meta.format.clone(),
                temporary: false,
            });
            self.pdv.mark_from_input(slot);
            var_slots.push(slot);
            out_vars.push(meta.clone());
            let s = col.as_materialized_series();
            let values: Vec<Value> = match meta.ty {
                VarType::Num => s.f64()?.iter().map(num_to_value).collect(),
                VarType::Char => s
                    .str()?
                    .iter()
                    .map(|o| Value::Char(o.unwrap_or("").to_string()))
                    .collect(),
            };
            columns.push(values);
        }
        let n_rows = ds.n_obs();
        Ok((
            InputDataset {
                display: dref.display(),
                columns,
                var_slots,
                n_rows,
                where_: None,
                by_cols: Vec::new(),
            },
            out_vars,
        ))
    }

    fn compile_set(&mut self, spec: &DatasetSpec) -> Result<()> {
        let r = &spec.dref;
        let opts = &spec.options;
        let libref = r.libref_or_work();
        let provider = self.session.libs.get(&libref)?;
        if !provider.exists(&r.name) {
            return Err(SasError::runtime(format!(
                "File {}.DATA does not exist.",
                r.display()
            )));
        }
        let (ds, notes) = provider.read(&r.name)?;
        for note in &notes {
            self.session.log.forward(note);
        }

        // Validation des options : KEEP=/DROP=/RENAME= référencent les noms
        // D'ORIGINE de l'input (règle SAS : KEEP/DROP s'appliquent AVANT
        // RENAME). Un nom absent de l'input → même erreur que l'existant.
        let input_names: HashSet<String> =
            ds.vars.iter().map(|v| v.name.to_uppercase()).collect();
        for name in opts
            .keep
            .iter()
            .flatten()
            .chain(opts.drop.iter().flatten())
            .chain(opts.rename.iter().map(|(old, _)| old))
        {
            if !input_names.contains(&name.to_uppercase()) {
                return Err(SasError::runtime(format!(
                    "The variable {name} in the DROP, KEEP, or RENAME list has never been referenced."
                )));
            }
        }
        let keep_set: Option<HashSet<String>> = opts
            .keep
            .as_ref()
            .map(|v| v.iter().map(|n| n.to_uppercase()).collect());
        let drop_set: HashSet<String> = opts
            .drop
            .iter()
            .flatten()
            .map(|n| n.to_uppercase())
            .collect();
        let rename: HashMap<String, String> = opts
            .rename
            .iter()
            .map(|(old, new)| (old.to_uppercase(), new.clone()))
            .collect();

        let mut columns = Vec::with_capacity(ds.vars.len());
        let mut var_slots = Vec::with_capacity(ds.vars.len());
        for (col, meta) in ds.df.get_columns().iter().zip(&ds.vars) {
            let upper = meta.name.to_uppercase();
            // KEEP=/DROP= filtrent quelles variables d'input entrent au PDV
            // (une variable renommée mais non gardée est ignorée).
            if keep_set.as_ref().is_some_and(|k| !k.contains(&upper))
                || drop_set.contains(&upper)
            {
                continue;
            }
            // RENAME= : la variable entre au PDV sous le NOUVEAU nom
            // (appliqué APRÈS keep/drop).
            let pdv_name = rename
                .get(&upper)
                .cloned()
                .unwrap_or_else(|| meta.name.clone());
            // Une variable déjà au PDV avec un type INCOMPATIBLE (présente
            // dans un autre dataset du SET, ou référencée avant) → erreur
            // de compilation, comme SAS.
            if let Some(slot) = self.pdv.slot(&pdv_name) {
                if self.pdv.vars()[slot].ty != meta.ty {
                    return Err(SasError::runtime(format!(
                        "Variable {pdv_name} has been defined as both character and numeric."
                    )));
                }
            }
            let slot = self.pdv.add_var(PdvVar {
                name: pdv_name,
                ty: meta.ty,
                length: meta.length,
                retained: false,
                from_input: true,
                format: meta.format.clone(),
                temporary: false,
            });
            // Si la variable existait déjà (référence textuelle antérieure
            // au SET), la marquer issue de l'input malgré tout.
            self.pdv.mark_from_input(slot);
            var_slots.push(slot);

            // Downcast UNE FOIS par colonne — jamais de get_row.
            let s = col.as_materialized_series();
            let values: Vec<Value> = match meta.ty {
                VarType::Num => s.f64()?.iter().map(num_to_value).collect(),
                VarType::Char => s
                    .str()?
                    .iter()
                    .map(|o| Value::Char(o.unwrap_or("").to_string()))
                    .collect(),
            };
            columns.push(values);
        }

        // WHERE= : PAS de filtrage à la compilation — l'Expr est stockée et
        // évaluée par l'exécuteur après chaque chargement de ligne. On
        // walke ses variables pour valider qu'elles existent (elles doivent
        // référencer des variables d'input, déjà au PDV à ce point —
        // post-rename, cf. doc d'InputData).
        if let Some(w) = &opts.where_ {
            self.validate_where_vars(w, &r.display())?;
        }

        // OPTIONS FIRSTOBS=/OBS= : restreindre la fenêtre des observations
        // PHYSIQUES lues, AVANT le filtre WHERE= (ordre SAS). FIRSTOBS=k saute
        // les k-1 premières ; OBS=n borne le numéro de la dernière obs lue.
        let n = ds.n_obs();
        let start = self.session.options.firstobs.saturating_sub(1).min(n);
        let end = self.session.options.obs.map_or(n, |o| o.min(n)).max(start);
        if start != 0 || end != n {
            for c in &mut columns {
                *c = c[start..end].to_vec();
            }
        }

        self.input_datasets.push(InputDataset {
            display: r.display(),
            columns,
            var_slots,
            n_rows: end - start,
            where_: opts.where_.clone(),
            by_cols: Vec::new(),
        });
        Ok(())
    }

    /// Assemble l'`InputData` final : résolution des clés BY en slots PDV,
    /// localisation de chaque clé dans CHAQUE dataset (`by_cols`),
    /// validation des références FIRST./LAST. contre les variables BY.
    fn build_input(&mut self) -> Result<Option<InputData>> {
        // UPDATE/MODIFY gèrent leur propre BY (résolu dans build_update/
        // build_modify) : ne pas consommer `by`/`first_last_refs` ici.
        if self.update.is_some() || self.modify.is_some() {
            return Ok(None);
        }
        let mut datasets = std::mem::take(&mut self.input_datasets);
        let by_items = self.by.take();
        if datasets.is_empty() {
            // BY ou FIRST./LAST. sans SET : message SAS.
            if by_items.is_some() || !self.first_last_refs.is_empty() {
                return Err(SasError::runtime(
                    "No SET, MERGE, UPDATE, or MODIFY statement.",
                ));
            }
            return Ok(None);
        }
        let mut by: Vec<ByVar> = Vec::new();
        if let Some(items) = by_items {
            for (name, descending) in items {
                let Some(slot) = self.pdv.slot(&name) else {
                    return Err(SasError::runtime(format!(
                        "BY variable {name} is not on input data set {}.",
                        datasets[0].display
                    )));
                };
                by.push(ByVar {
                    name: name.to_uppercase(),
                    slot,
                    descending,
                });
            }
            // Chaque variable BY doit exister dans CHAQUE dataset du SET
            // (après keep=/drop=/rename=).
            for ds in &mut datasets {
                for bv in &by {
                    let Some(pos) = ds.var_slots.iter().position(|&s| s == bv.slot) else {
                        return Err(SasError::runtime(format!(
                            "BY variable {} is not on input data set {}.",
                            bv.name, ds.display
                        )));
                    };
                    ds.by_cols.push(pos);
                }
            }
        }
        // FIRST.x / LAST.x : x doit être une variable BY.
        for full in &self.first_last_refs {
            let suffix = full
                .split_once('.')
                .map(|(_, s)| s)
                .unwrap_or(full.as_str());
            if !by.iter().any(|b| b.name == suffix) {
                return Err(SasError::runtime(format!(
                    "Variable {full} is not defined: {suffix} is not a BY variable."
                )));
            }
        }
        // MERGE exige un BY (sinon match-merge non défini : SAS le tolère en
        // « one-to-one merge » positionnel, hors périmètre M3 → erreur).
        if self.seen_merge && by.is_empty() {
            return Err(SasError::runtime(
                "A MERGE statement requires a BY statement.",
            ));
        }
        let in_flags = std::mem::take(&mut self.in_flags);

        // Options de niveau statement du SET (M16.4).
        let opts = std::mem::take(&mut self.set_options);
        let end_var = opts.end.as_ref().map(|n| n.to_uppercase());
        let nobs_slot = match &opts.nobs {
            Some(n) => Some(self.pdv.slot(n).ok_or_else(|| {
                SasError::runtime(format!("NOBS= variable {n} is not addressable."))
            })?),
            None => None,
        };
        let point_slot = match &opts.point {
            Some(n) => Some(self.pdv.slot(n).ok_or_else(|| {
                SasError::runtime(format!("POINT= variable {n} is not addressable."))
            })?),
            None => None,
        };
        // POINT= remplace la boucle implicite : il est incompatible avec un
        // interclassement BY (l'accès direct n'a pas de sémantique BY) et avec
        // un MERGE. Les datasets multiples en concaténation sont tolérés (index
        // global 1..total), mais SAS le déconseille (documenté).
        if point_slot.is_some() {
            if !by.is_empty() {
                return Err(SasError::runtime(
                    "POINT= cannot be used with a BY statement.",
                ));
            }
            if self.seen_merge {
                return Err(SasError::runtime(
                    "POINT= cannot be used with a MERGE statement.",
                ));
            }
        }

        Ok(Some(InputData {
            datasets,
            by,
            merge: self.seen_merge,
            in_flags,
            end_var,
            nobs_slot,
            point_slot,
        }))
    }

    /// Assemble l'`UpdateData` final (M16.5) : résolution des clés en slots
    /// PDV, calcul des slots overlay (variables transaction hors clés),
    /// résolution du BY optionnel (FIRST./LAST.). Renvoie `None` si pas
    /// d'UPDATE dans l'étape.
    fn build_update(&mut self) -> Result<Option<UpdateData>> {
        let Some(pending) = self.update.take() else {
            return Ok(None);
        };
        let by_items = self.by.take();
        // Clés : doivent exister dans le PDV (donc dans le maître OU la
        // transaction). On résout par nom ; une clé absente du maître ET de la
        // transaction → erreur.
        let mut key_slots = Vec::with_capacity(pending.key_names.len());
        for name in &pending.key_names {
            let Some(slot) = self.pdv.slot(name) else {
                return Err(SasError::runtime(format!(
                    "KEY variable {name} is not on the UPDATE data sets."
                )));
            };
            // La clé doit appartenir à la transaction (sert la recherche) ET
            // au maître (l'obs maître la porte).
            if !pending.transaction.var_slots.contains(&slot) {
                return Err(SasError::runtime(format!(
                    "KEY variable {name} is not on the transaction data set {}.",
                    pending.transaction.display
                )));
            }
            if !pending.master.var_slots.contains(&slot) {
                return Err(SasError::runtime(format!(
                    "KEY variable {name} is not on the master data set {}.",
                    pending.master_display
                )));
            }
            key_slots.push(slot);
        }
        // Slots overlay : toutes les variables de la transaction SAUF les clés.
        let overlay_slots: Vec<usize> = pending
            .transaction
            .var_slots
            .iter()
            .copied()
            .filter(|s| !key_slots.contains(s))
            .collect();

        // BY optionnel : chaque clé BY doit exister au PDV ; on remplit
        // `by_cols` du maître (pilote l'itération / FIRST./LAST.).
        let mut by: Vec<ByVar> = Vec::new();
        let mut master = pending.master;
        if let Some(items) = by_items {
            for (name, descending) in items {
                let Some(slot) = self.pdv.slot(&name) else {
                    return Err(SasError::runtime(format!(
                        "BY variable {name} is not on the master data set {}.",
                        master.display
                    )));
                };
                let Some(pos) = master.var_slots.iter().position(|&s| s == slot) else {
                    return Err(SasError::runtime(format!(
                        "BY variable {name} is not on the master data set {}.",
                        master.display
                    )));
                };
                master.by_cols.push(pos);
                by.push(ByVar {
                    name: name.to_uppercase(),
                    slot,
                    descending,
                });
            }
        }
        // FIRST.x / LAST.x : x doit être une variable BY.
        for full in &self.first_last_refs {
            let suffix = full.split_once('.').map(|(_, s)| s).unwrap_or(full.as_str());
            if !by.iter().any(|b| b.name == suffix) {
                return Err(SasError::runtime(format!(
                    "Variable {full} is not defined: {suffix} is not a BY variable."
                )));
            }
        }
        Ok(Some(UpdateData {
            master,
            transaction: pending.transaction,
            key_slots,
            overlay_slots,
            master_where: pending.master_where,
            by,
        }))
    }

    /// Assemble le `ModifyData` final (M16.5) : résolution des clés et des
    /// slots POINT=/NOBS=. Renvoie `None` si pas de MODIFY dans l'étape.
    fn build_modify(&mut self) -> Result<Option<ModifyData>> {
        let Some(pending) = self.modify.take() else {
            return Ok(None);
        };
        let mut key_slots = Vec::with_capacity(pending.key_names.len());
        for name in &pending.key_names {
            let Some(slot) = self.pdv.slot(name) else {
                return Err(SasError::runtime(format!(
                    "KEY variable {name} is not on the MODIFY data set {}.",
                    pending.display
                )));
            };
            if !pending.data.var_slots.contains(&slot) {
                return Err(SasError::runtime(format!(
                    "KEY variable {name} is not on the MODIFY data set {}.",
                    pending.display
                )));
            }
            key_slots.push(slot);
        }
        let point_slot = match &pending.point {
            Some(n) => Some(self.pdv.slot(n).ok_or_else(|| {
                SasError::runtime(format!("POINT= variable {n} is not addressable."))
            })?),
            None => None,
        };
        let nobs_slot = match &pending.nobs {
            Some(n) => Some(self.pdv.slot(n).ok_or_else(|| {
                SasError::runtime(format!("NOBS= variable {n} is not addressable."))
            })?),
            None => None,
        };
        Ok(Some(ModifyData {
            libref: pending.libref,
            table: pending.table,
            display: pending.display,
            data: pending.data,
            key_slots,
            point_slot,
            nobs_slot,
            out_vars: pending.out_vars,
        }))
    }

    /// Assemble la source d'entrée TEXTE (M14) à partir de l'INFILE, de
    /// l'INPUT et du bloc DATALINES rencontrés. Renvoie `None` si l'étape
    /// n'a ni INFILE ni INPUT ni DATALINES (= pas de lecture texte).
    fn build_text_input(&mut self) -> Result<Option<TextInput>> {
        let infile = self.infile.take();
        let datalines = self.datalines.take();

        // Pas de lecture texte du tout.
        if infile.is_none() && !self.seen_input && datalines.is_none() {
            return Ok(None);
        }

        // Source : INFILE explicite, sinon DATALINES inline implicite. Un
        // chemin relatif résout sous `base_dir` (cohérent avec LIBNAME) ; la
        // NOTE affiche le chemin SOURCE tel quel (entre guillemets, fidèle à
        // SAS, et stable pour les snapshots — pas de tempdir absolu).
        let (lines, display, is_file) = match &infile {
            Some((crate::ast::InfileSource::Path(path), _)) => {
                let resolved = self.session.resolve_path(path);
                let content = std::fs::read_to_string(&resolved).map_err(|e| {
                    SasError::runtime(format!("Unable to read INFILE '{path}': {e}"))
                })?;
                // Lignes sans le `\n` ; un `\r` final est retiré.
                let lines: Vec<String> = content
                    .lines()
                    .map(|l| l.to_string())
                    .collect();
                (lines, format!("the infile '{path}'"), true)
            }
            Some((crate::ast::InfileSource::Datalines, _)) | None => {
                let lines = datalines.clone().ok_or_else(|| {
                    SasError::runtime(
                        "INPUT/INFILE DATALINES used but no DATALINES block is present.",
                    )
                })?;
                (lines, "the infile DATALINES".to_string(), false)
            }
        };

        // Options d'exécution.
        let opts = infile.as_ref().map(|(_, o)| o);
        let dsd = opts.is_some_and(|o| o.dsd);
        let delimiter = opts.and_then(|o| o.delimiter.clone());
        let short = match opts {
            Some(o) if o.stopover => ShortMode::Stopover,
            Some(o) if o.truncover => ShortMode::Truncover,
            Some(o) if o.missover => ShortMode::Missover,
            _ => ShortMode::Default,
        };
        let firstobs = opts.and_then(|o| o.firstobs).unwrap_or(1).max(1);
        let obs = opts.and_then(|o| o.obs);

        let options = TextOptions {
            delimiter,
            dsd,
            firstobs,
            obs,
            short,
        };

        Ok(Some(TextInput {
            display,
            lines,
            options,
            is_file,
        }))
    }

    /// Toute variable d'un WHERE= de SET doit déjà être au PDV (= une
    /// variable de l'input, après keep/drop/rename) — message proche du
    /// SAS "Variable x is not on file WORK.A.". On ne walke PAS via
    /// `walk_expr` : cela créerait des variables Num parasites au PDV.
    fn validate_where_vars(&self, expr: &Expr, file: &str) -> Result<()> {
        match expr {
            Expr::Num(_) | Expr::Str(_) | Expr::Missing(_) => Ok(()),
            Expr::Var(name) => {
                let upper = name.to_uppercase();
                if upper == "_N_" || upper == "_ERROR_" || self.pdv.slot(name).is_some() {
                    Ok(())
                } else {
                    Err(SasError::runtime(format!(
                        "Variable {name} is not on file {file}."
                    )))
                }
            }
            Expr::Unary { expr, .. } => self.validate_where_vars(expr, file),
            Expr::Binary { left, right, .. } => {
                self.validate_where_vars(left, file)?;
                self.validate_where_vars(right, file)
            }
            Expr::In { expr, list } => {
                self.validate_where_vars(expr, file)?;
                for e in list {
                    self.validate_where_vars(e, file)?;
                }
                Ok(())
            }
            Expr::Index { indices, .. } => {
                for index in indices {
                    self.validate_where_vars(index, file)?;
                }
                Ok(())
            }
            Expr::Call { args, .. } => {
                for a in args {
                    self.validate_where_vars(a, file)?;
                }
                Ok(())
            }
        }
    }

    /// Type et longueur inférés d'une expression (compile-time, comme SAS).
    fn infer(&self, expr: &Expr) -> (VarType, usize) {
        match expr {
            Expr::Num(_) | Expr::Missing(_) => (VarType::Num, 8),
            Expr::Str(s) => (VarType::Char, s.chars().count().max(1)),
            Expr::Var(name) => match self.pdv.slot(name) {
                Some(slot) => {
                    let v = &self.pdv.vars()[slot];
                    (v.ty, v.length)
                }
                // Inconnue au moment de l'inférence : numérique.
                None => (VarType::Num, 8),
            },
            Expr::Unary { .. } => (VarType::Num, 8),
            Expr::Binary {
                op: BinaryOp::Concat,
                left,
                right,
            } => (VarType::Char, self.char_len(left) + self.char_len(right)),
            Expr::Binary { .. } | Expr::In { .. } => (VarType::Num, 8),
            // `arr{i}` : type/longueur des éléments de l'array.
            Expr::Index { name, .. } => self.array_elem_type(name),
            Expr::Call { name, args } => {
                // Forme parenthèses `arr(i)`/`arr(i,j)` : l'array masque la
                // fonction.
                if !args.is_empty() && self.arrays.contains_key(&name.to_uppercase()) {
                    return self.array_elem_type(name);
                }
                let lower = name.to_ascii_lowercase();
                match lower.as_str() {
                    "upcase" | "lowcase" | "trim" | "strip" | "left" | "right" => {
                        let len = args.first().map_or(200, |a| self.char_len(a));
                        (VarType::Char, len)
                    }
                    "substr" => {
                        let len = args.first().map_or(200, |a| self.char_len(a));
                        (VarType::Char, len)
                    }
                    _ if lower.starts_with("cat") => (VarType::Char, 200),
                    "put" => (VarType::Char, put_width(args)),
                    _ => (VarType::Num, 8),
                }
            }
        }
    }

    /// Longueur d'un opérande en contexte caractère : un opérande numérique
    /// contribue 12 (conversion implicite BEST12., comme SAS).
    fn char_len(&self, expr: &Expr) -> usize {
        match self.infer(expr) {
            (VarType::Char, l) => l,
            (VarType::Num, _) => 12,
        }
    }

    fn resolve_outputs(&mut self, specs: &[DatasetSpec]) -> Result<Vec<OutputSpec>> {
        // Toute variable de KEEP/DROP (statements) doit exister au PDV.
        for name in self.keeps.iter().chain(self.drops.iter()) {
            if self.pdv.slot(name).is_none() {
                return Err(SasError::runtime(format!(
                    "The variable {} in the DROP, KEEP, or RENAME list has never been referenced.",
                    name
                )));
            }
        }
        let stmt_keep: Option<HashSet<String>> = if self.keeps.is_empty() {
            None
        } else {
            Some(self.keeps.iter().map(|n| n.to_uppercase()).collect())
        };
        let stmt_drop: HashSet<String> = self.drops.iter().map(|n| n.to_uppercase()).collect();

        // KEEP ∩ DROP (statements) : DROP gagne, avec WARNING.
        if let Some(ref ks) = stmt_keep {
            for d in &stmt_drop {
                if ks.contains(d) {
                    self.session.log.warning(&format!(
                        "Variable {d} is in both the KEEP and DROP lists; it will be dropped."
                    ));
                }
            }
        }

        let mut outputs = Vec::with_capacity(specs.len());
        for spec in specs {
            let opts = &spec.options;
            // WHERE= n'est pas valide sur une sortie (règle SAS).
            if opts.where_.is_some() {
                return Err(SasError::runtime(
                    "WHERE= is not a valid data set option for output data sets.",
                ));
            }
            // IN= n'est valide qu'en INPUT de MERGE (règle SAS).
            if opts.in_.is_some() {
                return Err(SasError::runtime(
                    "IN= is not a valid data set option for output data sets.",
                ));
            }
            // Les variables des options KEEP=/DROP=/RENAME= doivent exister
            // au PDV (KEEP/DROP avant RENAME : tout référence les noms PDV).
            for name in opts
                .keep
                .iter()
                .flatten()
                .chain(opts.drop.iter().flatten())
                .chain(opts.rename.iter().map(|(old, _)| old))
            {
                if self.pdv.slot(name).is_none() {
                    return Err(SasError::runtime(format!(
                        "The variable {name} in the DROP, KEEP, or RENAME list has never been referenced."
                    )));
                }
            }
            let opt_keep: Option<HashSet<String>> = opts
                .keep
                .as_ref()
                .map(|v| v.iter().map(|n| n.to_uppercase()).collect());
            let opt_drop: HashSet<String> = opts
                .drop
                .iter()
                .flatten()
                .map(|n| n.to_uppercase())
                .collect();
            let rename: HashMap<String, String> = opts
                .rename
                .iter()
                .map(|(old, new)| (old.to_uppercase(), new.clone()))
                .collect();

            // Combinaison statements + options : INTERSECTION des keeps
            // (un slot doit passer tous les KEEP présents), union des
            // drops (DROP gagne, sans WARNING supplémentaire pour les
            // options — simplification documentée).
            let mut kept_slots = Vec::new();
            let mut out_names = Vec::new();
            for (i, v) in self.pdv.vars().iter().enumerate() {
                // Les éléments d'array _TEMPORARY_ ne sont JAMAIS écrits.
                if v.temporary {
                    continue;
                }
                let u = v.name.to_uppercase();
                let kept = stmt_keep.as_ref().is_none_or(|k| k.contains(&u))
                    && opt_keep.as_ref().is_none_or(|k| k.contains(&u))
                    && !stmt_drop.contains(&u)
                    && !opt_drop.contains(&u);
                if kept {
                    kept_slots.push(i);
                    // RENAME= : la colonne ÉCRITE porte le nouveau nom (le
                    // slot PDV, lui, garde son nom).
                    out_names.push(rename.get(&u).cloned().unwrap_or_else(|| v.name.clone()));
                }
            }
            outputs.push(OutputSpec {
                libref: spec.libref_or_work(),
                table: spec.dref.name.clone(),
                display: spec.display(),
                kept_slots,
                out_names,
            });
        }
        Ok(outputs)
    }
}

/// Type, longueur et valeur d'un littéral d'init RETAIN. Le parser ne
/// produit que `Num` (le `-` unaire y est replié), `Str` ou `Missing` ;
/// tout autre nœud est un garde-fou.
/// Type/longueur de l'index d'un `DO sur liste de valeurs` (M16.3). SAS
/// infère caractère ssi la liste contient au moins une valeur chaîne ; sinon
/// numérique. La longueur caractère est la plus grande des chaînes
/// littérales (défaut 8 si aucune n'est un littéral). Les ranges sont
/// numériques par construction.
fn do_list_index_type(items: &[DoListItem]) -> (VarType, usize) {
    let mut is_char = false;
    let mut max_len = 0usize;
    for item in items {
        if let DoListItem::Value(Expr::Str(s)) = item {
            is_char = true;
            max_len = max_len.max(s.chars().count());
        }
    }
    if is_char {
        (VarType::Char, max_len.max(1))
    } else {
        (VarType::Num, 8)
    }
}

fn retain_literal(expr: &Expr) -> Result<(VarType, usize, Value)> {
    match expr {
        Expr::Num(n) => Ok((VarType::Num, 8, Value::Num(*n))),
        Expr::Missing(k) => Ok((VarType::Num, 8, Value::Missing(*k))),
        Expr::Str(s) => Ok((
            VarType::Char,
            s.chars().count().max(1),
            Value::Char(s.clone()),
        )),
        _ => Err(SasError::runtime(
            "RETAIN initial values must be literals.",
        )),
    }
}

/// Type et longueur d'une variable d'INPUT (M14). Caractère si `$` OU si
/// l'informat porte un `$` (ex. `$char10.`) ; longueur = largeur de
/// l'informat, sinon 8 par défaut. Numérique : longueur 8 (métadonnée).
fn input_var_type(is_char: bool, informat: Option<&str>) -> Result<(VarType, usize)> {
    let spec = informat
        .map(|tok| {
            crate::formats::FormatSpec::parse(tok)
                .ok_or_else(|| SasError::runtime(format!("The informat {tok} is not valid.")))
        })
        .transpose()?;
    let char_informat = spec.as_ref().is_some_and(|s| s.name.starts_with('$'));
    let char = is_char || char_informat;
    if char {
        // Longueur = largeur de l'informat caractère, défaut 8.
        let len = spec.as_ref().and_then(|s| s.w).map(|w| w as usize).unwrap_or(8);
        Ok((VarType::Char, len.max(1)))
    } else {
        Ok((VarType::Num, 8))
    }
}

/// Largeur du format d'un `put(x, fmt)` : chiffres finaux du nom du format
/// (`best12` → 12), sinon 200. Le parser M1 ne produit pas encore de
/// littéral de format complet — best-effort.
fn put_width(args: &[Expr]) -> usize {
    let Some(fmt) = args.get(1) else { return 200 };
    let name = match fmt {
        Expr::Var(n) => n.as_str(),
        Expr::Str(s) => s.as_str(),
        _ => return 200,
    };
    // La largeur du résultat de PUT est la largeur `w` du format, PAS les
    // chiffres finaux du token : pour `dollar10.2` c'est 10 (pas 2, le nombre
    // de décimales). On s'appuie donc sur le parseur de FormatSpec.
    crate::formats::FormatSpec::parse(name)
        .and_then(|spec| spec.w)
        .map(|w| w as usize)
        .unwrap_or(200)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::parser::StatementStream;
    use crate::source::SourceFile;
    use polars::df;
    use std::path::PathBuf;

    fn session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    /// Écrit un petit dataset (age num avec un missing, name char) dans WORK.
    fn write_class(session: &Session, table: &str) {
        let df = df!(
            "Age" => [Some(14.0), None, Some(13.0)],
            "Name" => ["Alfred", "Alice", "Barbara"],
        )
        .unwrap();
        let vars = vec![
            VarMeta {
                name: "Age".into(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            },
            VarMeta {
                name: "Name".into(),
                ty: VarType::Char,
                length: 7,
                format: None,
                label: None,
            },
        ];
        session
            .libs
            .get("WORK")
            .unwrap()
            .write(table, &SasDataset { df, vars })
            .unwrap();
    }

    fn parse_step(src: &str) -> DataStepAst {
        let file = SourceFile::new(src);
        let mut ts = StatementStream::new(&file).unwrap();
        assert!(ts.next().is_kw("data"));
        crate::parser::datastep::parse_data_step(&mut ts).unwrap()
    }

    fn compile_src(src: &str, session: &mut Session) -> Result<StepProgram> {
        compile(&parse_step(src), session)
    }

    #[test]
    fn set_brings_input_vars_in_dataset_order() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src("data out; set inp; x = age + 1; run;", &mut s).unwrap();

        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["Age", "Name", "x"]);
        assert!(prog.pdv.vars()[0].from_input);
        assert!(prog.pdv.vars()[1].from_input);
        assert!(!prog.pdv.vars()[2].from_input);
        assert_eq!(prog.pdv.vars()[1].ty, VarType::Char);
        assert_eq!(prog.pdv.vars()[1].length, 7);

        let input = prog.input.as_ref().unwrap();
        assert!(input.by.is_empty());
        let ds0 = &input.datasets[0];
        assert_eq!(ds0.n_rows, 3);
        assert_eq!(ds0.display, "WORK.INP");
        assert_eq!(ds0.var_slots, vec![0, 1]);
        // Colonnes décodées en Value, missing `.` inclus.
        assert_eq!(ds0.columns[0][0], Value::Num(14.0));
        assert_eq!(ds0.columns[0][1], Value::missing());
        assert_eq!(ds0.columns[1][2], Value::Char("Barbara".into()));

        // Implicit output : pas de OUTPUT explicite.
        assert!(!prog.has_explicit_output);
        assert!(prog.uninitialized.is_empty());
        assert_eq!(prog.outputs.len(), 1);
        assert_eq!(prog.outputs[0].display, "WORK.OUT");
        assert_eq!(prog.outputs[0].kept_slots, vec![0, 1, 2]);
    }

    #[test]
    fn first_reference_order_without_set() {
        let mut s = session();
        let prog = compile_src("data o; x = y; z = 'abc'; run;", &mut s).unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        // Cible avant les variables de l'expression, ordre textuel.
        assert_eq!(names, vec!["x", "y", "z"]);
        // x inféré Num (y inconnue au moment de l'inférence).
        assert_eq!(prog.pdv.vars()[0].ty, VarType::Num);
        // z : Char(3) du littéral.
        assert_eq!(prog.pdv.vars()[2].ty, VarType::Char);
        assert_eq!(prog.pdv.vars()[2].length, 3);
        // y : référencée jamais assignée → uninitialized.
        assert_eq!(prog.uninitialized, vec!["y".to_string()]);
    }

    #[test]
    fn first_assignment_freezes_type_and_length() {
        let mut s = session();
        let prog = compile_src("data o; s = 'ab'; s = 'abcdef'; run;", &mut s).unwrap();
        assert_eq!(prog.pdv.vars()[0].ty, VarType::Char);
        // La première assignation fige la longueur à 2.
        assert_eq!(prog.pdv.vars()[0].length, 2);
    }

    #[test]
    fn concat_length_is_sum_with_num_as_12() {
        let mut s = session();
        let prog = compile_src("data o; c = 'ab' || 'cde'; d = c || x; run;", &mut s).unwrap();
        // c = 2 + 3.
        assert_eq!(prog.pdv.vars()[0].length, 5);
        assert_eq!(prog.pdv.vars()[0].ty, VarType::Char);
        // d = len(c) + 12 (x numérique).
        let d = &prog.pdv.vars()[1];
        assert_eq!(d.name, "d");
        assert_eq!(d.length, 5 + 12);
    }

    #[test]
    fn call_inference_table() {
        let mut s = session();
        let prog = compile_src(
            "data o; a = 'xyz'; u = upcase(a); t = cats(a, a); n = sum(1, 2); run;",
            &mut s,
        )
        .unwrap();
        let var = |n: &str| {
            let slot = prog.pdv.slot(n).unwrap();
            &prog.pdv.vars()[slot]
        };
        assert_eq!((var("u").ty, var("u").length), (VarType::Char, 3));
        assert_eq!((var("t").ty, var("t").length), (VarType::Char, 200));
        assert_eq!(var("n").ty, VarType::Num);
    }

    #[test]
    fn keep_drop_interaction() {
        let mut s = session();
        let prog = compile_src(
            "data o; x = 1; y = 2; z = 3; keep x y; drop y; run;",
            &mut s,
        )
        .unwrap();
        // keep {x,y} puis drop y → x seul.
        assert_eq!(prog.outputs[0].kept_slots, vec![0]);
        // Le WARNING de l'intersection est dans le log.
        let log = s.log.into_string();
        assert!(log.contains("WARNING"), "log was: {log}");
        assert!(log.contains("KEEP and DROP"), "log was: {log}");
    }

    #[test]
    fn keep_unknown_variable_errors() {
        let mut s = session();
        let err = compile_src("data o; x = 1; keep nosuch; run;", &mut s).err().unwrap();
        assert!(
            err.to_string()
                .contains("in the DROP, KEEP, or RENAME list has never been referenced"),
            "got: {err}"
        );
    }

    #[test]
    fn data_null_has_no_outputs() {
        let mut s = session();
        let prog = compile_src("data _null_; x = 1; run;", &mut s).unwrap();
        assert!(prog.outputs.is_empty());
    }

    #[test]
    fn set_missing_table_errors() {
        let mut s = session();
        let err = compile_src("data o; set nosuch; run;", &mut s).err().unwrap();
        assert_eq!(err.to_string(), "File WORK.NOSUCH.DATA does not exist.");
    }

    #[test]
    fn second_set_errors_m1() {
        let mut s = session();
        write_class(&s, "a");
        write_class(&s, "b");
        let err = compile_src("data o; set a; set b; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("not yet implemented"));
    }

    #[test]
    fn explicit_output_detected_inside_if() {
        let mut s = session();
        let prog = compile_src(
            "data o; x = 1; if x then do; output; end; run;",
            &mut s,
        )
        .unwrap();
        assert!(prog.has_explicit_output);
    }

    #[test]
    fn multiple_outputs_share_kept_slots() {
        let mut s = session();
        let prog = compile_src("data a b; x = 1; y = 2; drop y; run;", &mut s).unwrap();
        assert_eq!(prog.outputs.len(), 2);
        assert_eq!(prog.outputs[0].kept_slots, vec![0]);
        assert_eq!(prog.outputs[1].kept_slots, vec![0]);
        assert_eq!(prog.outputs[0].libref, "WORK");
        assert_eq!(prog.outputs[1].display, "WORK.B");
    }

    #[test]
    fn assign_before_set_still_marks_from_input() {
        let mut s = session();
        write_class(&s, "inp");
        // `age` référencée avant le SET : elle doit malgré tout être
        // marquée from_input (pas de reset à chaque itération).
        let prog = compile_src("data o; age = 0; set inp; run;", &mut s).unwrap();
        let slot = prog.pdv.slot("age").unwrap();
        assert!(prog.pdv.vars()[slot].from_input);
        // Et l'ordre de première référence place age en tête.
        assert_eq!(prog.pdv.vars()[0].name, "age");
    }

    // ── RETAIN (M2) ──────────────────────────────────────────────────────

    #[test]
    fn retain_with_init_creates_retained_var_with_initial_value() {
        let mut s = session();
        let prog = compile_src("data o; retain x 5 s 'ab'; y = 1; run;", &mut s).unwrap();
        // Ordre de première référence : x et s entrent au RETAIN, avant y.
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["x", "s", "y"]);
        assert!(prog.pdv.vars()[0].retained);
        assert!(prog.pdv.vars()[1].retained);
        assert!(!prog.pdv.vars()[2].retained);
        // Types/longueurs des littéraux.
        assert_eq!(prog.pdv.vars()[0].ty, VarType::Num);
        assert_eq!(prog.pdv.vars()[1].ty, VarType::Char);
        assert_eq!(prog.pdv.vars()[1].length, 2);
        // Valeurs initiales.
        assert_eq!(
            prog.initial_values,
            vec![(0, Value::Num(5.0)), (1, Value::Char("ab".into()))]
        );
        // RETAIN avec init = initialisée : pas de NOTE uninitialized.
        assert!(prog.uninitialized.is_empty());
    }

    #[test]
    fn retain_without_init_flags_later_reference_or_creates_num() {
        let mut s = session();
        // k : retenue sans init, type figé par l'assignation ultérieure.
        // j : retenue sans init, jamais référencée → Num + uninitialized,
        // créée en FIN d'ordre PDV (simplification M2 documentée).
        let prog = compile_src("data o; retain k j; x = 1; k = 'ab'; run;", &mut s).unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["x", "k", "j"]);
        let var = |n: &str| &prog.pdv.vars()[prog.pdv.slot(n).unwrap()];
        assert!(var("k").retained);
        assert_eq!(var("k").ty, VarType::Char);
        assert_eq!(var("k").length, 2);
        assert!(var("j").retained);
        assert_eq!(var("j").ty, VarType::Num);
        assert!(!var("x").retained);
        assert_eq!(prog.uninitialized, vec!["j".to_string()]);
        // Pas de valeur initiale sans init.
        assert!(prog.initial_values.is_empty());
    }

    #[test]
    fn retain_bare_retains_whole_pdv() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src("data o; set inp; retain; x = 1; run;", &mut s).unwrap();
        assert!(prog.pdv.vars().iter().all(|v| v.retained));
    }

    // ── Sum statement (M2) ───────────────────────────────────────────────

    #[test]
    fn sum_statement_compiles_retained_num_with_initial_zero() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src("data o; set inp; total + age; run;", &mut s).unwrap();
        let slot = prog.pdv.slot("total").unwrap();
        let v = &prog.pdv.vars()[slot];
        assert_eq!(v.ty, VarType::Num);
        assert_eq!(v.length, 8);
        assert!(v.retained);
        assert_eq!(prog.initial_values, vec![(slot, Value::Num(0.0))]);
        // La cible d'un sum statement compte comme initialisée.
        assert!(prog.uninitialized.is_empty());
    }

    #[test]
    fn retain_init_wins_over_sum_zero_in_both_orders() {
        let mut s = session();
        // RETAIN d'abord : le sum statement ne pousse pas son 0.
        let prog = compile_src("data o; retain n 100; n + 1; run;", &mut s).unwrap();
        let slot = prog.pdv.slot("n").unwrap();
        assert_eq!(prog.initial_values, vec![(slot, Value::Num(100.0))]);
        // Sum d'abord : les deux entrées coexistent, le RETAIN (appliqué en
        // dernier par l'exécuteur) gagne.
        let prog = compile_src("data o; n + 1; retain n 100; run;", &mut s).unwrap();
        let slot = prog.pdv.slot("n").unwrap();
        assert_eq!(
            prog.initial_values,
            vec![(slot, Value::Num(0.0)), (slot, Value::Num(100.0))]
        );
    }

    // ── LENGTH (M2) ──────────────────────────────────────────────────────

    #[test]
    fn length_before_first_reference_fixes_type_and_length() {
        let mut s = session();
        let prog = compile_src(
            "data o; length c $ 3 n 4; c = 'abcdef'; n = 1; run;",
            &mut s,
        )
        .unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["c", "n"]);
        let c = &prog.pdv.vars()[0];
        assert_eq!(c.ty, VarType::Char);
        // La longueur du LENGTH gagne sur celle du littéral assigné.
        assert_eq!(c.length, 3);
        let n = &prog.pdv.vars()[1];
        assert_eq!(n.ty, VarType::Num);
        // Pour une numérique, la longueur est une métadonnée (stockage f64).
        assert_eq!(n.length, 4);
        assert!(prog.uninitialized.is_empty());
    }

    #[test]
    fn length_after_reference_warns_for_differing_char() {
        let mut s = session();
        let prog = compile_src("data o; c = 'ab'; length c $ 10; run;", &mut s).unwrap();
        // La longueur reste figée par la première référence.
        assert_eq!(prog.pdv.vars()[0].length, 2);
        let log = s.log.into_string();
        assert!(
            log.contains("WARNING: Length of character variable c has already been set."),
            "log was: {log}"
        );
    }

    #[test]
    fn length_after_reference_is_silent_for_num_and_same_char_length() {
        let mut s = session();
        let prog = compile_src(
            "data o; x = 1; length x 5; c = 'ab'; length c $ 2; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(prog.pdv.vars()[0].length, 8);
        let log = s.log.into_string();
        assert!(!log.contains("WARNING"), "log was: {log}");
    }

    #[test]
    fn length_out_of_range_errors() {
        let mut s = session();
        let err = compile_src("data o; length n 9; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("out of range (3-8)"), "got: {err}");
        let err = compile_src("data o; length c $ 40000; run;", &mut s)
            .err()
            .unwrap();
        assert!(
            err.to_string().contains("out of range (1-32767)"),
            "got: {err}"
        );
        let err = compile_src("data o; length n 2; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("out of range"), "got: {err}");
    }

    // ── DO itératif / DELETE (M2) ────────────────────────────────────────

    #[test]
    fn do_loop_index_enters_pdv_not_retained_and_assigned() {
        let mut s = session();
        let prog = compile_src("data o; do i = 1 to 3; x = i; end; run;", &mut s).unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        // L'index entre au point du DO, avant les variables du corps.
        assert_eq!(names, vec!["i", "x"]);
        let i = &prog.pdv.vars()[0];
        assert_eq!(i.ty, VarType::Num);
        assert_eq!(i.length, 8);
        assert!(!i.retained);
        // L'index compte comme assigné : pas de NOTE uninitialized.
        assert!(prog.uninitialized.is_empty());
    }

    #[test]
    fn do_loop_bound_and_condition_vars_enter_pdv() {
        let mut s = session();
        let prog = compile_src(
            "data o; do i = a to b by c while(w) until(u); end; run;",
            &mut s,
        )
        .unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        // Index d'abord, puis from/to/by/while/until en ordre textuel.
        assert_eq!(names, vec!["i", "a", "b", "c", "w", "u"]);
        // Les bornes sont référencées jamais assignées → uninitialized.
        assert_eq!(
            prog.uninitialized,
            vec!["a", "b", "c", "w", "u"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn delete_compiles_and_output_in_do_body_is_detected() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src(
            "data o; set inp; if age = . then delete; do i = 1 to 2; output; end; run;",
            &mut s,
        )
        .unwrap();
        assert!(prog.has_explicit_output);
        assert!(prog.pdv.slot("i").is_some());
    }

    // ── ARRAY (M2, lot 3) ────────────────────────────────────────────────

    #[test]
    fn array_elements_enter_pdv_in_order_and_registry_is_filled() {
        let mut s = session();
        let prog = compile_src("data o; array a{3} x y z; b = 1; run;", &mut s).unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        // Les éléments entrent au PDV au point de l'ARRAY, avant b.
        assert_eq!(names, vec!["x", "y", "z", "b"]);
        assert_eq!(prog.arrays.get("A").map(|d| &d.slots), Some(&vec![0, 1, 2]));
        assert_eq!(prog.pdv.vars()[0].ty, VarType::Num);
        // Le nom de l'array n'est PAS une variable du PDV.
        assert!(prog.pdv.slot("a").is_none());
    }

    #[test]
    fn array_star_size_deduced_and_char_length_applied() {
        let mut s = session();
        let prog = compile_src("data o; array c{*} $ 5 c1 c2; run;", &mut s).unwrap();
        assert_eq!(prog.arrays.get("C").map(|d| &d.slots), Some(&vec![0, 1]));
        for v in prog.pdv.vars() {
            assert_eq!(v.ty, VarType::Char);
            assert_eq!(v.length, 5);
        }
    }

    #[test]
    fn array_auto_named_elements() {
        let mut s = session();
        let prog = compile_src("data o; array a{3}; a{1} = 1; run;", &mut s).unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["a1", "a2", "a3"]);
        assert_eq!(prog.arrays.get("A").map(|d| &d.slots), Some(&vec![0, 1, 2]));
    }

    #[test]
    fn array_size_mismatch_errors() {
        let mut s = session();
        let err = compile_src("data o; array a{3} x y; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("does not match"), "got: {err}");
        let err = compile_src("data o; array a{2} x y z; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("does not match"), "got: {err}");
    }

    #[test]
    fn array_star_without_vars_errors() {
        let mut s = session();
        let err = compile_src("data o; array a{*}; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("zero elements"), "got: {err}");
    }

    #[test]
    fn array_redeclaration_errors() {
        let mut s = session();
        let err = compile_src("data o; array a{2} x y; array a{2} u v; run;", &mut s)
            .err()
            .unwrap();
        assert!(
            err.to_string().contains("already been defined"),
            "got: {err}"
        );
    }

    #[test]
    fn undeclared_array_lvalue_errors() {
        let mut s = session();
        // Forme accolades.
        let err = compile_src("data o; nosuch{1} = 0; run;", &mut s).err().unwrap();
        assert!(
            err.to_string().contains("Undeclared array referenced"),
            "got: {err}"
        );
        // Forme parenthèses : validée array à la COMPILATION.
        let err = compile_src("data o; nosuch(1) = 0; run;", &mut s).err().unwrap();
        assert!(
            err.to_string().contains("Undeclared array referenced"),
            "got: {err}"
        );
    }

    #[test]
    fn undeclared_array_rvalue_errors() {
        let mut s = session();
        let err = compile_src("data o; x = nosuch{1}; run;", &mut s).err().unwrap();
        assert!(
            err.to_string().contains("Undeclared array referenced"),
            "got: {err}"
        );
    }

    #[test]
    fn dim_of_array_does_not_create_variable() {
        let mut s = session();
        let prog = compile_src("data o; array a{3} x y z; n = dim(a); run;", &mut s).unwrap();
        // Pas de variable `a` au PDV, et n est bien là.
        assert!(prog.pdv.slot("a").is_none());
        assert!(prog.pdv.slot("n").is_some());
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["x", "y", "z", "n"]);
    }

    #[test]
    fn array_indexed_assignment_marks_elements_initialized() {
        let mut s = session();
        let prog =
            compile_src("data o; array a{3} x y z; do i = 1 to 3; a{i} = i; end; run;", &mut s)
                .unwrap();
        // x, y, z assignés via l'indice : pas de NOTE uninitialized.
        assert!(prog.uninitialized.is_empty());
    }

    #[test]
    fn bare_array_name_reference_errors() {
        let mut s = session();
        // Un nom d'array n'est pas une variable : référence nue illégale.
        let err = compile_src("data o; array a{2} x y; z = a; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("Illegal reference"), "got: {err}");
        let err = compile_src("data o; array a{2} x y; a = 1; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("Illegal reference"), "got: {err}");
    }

    #[test]
    fn array_indexed_rvalue_infers_element_type() {
        let mut s = session();
        let prog = compile_src(
            "data o; array c{2} $ 4 u v; s = c{1}; t = c(2); n = a(1); array a{2} p q; run;",
            &mut s,
        );
        // `a` est déclaré APRÈS son usage en forme parenthèses : Call normal
        // (fonction inconnue à l'évaluation) — la compilation passe et
        // infère Num. On vérifie surtout s et t.
        let prog = prog.unwrap();
        let var = |n: &str| &prog.pdv.vars()[prog.pdv.slot(n).unwrap()];
        assert_eq!((var("s").ty, var("s").length), (VarType::Char, 4));
        assert_eq!((var("t").ty, var("t").length), (VarType::Char, 4));
        assert_eq!(var("n").ty, VarType::Num);
    }

    #[test]
    fn put_width_parsing() {
        assert_eq!(put_width(&[Expr::Num(1.0), Expr::Var("best12".into())]), 12);
        assert_eq!(put_width(&[Expr::Num(1.0), Expr::Str("date9.".into())]), 9);
        assert_eq!(put_width(&[Expr::Num(1.0), Expr::Var("words".into())]), 200);
        assert_eq!(put_width(&[Expr::Num(1.0)]), 200);
        // Forme w.d : la largeur est `w`, pas le nombre de décimales.
        assert_eq!(put_width(&[Expr::Num(1.0), Expr::Str("dollar10.2".into())]), 10);
        assert_eq!(put_width(&[Expr::Num(1.0), Expr::Str("percent8.1".into())]), 8);
        assert_eq!(put_width(&[Expr::Num(1.0), Expr::Str("comma12.".into())]), 12);
    }

    // ── Options de dataset + OUTPUT ciblé (M2, lot 4) ────────────────────

    #[test]
    fn input_keep_filters_pdv() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src("data o; set inp(keep=name); run;", &mut s).unwrap();
        // Seule Name entre au PDV (Age filtrée AVANT le PDV).
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["Name"]);
        let input = &prog.input.as_ref().unwrap().datasets[0];
        assert_eq!(input.columns.len(), 1);
        assert_eq!(input.var_slots, vec![0]);
        assert_eq!(prog.outputs[0].kept_slots, vec![0]);
    }

    #[test]
    fn input_drop_filters_pdv() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src("data o; set inp(drop=age); run;", &mut s).unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["Name"]);
    }

    #[test]
    fn input_rename_renames_pdv_slot() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src("data o; set inp(rename=(age=years)); x = years; run;", &mut s)
            .unwrap();
        assert!(prog.pdv.slot("years").is_some());
        assert!(prog.pdv.slot("age").is_none());
        // Le slot renommé reste from_input (pas de reset par itération).
        let slot = prog.pdv.slot("years").unwrap();
        assert!(prog.pdv.vars()[slot].from_input);
    }

    #[test]
    fn input_where_is_stored_not_filtered_at_compile() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src("data o; set inp(where=(age > 13)); run;", &mut s).unwrap();
        let input = &prog.input.as_ref().unwrap().datasets[0];
        // Pas de filtrage à la compilation : toutes les lignes présentes.
        assert_eq!(input.n_rows, 3);
        assert!(input.where_.is_some());
    }

    #[test]
    fn input_where_unknown_variable_errors() {
        let mut s = session();
        write_class(&s, "inp");
        let err = compile_src("data o; set inp(where=(nosuch > 1)); run;", &mut s)
            .err()
            .unwrap();
        assert!(
            err.to_string().contains("Variable nosuch is not on file WORK.INP."),
            "got: {err}"
        );
    }

    #[test]
    fn output_option_keep_drop_combined_with_statements() {
        let mut s = session();
        // PDV : x y z. Statement keep x y ; option drop=y → x seul.
        let prog = compile_src(
            "data o(drop=y); x = 1; y = 2; z = 3; keep x y; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(prog.outputs[0].kept_slots, vec![0]);
        assert_eq!(prog.outputs[0].out_names, vec!["x".to_string()]);
    }

    #[test]
    fn output_option_keep_is_per_output() {
        let mut s = session();
        let prog = compile_src("data a(keep=x) b; x = 1; y = 2; run;", &mut s).unwrap();
        assert_eq!(prog.outputs[0].kept_slots, vec![0]);
        assert_eq!(prog.outputs[1].kept_slots, vec![0, 1]);
    }

    #[test]
    fn output_rename_changes_written_name_not_slot() {
        let mut s = session();
        let prog = compile_src("data o(rename=(x=xx)); x = 1; y = 2; run;", &mut s).unwrap();
        // Le slot PDV garde son nom ; seul le nom d'écriture change.
        assert_eq!(prog.pdv.vars()[0].name, "x");
        assert_eq!(
            prog.outputs[0].out_names,
            vec!["xx".to_string(), "y".to_string()]
        );
        assert_eq!(prog.outputs[0].kept_slots, vec![0, 1]);
    }

    #[test]
    fn where_on_output_dataset_errors() {
        let mut s = session();
        let err = compile_src("data o(where=(x > 1)); x = 1; run;", &mut s)
            .err()
            .unwrap();
        assert_eq!(
            err.to_string(),
            "WHERE= is not a valid data set option for output data sets."
        );
    }

    #[test]
    fn targeted_output_unknown_dataset_errors() {
        let mut s = session();
        let err = compile_src("data a b; x = 1; output c; run;", &mut s)
            .err()
            .unwrap();
        assert_eq!(
            err.to_string(),
            "Output dataset WORK.C is not in the DATA statement output list."
        );
    }

    #[test]
    fn targeted_output_known_dataset_compiles() {
        let mut s = session();
        let prog = compile_src("data a b; x = 1; output a; output a b; run;", &mut s).unwrap();
        assert!(prog.has_explicit_output);
    }

    #[test]
    fn option_variable_never_referenced_errors() {
        let mut s = session();
        write_class(&s, "inp");
        // En entrée : keep= d'une variable absente de l'input.
        let err = compile_src("data o; set inp(keep=nosuch); run;", &mut s)
            .err()
            .unwrap();
        assert!(
            err.to_string()
                .contains("in the DROP, KEEP, or RENAME list has never been referenced"),
            "got: {err}"
        );
        // En entrée : rename= d'une variable absente.
        let err = compile_src("data o; set inp(rename=(nosuch=x)); run;", &mut s)
            .err()
            .unwrap();
        assert!(
            err.to_string().contains("has never been referenced"),
            "got: {err}"
        );
        // En sortie : drop= d'une variable absente du PDV.
        let err = compile_src("data o(drop=nosuch); x = 1; run;", &mut s)
            .err()
            .unwrap();
        assert!(
            err.to_string().contains("has never been referenced"),
            "got: {err}"
        );
    }

    // ── SET multi-datasets + BY + FIRST./LAST. (M3) ──────────────────────

    /// Petit dataset numérique (Age, Weight) pour les unions de variables.
    fn write_weights(session: &Session, table: &str) {
        let df = df!(
            "Age" => [11.0, 12.0],
            "Weight" => [50.0, 60.0],
        )
        .unwrap();
        let vars = vec![
            VarMeta {
                name: "Age".into(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            },
            VarMeta {
                name: "Weight".into(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            },
        ];
        session
            .libs
            .get("WORK")
            .unwrap()
            .write(table, &SasDataset { df, vars })
            .unwrap();
    }

    #[test]
    fn set_two_datasets_union_of_variables_in_first_appearance_order() {
        let mut s = session();
        write_class(&s, "a"); // Age, Name
        write_weights(&s, "b"); // Age, Weight
        let prog = compile_src("data o; set a b; run;", &mut s).unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        // Variables de a, puis les NOUVELLES de b.
        assert_eq!(names, vec!["Age", "Name", "Weight"]);
        assert!(prog.pdv.vars().iter().all(|v| v.from_input));
        let input = prog.input.as_ref().unwrap();
        assert_eq!(input.datasets.len(), 2);
        assert!(input.by.is_empty());
        // Age de b pointe le slot partagé 0.
        assert_eq!(input.datasets[1].var_slots, vec![0, 2]);
    }

    #[test]
    fn first_last_have_no_pdv_slot_and_by_is_resolved() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src(
            "data o; set inp; by age; f = first.age; l = last.age; run;",
            &mut s,
        )
        .unwrap();
        // Pas de slot PDV pour FIRST./LAST. (comme _N_/_ERROR_) : ni
        // colonne de sortie ni NOTE uninitialized.
        assert!(prog.pdv.slot("FIRST.AGE").is_none());
        assert!(prog.pdv.slot("LAST.AGE").is_none());
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["Age", "Name", "f", "l"]);
        assert!(prog.uninitialized.is_empty());
        let input = prog.input.as_ref().unwrap();
        assert_eq!(input.by.len(), 1);
        assert_eq!(input.by[0].name, "AGE");
        assert_eq!(input.by[0].slot, 0);
        assert!(!input.by[0].descending);
        assert_eq!(input.datasets[0].by_cols, vec![0]);
    }

    #[test]
    fn incompatible_types_across_set_datasets_error() {
        let mut s = session();
        write_class(&s, "a"); // Age numérique
        // Dataset où Age est CARACTÈRE.
        let df = df!("Age" => ["x", "y"]).unwrap();
        let vars = vec![VarMeta {
            name: "Age".into(),
            ty: VarType::Char,
            length: 1,
            format: None,
            label: None,
        }];
        s.libs
            .get("WORK")
            .unwrap()
            .write("cage", &SasDataset { df, vars })
            .unwrap();
        let err = compile_src("data o; set a cage; run;", &mut s).err().unwrap();
        assert_eq!(
            err.to_string(),
            "Variable Age has been defined as both character and numeric."
        );
    }

    #[test]
    fn by_without_set_errors() {
        let mut s = session();
        let err = compile_src("data o; by x; x = 1; run;", &mut s).err().unwrap();
        assert_eq!(
            err.to_string(),
            "No SET, MERGE, UPDATE, or MODIFY statement."
        );
    }

    #[test]
    fn by_variable_missing_from_one_dataset_errors() {
        let mut s = session();
        write_class(&s, "a"); // Age, Name
        write_weights(&s, "b"); // Age, Weight (pas de Name)
        let err = compile_src("data o; set a b; by name; run;", &mut s)
            .err()
            .unwrap();
        assert_eq!(
            err.to_string(),
            "BY variable NAME is not on input data set WORK.B."
        );
        // Variable BY absente de TOUS les inputs.
        let err = compile_src("data o; set a; by nosuch; run;", &mut s)
            .err()
            .unwrap();
        assert!(
            err.to_string().contains("BY variable nosuch is not on input data set"),
            "got: {err}"
        );
    }

    #[test]
    fn first_last_on_non_by_variable_errors() {
        let mut s = session();
        write_class(&s, "inp");
        // Pas de BY du tout.
        let err = compile_src("data o; set inp; f = first.age; run;", &mut s)
            .err()
            .unwrap();
        assert!(
            err.to_string().contains("AGE is not a BY variable"),
            "got: {err}"
        );
        // BY présent mais sur une autre variable.
        let err = compile_src("data o; set inp; by age; f = last.name; run;", &mut s)
            .err()
            .unwrap();
        assert!(
            err.to_string().contains("NAME is not a BY variable"),
            "got: {err}"
        );
    }

    #[test]
    fn renamed_but_dropped_input_variable_is_ignored() {
        let mut s = session();
        write_class(&s, "inp");
        // age est dropée : le rename la concernant est ignoré (pas d'erreur,
        // pas de variable years).
        let prog = compile_src("data o; set inp(drop=age rename=(age=years)); run;", &mut s)
            .unwrap();
        assert!(prog.pdv.slot("years").is_none());
        assert!(prog.pdv.slot("age").is_none());
        assert!(prog.pdv.slot("name").is_some());
    }
}
