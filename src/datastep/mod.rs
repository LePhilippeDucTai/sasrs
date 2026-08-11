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
//! - Plusieurs statements SET par étape (M40.2) : chaque statement est un
//!   SITE DE LECTURE indépendant (site 0 = `input`, suivants =
//!   `extra_inputs` ; BY/POINT= refusés avec plusieurs sites). Un SET peut
//!   lister PLUSIEURS datasets (M3) : le PDV reçoit l'UNION de leurs
//!   variables en ordre de première apparition ; une variable présente
//!   avec des types incompatibles → ERROR "Variable X has been defined as
//!   both character and numeric.".
//!   Le statement BY est résolu en fin de compilation (`build_input`) :
//!   chaque clé doit exister dans CHAQUE dataset du SET, et toute
//!   référence FIRST.x/LAST.x exige que x soit une clé BY. FIRST./LAST.
//!   ne créent jamais de slot PDV (comme _N_/_ERROR_).

mod program;

pub use program::StepProgram;
pub use program::compile;

mod types;

mod array;

mod hash;

mod helpers;

pub use array::ArrayDef;

pub use hash::HashIter;

pub use hash::HashObject;

pub use hash::hash_key;

pub use types::ByVar;

pub use types::InputData;

pub use types::InputDataset;

pub use types::ModifyData;

pub use types::OutputSpec;

pub use types::TextInput;

pub use types::TextOptions;

pub use types::UpdateData;

use helpers::*;

pub mod eval;

pub mod exec;

pub mod fastpath;

pub mod functions;

pub mod pdv;

use crate::ast::{
    AttribItem, BinaryOp, DataStepAst, DatasetOptions, DatasetRef, DatasetSpec, DoListItem, DsStmt,
    Expr, LengthSpec, SetOptions, WhenClause,
};

use crate::error::{Result, SasError};

use crate::missing::num_to_value;

use crate::session::Session;

use crate::value::{Value, VarType};

use pdv::{Pdv, PdvVar};

use std::collections::{HashMap, HashSet};

/// Spécification de lecture d'UNE variable d'un statement INPUT.
///
/// MQ9.5 — ces cinq champs étaient dépliés en autant de paramètres positionnels
/// de `Runner::read_one_var`, qui en comptait onze (dont trois `bool`
/// consécutifs : `is_char`, `list_modifier`, `dsd`). Les regrouper rend
/// l'inversion de deux booléens impossible.
#[derive(Debug, Clone)]
pub struct InputVarSpec {
    /// Slot PDV cible.
    pub slot: usize,
    /// Type cible : caractère ou numérique.
    pub is_char: bool,
    /// Colonnes 1-based inclusives (mode colonne).
    pub cols: Option<(usize, usize)>,
    /// `FormatSpec` du mode formaté.
    pub informat: Option<crate::formats::FormatSpec>,
    /// Informat appliqué en mode liste.
    pub list_modifier: bool,
}

/// Item INPUT compilé (M14) : un item AST dont les noms de variable sont
/// résolus en slots PDV et les informats en `FormatSpec`.
#[derive(Clone)]
pub enum InputAction {
    /// Lire une variable (cf. [`InputVarSpec`]).
    Var(InputVarSpec),
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

#[derive(Clone, Copy, PartialEq)]
pub enum ShortMode {
    Default,
    Missover,
    Truncover,
    Stopover,
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
    /// Étiquettes de statement définies dans l'étape (M16.6 : `name: stmt`),
    /// en MAJUSCULES. Une étiquette dupliquée → erreur de compilation.
    labels_defined: HashSet<String>,
    /// Références d'étiquette des GOTO/LINK (M16.6), en MAJUSCULES. Validées en
    /// fin de compilation contre `labels_defined` (étiquette inconnue → erreur).
    goto_link_refs: Vec<String>,
    /// Objets hash déclarés (M17.1) : nom UPPERCASE → objet initial (options
    /// résolues). Un DECLARE HASH y enregistre l'objet ; un appel de méthode
    /// le référence (objet inconnu → erreur de compilation).
    hash_objects: HashMap<String, HashObject>,
    /// Itérateurs de hash déclarés (M17.2) : nom UPPERCASE → itérateur.
    hash_iters: HashMap<String, HashIter>,
    /// M40.2 — sites SET SUPPLÉMENTAIRES (2ᵉ, 3ᵉ… statements SET de
    /// l'étape) : datasets matérialisés + options de niveau statement, par
    /// site (index = site − 1 ; le site 0 reste `input_datasets` /
    /// `set_options`). Résolus en `InputData` dans `build_input`.
    extra_set_sites: Vec<(Vec<InputDataset>, crate::ast::SetOptions)>,
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

mod compile_io;

mod compile_control;

mod compile_decl;

mod compile_array;

mod compile_hash;

mod build;

mod infer;

impl Compiler<'_> {
    fn walk_stmt(&mut self, stmt: &DsStmt) -> Result<()> {
        match stmt {
            DsStmt::Set {
                specs,
                options,
                site,
            } => self.compile_set_stmt(specs, options, *site),
            // MERGE (M3) : comme SET multi-datasets mais en match-merge par
            // BY. Chaque dataset peut porter une option `in=`. Un SET/MERGE
            // a déjà été vu → erreur.
            DsStmt::Merge(specs) => self.compile_merge(specs),
            // UPDATE (M16.5) : maître + transaction, fusion par KEY=. Comme
            // SET/MERGE, exclusif (un seul SET/MERGE/UPDATE/MODIFY par étape).
            DsStmt::Update {
                master,
                master_where,
                transaction,
                key_vars,
            } => self.compile_update(master, master_where, transaction, key_vars),
            // MODIFY (M16.5) : un dataset, modification EN PLACE.
            DsStmt::Modify {
                dataset,
                key_vars,
                point,
                nobs,
            } => self.compile_modify(dataset, key_vars, point, nobs),
            // BY : purement déclaratif ici ; résolu en fin de compilation
            // (`build_input`). Les variables BY doivent venir des inputs —
            // on ne crée donc AUCUN slot ici.
            DsStmt::By(items) => {
                self.by = Some(items.clone());
                Ok(())
            }
            DsStmt::Assign { var, expr } => self.compile_assign(var, expr),
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
            } => self.compile_do_loop(index, to, by, while_, until, body),
            // DO sur liste de valeurs (M16.3) : l'index entre au PDV (Num 8,
            // assigné — pas de NOTE "uninitialized"). Le type est déduit des
            // valeurs ? SAS : numérique sauf si TOUTES les valeurs explicites
            // sont des chaînes → caractère. On infère le type/longueur de la
            // 1re valeur (suffisant pour les cas usuels).
            DsStmt::DoList { index, items, body } => self.compile_do_list(index, items, body),
            // DO OVER (M16.3) : itération implicite sur un array. L'array doit
            // être déclaré ; pendant le corps, une référence nue au nom de
            // l'array désigne l'élément courant (autorisée en lecture comme en
            // écriture). On installe le nom dans `do_over_arrays` le temps de
            // walker le corps.
            DsStmt::DoOver { array, body } => self.compile_do_over(array, body),
            // SELECT (M16.1) : vérifie les références de variables du
            // sélecteur, de chaque valeur/condition de WHEN, et des corps
            // (WHEN + OTHERWISE), en ordre textuel.
            DsStmt::Select {
                selector,
                whens,
                otherwise,
            } => self.compile_select(selector, whens, otherwise),
            // DELETE : purement exécutif, rien à compiler.
            DsStmt::Delete => Ok(()),
            DsStmt::Output(targets) => self.compile_output(targets),
            DsStmt::Keep(names) => {
                self.keeps.extend(names.iter().cloned());
                Ok(())
            }
            DsStmt::Drop(names) => {
                self.drops.extend(names.iter().cloned());
                Ok(())
            }
            DsStmt::Stop => Ok(()),
            DsStmt::Retain(items) => self.compile_retain(items),
            DsStmt::Sum { var, expr } => self.compile_sum(var, expr),
            DsStmt::Array {
                name,
                dims,
                char_len,
                vars,
                initial,
                temporary,
                special,
            } => self.compile_array(
                name,
                dims.as_deref(),
                *char_len,
                vars,
                initial,
                *temporary,
                *special,
            ),
            DsStmt::AssignIndexed {
                array,
                indices,
                expr,
            } => self.compile_assign_indexed(array, indices, expr),
            DsStmt::Length(items) => self.compile_length(items),
            // FORMAT/LABEL/ATTRIB : déclarations de compilation. Le format
            // (validé via FormatSpec::parse) et le libellé sont mémorisés
            // dans des maps appliquées en fin de compilation (l'ordre
            // déclaration/référence n'importe donc pas). Une variable
            // inconnue est ignorée (SIMPLIFICATION M4 documentée : en vrai
            // SAS la variable serait créée sur le PDV).
            DsStmt::Format(groups) => self.compile_format(groups),
            DsStmt::Label(pairs) => {
                for (name, label) in pairs {
                    self.labels.insert(name.to_uppercase(), label.clone());
                }
                Ok(())
            }
            DsStmt::Attrib(items) => self.compile_attrib(items),
            // `call <name>(args);` (M11.5) : les arguments sont des
            // expressions rvalue ordinaires (la routine ne crée pas de
            // variable PDV — `call symput` écrit dans la table macro, pas
            // dans le PDV). On parcourt donc simplement les arguments pour
            // découvrir les variables référencées.
            DsStmt::CallRoutine { name, args } => self.compile_call_routine(name, args),
            // INFILE (M14) : déclaratif. Un second INFILE écrase le premier
            // (SAS le permet — le dernier gagne). On mémorise source+options.
            DsStmt::Infile { source, options } => {
                self.infile = Some((source.clone(), options.clone()));
                Ok(())
            }
            // INPUT (M14) : les variables nommées entrent au PDV en ordre de
            // première référence (char → longueur du `$ w`/informat, défaut
            // 8 ; num → 8). Plusieurs INPUT par étape sont autorisés.
            DsStmt::Input(items) => self.compile_input(items),
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
            // Étiquette (M16.6) : enregistre l'étiquette (doublon → erreur) puis
            // walke le statement étiqueté (il participe pleinement au PDV / aux
            // validations comme s'il était nu).
            DsStmt::Labeled { name, stmt } => {
                let upper = name.to_uppercase();
                if !self.labels_defined.insert(upper.clone()) {
                    return Err(SasError::runtime(format!(
                        "The label {} is defined more than once in the DATA step.",
                        upper
                    )));
                }
                self.walk_stmt(stmt)
            }
            // GOTO/LINK (M16.6) : mémorisent leur référence d'étiquette ; la
            // validation (étiquette définie ?) a lieu en fin de compilation,
            // quand TOUTES les étiquettes ont été collectées (une cible peut
            // apparaître APRÈS le GOTO/LINK).
            DsStmt::Goto(label) | DsStmt::Link(label) => {
                self.goto_link_refs.push(label.to_uppercase());
                Ok(())
            }
            // RETURN (M16.6) : aucune validation compile-time (un RETURN sans
            // LINK actif est licite — termine l'itération courante).
            DsStmt::Return => Ok(()),
            // DECLARE HASH (M17.1) : enregistre l'objet hash avec ses options
            // résolues. Un objet redéclaré écrase le précédent (SAS permet de
            // re-DECLARE ; le dernier gagne). Les options inconnues → erreur.
            DsStmt::DeclareHash { name, options } => self.compile_hash_decl(name, options),
            // DECLARE HITER (M17.2) : l'objet hash lié doit être déclaré.
            DsStmt::DeclareHiter { name, hash_name } => self.compile_hiter_decl(name, hash_name),
            // Appel de méthode d'objet hash (M17.1/M17.2) : l'objet doit être
            // déclaré. Pour defineKey/defineData, les arguments positionnels
            // sont des littéraux chaîne nommant des variables du PDV (validées).
            // Les autres méthodes valident leurs arguments d'expression.
            DsStmt::HashMethod(call) => {
                self.validate_hash_method(&call.object, &call.method, &call.args)
            }
        }
    }
}

#[cfg(test)]
mod tests;
