//! Évaluateur d'expressions (tree-walking) sur le PDV.
//!
//! # Plan du fichier — voir PLAN.md  (difficulté : MOYENNE-ÉLEVÉE)
//!
//! ## Règles de coercition / missing (fidélité SAS)
//! - Arithmétique (`+ - * / **`) : si UN opérande est missing → résultat
//!   `.` + incrément `ctx.missing_generated` (PAS d'erreur).
//! - Division par zéro → `.` + note dédiée + `_ERROR_`.
//! - `0 ** 0` = 1 ; `(-2) ** 0.5` → missing + note (SAS).
//! - Comparaisons : via `Value::sas_cmp` → 1.0/0.0. ATTENTION : les
//!   missings SONT comparables (`. = .` vrai, `. < 0` vrai). Comparaison
//!   num/char → en SAS c'est une ERREUR de compilation ; ici : note +
//!   conversion automatique (cf. ci-dessous) pour rester permissif.
//! - `AND`/`OR`/`NOT` : `truthy()` de chaque opérande → 1.0/0.0 (pas de
//!   court-circuit nécessaire, pas d'effets de bord).
//! - `||` : opérandes num convertis en char via BEST12. JUSTIFIÉ À
//!   DROITE sur 12 (oui, avec les espaces de tête — fidèle à SAS) + note
//!   "Numeric values have been converted to character values..." UNE
//!   fois par étape.
//! - Conversion char→num automatique (char utilisé en contexte
//!   numérique) : trim puis parse f64 ; vide/invalide → `.` + note
//!   "Invalid numeric data" + `_ERROR_`. Note "Character values have
//!   been converted to numeric values..." une fois par étape.
//! - `IN` : égalités successives via sas_cmp.
//! - `Call` : déléguer à `functions::call` ; fonction inconnue → erreur
//!   de compilation en SAS ; ici ERROR à la première évaluation.
//!
//! ## EvalCtx
//! Collecte les notes uniques (conversions), les compteurs (missing
//! generated, division par zéro avec n° de ligne plus tard), et le flag
//! `_ERROR_` à reporter au PDV par l'exécuteur.

use super::functions;
use super::pdv::Pdv;
use crate::ast::{BinaryOp, Expr, UnaryOp};
use crate::error::SasError;
use crate::value::Value;
use std::collections::HashMap;

mod call;
mod ops;

use call::*;
pub(crate) use ops::*;

pub struct EvalCtx {
    pub missing_generated: u32,
    pub division_by_zero: u32,
    pub note_num_to_char: bool,
    pub note_char_to_num: bool,
    pub invalid_data: u32,
    pub error_flag: bool,
    /// Erreur fatale (fonction inconnue, indice d'array hors bornes...) —
    /// stoppe l'étape. Le message est SANS préfixe « ERROR: » : c'est
    /// `log.error` qui l'ajoute au moment de l'affichage.
    pub fatal: Option<SasError>,
    /// Arrays de l'étape : nom UPPERCASE → définition (slots + dimensions)
    /// (copié depuis `StepProgram.arrays` par l'exécuteur).
    pub arrays: HashMap<String, super::ArrayDef>,
    /// Flags de groupe BY `(nom UPPERCASE, first, last)`, dans l'ordre du
    /// BY — mis à jour par le Runner à chaque observation servie par
    /// l'interclassement. Servent les variables automatiques FIRST.x /
    /// LAST.x (jamais de slot PDV, donc jamais écrites en sortie).
    pub by_flags: Vec<(String, bool, bool)>,
    /// Flags IN= du MERGE `(nom UPPERCASE, valeur 0/1)` : 1 si le dataset
    /// associé a participé au groupe de clé BY de l'observation courante.
    /// Mis à jour par le Runner à chaque obs de sortie du MERGE. Servent
    /// les variables automatiques IN= (jamais de slot PDV).
    pub in_flags: Vec<(String, bool)>,
    /// Files FIFO de LAG/DIF, une PAR SITE D'APPEL lexical (clé = pointeur
    /// du slice d'arguments de l'AST, stable d'une itération de la boucle
    /// implicite à l'autre). Voir PLAN.md §Checklist pitfall #8 : LAGn /
    /// DIFn renvoient la valeur d'il y a `n` exécutions du MÊME site, pas la
    /// valeur d'il y a `n` lignes de la variable.
    pub lag_queues: HashMap<usize, std::collections::VecDeque<Value>>,
    /// CALL SYMPUT (M11.5) : écritures DIFFÉRÉES vers la table macro,
    /// `(nom, valeur)` dans l'ordre d'exécution. La visibilité SAS impose
    /// que le symbole ne soit posé qu'APRÈS le RUN de l'étape : on
    /// accumule donc ici et le drain est fait par `exec::execute` une fois
    /// la boucle implicite terminée (là où `&mut Session` est disponible).
    /// Sous le build par défaut ce vecteur se remplit toujours mais le
    /// drain est un no-op (l'engine identité n'a pas de table).
    pub symput_writes: Vec<(String, String)>,
    /// SYMGET (M11.5) : instantané de la table macro pris au DÉBUT de
    /// l'étape (`MacroEngine::symbols_snapshot`), clés en MAJUSCULES. Sous
    /// la feature `macros` il reflète l'état des `%let`/symput antérieurs ;
    /// sous le build par défaut il est vide (aucune résolution macro).
    pub macro_symbols: HashMap<String, String>,
    /// RNG state for RAND*, RANUNI, RANNOR, RANEXP, RANBIN, CALL STREAMINIT
    /// (M15.5). Uses a simple LCG seeded at construction time. CALL STREAMINIT
    /// resets it. Box-Muller stores a spare normal variate in `rng_spare`.
    pub rng_state: u64,
    /// Cached spare normal variate from Box-Muller (set when a pair is
    /// generated; consumed on the next RANNOR call).
    pub rng_spare: Option<f64>,
    /// `DO OVER` actifs (M16.3) : nom d'array UPPERCASE → slot PDV de
    /// l'élément courant. Une référence NUE au nom de l'array (lecture ou
    /// écriture) y est redirigée. Empilé/dépilé par le Runner à chaque tour.
    pub do_over: HashMap<String, usize>,
    /// Variable END= du SET (M16.4) : `(nom UPPERCASE, valeur 0/1)`. Mise à
    /// jour par le Runner après chaque lecture (1 = dernière obs lue). Servie
    /// comme variable automatique (jamais de slot PDV).
    pub end_flag: Option<(String, f64)>,
    /// Objets hash de l'étape (M17.1) : nom UPPERCASE → objet (clés, données,
    /// lignes). Copié depuis `StepProgram.hash_objects` par l'exécuteur ;
    /// defineKey/defineData/defineDone (et M17.2 find/add/...) y opèrent.
    pub hashes: HashMap<String, super::HashObject>,
    /// Itérateurs de hash de l'étape (M17.2) : nom UPPERCASE → itérateur
    /// (objet lié + position). Copié depuis `StepProgram.hash_iters` par
    /// l'exécuteur ; first/next/last/prev y opèrent.
    pub hash_iters: HashMap<String, super::HashIter>,
    /// Sorties de hash en attente (M17.2) : `h.output(dataset:'lib.tab')`
    /// accumule ici `(libref, table, vars, lignes)` ; le drain (écriture via
    /// le provider de bibliothèque) est fait par `exec::execute` APRÈS la
    /// boucle implicite, là où `&mut Session` est disponible.
    pub hash_outputs: Vec<HashOutput>,
    /// Catalogue de formats/informats (M18.2) — copié depuis la session pour
    /// que les fonctions PUT() et INPUT() puissent résoudre les formats et
    /// informats utilisateur. L'évaluateur n'a pas accès à `Session`, donc
    /// on passe le catalogue ici au début de chaque étape DATA.
    pub format_catalog: std::rc::Rc<crate::formats::FormatCatalog>,
    /// M38.2 — YEARCUTOFF= mirrored from `session.options.yearcutoff`.
    /// Used by date functions (e.g. DATEJUL) to interpret 2-digit years.
    /// Default 1900 preserves the pre-M38.2 behaviour (0–99 → 1900–1999).
    pub yearcutoff: u16,
    /// Patterns PRX compilés (M40.1) : PRXPARSE y alloue des ids valables
    /// pour la durée de l'étape ; fonctions et CALL routines PRX* y opèrent.
    pub prx: functions::prx::PrxState,
    /// ERREURs d'exécution NON fatales (M40.1 : pattern PRX invalide…).
    /// L'évaluateur n'a pas accès au log — les messages sont rejoués en
    /// `log.error` par `drain_runner_side_effects` en fin d'étape, comme
    /// les NOTEs de conversion.
    pub runtime_errors: Vec<String>,
}

/// Une sortie de hash en attente (M17.2). `vars` porte les `VarMeta` des
/// colonnes (clés puis données), `rows` les lignes décodées (parallèles à
/// `vars`).
#[derive(Debug, Clone)]
pub struct HashOutput {
    pub libref: String,
    pub table: String,
    pub display: String,
    pub vars: Vec<crate::dataset::VarMeta>,
    pub rows: Vec<Vec<Value>>,
}

impl Default for EvalCtx {
    fn default() -> Self {
        EvalCtx {
            missing_generated: 0,
            division_by_zero: 0,
            note_num_to_char: false,
            note_char_to_num: false,
            invalid_data: 0,
            error_flag: false,
            fatal: None,
            arrays: HashMap::new(),
            by_flags: Vec::new(),
            in_flags: Vec::new(),
            lag_queues: HashMap::new(),
            symput_writes: Vec::new(),
            macro_symbols: HashMap::new(),
            // Default seed: 1960 (SAS epoch year), shifted to avoid zero.
            rng_state: 0x0000_0007_A120_1960_u64,
            rng_spare: None,
            do_over: HashMap::new(),
            end_flag: None,
            hashes: HashMap::new(),
            hash_iters: HashMap::new(),
            hash_outputs: Vec::new(),
            format_catalog: std::rc::Rc::new(crate::formats::FormatCatalog::default()),
            yearcutoff: 1900,
            prx: functions::prx::PrxState::default(),
            runtime_errors: Vec::new(),
        }
    }
}

/// Coerce une `Value` en f64 pour un CONTEXTE NUMÉRIQUE (arithmétique,
/// comparaison après conversion, etc.). Suit fidèlement SAS :
/// - `Num` → la valeur.
/// - `Missing` → `None` (le missing se propage, sans note ni compteur :
///   c'est l'opération arithmétique englobante qui décide d'incrémenter
///   `missing_generated`).
/// - `Char` → trim puis parse. La NOTE "Character values have been
///   converted to numeric values..." apparaît dès qu'une conversion
///   automatique est TENTÉE (réussie ou non), donc `note_char_to_num`
///   passe à `true` dans tous les cas char. Chaîne vide → `.` +
///   `missing_generated`. Chaîne invalide → `.` + `invalid_data` +
///   `error_flag`.
///
/// Le `bool` renvoyé indique si l'opérande source était missing (Num ou
/// Char vide/invalide tombés à `None`) — utile pour distinguer un missing
/// d'entrée d'une simple absence dans les agrégats. Ici on renvoie juste
/// l'`Option<f64>` ; `None` couvre les deux cas (missing propagé).
pub(super) fn coerce_num(v: &Value, ctx: &mut EvalCtx) -> Option<f64> {
    match v {
        Value::Num(f) => Some(*f),
        Value::Missing(_) => None,
        Value::Char(s) => {
            // Toute conversion char→num automatique déclenche la NOTE SAS,
            // qu'elle réussisse ou non.
            ctx.note_char_to_num = true;
            let trimmed = s.trim();
            if trimmed.is_empty() {
                ctx.missing_generated += 1;
                None
            } else {
                match trimmed.parse::<f64>() {
                    Ok(f) => Some(f),
                    Err(_) => {
                        ctx.invalid_data += 1;
                        ctx.error_flag = true;
                        None
                    }
                }
            }
        }
    }
}

/// Convertit une `Value` en chaîne pour le CONTEXTE CARACTÈRE de `||`.
/// Un opérande numérique est rendu via BEST12. puis JUSTIFIÉ À DROITE sur
/// 12 colonnes (avec les espaces de tête, fidèle à SAS) et lève la NOTE
/// "Numeric values have been converted to character values...".
fn concat_operand(v: &Value, ctx: &mut EvalCtx) -> String {
    match v {
        Value::Char(s) => s.clone(),
        Value::Num(f) => {
            ctx.note_num_to_char = true;
            format!("{:>12}", crate::value::format_best(*f, 12))
        }
        Value::Missing(_) => {
            // Un missing numérique en contexte caractère devient 12 blancs
            // (BEST12. d'un `.` est un point cadré à droite ; SAS imprime un
            // simple "." cadré à droite). On reste fidèle au cadrage à
            // droite sur 12.
            ctx.note_num_to_char = true;
            format!("{:>12}", ".")
        }
    }
}

pub fn eval(expr: &Expr, pdv: &Pdv, ctx: &mut EvalCtx) -> Value {
    match expr {
        Expr::Num(n) => Value::Num(*n),
        Expr::Str(s) => Value::Char(s.clone()),
        Expr::Missing(k) => Value::Missing(*k),
        Expr::Var(name) => eval_var(name, pdv, ctx),
        Expr::Unary { op, expr } => eval_unary(op, expr, pdv, ctx),
        Expr::Binary { op, left, right } => eval_binary(*op, left, right, pdv, ctx),
        Expr::In { expr, list } => eval_in(expr, list, pdv, ctx),
        Expr::Call { name, args } => eval_call(name, args, pdv, ctx),
        Expr::Index { name, indices } => eval_array_ref(name, indices, pdv, ctx),
        // Méthode d'objet hash en expression (M17.2) : nécessite une mutation
        // du PDV/des objets hash → traitée par `exec::eval_checked` (qui a
        // `&mut self`). Ici, l'évaluateur immuable ne peut rien faire : on
        // signale un fatal de garde (ne devrait jamais être atteint en
        // production — `eval_checked` intercepte d'abord).
        Expr::HashMethod(_) => {
            ctx.fatal = Some(SasError::runtime(
                "Hash method calls cannot be evaluated in this context.",
            ));
            Value::missing()
        }
    }
}

#[cfg(test)]
mod tests;
