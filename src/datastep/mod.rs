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

#![allow(unused_variables, dead_code)]

pub mod eval;
pub mod exec;
pub mod functions;
pub mod pdv;

use crate::ast::DataStepAst;
use crate::error::Result;
use crate::session::Session;
use crate::value::Value;
use pdv::Pdv;

/// Données d'entrée matérialisées (M1 : un seul SET).
pub struct InputData {
    /// "WORK.B" pour les NOTEs du log.
    pub display: String,
    /// Colonnes décodées en `Value` (via `missing::num_to_value`), une
    /// seule passe de downcast par colonne — jamais de get_row.
    pub columns: Vec<Vec<Value>>,
    /// Slot PDV de chaque colonne (parallèle à `columns`).
    pub var_slots: Vec<usize>,
    pub n_rows: usize,
}

/// Une sortie : où écrire et quels slots du PDV.
pub struct OutputSpec {
    pub libref: String,
    pub table: String,
    /// "WORK.A" pour les NOTEs.
    pub display: String,
    /// Slots PDV conservés, dans l'ordre PDV (= ordre des colonnes).
    pub kept_slots: Vec<usize>,
}

pub struct StepProgram {
    pub pdv: Pdv,
    pub stmts: Vec<crate::ast::DsStmt>,
    pub input: Option<InputData>,
    pub outputs: Vec<OutputSpec>,
    pub has_explicit_output: bool,
}

pub fn compile(ast: &DataStepAst, session: &mut Session) -> Result<StepProgram> {
    todo!("cf. plan en tête de fichier")
}
