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

use super::eval::{coerce_num, eval, sas_values_equal, EvalCtx};
use super::pdv::Pdv;
use super::{
    ByVar, InputAction, InputData, InputDataset, OutputSpec, ShortMode, StepProgram, TextInput,
};
use crate::ast::DsStmt;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::missing::value_to_num;
use crate::session::Session;
use crate::value::{format_best, Value, VarType};
use polars::prelude::{Column, DataFrame, NamedFrom, Series};
use std::cmp::Ordering;
use std::collections::HashMap;

mod hash;
mod input;
mod put;
mod setmerge;
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

/// Enregistrement maintenu par un hold `@`/`@@` (M14).
struct HeldLine {
    line: String,
    cursor: usize,
    /// `@@` : survit aux itérations ; `@` : relâché à la prochaine itération.
    double: bool,
}

/// Destination résolue d'un PUT (M14.2). `Path` porte le chemin du fichier
/// externe ; `Log`/`Print` routent vers le journal / le listing.
#[derive(Clone, PartialEq)]
enum PutDestKind {
    Path(String),
    Log,
    Print,
}

/// État de sortie texte du PUT (M14.2). Mirroir de sortie du held-line de
/// l'INPUT : une destination courante, une ligne de sortie en construction
/// (`line`), un curseur de colonne 0-based, et le drapeau de hold `@`/`@@`.
struct PutState {
    /// Destination courante (par défaut le LOG, conformément à SAS).
    dest: PutDestKind,
    /// Ligne de sortie en construction (avant relâchement/flush).
    line: String,
    /// Position d'écriture courante (colonne 0-based) dans `line`.
    cursor: usize,
    /// Une ligne est-elle en cours de construction (au moins un PUT l'a
    /// commencée) ? Sert à distinguer une ligne vide explicite d'un état
    /// vierge au flush de fin d'étape.
    started: bool,
    /// Hold simple `@` actif : la ligne n'est PAS relâchée en fin de PUT ;
    /// relâchée au début de l'itération suivante.
    hold: bool,
    /// Hold double `@@` actif : la ligne survit aux itérations.
    hold_double: bool,
    /// Lignes de sortie complètes, dans l'ordre de production, taguées par
    /// leur destination. Rejouées vers le LOG / le listing / les fichiers
    /// APRÈS la boucle implicite (exec.rs n'a pas `&mut session` en boucle).
    out: Vec<(PutDestKind, String)>,
}

impl PutState {
    fn new() -> Self {
        PutState {
            dest: PutDestKind::Log,
            line: String::new(),
            cursor: 0,
            started: false,
            hold: false,
            hold_double: false,
            out: Vec::new(),
        }
    }
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
    /// Source d'entrée TEXTE (M14 : INFILE/INPUT/DATALINES).
    text: Option<TextInput>,
    /// Prochaine ligne brute (index dans `text.lines`) à charger.
    text_line: usize,
    /// Nombre d'enregistrements (lignes) lus de la source texte.
    text_read: usize,
    /// Enregistrement maintenu par `@`/`@@` : la ligne courante, le curseur
    /// (colonne 0-based) et un drapeau `double` (`@@` survit aux itérations ;
    /// `@` simple est relâché au début de l'itération suivante). `Some` quand
    /// un hold est actif.
    held: Option<HeldLine>,
    /// Catalogue de formats/informats (clone de session) pour appliquer les
    /// informats de l'INPUT (M14).
    format_catalog: crate::formats::FormatCatalog,
    /// État de sortie texte des PUT (M14.2 : FILE/PUT).
    put: PutState,
    /// Mode CONCATÉNATION (sans BY) : index du dataset en cours de lecture.
    cur_ds: usize,
    /// Curseur PAR dataset : sans BY, prochaine ligne brute à charger (y
    /// compris celles rejetées par WHERE=) ; avec BY, position dans
    /// `filtered`.
    cursors: Vec<usize>,
    /// Mode INTERCLASSEMENT (avec BY) : indices des lignes qui passent le
    /// WHERE= (pré-filtrage), par dataset. Sans WHERE=, toutes les lignes.
    filtered: Vec<Vec<usize>>,
    /// Clés BY de la dernière observation servie : FIRST. et détection de
    /// désordre.
    prev_keys: Option<Vec<Value>>,
    /// Lignes lues au sens SAS, PAR dataset : celles qui PASSENT le WHERE=.
    /// C'est ce compteur qu'affiche la NOTE "There were N observations
    /// read".
    rows_read: Vec<usize>,
    ctx: EvalCtx,
    outputs: Vec<OutputSpec>,
    /// builders[output][colonne], parallèle à outputs[o].kept_slots.
    builders: Vec<Vec<ColBuilder>>,
    /// Observations poussées PAR sortie (l'OUTPUT ciblé rend les comptes
    /// indépendants).
    out_rows: Vec<usize>,
    /// MERGE (M3) : séquence pré-calculée des observations de sortie
    /// (groupe par groupe). Vide hors MERGE.
    merge_plan: Vec<MergeObs>,
    /// Curseur dans `merge_plan` (prochaine obs à servir).
    merge_cursor: usize,
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

/// État partagé d'un MODIFY+POINT= (M16.5). Le bras `DsStmt::Modify` de
/// `exec_stmt` y charge l'obs à l'index POINT= courant (et capture la
/// précédente). `cols` est le tampon de réécriture (parallèle à `var_slots`).
struct ModifyState {
    /// Slot PDV de la variable d'index POINT=.
    point_slot: usize,
    /// Tampon de réécriture : colonnes décodées, modifiées au fil des captures.
    cols: Vec<Vec<Value>>,
    /// Slots PDV de chaque colonne (parallèle à `cols`).
    var_slots: Vec<usize>,
    /// Ligne actuellement chargée (à capturer au prochain marqueur / en fin).
    cur_row: Option<usize>,
    /// "WORK.A" pour les messages d'erreur POINT=.
    display: String,
    /// Nombre total d'observations (bornes de l'index POINT=).
    n_rows: usize,
    /// Erreur POINT= différée (index invalide), remontée par la boucle externe.
    error: Option<String>,
    /// Lignes touchées (chargées au moins une fois) — compteur de lecture.
    touched: Vec<bool>,
}

/// Une observation de sortie d'un MERGE, pré-calculée par `build_merge_plan`.
struct MergeObs {
    /// Slots à remettre à MISSING AVANT les chargements (variables PROPRES
    /// des datasets absents du groupe) — non vide seulement à la 1re obs du
    /// groupe.
    blank_slots: Vec<usize>,
    /// Chargements à appliquer dans l'ORDRE (gauche→droite du MERGE) : le
    /// dernier dataset qui contribue écrase les variables partagées.
    /// `(index dataset, ligne)`.
    loads: Vec<(usize, usize)>,
    /// État IN= par dataset pour ce groupe (`true` = a participé).
    in_active: Vec<bool>,
    /// FIRST./LAST. par variable BY (préfixe de clés), parallèle à
    /// `input.by`.
    first: Vec<bool>,
    last: Vec<bool>,
}

pub fn execute(prog: StepProgram, session: &mut Session) -> Result<StepStats> {
    // Fast-path vectorisé OPTIONNEL (OFF par défaut). Ne s'active que pour les
    // étapes prouvées équivalentes ET une fenêtre d'entrée pleine
    // (FIRSTOBS=1 / OBS=MAX) ; sinon on garde la boucle ligne-à-ligne.
    if session.vectorize
        && session.options.firstobs == 1
        && session.options.obs.is_none()
        && super::fastpath::eligible(&prog)
    {
        return super::fastpath::run(prog, session);
    }

    // UPDATE/MODIFY (M16.5) ont leur propre boucle d'exécution (sémantique
    // distincte du SET/MERGE) : on les détourne avant la boucle implicite
    // générique.
    if prog.update.is_some() {
        return execute_update(prog, session);
    }
    if prog.modify.is_some() {
        return execute_modify(prog, session);
    }

    let StepProgram {
        pdv,
        stmts,
        input,
        update: _,
        modify: _,
        text_input,
        outputs,
        has_explicit_output,
        uninitialized,
        initial_values,
        arrays,
        labels,
        flow_labels,
        hash_objects,
        hash_iters,
    } = prog;

    for name in &uninitialized {
        session.log.note(&format!("Variable {name} is uninitialized."));
    }

    let builders = outputs
        .iter()
        .map(|o| {
            o.kept_slots
                .iter()
                .map(|s| match pdv.vars()[*s].ty {
                    VarType::Num => ColBuilder::Num(Vec::new()),
                    VarType::Char => ColBuilder::Char(Vec::new()),
                })
                .collect()
        })
        .collect();

    // Une étape avec une source texte (INFILE/INPUT) boucle comme un SET ;
    // sans aucune source (ni SET ni texte) elle ne tourne qu'une fois.
    let single_iteration = input.is_none() && text_input.is_none();
    let n_rows: usize = input
        .as_ref()
        .map_or(0, |i| i.datasets.iter().map(|d| d.n_rows).sum());
    // Garde-fou anti-boucle infinie pour la source texte.
    let n_text_lines = text_input.as_ref().map_or(0, |t| t.lines.len());
    let n_datasets = input.as_ref().map_or(0, |i| i.datasets.len());
    // FIRST./LAST. valent 1 tant qu'aucune observation n'a été servie.
    let by_flags = input
        .as_ref()
        .map_or(Vec::new(), |i| {
            i.by.iter().map(|b| (b.name.clone(), true, true)).collect()
        });
    // IN= : initialisées à 0 (aucun groupe encore servi).
    let in_flags = input.as_ref().map_or(Vec::new(), |i| {
        i.in_flags
            .iter()
            .map(|(name, _)| (name.clone(), false))
            .collect()
    });
    // END= (M16.4) : variable automatique 0/1, initialisée à 0.
    let end_flag = input
        .as_ref()
        .and_then(|i| i.end_var.as_ref().map(|n| (n.clone(), 0.0)));
    // POINT= (M16.4) : si présent, la boucle implicite est REMPLACÉE par un
    // contrôle manuel (pas d'avance de curseur automatique, pas d'output
    // implicite, pas de fin d'étape à l'épuisement). On mémorise le slot.
    let point_slot = input.as_ref().and_then(|i| i.point_slot);
    // NOBS= (M16.4) : slot + total d'observations (somme des datasets).
    let nobs = input.as_ref().and_then(|i| {
        i.nobs_slot
            .map(|slot| (slot, i.datasets.iter().map(|d| d.n_rows).sum::<usize>()))
    });
    let n_outputs = outputs.len();
    // SYMGET (M11.5) : instantané de la table macro pris AU DÉBUT de
    // l'étape. Sous la feature `macros` il porte les `%let`/symput
    // antérieurs ; sous le build par défaut il est vide.
    let macro_symbols = session.macro_engine.symbols_snapshot();

    let mut r = Runner {
        pdv,
        input,
        text: text_input,
        text_line: 0,
        text_read: 0,
        held: None,
        format_catalog: session.format_catalog.clone(),
        put: PutState::new(),
        cur_ds: 0,
        cursors: vec![0; n_datasets],
        filtered: vec![Vec::new(); n_datasets],
        prev_keys: None,
        rows_read: vec![0; n_datasets],
        ctx: EvalCtx {
            arrays,
            by_flags,
            in_flags,
            end_flag,
            macro_symbols,
            hashes: hash_objects,
            hash_iters,
            format_catalog: session.format_catalog.clone(),
            yearcutoff: session.options.yearcutoff,
            ..EvalCtx::default()
        },
        outputs,
        builders,
        out_rows: vec![0; n_outputs],
        merge_plan: Vec::new(),
        merge_cursor: 0,
        labels,
        call_execute_queue: Vec::new(),
        modify_state: None,
        program: std::rc::Rc::new(Vec::new()),
        flow_labels: std::rc::Rc::new(HashMap::new()),
    };

    // Interclassement / match-merge : pré-application des WHERE= par dataset
    // (les lignes rejetées ne comptent pas comme lues), AVANT la boucle. Les
    // NOTEs de conversion d'un WHERE= peuvent donc être émises pour des
    // lignes jamais atteintes (STOP précoce) — divergence mineure assumée.
    if r.input.as_ref().is_some_and(|i| !i.by.is_empty()) {
        r.prefilter()?;
    }
    // MERGE : pré-calcul de la séquence des obs de sortie (groupe par
    // groupe), à partir des lignes retenues par le pré-filtrage. La
    // détection de désordre y est faite (clé de groupe qui régresse →
    // ERROR).
    if r.input.as_ref().is_some_and(|i| i.merge) {
        r.merge_plan = r.build_merge_plan()?;
    }

    // Valeurs initiales (RETAIN avec init, sum statements) : posées AVANT
    // la première itération via `pdv.set` (la troncature char des inits
    // trop longues s'applique donc normalement). Ces slots sont retenus,
    // `reset_non_retained` ne les touchera jamais. Une entrée ultérieure
    // pour le même slot écrase la précédente (le RETAIN gagne sur le 0
    // implicite d'un sum statement).
    for (slot, v) in initial_values {
        r.pdv.set(slot, v);
    }

    // NOBS= (M16.4) : affectée AVANT la boucle (disponible dès la 1re
    // itération, p.ex. `do i = 1 to n;`). Slot retenu ⇒ persiste.
    if let Some((slot, total)) = nobs {
        r.pdv.set(slot, Value::Num(total as f64));
    }

    // POINT= (M16.4) : l'output implicite est SUPPRIMÉ (SAS exige un OUTPUT
    // explicite), et la boucle ne se termine pas sur épuisement d'entrée
    // (c'est l'utilisateur qui pilote l'itération via DO/STOP).
    let suppress_implicit_output = has_explicit_output || point_slot.is_some();

    // M16.6 : programme + étiquettes partagés avec le Runner (LINK exécuté
    // inline, GOTO résolu par `run_step_body`).
    r.program = std::rc::Rc::new(stmts);
    r.flow_labels = std::rc::Rc::new(flow_labels);

    loop {
        r.pdv.n_ += 1;
        r.pdv.error_ = false;
        r.pdv.reset_non_retained();
        // Hold de ligne (M14) : un `@` simple est relâché au DÉBUT de
        // l'itération suivante (le prochain INPUT lira un nouvel
        // enregistrement) ; un `@@` survit.
        if let Some(h) = &r.held {
            if !h.double {
                r.held = None;
            }
        }
        // Hold de ligne PUT (M14.2) : un `@` simple relâche la ligne au DÉBUT
        // de l'itération suivante (flush + clear) ; un `@@` la conserve.
        if r.put.hold && !r.put.hold_double {
            r.put_release_line();
        }

        let flow = r.run_step_body()?;
        if flow == Flow::EndStep {
            break;
        }
        if flow != Flow::NextIter && !suppress_implicit_output {
            r.push_outputs();
        }
        if single_iteration {
            break;
        }
        // Garde-fou anti-boucle infinie (cf. en-tête).
        if r.pdv.n_ as usize > n_rows + n_text_lines + 10_000 {
            return Err(SasError::runtime(
                "DATA step appears to loop infinitely (no input rows consumed); stopping.",
            ));
        }
    }

    // Test-only (M17.1) : expose l'état final des objets hash à la session
    // pour inspection unitaire (keys/data_vars/options/defined). En production
    // ce bloc n'existe pas.
    #[cfg(test)]
    {
        session.debug_hashes = r.ctx.hashes.clone();
    }

    // Drains post-boucle (symput, hash outputs, call execute, PUT) + NOTEs
    // d'erreurs/conversions — partagés avec les boucles UPDATE/MODIFY.
    drain_runner_side_effects(&mut r, session)?;

    let mut stats = StepStats {
        read: Vec::new(),
        written: Vec::new(),
    };
    if let Some(input) = &r.input {
        // Avec WHERE=, seules les lignes qui PASSENT comptent comme lues
        // (fidèle à la NOTE SAS). Une NOTE par dataset, dans l'ordre du
        // statement SET.
        for (ds, n) in input.datasets.iter().zip(&r.rows_read) {
            session.log.note(&format!(
                "There were {} observations read from the data set {}.",
                n, ds.display
            ));
            stats.read.push((ds.display.clone(), *n));
        }
    }
    // Source texte (M14) : NOTE "N records were read from the infile ..."
    // UNIQUEMENT pour un fichier externe. Pour les données instream
    // DATALINES/CARDS, SAS n'émet aucune NOTE de ce type (elle est réservée
    // aux fichiers physiques).
    if let Some(text) = &r.text {
        if text.is_file {
            session.log.note(&format!(
                "{} records were read from {}.",
                r.text_read, text.display
            ));
            stats.read.push((text.display.clone(), r.text_read));
        }
    }

    // Écriture des sorties (ordre du statement DATA ; _LAST_ = la dernière).
    write_runner_outputs(&mut r, session, &mut stats)?;

    Ok(stats)
}

/// Colonne Polars construite depuis des `Value` selon le type SAS déclaré :
/// numérique via `value_to_num` (missings → null), caractère tel quel (toute
/// cellule non-Char devient chaîne vide).
pub(crate) fn column_from_values<'a>(
    name: &str,
    ty: VarType,
    vals: impl Iterator<Item = &'a Value>,
) -> Column {
    match ty {
        VarType::Num => {
            let nums: Vec<Option<f64>> = vals.map(value_to_num).collect();
            Series::new(name.into(), nums).into()
        }
        VarType::Char => {
            let strs: Vec<String> = vals
                .map(|v| match v {
                    Value::Char(s) => s.clone(),
                    _ => String::new(),
                })
                .collect();
            Series::new(name.into(), strs).into()
        }
    }
}

/// Écrit `ds` dans sa bibliothèque, met à jour `_LAST_` et émet la NOTE
/// « The data set X has N observations and M variables. » ; enregistre la
/// ligne dans `stats.written` quand un `StepStats` est fourni.
pub(crate) fn write_dataset_with_note(
    session: &mut Session,
    libref: &str,
    table: &str,
    display: &str,
    ds: &SasDataset,
    n_obs: usize,
    stats: Option<&mut StepStats>,
) -> Result<()> {
    session.libs.get(libref)?.write(table, ds)?;
    session.last_dataset = Some(display.to_string());
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display,
        n_obs,
        ds.vars.len()
    ));
    if let Some(stats) = stats {
        stats.written.push((display.to_string(), n_obs, ds.vars.len()));
    }
    Ok(())
}

/// Écrit les sorties de hash en attente (M17.2) accumulées par
/// `h.output(dataset:)` vers leurs providers de bibliothèque. Appelée APRÈS la
/// boucle implicite (`&mut Session` disponible). Chaque sortie devient un
/// dataset (clés puis données, dans l'ordre de l'objet hash).
fn flush_hash_outputs(r: &mut Runner, session: &mut Session) -> Result<()> {
    for out in std::mem::take(&mut r.ctx.hash_outputs) {
        let columns: Vec<Column> = out
            .vars
            .iter()
            .enumerate()
            .map(|(c, meta)| {
                column_from_values(&meta.name, meta.ty, out.rows.iter().map(|row| &row[c]))
            })
            .collect();
        let df = DataFrame::new(columns)?;
        let ds = SasDataset {
            df,
            vars: out.vars.clone(),
        };
        write_dataset_with_note(
            session,
            &out.libref,
            &out.table,
            &out.display,
            &ds,
            out.rows.len(),
            None,
        )?;
    }
    Ok(())
}

/// Construit un Runner « squelette » pour les boucles UPDATE/MODIFY (M16.5),
/// avec le PDV, les arrays, les builders de sortie et l'instantané macro déjà
/// posés. Les champs spécifiques au SET/MERGE/texte sont vides.
fn build_um_runner(
    pdv: Pdv,
    outputs: Vec<OutputSpec>,
    arrays: HashMap<String, super::ArrayDef>,
    labels: HashMap<String, String>,
    by: &[ByVar],
    macro_symbols: HashMap<String, String>,
    format_catalog: crate::formats::FormatCatalog,
    yearcutoff: u16,
) -> Runner {
    let builders: Vec<Vec<ColBuilder>> = outputs
        .iter()
        .map(|o| {
            o.kept_slots
                .iter()
                .map(|s| match pdv.vars()[*s].ty {
                    VarType::Num => ColBuilder::Num(Vec::new()),
                    VarType::Char => ColBuilder::Char(Vec::new()),
                })
                .collect()
        })
        .collect();
    let n_outputs = outputs.len();
    let by_flags = by.iter().map(|b| (b.name.clone(), true, true)).collect();
    Runner {
        pdv,
        input: None,
        text: None,
        text_line: 0,
        text_read: 0,
        held: None,
        format_catalog: format_catalog.clone(),
        put: PutState::new(),
        cur_ds: 0,
        cursors: Vec::new(),
        filtered: Vec::new(),
        prev_keys: None,
        rows_read: vec![0; 1],
        ctx: EvalCtx {
            arrays,
            by_flags,
            macro_symbols,
            format_catalog,
            yearcutoff,
            ..EvalCtx::default()
        },
        outputs,
        builders,
        out_rows: vec![0; n_outputs],
        merge_plan: Vec::new(),
        merge_cursor: 0,
        labels,
        call_execute_queue: Vec::new(),
        modify_state: None,
        program: std::rc::Rc::new(Vec::new()),
        flow_labels: std::rc::Rc::new(HashMap::new()),
    }
}

/// Charge la ligne `row` du dataset matérialisé `ds` dans le PDV (tous ses
/// slots). Downcast déjà fait à la compilation (colonnes décodées).
fn load_row(pdv: &mut Pdv, ds: &InputDataset, row: usize) {
    for (col, slot) in ds.columns.iter().zip(&ds.var_slots) {
        pdv.set(*slot, col[row].clone());
    }
}

/// Clé d'appariement canonique d'une liste de `Value` (UPDATE/MODIFY KEY=).
/// Encode la sémantique d'égalité SAS : `. == .`, char insensible aux blancs
/// finaux. Sert de clé de `HashMap`.
fn key_string(values: &[Value]) -> String {
    let mut s = String::new();
    for v in values {
        match v {
            Value::Num(n) => {
                s.push('N');
                s.push_str(&format!("{:?}", n));
            }
            Value::Missing(k) => {
                s.push('M');
                s.push_str(&k.display());
            }
            Value::Char(c) => {
                s.push('C');
                s.push_str(c.trim_end());
            }
        }
        s.push('\u{1}');
    }
    s
}

/// Émet les NOTEs d'erreurs/conversions accumulées par l'évaluateur + draine
/// CALL SYMPUT / hash outputs / CALL EXECUTE / PUT. Partagé par les trois
/// boucles d'exécution (principale, UPDATE, MODIFY) ; les ordres relatifs sont
/// sémantiques (le rejeu PUT précède les NOTEs de fin d'étape, cf. SAS).
fn drain_runner_side_effects(r: &mut Runner, session: &mut Session) -> Result<()> {
    // CALL SYMPUT (M11.5) : drain des écritures différées vers la table
    // macro APRÈS le RUN de l'étape (règle de visibilité SAS — le symbole
    // n'est pas visible dans la même étape). Sous le build par défaut,
    // `set_symbol_global` est un no-op (l'engine identité n'a pas de table) :
    // `call symput` parse et s'exécute mais n'a aucun effet macro.
    for (name, value) in std::mem::take(&mut r.ctx.symput_writes) {
        session.macro_engine.set_symbol_global(&name, value);
    }
    // Hash output (M17.2) : drain des `h.output(dataset:)` accumulés vers les
    // providers de bibliothèque (où `&mut Session` est disponible). No-op sur
    // les chemins UPDATE/MODIFY (leur `EvalCtx` n'a pas d'objets hash, donc
    // `hash_outputs` y est toujours vide).
    flush_hash_outputs(r, session)?;
    // CALL EXECUTE (M15.6) : drain de la file de code généré pendant l'étape
    // vers la session. L'exécuteur le rejoue APRÈS le RUN de l'étape (fidèle à
    // SAS : les pas mis en file par CALL EXECUTE s'exécutent une fois l'étape
    // courante terminée). On préserve l'ordre d'accumulation.
    session
        .call_execute_queue
        .extend(std::mem::take(&mut r.call_execute_queue));
    // PUT (M14.2) : flush de la ligne maintenue en fin d'étape, puis rejeu
    // des lignes produites vers leurs destinations. Le rejeu a lieu AVANT les
    // NOTEs de fin d'étape (la sortie PUT « pendant » l'étape précède la NOTE
    // « N records were read »/« data set has N obs » dans le log SAS).
    r.put_flush_at_step_end();
    r.put_replay(session)?;
    // NOTEs d'erreurs/conversions collectées par l'évaluateur.
    if r.ctx.note_num_to_char {
        session
            .log
            .note("Numeric values have been converted to character values.");
    }
    if r.ctx.note_char_to_num {
        session
            .log
            .note("Character values have been converted to numeric values.");
    }
    if r.ctx.division_by_zero > 0 {
        session.log.note("Division by zero detected.");
    }
    if r.ctx.invalid_data > 0 {
        session.log.note("Invalid numeric data.");
    }
    if r.ctx.missing_generated > 0 {
        session.log.note(
            "Missing values were generated as a result of performing an operation on missing values.",
        );
    }
    Ok(())
}

/// Écrit les sorties DATA (ordre du statement DATA ; _LAST_ = la dernière) à
/// partir des builders du Runner. Partagé par les trois boucles d'exécution
/// (principale, UPDATE, MODIFY). Consomme `outputs`/`builders` (mem::take).
fn write_runner_outputs(r: &mut Runner, session: &mut Session, stats: &mut StepStats) -> Result<()> {
    let outputs = std::mem::take(&mut r.outputs);
    let builders = std::mem::take(&mut r.builders);
    for ((spec, bset), n_out) in outputs.iter().zip(builders).zip(&r.out_rows) {
        let mut columns: Vec<Column> = Vec::with_capacity(spec.kept_slots.len());
        let mut vars: Vec<VarMeta> = Vec::with_capacity(spec.kept_slots.len());
        for ((slot, b), out_name) in spec.kept_slots.iter().zip(bset).zip(&spec.out_names) {
            let v = &r.pdv.vars()[*slot];
            // RENAME= de sortie : la colonne écrite porte `out_name` (le
            // slot PDV garde son nom).
            let series = match b {
                ColBuilder::Num(vals) => Series::new(out_name.as_str().into(), vals),
                ColBuilder::Char(vals) => Series::new(out_name.as_str().into(), vals),
            };
            columns.push(series.into());
            let label = r.labels.get(&v.name.to_uppercase()).cloned();
            vars.push(VarMeta {
                name: out_name.clone(),
                ty: v.ty,
                length: v.length,
                format: v.format.clone(),
                label,
            });
        }
        let df = DataFrame::new(columns)?;
        let ds = SasDataset { df, vars };
        write_dataset_with_note(
            session,
            &spec.libref,
            &spec.table,
            &spec.display,
            &ds,
            *n_out,
            Some(stats),
        )?;
    }
    Ok(())
}

/// Exécute une étape DATA pilotée par un UPDATE (M16.5).
///
/// Le maître est lu séquentiellement (pilote l'itération). Pour chaque obs
/// maître (qui passe le WHERE= du maître), on cherche la PREMIÈRE obs de la
/// transaction de même clé ; si trouvée, on superpose ses variables NON
/// MANQUANTES (hors clés) au PDV. Le corps de l'étape s'exécute puis l'obs est
/// sortie (output implicite, sauf OUTPUT explicite). Les obs de transaction
/// sans maître correspondant sont IGNORÉES en v1 (divergence documentée vs SAS,
/// qui les insère). Plusieurs transactions pour une même clé : seule la
/// PREMIÈRE est appliquée.
fn execute_update(prog: StepProgram, session: &mut Session) -> Result<StepStats> {
    let StepProgram {
        pdv,
        stmts,
        update,
        outputs,
        has_explicit_output,
        uninitialized,
        initial_values,
        arrays,
        labels,
        flow_labels,
        ..
    } = prog;
    let upd = update.expect("execute_update requires UpdateData");

    for name in &uninitialized {
        session.log.note(&format!("Variable {name} is uninitialized."));
    }

    let trans = &upd.transaction;
    let trans_key_pos: Vec<usize> = upd
        .key_slots
        .iter()
        .map(|&slot| trans.var_slots.iter().position(|&s| s == slot).unwrap())
        .collect();
    let mut trans_index: HashMap<String, usize> = HashMap::new();
    for row in 0..trans.n_rows {
        let key_vals: Vec<Value> = trans_key_pos
            .iter()
            .map(|&pos| trans.columns[pos][row].clone())
            .collect();
        trans_index.entry(key_string(&key_vals)).or_insert(row);
    }
    let overlay_pos: Vec<(usize, usize)> = upd
        .overlay_slots
        .iter()
        .map(|&slot| (slot, trans.var_slots.iter().position(|&s| s == slot).unwrap()))
        .collect();

    let macro_symbols = session.macro_engine.symbols_snapshot();
    let format_catalog = session.format_catalog.clone();
    let mut r = build_um_runner(
        pdv,
        outputs,
        arrays,
        labels,
        &upd.by,
        macro_symbols,
        format_catalog,
        session.options.yearcutoff,
    );

    for (slot, v) in initial_values {
        r.pdv.set(slot, v);
    }

    // M16.6 : programme + étiquettes partagés (LINK/GOTO dans un UPDATE).
    r.program = std::rc::Rc::new(stmts);
    r.flow_labels = std::rc::Rc::new(flow_labels);

    let master = &upd.master;
    let mut master_read = 0usize;
    let suppress_implicit_output = has_explicit_output;

    // Slots issus UNIQUEMENT de la transaction (absents du maître). Comme ils
    // sont `from_input`, `reset_non_retained` ne les blanchit pas ; il faut les
    // remettre à MISSING au début de CHAQUE obs maître pour qu'une obs sans
    // transaction correspondante ne « traîne » pas la valeur d'une précédente.
    let trans_only_slots: Vec<usize> = upd
        .overlay_slots
        .iter()
        .copied()
        .filter(|s| !master.var_slots.contains(s))
        .collect();

    // Séquence des obs maître RETENUES (après WHERE=). FIRST./LAST. sont
    // calculés sur les transitions de clé BY DANS cette séquence.
    let mut kept_rows: Vec<usize> = Vec::with_capacity(master.n_rows);
    for m_row in 0..master.n_rows {
        if let Some(w) = &upd.master_where {
            // Charger seulement les variables maître pour évaluer le WHERE=.
            load_row(&mut r.pdv, master, m_row);
            let v = eval(w, &r.pdv, &mut r.ctx);
            if let Some(msg) = r.ctx.fatal.take() {
                let msg = msg.strip_prefix("ERROR: ").unwrap_or(&msg).to_string();
                return Err(SasError::runtime(msg));
            }
            if !v.truthy() {
                continue;
            }
        }
        kept_rows.push(m_row);
    }
    // Clés BY de chaque obs retenue (vide si pas de BY).
    let by_keys: Vec<Vec<Value>> = kept_rows
        .iter()
        .map(|&row| keys_at(master, row))
        .collect();

    for (seq, &m_row) in kept_rows.iter().enumerate() {
        r.pdv.n_ += 1;
        r.pdv.error_ = false;
        r.pdv.reset_non_retained();
        for &slot in &trans_only_slots {
            let init = match r.pdv.vars()[slot].ty {
                VarType::Num => Value::missing(),
                VarType::Char => Value::Char(String::new()),
            };
            r.pdv.set(slot, init);
        }
        load_row(&mut r.pdv, master, m_row);
        // FIRST./LAST. par variable BY (préfixe de clés vs voisins retenus).
        if !upd.by.is_empty() {
            let cur = &by_keys[seq];
            for (i, flags) in r.ctx.by_flags.iter_mut().enumerate() {
                let first = match seq.checked_sub(1) {
                    None => true,
                    Some(p) => prefix_changed(cur, &by_keys[p], i),
                };
                let last = match by_keys.get(seq + 1) {
                    None => true,
                    Some(next) => prefix_changed(cur, next, i),
                };
                flags.1 = first;
                flags.2 = last;
            }
        }
        master_read += 1;
        let key_vals: Vec<Value> = upd
            .key_slots
            .iter()
            .map(|&slot| r.pdv.get(slot).clone())
            .collect();
        if let Some(&t_row) = trans_index.get(&key_string(&key_vals)) {
            for &(slot, pos) in &overlay_pos {
                let tv = &trans.columns[pos][t_row];
                if !tv.is_missing() {
                    r.pdv.set(slot, tv.clone());
                }
            }
        }
        let flow = r.run_step_body()?;
        if flow == Flow::EndStep {
            break;
        }
        if flow != Flow::NextIter && !suppress_implicit_output {
            r.push_outputs();
        }
    }

    drain_runner_side_effects(&mut r, session)?;

    let mut stats = StepStats {
        read: Vec::new(),
        written: Vec::new(),
    };
    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        master_read, master.display
    ));
    stats.read.push((master.display.clone(), master_read));
    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        trans.n_rows, trans.display
    ));
    stats.read.push((trans.display.clone(), trans.n_rows));

    write_runner_outputs(&mut r, session, &mut stats)?;
    Ok(stats)
}

/// Exécute une étape DATA pilotée par un MODIFY (M16.5) : modification EN
/// PLACE. Le dataset est lu (séquentiellement, ou via POINT= en accès direct),
/// le corps modifie ses variables, et le dataset est RÉÉCRIT à l'identique
/// (mêmes colonnes/ordre) avec les valeurs modifiées. Pas d'output implicite ;
/// OUTPUT interdit (vérifié à la compilation).
fn execute_modify(prog: StepProgram, session: &mut Session) -> Result<StepStats> {
    let StepProgram {
        pdv,
        stmts,
        modify,
        outputs,
        uninitialized,
        initial_values,
        arrays,
        labels,
        flow_labels,
        ..
    } = prog;
    let m = modify.expect("execute_modify requires ModifyData");

    for name in &uninitialized {
        session.log.note(&format!("Variable {name} is uninitialized."));
    }

    let macro_symbols = session.macro_engine.symbols_snapshot();
    let format_catalog = session.format_catalog.clone();
    let mut r = build_um_runner(pdv, outputs, arrays, labels, &[], macro_symbols, format_catalog, session.options.yearcutoff);

    for (slot, v) in initial_values {
        r.pdv.set(slot, v);
    }
    // M16.6 : programme + étiquettes partagés (LINK/GOTO dans un MODIFY).
    r.program = std::rc::Rc::new(stmts);
    r.flow_labels = std::rc::Rc::new(flow_labels);
    let n_rows = m.data.n_rows;
    if let Some(slot) = m.nobs_slot {
        r.pdv.set(slot, Value::Num(n_rows as f64));
    }

    let mut buffer: Vec<Vec<Value>> = m.data.columns.clone();
    let mut rows_processed = 0usize;

    if let Some(point_slot) = m.point_slot {
        // ACCÈS DIRECT par POINT= : boucle implicite supprimée. Le corps
        // (typiquement `do i = 1 to nobs; p = i; modify ds; ...; end;`) pilote
        // l'itération ; chaque marqueur MODIFY charge l'obs à l'index POINT=
        // courant et capture la ligne PRÉCÉDEMMENT chargée (les assignations
        // entre deux marqueurs modifient l'obs courante). La dernière ligne est
        // capturée en fin d'étape. L'état partagé vit sur le Runner pour que le
        // bras `DsStmt::Modify` standard l'utilise.
        r.modify_state = Some(ModifyState {
            point_slot,
            cols: m.data.columns.clone(),
            var_slots: m.data.var_slots.clone(),
            cur_row: None,
            display: m.display.clone(),
            n_rows,
            error: None,
            touched: vec![false; n_rows],
        });
        r.pdv.n_ += 1;
        r.pdv.error_ = false;
        let _flow = r.run_step_body()?;
        if let Some(msg) = r.modify_state.as_mut().and_then(|st| st.error.take()) {
            return Err(SasError::runtime(msg));
        }
        if let Some(mut state) = r.modify_state.take() {
            capture_modify_state(&mut state, &r.pdv);
            buffer[..m.data.var_slots.len()]
                .clone_from_slice(&state.cols[..m.data.var_slots.len()]);
            rows_processed = state.touched.iter().filter(|t| **t).count();
        }
    } else {
        // `row` indexe à la fois le chargement et la capture du tampon : la
        // boucle range est intentionnelle.
        #[allow(clippy::needless_range_loop)]
        for row in 0..n_rows {
            r.pdv.n_ += 1;
            r.pdv.error_ = false;
            r.pdv.reset_non_retained();
            load_row(&mut r.pdv, &m.data, row);
            rows_processed += 1;
            let flow = r.run_step_body()?;
            for (pos, &slot) in m.data.var_slots.iter().enumerate() {
                buffer[pos][row] = r.pdv.get(slot).clone();
            }
            if flow == Flow::EndStep {
                break;
            }
        }
    }

    drain_runner_side_effects(&mut r, session)?;

    let columns: Vec<Column> = m
        .out_vars
        .iter()
        .enumerate()
        .map(|(pos, meta)| column_from_values(&meta.name, meta.ty, buffer[pos].iter()))
        .collect();
    let df = DataFrame::new(columns)?;
    let ds = SasDataset {
        df,
        vars: m.out_vars.clone(),
    };
    session.libs.get(&m.libref)?.write(&m.table, &ds)?;
    session.last_dataset = Some(m.display.clone());

    let mut stats = StepStats {
        read: Vec::new(),
        written: Vec::new(),
    };
    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        rows_processed, m.display
    ));
    stats.read.push((m.display.clone(), rows_processed));
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        m.display,
        n_rows,
        m.out_vars.len()
    ));
    stats
        .written
        .push((m.display.clone(), n_rows, m.out_vars.len()));

    // Les sorties DATA (le dataset nommé par `data X;`) coïncident avec la
    // table MODIFY réécrite en place : on les IGNORE (pas d'output implicite, et
    // l'écriture vide des builders écraserait la réécriture). OUTPUT explicite
    // est déjà interdit à la compilation ; un OUT= vers un autre dataset n'est
    // pas supporté en v1.
    let _ = &r.outputs;
    Ok(stats)
}

/// Capture les valeurs courantes du PDV dans le tampon `cols` à la ligne MODIFY
/// chargée (`cur_row`), puis remet le marqueur à `None`. No-op si aucune ligne
/// n'est chargée.
fn capture_modify_state(state: &mut ModifyState, pdv: &Pdv) {
    if let Some(row) = state.cur_row.take() {
        for (pos, &slot) in state.var_slots.iter().enumerate() {
            state.cols[pos][row] = pdv.get(slot).clone();
        }
    }
}

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

    fn exec_stmt(&mut self, stmt: &DsStmt) -> Result<Flow> {
        match stmt {
            DsStmt::Set { .. } => {
                let Some(input) = &self.input else {
                    // Impossible après compile() ; garde-fou.
                    return Err(SasError::runtime("SET statement without input data."));
                };
                if input.point_slot.is_some() {
                    self.exec_set_point()
                } else if input.by.is_empty() {
                    self.exec_set_concat()
                } else {
                    self.exec_set_interleave()
                }
            }
            DsStmt::Merge(_) => self.exec_merge(),
            // UPDATE (M16.5) : marqueur. La ligne maître est chargée par la
            // boucle externe (execute_update) AVANT le corps ; ici no-op.
            DsStmt::Update { .. } => Ok(Flow::Normal),
            // MODIFY (M16.5) : en lecture séquentielle, marqueur no-op (la
            // boucle externe charge/capture). En MODIFY+POINT= (modify_state
            // présent), le marqueur capture la ligne précédente puis charge
            // l'obs à l'index POINT= courant.
            DsStmt::Modify { .. } => {
                if self.modify_state.is_some() {
                    self.exec_modify_point()
                } else {
                    Ok(Flow::Normal)
                }
            }
            DsStmt::Assign { var, expr } => {
                let value = self.eval_checked(expr)?;
                // `arr = e;` sous un `DO OVER arr` : la cible est l'élément
                // courant (slot dans `ctx.do_over`), pas une variable du PDV.
                let slot = if let Some(s) = self.ctx.do_over.get(&var.to_uppercase()) {
                    *s
                } else if let Some(s) = self.pdv.slot(var) {
                    s
                } else {
                    return Err(SasError::runtime(format!(
                        "Variable {var} is not addressable."
                    )));
                };
                let coerced = self.coerce_assign(value, self.pdv.vars()[slot].ty);
                self.pdv.set(slot, coerced);
                Ok(Flow::Normal)
            }
            DsStmt::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let c = self.eval_checked(cond)?;
                if c.truthy() {
                    self.exec_stmt(then_branch)
                } else if let Some(e) = else_branch {
                    self.exec_stmt(e)
                } else {
                    Ok(Flow::Normal)
                }
            }
            DsStmt::SubsettingIf(cond) => {
                let c = self.eval_checked(cond)?;
                if c.truthy() {
                    Ok(Flow::Normal)
                } else {
                    Ok(Flow::NextIter)
                }
            }
            DsStmt::Block(stmts) => {
                for s in stmts {
                    let f = self.exec_stmt(s)?;
                    if f != Flow::Normal {
                        return Ok(f);
                    }
                }
                Ok(Flow::Normal)
            }
            DsStmt::DoLoop {
                index,
                to,
                by,
                while_,
                until,
                body,
            } => self.exec_do_loop(
                index.as_ref(),
                to.as_ref(),
                by.as_ref(),
                while_.as_ref(),
                until.as_ref(),
                body,
            ),
            DsStmt::DoList { index, items, body } => self.exec_do_list(index, items, body),
            DsStmt::DoOver { array, body } => self.exec_do_over(array, body),
            DsStmt::Select {
                selector,
                whens,
                otherwise,
            } => self.exec_select(selector.as_ref(), whens, otherwise.as_deref()),
            DsStmt::Delete => Ok(Flow::NextIter),
            DsStmt::Output(targets) => {
                if targets.is_empty() {
                    // `output;` : toutes les sorties.
                    self.push_outputs();
                } else {
                    // OUTPUT ciblé : uniquement les sorties nommées
                    // (résolues par display "WORK.A" — la compilation a
                    // validé qu'elles existent).
                    for t in targets {
                        let disp = t.display();
                        let Some(o) =
                            self.outputs.iter().position(|s| s.display == disp)
                        else {
                            // Impossible après compile() ; garde-fou.
                            return Err(SasError::runtime(format!(
                                "Output dataset {disp} is not in the DATA statement output list."
                            )));
                        };
                        self.push_one(o);
                    }
                }
                Ok(Flow::Normal)
            }
            DsStmt::Stop => Ok(Flow::EndStep),
            DsStmt::Sum { var, expr } => {
                // Sum statement `var + expr;` — sémantique SUM de SAS : les
                // missings sont IGNORÉS, jamais propagés. Un incrément
                // missing ajoute 0 (sans `missing_generated`), et un
                // accumulateur missing (l'utilisateur a pu assigner `.`)
                // est traité comme 0 : `total=.; total+x;` donne x.
                let value = self.eval_checked(expr)?;
                let incr = self.coerce_sum_operand(value);
                let Some(slot) = self.pdv.slot(var) else {
                    return Err(SasError::runtime(format!(
                        "Variable {var} is not addressable."
                    )));
                };
                let acc = match self.pdv.get(slot) {
                    Value::Num(f) => *f,
                    // Missing (ou char dégénéré) : repart de 0.
                    _ => 0.0,
                };
                self.pdv.set(slot, Value::Num(acc + incr));
                Ok(Flow::Normal)
            }
            DsStmt::AssignIndexed {
                array,
                indices,
                expr,
            } => {
                // Indices évalués avec les MÊMES règles que les rvalues
                // (coercition num + arrondi ; missing/hors bornes → l'étape
                // s'arrête), puis coercition vers le type de l'élément.
                let mut idx_vals = Vec::with_capacity(indices.len());
                for index in indices {
                    idx_vals.push(self.eval_checked(index)?);
                }
                let slot = self.resolve_subscript(array, &idx_vals)?;
                let value = self.eval_checked(expr)?;
                let coerced = self.coerce_assign(value, self.pdv.vars()[slot].ty);
                self.pdv.set(slot, coerced);
                Ok(Flow::Normal)
            }
            DsStmt::CallRoutine { name, args } => self.exec_call_routine(name, args),
            // Étiquette (M16.6) : l'étiquette elle-même est un marqueur
            // (résolue par index dans le pilote de niveau supérieur) ; on
            // exécute simplement le statement étiqueté.
            DsStmt::Labeled { stmt, .. } => self.exec_stmt(stmt),
            // GOTO/LINK/RETURN (M16.6) : remontent comme Flow non-Normal
            // jusqu'au pilote de niveau supérieur (`run_step_body`), qui pilote
            // le compteur de programme et la pile de retour. Traversent les
            // boucles DO englobantes (mêmes règles de propagation que EndStep).
            DsStmt::Goto(label) => Ok(Flow::Goto(label.to_uppercase())),
            // LINK : exécute la sous-routine INLINE (du label au prochain
            // RETURN) puis reprend après le LINK (Flow::Normal). Exécuté ICI —
            // et non remonté — pour qu'un LINK à l'intérieur d'une boucle DO
            // n'abandonne PAS la boucle (la pile d'appels Rust = pile de
            // retour). Un Flow non-Normal de la sous-routine (GOTO non local,
            // DELETE, STOP, fin d'entrée) est propagé tel quel.
            DsStmt::Link(label) => self.exec_link_subroutine(&label.to_uppercase()),
            DsStmt::Return => Ok(Flow::Return),
            // DECLARE HASH (M17.1) : l'objet et ses options sont déjà résolus à
            // la compilation (dans `EvalCtx.hashes`). Le statement DECLARE est un
            // marqueur déclaratif ; aucune action runtime (l'objet existe pour
            // toute l'étape, comme les objets hash SAS au sein d'une étape DATA).
            DsStmt::DeclareHash { .. } => Ok(Flow::Normal),
            // DECLARE HITER (M17.2) : itérateur enregistré à la compilation
            // (dans `EvalCtx.hash_iters`). Marqueur déclaratif, aucune action.
            DsStmt::DeclareHiter { .. } => Ok(Flow::Normal),
            // Appel de méthode d'objet hash en STATEMENT (M17.1/M17.2) : le code
            // retour est ignoré.
            DsStmt::HashMethod(call) => {
                self.exec_hash_method_rc(&call.object, &call.method, &call.args)?;
                Ok(Flow::Normal)
            }
            // INPUT (M14) : lit le PROCHAIN enregistrement de la source texte
            // dans le PDV. Comme SET, l'épuisement de la source termine
            // l'étape IMMÉDIATEMENT (au milieu de l'itération).
            DsStmt::Input(items) => self.exec_input(items),
            // FILE (M14.2) : change la destination courante des PUT.
            DsStmt::File { dest } => self.exec_file(dest),
            // PUT (M14.2) : rend les items dans la ligne de sortie courante.
            DsStmt::Put(items) => self.exec_put(items),
            // Directives de compilation / déclaratives : rien à exécuter.
            DsStmt::Keep(_)
            | DsStmt::Drop(_)
            | DsStmt::Retain(_)
            | DsStmt::Length(_)
            | DsStmt::By(_)
            | DsStmt::Format(_)
            | DsStmt::Label(_)
            | DsStmt::Attrib(_)
            | DsStmt::Infile { .. }
            | DsStmt::Datalines(_)
            | DsStmt::Array { .. } => Ok(Flow::Normal),
        }
    }

    /// Exécute une CALL routine. Routines supportées (M11.5 + M15.6) :
    /// STREAMINIT, SYMPUT, SYMPUTX, MISSING, EXECUTE, SORTN, SORTC, CATS,
    /// SCAN, LABEL, VNAME. Toute autre → erreur « not yet implemented ».
    ///
    /// Les routines qui ÉCRIVENT dans un argument (MISSING, SORTN/SORTC, CATS,
    /// SCAN, LABEL, VNAME) résolvent cet argument en lvalue (variable ou
    /// élément d'array) via `resolve_lvalue_slot`. SYMPUT/SYMPUTX diffèrent
    /// l'écriture macro à la fin de l'étape (règle de visibilité SAS) ;
    /// EXECUTE met du code en file pour exécution post-étape.
    fn exec_call_routine(&mut self, name: &str, args: &[crate::ast::Expr]) -> Result<Flow> {
        // CALL STREAMINIT(seed) — initialise the RNG stream. Accepts an
        // optional single argument (integer seed); no argument → no-op.
        if name.eq_ignore_ascii_case("streaminit") {
            if let Some(seed_expr) = args.first() {
                let seed_val = self.eval_checked(seed_expr)?;
                if let Value::Num(f) = seed_val {
                    self.ctx.rng_state = super::functions::streaminit_seed(f as i64);
                    self.ctx.rng_spare = None; // invalidate cached Box-Muller spare
                }
                // missing seed value → no-op (as per spec)
            }
            return Ok(Flow::Normal);
        }

        let upper = name.to_uppercase();
        match upper.as_str() {
            "SYMPUT" => self.call_symput(args, false),
            "SYMPUTX" => self.call_symput(args, true),
            "MISSING" => self.call_missing(args),
            "EXECUTE" => self.call_execute(args),
            "SORTN" => self.call_sort(args, false),
            "SORTC" => self.call_sort(args, true),
            "CATS" => self.call_cats(args),
            "SCAN" => self.call_scan(args),
            "LABEL" => self.call_label(args),
            "VNAME" => self.call_vname(args),
            _ => Err(SasError::runtime(format!(
                "CALL routine {upper} is not yet implemented."
            ))),
        }
    }

    /// Résout un argument qui DOIT être une variable scalaire ou un élément
    /// d'array indexé (`var` ou `arr{i}`) en son slot PDV. Utilisé par les
    /// CALL routines qui écrivent dans leurs arguments (MISSING, CATS, SCAN,
    /// LABEL, VNAME). Une expression qui n'est pas une lvalue → erreur.
    fn resolve_lvalue_slot(&mut self, arg: &crate::ast::Expr) -> Result<usize> {
        use crate::ast::Expr;
        match arg {
            Expr::Var(name) => self.pdv.slot(name).ok_or_else(|| {
                SasError::runtime(format!("Variable {name} is not addressable."))
            }),
            Expr::Index { name, indices } => {
                let mut idx_vals = Vec::with_capacity(indices.len());
                for index in indices {
                    idx_vals.push(self.eval_checked(index)?);
                }
                self.resolve_subscript(name, &idx_vals)
            }
            // `arr(i)` / `arr(i,j)` se parse en Call ; si le nom est un array,
            // c'est une référence d'élément.
            Expr::Call { name, args } if !args.is_empty()
                && self.ctx.arrays.contains_key(&name.to_uppercase()) =>
            {
                let mut idx_vals = Vec::with_capacity(args.len());
                for a in args {
                    idx_vals.push(self.eval_checked(a)?);
                }
                self.resolve_subscript(name, &idx_vals)
            }
            _ => Err(SasError::runtime(
                "CALL routine argument must be a variable reference.",
            )),
        }
    }

    /// CALL SYMPUT(name, value) / CALL SYMPUTX(name, value) — écrit un
    /// symbole macro. SYMPUTX rogne EN PLUS les blancs de tête ET de fin de
    /// la valeur (et un nombre est formaté sans blancs) ; SYMPUT garde la
    /// valeur char telle quelle. Les deux trim­ent le nom.
    fn call_symput(&mut self, args: &[crate::ast::Expr], x: bool) -> Result<Flow> {
        if args.len() != 2 {
            return Err(SasError::runtime(if x {
                "CALL SYMPUTX requires exactly two arguments (name, value)."
            } else {
                "CALL SYMPUT requires exactly two arguments (name, value)."
            }));
        }
        let name_val = self.eval_checked(&args[0])?;
        let value_val = self.eval_checked(&args[1])?;
        let sym_name = symput_string(name_val);
        let sym_value = symput_string(value_val);
        // SYMPUTX rogne les deux bords de la valeur ; SYMPUT la garde telle
        // quelle (mais BEST12. d'un nombre est déjà cadré à gauche).
        let sym_value = if x {
            sym_value.trim().to_string()
        } else {
            sym_value
        };
        self.ctx
            .symput_writes
            .push((sym_name.trim().to_string(), sym_value));
        Ok(Flow::Normal)
    }

    /// CALL MISSING(var, var, ...) — met chaque variable argument à missing
    /// (`.` pour numérique, `""` pour caractère). Chaque argument doit être
    /// une lvalue (variable scalaire ou élément d'array).
    fn call_missing(&mut self, args: &[crate::ast::Expr]) -> Result<Flow> {
        for arg in args {
            let slot = self.resolve_lvalue_slot(arg)?;
            let init = match self.pdv.vars()[slot].ty {
                VarType::Num => Value::missing(),
                VarType::Char => Value::Char(String::new()),
            };
            self.pdv.set(slot, init);
        }
        Ok(Flow::Normal)
    }

    /// CALL EXECUTE(arg) — met le texte résolu de `arg` en file pour
    /// exécution APRÈS l'étape DATA courante. `arg` est évalué comme une
    /// expression caractère ; sa valeur est concaténée (avec un espace de
    /// séparation) au code mis en file. La file est rejouée par l'exécuteur
    /// une fois l'étape terminée.
    ///
    /// Limites documentées : la résolution macro (`%nrstr`, exécution macro à
    /// l'évaluation vs à l'exécution) n'est PAS distinguée — le texte est
    /// rejoué tel quel comme un programme SAS ordinaire (qui passe par le
    /// processeur macro à son tour). Les références `&`/`%` du texte mis en
    /// file sont donc résolues au MOMENT du rejeu, pas de l'appel.
    fn call_execute(&mut self, args: &[crate::ast::Expr]) -> Result<Flow> {
        if args.len() != 1 {
            return Err(SasError::runtime(
                "CALL EXECUTE requires exactly one argument.",
            ));
        }
        let v = self.eval_checked(&args[0])?;
        let code = match v {
            Value::Char(s) => s,
            Value::Num(f) => format_best(f, 12).trim().to_string(),
            Value::Missing(_) => String::new(),
        };
        self.call_execute_queue.push(code);
        Ok(Flow::Normal)
    }

    /// CALL SORTN(arr, ...) / CALL SORTC(arr, ...) — trie EN PLACE, par ordre
    /// croissant (`sas_cmp`), les valeurs des variables/éléments passés en
    /// arguments. La forme habituelle est un nom d'array (`call sortn(of a[*])`
    /// — ici on accepte chaque élément ou un array entier), mais SAS accepte
    /// aussi une liste de variables. On collecte donc tous les slots cibles
    /// (un argument array entier dépliant ses slots), on récupère les valeurs,
    /// on les trie, puis on les ré-assigne dans l'ordre des slots.
    fn call_sort(&mut self, args: &[crate::ast::Expr], char_sort: bool) -> Result<Flow> {
        use crate::ast::Expr;
        // Collecte des slots cibles, dans l'ordre des arguments. Un argument
        // qui nomme un array entier (`call sortn(arr)`) déplie tous ses slots.
        let mut slots: Vec<usize> = Vec::new();
        for arg in args {
            match arg {
                Expr::Var(name) if self.ctx.arrays.contains_key(&name.to_uppercase()) => {
                    let elems = self.ctx.arrays[&name.to_uppercase()].slots.clone();
                    slots.extend(elems);
                }
                _ => slots.push(self.resolve_lvalue_slot(arg)?),
            }
        }
        if slots.is_empty() {
            return Ok(Flow::Normal);
        }
        // Cohérence de type : SORTN attend du numérique, SORTC du caractère.
        // On ne bloque pas (SAS est permissif) mais on lit les valeurs telles
        // quelles ; `sas_cmp` ordonne num et char dans leur domaine.
        let _ = char_sort;
        let mut values: Vec<Value> = slots.iter().map(|&s| self.pdv.get(s).clone()).collect();
        values.sort_by(|a, b| a.sas_cmp(b));
        for (&slot, v) in slots.iter().zip(values) {
            let coerced = self.coerce_assign(v, self.pdv.vars()[slot].ty);
            self.pdv.set(slot, coerced);
        }
        Ok(Flow::Normal)
    }

    /// CALL CATS(result, item, ...) — concatène `item...` (chacun rogné des
    /// blancs de bord, comme la fonction CATS) dans la variable caractère
    /// `result`. Le résultat est tronqué à la longueur de `result` (sémantique
    /// PDV normale via `set`). Le premier argument est l'lvalue de sortie.
    fn call_cats(&mut self, args: &[crate::ast::Expr]) -> Result<Flow> {
        if args.is_empty() {
            return Err(SasError::runtime(
                "CALL CATS requires at least one argument (the result variable).",
            ));
        }
        let result_slot = self.resolve_lvalue_slot(&args[0])?;
        let mut out = String::new();
        for arg in &args[1..] {
            let v = self.eval_checked(arg)?;
            let s = match v {
                Value::Char(s) => s,
                Value::Num(f) => format_best(f, 12).trim().to_string(),
                Value::Missing(k) => k.display(),
            };
            out.push_str(s.trim());
        }
        let coerced = self.coerce_assign(Value::Char(out), self.pdv.vars()[result_slot].ty);
        self.pdv.set(result_slot, coerced);
        Ok(Flow::Normal)
    }

    /// CALL SCAN(string, n, result[, delims]) — extrait le n-ième mot de
    /// `string` (n<0 = depuis la fin) dans la variable caractère `result`.
    /// Réutilise la sémantique de la fonction SCAN. Le 3e argument est
    /// l'lvalue de sortie.
    fn call_scan(&mut self, args: &[crate::ast::Expr]) -> Result<Flow> {
        if args.len() < 3 {
            return Err(SasError::runtime(
                "CALL SCAN requires at least three arguments (string, n, result).",
            ));
        }
        // Le mot est calculé par la fonction SCAN (string, n[, delims]).
        let mut fn_args = vec![self.eval_checked(&args[0])?, self.eval_checked(&args[1])?];
        if let Some(delim_arg) = args.get(3) {
            fn_args.push(self.eval_checked(delim_arg)?);
        }
        let result_slot = self.resolve_lvalue_slot(&args[2])?;
        let word = super::functions::call("SCAN", &fn_args, &mut self.ctx)
            .unwrap_or(Value::Char(String::new()));
        if let Some(msg) = self.ctx.fatal.take() {
            let msg = msg.strip_prefix("ERROR: ").unwrap_or(&msg).to_string();
            return Err(SasError::runtime(msg));
        }
        let coerced = self.coerce_assign(word, self.pdv.vars()[result_slot].ty);
        self.pdv.set(result_slot, coerced);
        Ok(Flow::Normal)
    }

    /// CALL LABEL(var, result) — pose dans la variable caractère `result` le
    /// libellé de `var`. Si `var` n'a pas de libellé, SAS renvoie le NOM de la
    /// variable (comportement reproduit ici).
    fn call_label(&mut self, args: &[crate::ast::Expr]) -> Result<Flow> {
        if args.len() != 2 {
            return Err(SasError::runtime(
                "CALL LABEL requires exactly two arguments (variable, result).",
            ));
        }
        let var_slot = self.resolve_lvalue_slot(&args[0])?;
        let result_slot = self.resolve_lvalue_slot(&args[1])?;
        let var_name = self.pdv.vars()[var_slot].name.clone();
        let label = self
            .labels
            .get(&var_name.to_uppercase())
            .cloned()
            .unwrap_or(var_name);
        let coerced = self.coerce_assign(Value::Char(label), self.pdv.vars()[result_slot].ty);
        self.pdv.set(result_slot, coerced);
        Ok(Flow::Normal)
    }

    /// CALL VNAME(var, result) — pose dans la variable caractère `result` le
    /// NOM de `var` (tel que stocké au PDV, casse de première référence).
    fn call_vname(&mut self, args: &[crate::ast::Expr]) -> Result<Flow> {
        if args.len() != 2 {
            return Err(SasError::runtime(
                "CALL VNAME requires exactly two arguments (variable, result).",
            ));
        }
        let var_slot = self.resolve_lvalue_slot(&args[0])?;
        let result_slot = self.resolve_lvalue_slot(&args[1])?;
        let var_name = self.pdv.vars()[var_slot].name.clone();
        let coerced =
            self.coerce_assign(Value::Char(var_name), self.pdv.vars()[result_slot].ty);
        self.pdv.set(result_slot, coerced);
        Ok(Flow::Normal)
    }

    /// Résout un sous-script d'array (un ou plusieurs indices) en slot PDV :
    /// coercition numérique (mêmes règles que `eval::coerce_num`), arrondi au
    /// plus proche ; missing, hors bornes ou nombre d'indices invalide →
    /// erreur qui stoppe l'étape. Un index unique sur un array multi-dim est
    /// interprété linéairement (row-major).
    fn resolve_subscript(&mut self, array: &str, idx_vals: &[Value]) -> Result<usize> {
        let mut idxs: Vec<i64> = Vec::with_capacity(idx_vals.len());
        for idx_val in idx_vals {
            let idx = coerce_num(idx_val, &mut self.ctx).map(f64::round);
            if self.ctx.error_flag {
                self.pdv.error_ = true;
                self.ctx.error_flag = false;
            }
            match idx {
                Some(i) => idxs.push(i as i64),
                None => return Err(SasError::runtime("Array subscript out of range.")),
            }
        }
        let Some(def) = self.ctx.arrays.get(&array.to_uppercase()) else {
            // Impossible après compile() ; garde-fou.
            return Err(SasError::runtime(format!(
                "Undeclared array referenced: {array}."
            )));
        };
        match def.linear_index(&idxs) {
            Some(lin) => Ok(def.slots[lin]),
            None => Err(SasError::runtime("Array subscript out of range.")),
        }
    }

    /// DO itératif / conditionnel — sémantique SAS exacte (cf. en-tête).
    /// from/to/by sont évalués UNE FOIS à l'entrée ; l'index vit au PDV
    /// (le corps peut le modifier). Tout Flow non Normal du corps sort de
    /// la boucle ET remonte (DELETE/STOP/subsetting-IF/SET épuisé).
    #[allow(clippy::too_many_arguments)]
    fn exec_do_loop(
        &mut self,
        index: Option<&(String, crate::ast::Expr)>,
        to: Option<&crate::ast::Expr>,
        by: Option<&crate::ast::Expr>,
        while_: Option<&crate::ast::Expr>,
        until: Option<&crate::ast::Expr>,
        body: &[DsStmt],
    ) -> Result<Flow> {
        // Bornes figées à l'entrée (règle SAS). BY défaut 1.0.
        let idx_slot = match index {
            Some((name, from_expr)) => {
                let from = self.loop_control(from_expr)?;
                let Some(slot) = self.pdv.slot(name) else {
                    return Err(SasError::runtime(format!(
                        "Variable {name} is not addressable."
                    )));
                };
                self.pdv.set(slot, Value::Num(from));
                Some(slot)
            }
            None => None,
        };
        let to_v = match to {
            Some(e) => Some(self.loop_control(e)?),
            None => None,
        };
        let by_v = match by {
            Some(e) => self.loop_control(e)?,
            None => 1.0,
        };

        // Garde-fou anti-boucle infinie, PAR exécution de la boucle.
        let mut iters: u64 = 0;
        loop {
            // (1) Test TO : by>0 → i<=to, by<0 → i>=to ; by==0 → jamais de
            // sortie par TO (boucle potentiellement infinie, comme SAS —
            // couverte par le garde-fou).
            if let (Some(slot), Some(stop)) = (idx_slot, to_v) {
                let cur = self.index_value(slot);
                if (by_v > 0.0 && cur > stop) || (by_v < 0.0 && cur < stop) {
                    break;
                }
            }
            // (2) Test WHILE (avant le corps).
            if let Some(cond) = while_ {
                if !self.eval_checked(cond)?.truthy() {
                    break;
                }
            }
            // (3) Corps : un Flow non Normal traverse le DO et remonte.
            for s in body {
                let f = self.exec_stmt(s)?;
                if f != Flow::Normal {
                    return Ok(f);
                }
            }
            // (4) Test UNTIL (après le corps : au moins un tour exécuté).
            if let Some(cond) = until {
                if self.eval_checked(cond)?.truthy() {
                    break;
                }
            }
            // (5) Incrément de l'index (missing + by = missing, comme
            // l'arithmétique SAS).
            if let Some(slot) = idx_slot {
                if let Value::Num(f) = self.pdv.get(slot) {
                    let next = f + by_v;
                    self.pdv.set(slot, Value::Num(next));
                }
            }
            iters += 1;
            if iters > 10_000_000 {
                return Err(SasError::runtime(
                    "DO loop exceeded 10000000 iterations; stopping (possible infinite loop).",
                ));
            }
        }
        Ok(Flow::Normal)
    }

    /// DO sur une liste de valeurs (M16.3). L'index prend successivement
    /// chaque valeur de la liste développée (valeurs explicites évaluées une
    /// par une ; sous-listes `from to e [by k]` énumérées comme un DO
    /// classique). Le corps s'exécute une fois par valeur ; un Flow non
    /// Normal du corps sort de la boucle et remonte.
    fn exec_do_list(
        &mut self,
        index: &str,
        items: &[crate::ast::DoListItem],
        body: &[DsStmt],
    ) -> Result<Flow> {
        use crate::ast::DoListItem;
        let Some(idx_slot) = self.pdv.slot(index) else {
            return Err(SasError::runtime(format!(
                "Variable {index} is not addressable."
            )));
        };
        let idx_ty = self.pdv.vars()[idx_slot].ty;
        let mut iters: u64 = 0;
        for item in items {
            match item {
                DoListItem::Value(e) => {
                    let v = self.eval_checked(e)?;
                    let coerced = self.coerce_assign(v, idx_ty);
                    self.pdv.set(idx_slot, coerced);
                    if let Some(f) = self.run_do_list_body(body)? {
                        return Ok(f);
                    }
                    self.bump_do_list_guard(&mut iters)?;
                }
                DoListItem::Range { from, to, by } => {
                    let from_v = self.loop_control(from)?;
                    let to_v = self.loop_control(to)?;
                    let by_v = match by {
                        Some(b) => self.loop_control(b)?,
                        None => 1.0,
                    };
                    if by_v == 0.0 {
                        return Err(SasError::runtime(
                            "Invalid DO loop control information.",
                        ));
                    }
                    let mut cur = from_v;
                    loop {
                        if (by_v > 0.0 && cur > to_v) || (by_v < 0.0 && cur < to_v) {
                            break;
                        }
                        self.pdv.set(idx_slot, Value::Num(cur));
                        if let Some(f) = self.run_do_list_body(body)? {
                            return Ok(f);
                        }
                        self.bump_do_list_guard(&mut iters)?;
                        cur += by_v;
                    }
                }
            }
        }
        Ok(Flow::Normal)
    }

    /// Exécute le corps d'un DO (liste/over) ; renvoie `Some(flow)` si un Flow
    /// non Normal doit remonter, `None` sinon.
    fn run_do_list_body(&mut self, body: &[DsStmt]) -> Result<Option<Flow>> {
        for s in body {
            let f = self.exec_stmt(s)?;
            if f != Flow::Normal {
                return Ok(Some(f));
            }
        }
        Ok(None)
    }

    /// Pilote de niveau supérieur d'UNE itération de l'étape (M16.6). Exécute
    /// les statements de premier niveau via un COMPTEUR DE PROGRAMME, ce qui
    /// permet GOTO (saut), LINK (appel de sous-routine, pile d'adresses de
    /// retour) et RETURN (dépile). Sans aucune de ces directives, c'est un
    /// parcours séquentiel équivalent à `for stmt in stmts`.
    ///
    /// Renvoie le `Flow` TERMINAL de l'itération vu par la boucle implicite :
    /// `Normal` (corps épuisé → output implicite), `NextIter` (DELETE / IF
    /// subsetting faux → pas d'output) ou `EndStep` (STOP / fin d'entrée).
    /// Les `Flow::Goto/Link/Return` sont entièrement consommés ici (jamais
    /// remontés au-delà).
    ///
    /// Sémantique RETURN : avec un LINK actif, dépile l'adresse de retour ;
    /// sans LINK actif (pile vide), RETURN termine l'itération NORMALEMENT
    /// (output implicite), comme en SAS. Un LINK sans RETURN atteignant la fin
    /// du corps fait simplement tomber le PC en bout de liste (retour implicite
    /// en fin d'étape).
    fn run_step_body(&mut self) -> Result<Flow> {
        let program = self.program.clone();
        let flow_labels = self.flow_labels.clone();
        let mut pc: usize = 0;
        // Garde-fou anti-boucle (GOTO pouvant boucler indéfiniment).
        let mut steps: u64 = 0;
        while pc < program.len() {
            steps += 1;
            if steps > 100_000_000 {
                return Err(SasError::runtime(
                    "DATA step control flow (GOTO/LINK) appears to loop infinitely; stopping.",
                ));
            }
            match self.exec_stmt(&program[pc])? {
                Flow::Normal => pc += 1,
                Flow::NextIter => return Ok(Flow::NextIter),
                Flow::EndStep => return Ok(Flow::EndStep),
                Flow::Goto(label) => {
                    // Cible validée à la compilation : présente dans flow_labels.
                    let Some(&target) = flow_labels.get(&label) else {
                        return Err(SasError::runtime(format!(
                            "The statement label {label} is not defined in the DATA step."
                        )));
                    };
                    pc = target;
                }
                // RETURN au niveau supérieur (hors sous-routine LINK) : fin
                // d'itération normale (output implicite), comme en SAS.
                Flow::Return => return Ok(Flow::Normal),
            }
        }
        // Corps épuisé : fin d'itération normale.
        Ok(Flow::Normal)
    }

    /// Exécute INLINE le corps d'une sous-routine LINK (M16.6) : du statement
    /// étiqueté `label` (premier niveau) jusqu'au prochain `RETURN` (ou la fin
    /// de l'étape). Renvoie le `Flow` à propager au-delà du LINK :
    /// - `Flow::Normal` après un RETURN (ou la fin de l'étape) → on reprend
    ///   normalement après le LINK ;
    /// - `Flow::NextIter`/`EndStep` (DELETE/STOP/fin d'entrée dans la
    ///   sous-routine) → remontés tels quels (terminent l'itération/l'étape) ;
    /// - `Flow::Goto` (GOTO dans la sous-routine) → remonté pour saut non local.
    ///
    /// Un LINK imbriqué (`link` dans la sous-routine) récursionne ici : la pile
    /// d'appels Rust EST la pile d'adresses de retour.
    fn exec_link_subroutine(&mut self, label: &str) -> Result<Flow> {
        let program = self.program.clone();
        let flow_labels = self.flow_labels.clone();
        let Some(&start) = flow_labels.get(label) else {
            return Err(SasError::runtime(format!(
                "The statement label {label} is not defined in the DATA step."
            )));
        };
        let mut pc = start;
        let mut steps: u64 = 0;
        while pc < program.len() {
            steps += 1;
            if steps > 100_000_000 {
                return Err(SasError::runtime(
                    "DATA step control flow (LINK) appears to loop infinitely; stopping.",
                ));
            }
            match self.exec_stmt(&program[pc])? {
                Flow::Normal => pc += 1,
                // RETURN : fin de la sous-routine → reprise après le LINK.
                Flow::Return => return Ok(Flow::Normal),
                // GOTO dans une sous-routine : saut non local (remonté au
                // pilote de niveau supérieur, qui repositionne le PC global —
                // la sous-routine est abandonnée, comme en SAS).
                Flow::Goto(label) => return Ok(Flow::Goto(label)),
                // DELETE / STOP / fin d'entrée : terminent l'itération/l'étape.
                Flow::NextIter => return Ok(Flow::NextIter),
                Flow::EndStep => return Ok(Flow::EndStep),
            }
        }
        // Fin de l'étape atteinte sans RETURN : retour implicite.
        Ok(Flow::Normal)
    }

    /// Garde-fou anti-boucle infinie partagé par DO liste / DO OVER.
    fn bump_do_list_guard(&self, iters: &mut u64) -> Result<()> {
        *iters += 1;
        if *iters > 10_000_000 {
            return Err(SasError::runtime(
                "DO loop exceeded 10000000 iterations; stopping (possible infinite loop).",
            ));
        }
        Ok(())
    }

    /// DO OVER (M16.3) : itère implicitement sur les éléments d'un array dans
    /// l'ordre row-major (= ordre des `slots`, déjà row-major par
    /// construction). À chaque tour, le slot de l'élément courant est exposé
    /// via `ctx.do_over` (référence nue au nom de l'array = élément courant).
    /// Un Flow non Normal du corps sort de la boucle, en restaurant l'état
    /// `do_over` précédent.
    fn exec_do_over(&mut self, array: &str, body: &[DsStmt]) -> Result<Flow> {
        let upper = array.to_uppercase();
        let Some(def) = self.ctx.arrays.get(&upper) else {
            return Err(SasError::runtime(format!(
                "Undeclared array referenced: {array}."
            )));
        };
        let slots = def.slots.clone();
        // Sauvegarde de l'entrée éventuellement masquée (DO OVER imbriqués sur
        // le même nom — improbable, mais correct).
        let prev = self.ctx.do_over.remove(&upper);
        let mut iters: u64 = 0;
        let mut out = Flow::Normal;
        for slot in slots {
            self.ctx.do_over.insert(upper.clone(), slot);
            if let Some(f) = self.run_do_list_body(body)? {
                out = f;
                break;
            }
            self.bump_do_list_guard(&mut iters)?;
        }
        // Restaure l'état précédent.
        match prev {
            Some(p) => {
                self.ctx.do_over.insert(upper, p);
            }
            None => {
                self.ctx.do_over.remove(&upper);
            }
        }
        Ok(out)
    }

    /// Valeur courante de l'index pour le test TO. Un index rendu missing
    /// par le corps se classe SOUS tous les nombres (ordre SAS) :
    /// -inf fait sortir avec by<0 et continuer avec by>0.
    fn index_value(&self, slot: usize) -> f64 {
        match self.pdv.get(slot) {
            Value::Num(f) => *f,
            Value::Missing(_) => f64::NEG_INFINITY,
            // Impossible : l'index est créé Num par la compilation.
            Value::Char(_) => 0.0,
        }
    }

    /// Évalue une borne de DO (from/to/by) en numérique. Missing (ou char
    /// vide/invalide) → erreur runtime "Invalid DO loop control
    /// information." qui stoppe l'étape (divergence documentée : SAS émet
    /// une erreur d'exécution équivalente et stoppe l'étape aussi).
    fn loop_control(&mut self, expr: &crate::ast::Expr) -> Result<f64> {
        let v = self.eval_checked(expr)?;
        match v {
            Value::Num(f) => Ok(f),
            Value::Char(s) => {
                self.ctx.note_char_to_num = true;
                if let Ok(f) = s.trim().parse::<f64>() {
                    return Ok(f);
                }
                self.ctx.invalid_data += 1;
                self.pdv.error_ = true;
                Err(SasError::runtime("Invalid DO loop control information."))
            }
            Value::Missing(_) => {
                Err(SasError::runtime("Invalid DO loop control information."))
            }
        }
    }

    /// Coercition numérique d'un opérande de sum statement. Mêmes règles de
    /// conversion char→num que l'évaluateur (note + invalid data + _ERROR_
    /// sur une chaîne invalide), MAIS un résultat missing contribue 0 sans
    /// incrémenter `missing_generated` (le SUM ignore les missings).
    fn coerce_sum_operand(&mut self, value: Value) -> f64 {
        match value {
            Value::Num(f) => f,
            Value::Missing(_) => 0.0,
            Value::Char(s) => {
                self.ctx.note_char_to_num = true;
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    0.0
                } else {
                    match trimmed.parse::<f64>() {
                        Ok(f) => f,
                        Err(_) => {
                            self.ctx.invalid_data += 1;
                            self.pdv.error_ = true;
                            0.0
                        }
                    }
                }
            }
        }
    }

    /// Évalue, propage les fatals, reporte `_ERROR_` au PDV.
    /// SELECT/WHEN/OTHERWISE (M16.1). Cherche la PREMIÈRE clause WHEN qui
    /// correspond, exécute son corps et retourne (pas de fall-through). Sinon
    /// OTHERWISE s'il existe, sinon erreur runtime fidèle à SAS.
    ///
    /// Forme sélecteur (`selector = Some`) : le sélecteur est évalué UNE seule
    /// fois ; chaque valeur de WHEN est comparée avec la sémantique `=` de SAS
    /// (`sas_values_equal`). Forme booléenne (`selector = None`) : chaque WHEN
    /// porte une unique condition évaluée en contexte booléen.
    fn exec_select(
        &mut self,
        selector: Option<&crate::ast::Expr>,
        whens: &[crate::ast::WhenClause],
        otherwise: Option<&DsStmt>,
    ) -> Result<Flow> {
        // Sélecteur évalué une seule fois (sémantique SAS).
        let sel_val = match selector {
            Some(expr) => Some(self.eval_checked(expr)?),
            None => None,
        };
        for when in whens {
            let matched = match &sel_val {
                // Forme sélecteur : vrai si le sélecteur égale l'une des
                // valeurs listées (court-circuit dès le premier match).
                Some(sv) => {
                    let mut hit = false;
                    for v in &when.values {
                        let val = self.eval_checked(v)?;
                        if sas_values_equal(sv.clone(), val, &mut self.ctx) {
                            hit = true;
                            break;
                        }
                    }
                    hit
                }
                // Forme booléenne : la condition (unique) est vraie ?
                None => {
                    // Le parser garantit exactement une expression ici.
                    let cond = &when.values[0];
                    self.eval_checked(cond)?.truthy()
                }
            };
            if matched {
                return self.exec_stmt(&when.body);
            }
        }
        match otherwise {
            Some(body) => self.exec_stmt(body),
            None => Err(SasError::runtime(
                "The WHEN list does not match any clause and there is no OTHERWISE clause.",
            )),
        }
    }

    fn eval_checked(&mut self, expr: &crate::ast::Expr) -> Result<Value> {
        // Méthode d'objet hash en expression (M17.2) : `rc = h.find();`.
        // Interceptée ICI (et non dans l'évaluateur immuable) car la méthode
        // mute le PDV et les objets hash. Renvoie le code retour numérique.
        if let crate::ast::Expr::HashMethod(call) = expr {
            let rc = self.exec_hash_method_rc(&call.object, &call.method, &call.args)?;
            return Ok(Value::Num(rc as f64));
        }
        let v = eval(expr, &self.pdv, &mut self.ctx);
        if let Some(msg) = self.ctx.fatal.take() {
            // Les fatals de l'évaluateur portent déjà le préfixe "ERROR: " ;
            // on l'enlève pour que `log.error` (qui le rajoute) ne le double
            // pas.
            let msg = msg.strip_prefix("ERROR: ").unwrap_or(&msg).to_string();
            return Err(SasError::runtime(msg));
        }
        if self.ctx.error_flag {
            self.pdv.error_ = true;
            self.ctx.error_flag = false;
        }
        Ok(v)
    }

    /// Coercition à l'assignation : expression d'un type vers une variable
    /// de l'autre type (mêmes règles que dans les expressions).
    fn coerce_assign(&mut self, value: Value, target: VarType) -> Value {
        match (value, target) {
            (v @ (Value::Num(_) | Value::Missing(_)), VarType::Num) => v,
            (v @ Value::Char(_), VarType::Char) => v,
            (Value::Char(s), VarType::Num) => {
                self.ctx.note_char_to_num = true;
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    self.ctx.missing_generated += 1;
                    Value::missing()
                } else {
                    match trimmed.parse::<f64>() {
                        Ok(f) => Value::Num(f),
                        Err(_) => {
                            self.ctx.invalid_data += 1;
                            self.pdv.error_ = true;
                            Value::missing()
                        }
                    }
                }
            }
            (Value::Num(f), VarType::Char) => {
                self.ctx.note_num_to_char = true;
                Value::Char(format!("{:>12}", format_best(f, 12)))
            }
            (Value::Missing(k), VarType::Char) => {
                self.ctx.note_num_to_char = true;
                Value::Char(format!("{:>12}", k.display()))
            }
        }
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

/// Convertit une `Value` en la chaîne stockée par CALL SYMPUT (M11.5).
///
/// - Char : la valeur telle quelle (SAS ne rogne PAS la valeur d'un
///   symput ; on garde la chaîne du PDV avec ses blancs internes/finaux).
/// - Num : formaté en BEST12. puis CADRÉ À GAUCHE (les blancs de tête de
///   BEST12. sont supprimés). `call symput('x', 42)` donne `&x` = "42".
/// - Missing : le point/lettre du missing (cadrage à gauche d'un BEST12.).
fn symput_string(value: Value) -> String {
    match value {
        Value::Char(s) => s,
        Value::Num(f) => format_best(f, 12),
        Value::Missing(k) => k.display(),
    }
}

