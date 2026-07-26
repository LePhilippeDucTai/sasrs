//! Exécution de l'étape DATA : la boucle implicite.
//!
//! # Plan du fichier — voir PLAN.md  (difficulté : ÉLEVÉE — cœur du langage)
//!
//! ## Boucle implicite
//! ```text
//! boucle :
//!   pdv.n_ += 1
//!   pdv.reset_non_retained()
//!   exécuter les statements dans l'ordre ; le statement SET, QUAND IL
//!     S'EXÉCUTE, lit la ligne suivante de l'input dans le PDV ; s'il n'y
//!     a plus de ligne, l'ÉTAPE SE TERMINE IMMÉDIATEMENT (au milieu de
//!     l'itération — c'est la règle SAS, pas un test en tête de boucle)
//!   en fin d'itération : output implicite (si !has_explicit_output)
//! ```
//! Étape SANS instruction de lecture (ni SET) : UNE seule itération puis
//! stop (sinon boucle infinie — règle SAS).
//!
//! ## SET multi-datasets + BY (M3)
//! - Sans BY : CONCATÉNATION — chaque exécution du SET sert la ligne
//!   suivante du dataset courant, puis passe au dataset suivant ; tous
//!   épuisés → EndStep. WHERE= est évalué à la volée (skip interne).
//! - Avec BY : INTERCLASSEMENT — à chaque exécution, parmi les datasets
//!   non épuisés, servir celui dont la tête porte la PLUS PETITE clé BY
//!   (`Value::sas_cmp` clé par clé, DESCENDING respecté ; égalité →
//!   l'ordre du statement SET). Les WHERE= sont PRÉ-APPLIQUÉS avant la
//!   boucle (cf. `Runner::prefilter` — divergence mineure : leurs NOTEs
//!   de conversion peuvent couvrir des lignes jamais servies, et le
//!   `_ERROR_` qu'ils lèveraient n'est pas reporté à l'itération).
//! - RETAIN implicite des variables de SET : une variable absente du
//!   dataset de l'obs courante GARDE sa valeur précédente (SAS ne la
//!   remet PAS à missing) ; elle reste missing avant sa première lecture.
//! - FIRST.v_i / LAST.v_i : recalculés à chaque obs servie en comparant
//!   le PRÉFIXE de clés 0..=i avec l'obs précédente (FIRST.) et la tête
//!   suivante de l'interclassement (LAST.) ; 1 aux bornes du step. Servis
//!   par `EvalCtx::by_flags`, jamais écrits en sortie.
//! - Désordre : la clé servie ne peut que croître ; si elle régresse
//!   (input non trié selon le BY), ERROR "BY variables are not properly
//!   sorted on data set X." et l'étape s'arrête.
//!
//! ## Flux de contrôle intra-itération
//! `enum Flow { Normal, NextIter, EndStep }` :
//! - `SubsettingIf` faux → NextIter (pas d'output implicite)
//! - `Delete` → NextIter (même effet qu'un subsetting IF faux)
//! - `Output` → pousser les valeurs des `kept_slots` dans les builders
//! - `Stop` → EndStep
//! - `If/Block/DoLoop` propagent le Flow de leurs branches (un DELETE,
//!   STOP ou SET épuisé dans un corps de DO sort de la boucle ET remonte).
//!
//! ## DO itératif (M2) — sémantique SAS exacte
//! from/to/by sont évalués UNE SEULE FOIS à l'entrée (les modifier dans
//! le corps ne change pas les bornes) ; BY défaut 1. L'INDEX, lui, est
//! une variable normale du PDV : le corps peut le modifier et cela
//! affecte le test et l'incrément. Ordre par tour : (1) test TO
//! (by>0 → i<=to, by<0 → i>=to ; by==0 → pas de sortie par TO),
//! (2) WHILE, (3) corps, (4) UNTIL, (5) i += by. À la sortie par le test
//! TO, l'index garde la PREMIÈRE valeur qui dépasse (`do i = 1 to 3;` →
//! i == 4 après la boucle — règle SAS).
//! DIVERGENCES DOCUMENTÉES :
//! - from/to/by évaluant à missing → SasError::runtime("Invalid DO loop
//!   control information.") qui stoppe l'étape (SAS émet une erreur
//!   d'exécution équivalente) ;
//! - garde-fou anti-boucle infinie : plus de 10 000 000 itérations pour
//!   UNE exécution de la boucle → erreur runtime (SAS bouclerait sans
//!   fin).
//!
//! ## Erreurs d'exécution (style SAS : on continue !)
//! Division par zéro, argument invalide, conversion char→num ratée :
//! résultat missing `.`, `pdv.error_ = true`, NOTE dans le log
//! ("Division by zero detected...", "Invalid numeric data..."),
//! compteur "Missing values were generated" pour la NOTE de fin d'étape.
//! Implémenté via `EvalCtx` (eval.rs) qui collecte notes + compteurs.
//!
//! ## Builders de sortie
//! Par output et par slot conservé : `Vec<Option<f64>>` (missing spéciaux
//! ré-encodés NaN-payload via `missing::value_to_num`) ou
//! `Vec<Option<String>>`. À la fin : construire les `Column` Polars dans
//! l'ordre PDV, créer `SasDataset` (VarMeta depuis le PDV), écrire via
//! `session.libs.get(libref)?.write(table, ds)`, et mettre à jour
//! `session.last_dataset`.
//!
//! ## NOTEs de fin d'étape (ordre SAS)
//! 1. "There were N observations read from the data set WORK.B."
//! 2. par output : "The data set WORK.A has N observations and M
//!    variables."  (M = nb de slots conservés ; SAS ne met jamais le
//!    singulier — garder "variables" même pour 1 !)
//! L'appelant (executor) ajoute ensuite la NOTE de timing.
//!
//! ## Choix d'implémentation
//! - Les NOTEs de conversion/erreur n'incluent pas les positions
//!   (Line):(Column) de SAS — divergence assumée, cf. PLAN.md (log sans
//!   numéros de page/date).
//! - La coercition à l'ASSIGNATION (expression num vers variable char et
//!   inversement) vit ici : num→char via BEST12. justifié à droite sur
//!   12, char→num via trim+parse (mêmes règles que dans eval).
//! - Garde-fou anti-boucle infinie (SET jamais exécuté alors qu'un input
//!   existe) : n_ > n_rows + 10_000 → erreur d'exécution. SAS bouclerait
//!   sans fin ; divergence assumée.

use super::eval::{EvalCtx, coerce_num, eval, sas_values_equal};
use super::pdv::Pdv;
use super::{
    ByVar, InputAction, InputData, InputDataset, OutputSpec, ShortMode, StepProgram, TextInput,
};
use crate::ast::DsStmt;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::missing::value_to_num;
use crate::session::Session;
use crate::value::{Value, VarType, format_best};
use polars::prelude::{Column, DataFrame, NamedFrom, Series};
use std::cmp::Ordering;
use std::collections::HashMap;

mod runner;
mod state;
mod update_modify;

use runner::*;
use state::*;
use update_modify::*;

mod hash;

mod input;

mod put;

mod setmerge;

mod run;

pub use run::execute;

pub(crate) use run::*;

#[cfg(test)]
mod tests;

use self::setmerge::{keys_at, prefix_changed};

pub struct StepStats {
    /// (display, lignes lues) par input.
    pub read: Vec<(String, usize)>,
    /// (display, obs, vars) par output écrit.
    pub written: Vec<(String, usize, usize)>,
}

#[derive(PartialEq, Clone)]
enum Flow {
    Normal,
    NextIter,
    EndStep,
    /// GOTO (M16.6) : saut inconditionnel vers l'étiquette nommée (index résolu
    /// par le pilote de niveau supérieur). Traverse les boucles DO englobantes.
    /// Émis depuis une sous-routine LINK, il l'abandonne et remonte jusqu'au
    /// pilote de premier niveau qui repositionne le compteur de programme.
    Goto(String),
    /// RETURN (M16.6) : retour de la sous-routine LINK courante (consommé par
    /// `exec_link_subroutine`). Sans LINK actif (premier niveau), équivaut à la
    /// fin d'itération (output implicite).
    Return,
}

enum ColBuilder {
    Num(Vec<Option<f64>>),
    Char(Vec<String>),
}

/// Destination résolue d'un PUT (M14.2). `Path` porte le chemin du fichier
/// externe ; `Log`/`Print` routent vers le journal / le listing.
#[derive(Clone, PartialEq)]
enum PutDestKind {
    Path(String),
    Log,
    Print,
}

/// Résultat de la lecture d'UNE variable d'INPUT (M14).
enum ReadOutcome {
    /// Lecture normale (valeur posée au PDV, missing inclus).
    Ok,
    /// Ligne trop courte, comportement MISSOVER/TRUNCOVER/défaut : on arrête
    /// la lecture des items restants (laissés à missing).
    ShortMissover,
    /// Ligne trop courte avec STOPOVER : erreur.
    Stopover,
}

struct Runner {
    pdv: Pdv,
    input: Option<InputData>,
    /// E/S TEXTE (M14 : INFILE/INPUT/DATALINES ; M14.2 : FILE/PUT).
    text_io: TextIo,
    /// Catalogue de formats/informats (clone de session) pour appliquer les
    /// informats de l'INPUT (M14) et les formats des PUT. Partagé entre les
    /// modes — laissé à plat.
    format_catalog: crate::formats::FormatCatalog,
    /// Curseurs du statement SET (concaténation / interclassement / POINT=).
    set_cursor: SetCursor,
    /// Lignes lues au sens SAS, PAR dataset : celles qui PASSENT le WHERE=.
    /// C'est ce compteur qu'affiche la NOTE "There were N observations
    /// read". Partagé par les modes SET et MERGE (`build_merge_plan` le
    /// remplit) — laissé à plat.
    rows_read: Vec<usize>,
    ctx: EvalCtx,
    outputs: Vec<OutputSpec>,
    /// builders[output][colonne], parallèle à outputs[o].kept_slots.
    builders: Vec<Vec<ColBuilder>>,
    /// Observations poussées PAR sortie (l'OUTPUT ciblé rend les comptes
    /// indépendants).
    out_rows: Vec<usize>,
    /// MERGE (M3) : plan pré-calculé + curseur.
    merge: MergeState,
    /// Labels des variables (nom UPPERCASE → libellé), copié depuis
    /// `StepProgram.labels`. Sert CALL LABEL(var, result) (M15.6).
    labels: HashMap<String, String>,
    /// CALL EXECUTE (M15.6) : texte SAS mis en file pour exécution APRÈS
    /// l'étape DATA courante. Drainé par `execute` vers
    /// `session.call_execute_queue` (l'exécuteur le rejoue ensuite). Chaque
    /// appel concatène son argument résolu dans l'ordre d'exécution.
    call_execute_queue: Vec<String>,
    /// MODIFY+POINT= (M16.5) : état partagé pour l'accès direct piloté par le
    /// corps. `None` hors de ce cas (boucle séquentielle MODIFY ou UPDATE).
    modify_state: Option<ModifyState>,
    /// Statements de PREMIER NIVEAU de l'étape (M16.6) — partagés avec les
    /// boucles d'exécution. Sert à exécuter INLINE le corps d'une sous-routine
    /// LINK (du statement étiqueté jusqu'au prochain RETURN) sans abandonner la
    /// structure de boucle DO englobante. Vide tant qu'aucun LINK n'est possible.
    program: std::rc::Rc<Vec<DsStmt>>,
    /// Étiquettes de contrôle (M16.6) : nom UPPERCASE → index dans `program`.
    /// Cibles des LINK exécutés inline et des GOTO résolus par le pilote.
    flow_labels: std::rc::Rc<HashMap<String, usize>>,
}

mod stmt;

mod call;

mod loops;

impl Runner {
    /// Objet hash déjà validé par l'appelant (nom en MAJUSCULES) : les
    /// méthodes hash vérifient l'existence en tête puis re-consultent la
    /// table sans re-tester.
    fn hash(&self, upper: &str) -> &super::HashObject {
        self.ctx.hashes.get(upper).expect("checked hash exists")
    }

    /// Variante mutable de [`Self::hash`].
    fn hash_mut(&mut self, upper: &str) -> &mut super::HashObject {
        self.ctx.hashes.get_mut(upper).expect("checked hash exists")
    }

    /// Pousse la ligne courante du PDV dans TOUTES les sorties.
    fn push_outputs(&mut self) {
        for o in 0..self.outputs.len() {
            self.push_one(o);
        }
    }

    /// Pousse la ligne courante du PDV dans la sortie d'indice `o`.
    fn push_one(&mut self, o: usize) {
        let spec = &self.outputs[o];
        for (slot, b) in spec.kept_slots.iter().zip(self.builders[o].iter_mut()) {
            let v = self.pdv.get(*slot);
            match b {
                ColBuilder::Num(vals) => vals.push(value_to_num(v)),
                ColBuilder::Char(vals) => vals.push(match v {
                    Value::Char(s) => s.clone(),
                    // Une variable char ne contient jamais autre chose
                    // après pdv.set ; blanc par sûreté.
                    _ => String::new(),
                }),
            }
        }
        self.out_rows[o] += 1;
    }
}
