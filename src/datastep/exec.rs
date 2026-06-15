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

    // CALL SYMPUT (M11.5) : drain des écritures différées vers la table
    // macro APRÈS le RUN de l'étape (règle de visibilité SAS — le symbole
    // n'est pas visible dans la même étape). Sous le build par défaut,
    // `set_symbol_global` est un no-op (l'engine identité n'a pas de table) :
    // `call symput` parse et s'exécute mais n'a aucun effet macro.
    for (name, value) in std::mem::take(&mut r.ctx.symput_writes) {
        session.macro_engine.set_symbol_global(&name, value);
    }

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
    for ((spec, bset), n_out) in r.outputs.iter().zip(r.builders).zip(&r.out_rows) {
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
            // Le libellé suit la variable (par son nom de PDV, pas le
            // nom renommé en sortie).
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
        session.libs.get(&spec.libref)?.write(&spec.table, &ds)?;
        session.last_dataset = Some(spec.display.clone());
        session.log.note(&format!(
            "The data set {} has {} observations and {} variables.",
            spec.display,
            n_out,
            spec.kept_slots.len()
        ));
        stats
            .written
            .push((spec.display.clone(), *n_out, spec.kept_slots.len()));
    }

    Ok(stats)
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
        format_catalog,
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
/// CALL SYMPUT / CALL EXECUTE / PUT (partagé entre les boucles UPDATE/MODIFY).
fn drain_runner_side_effects(r: &mut Runner, session: &mut Session) -> Result<()> {
    for (name, value) in std::mem::take(&mut r.ctx.symput_writes) {
        session.macro_engine.set_symbol_global(&name, value);
    }
    session
        .call_execute_queue
        .extend(std::mem::take(&mut r.call_execute_queue));
    r.put_flush_at_step_end();
    r.put_replay(session)?;
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

/// Écrit les sorties DATA additionnelles (ordre du statement DATA) à partir des
/// builders du Runner. Partagé par les boucles UPDATE/MODIFY.
fn write_runner_outputs(r: &mut Runner, session: &mut Session, stats: &mut StepStats) -> Result<()> {
    let outputs = std::mem::take(&mut r.outputs);
    let builders = std::mem::take(&mut r.builders);
    for ((spec, bset), n_out) in outputs.iter().zip(builders).zip(&r.out_rows) {
        let mut columns: Vec<Column> = Vec::with_capacity(spec.kept_slots.len());
        let mut vars: Vec<VarMeta> = Vec::with_capacity(spec.kept_slots.len());
        for ((slot, b), out_name) in spec.kept_slots.iter().zip(bset).zip(&spec.out_names) {
            let v = &r.pdv.vars()[*slot];
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
        session.libs.get(&spec.libref)?.write(&spec.table, &ds)?;
        session.last_dataset = Some(spec.display.clone());
        session.log.note(&format!(
            "The data set {} has {} observations and {} variables.",
            spec.display,
            n_out,
            spec.kept_slots.len()
        ));
        stats
            .written
            .push((spec.display.clone(), *n_out, spec.kept_slots.len()));
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
    let mut r = build_um_runner(pdv, outputs, arrays, labels, &[], macro_symbols, format_catalog);

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

    let mut columns: Vec<Column> = Vec::with_capacity(m.out_vars.len());
    for (pos, meta) in m.out_vars.iter().enumerate() {
        let series = match meta.ty {
            VarType::Num => {
                let vals: Vec<Option<f64>> = buffer[pos].iter().map(value_to_num).collect();
                Series::new(meta.name.as_str().into(), vals)
            }
            VarType::Char => {
                let vals: Vec<String> = buffer[pos]
                    .iter()
                    .map(|v| match v {
                        Value::Char(s) => s.clone(),
                        _ => String::new(),
                    })
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
        };
        columns.push(series.into());
    }
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
            // Appel de méthode d'objet hash (M17.1).
            DsStmt::HashMethod {
                object,
                method,
                args,
            } => self.exec_hash_method(object, method, args),
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

    /// INPUT (M14) : lit un enregistrement de la source texte et applique la
    /// spécification INPUT au PDV. Gère les modes liste/colonne/formaté, les
    /// pointeurs `@n`/`+n`/`/`, et les holds `@`/`@@`.
    ///
    /// Sémantique de fin de source (comme SET) : si aucun enregistrement
    /// n'est disponible quand on doit en lire un nouveau → EndStep.
    /// Résout les items AST d'un statement INPUT en `InputAction` (slots PDV
    /// + informats parsés). Plusieurs INPUT par étape sont ainsi gérés (chacun
    /// avec ses propres items).
    fn resolve_input_items(
        &self,
        ast_items: &[crate::ast::InputItem],
    ) -> Result<Vec<InputAction>> {
        use crate::ast::InputItem;
        let mut out = Vec::with_capacity(ast_items.len());
        for item in ast_items {
            let action = match item {
                InputItem::Var {
                    name,
                    is_char,
                    cols,
                    informat,
                    list_modifier,
                } => {
                    let slot = self.pdv.slot(name).ok_or_else(|| {
                        SasError::runtime(format!(
                            "Variable {name} is not on the INPUT statement."
                        ))
                    })?;
                    let spec = match informat {
                        Some(tok) => Some(crate::formats::FormatSpec::parse(tok).ok_or_else(
                            || SasError::runtime(format!("The informat {tok} is not valid.")),
                        )?),
                        None => None,
                    };
                    let pdv_is_char = self.pdv.vars()[slot].ty == VarType::Char;
                    InputAction::Var {
                        slot,
                        is_char: pdv_is_char || *is_char,
                        cols: *cols,
                        informat: spec,
                        list_modifier: *list_modifier,
                    }
                }
                InputItem::ColumnPointer(n) => InputAction::ColumnPointer(*n),
                InputItem::SkipColumns(n) => InputAction::SkipColumns(*n),
                InputItem::NextLine => InputAction::NextLine,
                InputItem::HoldLine => InputAction::HoldLine,
                InputItem::HoldLineDouble => InputAction::HoldLineDouble,
            };
            out.push(action);
        }
        Ok(out)
    }

    fn exec_input(&mut self, ast_items: &[crate::ast::InputItem]) -> Result<Flow> {
        // Récupérer la ligne de travail : soit un hold actif (avec encore des
        // données après le curseur), soit la prochaine ligne de la source. Un
        // hold `@@` dont le reste de ligne n'est que des blancs est épuisé →
        // on lit un nouvel enregistrement (sémantique SAS du « double hold »).
        let held = self.held.take().filter(|h| {
            let rest: String = h.line.chars().skip(h.cursor).collect();
            !rest.trim().is_empty()
        });
        let (mut line, mut cursor) = match held {
            Some(h) => (h.line, h.cursor),
            None => match self.next_record()? {
                Some(s) => (s, 0usize),
                None => return Ok(Flow::EndStep),
            },
        };

        if self.text.is_none() {
            return Err(SasError::runtime("INPUT statement without an INFILE source."));
        }
        // Items résolus en `InputAction` (slots PDV + informats parsés). On les
        // résout depuis l'AST de CE statement INPUT pour gérer plusieurs INPUT
        // par étape (chacun partage la même source mais a ses propres items).
        let items = self.resolve_input_items(ast_items)?;
        let short = self.text.as_ref().unwrap().options.short;
        let dsd = self.text.as_ref().unwrap().options.dsd;
        let delim = self.text.as_ref().unwrap().options.delimiter.clone();

        let mut hold_after = false;
        let mut hold_double = false;

        for action in &items {
            match action {
                InputAction::ColumnPointer(n) => {
                    cursor = n.saturating_sub(1);
                }
                InputAction::SkipColumns(n) => {
                    cursor += n;
                }
                InputAction::NextLine => {
                    // Passe à la ligne d'entrée suivante (curseur réinitialisé).
                    match self.next_record()? {
                        Some(s) => {
                            line = s;
                            cursor = 0;
                        }
                        None => return Ok(Flow::EndStep),
                    }
                }
                InputAction::HoldLine => hold_after = true,
                InputAction::HoldLineDouble => {
                    hold_after = true;
                    hold_double = true;
                }
                InputAction::Var {
                    slot,
                    is_char,
                    cols,
                    informat,
                    list_modifier,
                } => {
                    let outcome = self.read_one_var(
                        &line, &mut cursor, *slot, *is_char, *cols, informat, *list_modifier,
                        &delim, dsd, short,
                    )?;
                    match outcome {
                        ReadOutcome::Ok => {}
                        ReadOutcome::ShortMissover => {
                            // MISSOVER/TRUNCOVER/défaut liste : variables
                            // restantes laissées telles quelles (déjà missing
                            // par le reset). On arrête la lecture des items.
                            break;
                        }
                        ReadOutcome::Stopover => {
                            return Err(SasError::runtime(
                                "INPUT statement exceeded record length (STOPOVER).",
                            ));
                        }
                    }
                }
            }
        }

        // Hold : conserver la ligne pour le prochain INPUT.
        if hold_after {
            self.held = Some(HeldLine {
                line,
                cursor,
                double: hold_double,
            });
        }
        Ok(Flow::Normal)
    }

    /// Lit le prochain enregistrement brut de la source texte, en respectant
    /// FIRSTOBS=/OBS=. Incrémente `text_read`. Renvoie `None` à l'épuisement.
    fn next_record(&mut self) -> Result<Option<String>> {
        let text = match &self.text {
            Some(t) => t,
            None => return Ok(None),
        };
        let firstobs = text.options.firstobs;
        let obs = text.options.obs;
        loop {
            // FIRSTOBS= : sauter les lignes avant firstobs (1-based).
            if self.text_line + 1 < firstobs {
                self.text_line += 1;
                continue;
            }
            // OBS= : borne supérieure (1-based, inclusive).
            if let Some(o) = obs {
                if self.text_line + 1 > o {
                    return Ok(None);
                }
            }
            let Some(line) = text.lines.get(self.text_line) else {
                return Ok(None);
            };
            let line = line.clone();
            self.text_line += 1;
            self.text_read += 1;
            return Ok(Some(line));
        }
    }

    /// Lit UNE variable d'INPUT à partir de `line`, en avançant `cursor`.
    /// Couvre les trois modes (colonne / formaté / liste) et applique la
    /// coercition vers le slot PDV. Renvoie le devenir de la lecture (OK /
    /// ligne trop courte selon MISSOVER/TRUNCOVER/STOPOVER).
    #[allow(clippy::too_many_arguments)]
    fn read_one_var(
        &mut self,
        line: &str,
        cursor: &mut usize,
        slot: usize,
        is_char: bool,
        cols: Option<(usize, usize)>,
        informat: &Option<crate::formats::FormatSpec>,
        list_modifier: bool,
        delim: &Option<String>,
        dsd: bool,
        short: ShortMode,
    ) -> Result<ReadOutcome> {
        let chars: Vec<char> = line.chars().collect();

        // ── Mode COLONNE : champ fixe `a-b` (1-based inclusif). ──────────────
        if let Some((a, b)) = cols {
            let start = a - 1;
            let end = b; // exclusif sur la borne 1-based supérieure
            if start >= chars.len() {
                // Champ entièrement au-delà de la ligne.
                return Ok(self.handle_short(short, slot, is_char));
            }
            let stop = end.min(chars.len());
            let field: String = chars[start..stop].iter().collect();
            *cursor = end;
            self.apply_field(slot, &field, is_char, informat);
            return Ok(ReadOutcome::Ok);
        }

        // ── Modes LISTE et FORMATÉ-COLONNE ───────────────────────────────────
        // Un informat SANS `:`, en mode espace par défaut (ni DSD ni
        // délimiteur explicite), lit une largeur FIXE à partir du curseur
        // (mode formaté colonne). Avec `:`, DSD, ou un délimiteur, il lit un
        // jeton délimité puis applique l'informat (mode liste). En mode liste
        // pur (sans informat), on lit un jeton délimité.
        let delimited_mode = dsd || delim.is_some() || list_modifier;
        let formatted_fixed = informat.is_some() && !delimited_mode;
        if formatted_fixed {
            let w = informat.as_ref().and_then(|s| s.w).map(|w| w as usize);
            // Sans largeur explicite : se comporter comme un jeton délimité.
            if let Some(w) = w {
                if *cursor >= chars.len() {
                    return Ok(self.handle_short(short, slot, is_char));
                }
                let stop = (*cursor + w).min(chars.len());
                // TRUNCOVER/MISSOVER : un champ partiel est lu tel quel.
                let field: String = chars[*cursor..stop].iter().collect();
                *cursor = *cursor + w;
                self.apply_field(slot, &field, is_char, informat);
                return Ok(ReadOutcome::Ok);
            }
        }

        // ── Mode LISTE : jeton délimité ──────────────────────────────────────
        match self.scan_token(&chars, cursor, delim, dsd) {
            Some(field) => {
                self.apply_field(slot, &field, is_char, informat);
                Ok(ReadOutcome::Ok)
            }
            None => Ok(self.handle_short(short, slot, is_char)),
        }
    }

    /// Comportement « ligne trop courte » selon MISSOVER/TRUNCOVER/STOPOVER.
    /// En mode défaut/MISSOVER/TRUNCOVER, la variable reste à sa valeur de
    /// reset (missing num / chaîne vide) et on signale d'arrêter les items
    /// restants. STOPOVER → erreur.
    fn handle_short(&mut self, short: ShortMode, slot: usize, is_char: bool) -> ReadOutcome {
        if short == ShortMode::Stopover {
            return ReadOutcome::Stopover;
        }
        // La variable manquante reste à missing/blanc (le reset l'a déjà
        // posée ; on force par sûreté).
        let init = if is_char {
            Value::Char(String::new())
        } else {
            Value::missing()
        };
        self.pdv.set(slot, init);
        ReadOutcome::ShortMissover
    }

    /// Découpe le prochain jeton délimité à partir de `cursor`. En mode
    /// DSD : la virgule est le délimiteur par défaut, deux délimiteurs
    /// consécutifs encadrent une valeur manquante (chaîne vide), et les
    /// guillemets protègent les délimiteurs. Renvoie `None` si la fin de
    /// ligne est atteinte avant tout jeton (hors DSD-vide).
    fn scan_token(
        &self,
        chars: &[char],
        cursor: &mut usize,
        delim: &Option<String>,
        dsd: bool,
    ) -> Option<String> {
        // Jeu de délimiteurs.
        let delims: Vec<char> = match delim {
            Some(s) => s.chars().collect(),
            None if dsd => vec![','],
            None => vec![' ', '\t'],
        };
        let is_delim = |c: char| delims.contains(&c);

        if dsd {
            // En DSD, on lit exactement UN champ : il peut être vide (deux
            // délimiteurs consécutifs) ou entre guillemets.
            if *cursor > chars.len() {
                return None;
            }
            if *cursor == chars.len() {
                // Curseur en bout de ligne : plus de champ.
                return None;
            }
            let mut field = String::new();
            // Champ entre guillemets.
            if chars[*cursor] == '"' {
                *cursor += 1;
                while *cursor < chars.len() {
                    let c = chars[*cursor];
                    if c == '"' {
                        // Guillemet doublé = guillemet littéral.
                        if *cursor + 1 < chars.len() && chars[*cursor + 1] == '"' {
                            field.push('"');
                            *cursor += 2;
                            continue;
                        }
                        *cursor += 1;
                        break;
                    }
                    field.push(c);
                    *cursor += 1;
                }
                // Consommer le délimiteur de fin de champ s'il y en a un.
                if *cursor < chars.len() && is_delim(chars[*cursor]) {
                    *cursor += 1;
                }
                return Some(field);
            }
            // Champ nu : jusqu'au prochain délimiteur.
            while *cursor < chars.len() && !is_delim(chars[*cursor]) {
                field.push(chars[*cursor]);
                *cursor += 1;
            }
            // Consommer le délimiteur (sépare du champ suivant).
            if *cursor < chars.len() && is_delim(chars[*cursor]) {
                *cursor += 1;
            }
            return Some(field);
        }

        // Mode liste ordinaire : sauter les délimiteurs de tête, puis lire
        // jusqu'au prochain délimiteur.
        while *cursor < chars.len() && is_delim(chars[*cursor]) {
            *cursor += 1;
        }
        if *cursor >= chars.len() {
            return None;
        }
        let mut field = String::new();
        while *cursor < chars.len() && !is_delim(chars[*cursor]) {
            field.push(chars[*cursor]);
            *cursor += 1;
        }
        Some(field)
    }

    /// Applique un champ texte à un slot PDV : informat si présent, sinon
    /// décodage natif (char → tel quel ; num → parse/missing). La troncature
    /// char est gérée par `pdv.set`.
    fn apply_field(
        &mut self,
        slot: usize,
        field: &str,
        is_char: bool,
        informat: &Option<crate::formats::FormatSpec>,
    ) {
        let value = if let Some(spec) = informat {
            // Informat : on délègue au catalogue (gère le piège des décimales
            // implicites). Le champ est passé tel quel.
            self.format_informat(field, spec)
        } else if is_char {
            // Mode liste/colonne caractère : la valeur est le champ (les
            // blancs de bord sont rognés en mode liste ; en colonne, SAS rogne
            // aussi les blancs de tête/fin).
            Value::Char(field.trim().to_string())
        } else {
            // Numérique : trim + parse ; vide/"." → missing.
            let t = field.trim();
            if t.is_empty() || t == "." {
                Value::missing()
            } else {
                match t.parse::<f64>() {
                    Ok(f) => Value::Num(f),
                    Err(_) => {
                        // Donnée numérique invalide : missing + NOTE + _ERROR_.
                        self.ctx.invalid_data += 1;
                        self.pdv.error_ = true;
                        Value::missing()
                    }
                }
            }
        };
        let target = self.pdv.vars()[slot].ty;
        let coerced = self.coerce_assign(value, target);
        self.pdv.set(slot, coerced);
    }

    /// Applique un informat à un champ via le catalogue (clone de session).
    fn format_informat(&self, field: &str, spec: &crate::formats::FormatSpec) -> Value {
        self.format_catalog.informat(field, spec)
    }

    // ── FILE / PUT (M14.2) ───────────────────────────────────────────────

    /// FILE (M14.2) : change la destination courante des PUT. Si une ligne
    /// non maintenue est en construction et que la destination CHANGE, elle
    /// est d'abord relâchée vers l'ancienne destination (la ligne « en cours »
    /// appartient à la destination active au moment de son écriture).
    fn exec_file(&mut self, dest: &crate::ast::PutDest) -> Result<Flow> {
        let new_dest = match dest {
            crate::ast::PutDest::Path(p) => PutDestKind::Path(p.clone()),
            crate::ast::PutDest::Log => PutDestKind::Log,
            crate::ast::PutDest::Print => PutDestKind::Print,
        };
        if new_dest != self.put.dest {
            // Relâcher la ligne pendante (non maintenue) vers l'ancienne
            // destination avant de basculer.
            if self.put.started && !self.put.hold && !self.put.hold_double {
                self.put_release_line();
            }
            self.put.dest = new_dest;
        }
        Ok(Flow::Normal)
    }

    /// PUT (M14.2) : rend chaque item dans la ligne de sortie courante puis,
    /// sauf hold `@`/`@@` final, relâche la ligne vers la destination.
    fn exec_put(&mut self, items: &[crate::ast::PutItem]) -> Result<Flow> {
        use crate::ast::PutItem;
        // Un nouveau PUT efface le hold simple précédent (la ligne maintenue
        // par `@` est reprise telle quelle ; un nouveau PUT sans `@` final la
        // relâchera). Le hold est recalculé pour CE statement.
        self.put.hold = false;
        self.put.hold_double = false;
        self.put.started = true;

        for item in items {
            match item {
                PutItem::ColumnPointer(n) => {
                    self.put.cursor = n.saturating_sub(1);
                }
                PutItem::SkipColumns(n) => {
                    self.put.cursor += n;
                }
                PutItem::NextLine => {
                    // Saut de ligne DANS le même PUT : relâche la ligne
                    // courante et en commence une nouvelle (même destination).
                    self.put_release_line();
                    self.put.started = true;
                }
                PutItem::HoldLine => self.put.hold = true,
                PutItem::HoldLineDouble => {
                    self.put.hold = true;
                    self.put.hold_double = true;
                }
                PutItem::Literal(s) => {
                    self.put_write_at(s);
                    // Un blanc sépare l'item suivant en mode liste.
                    self.put.cursor += 1;
                }
                PutItem::Var { name, format } => {
                    let text = self.render_put_var(name, format.as_deref())?;
                    self.put_write_at(&text);
                    self.put.cursor += 1;
                }
                PutItem::NamedVar(name) => {
                    let val = self.render_put_var(name, None)?;
                    let text = format!("{}={}", name, val);
                    self.put_write_at(&text);
                    self.put.cursor += 1;
                }
                PutItem::All => {
                    // `var=value` pour chaque variable du PDV, séparés d'un
                    // blanc, dans l'ordre du PDV.
                    let n = self.pdv.vars().len();
                    for slot in 0..n {
                        // Les éléments d'array _TEMPORARY_ ne sont pas listés.
                        if self.pdv.vars()[slot].temporary {
                            continue;
                        }
                        let name = self.pdv.vars()[slot].name.clone();
                        let val = self.render_put_slot(slot, None);
                        let text = format!("{}={}", name, val);
                        self.put_write_at(&text);
                        self.put.cursor += 1;
                    }
                }
            }
        }

        // Fin du PUT : sauf hold, relâcher la ligne.
        if !self.put.hold && !self.put.hold_double {
            self.put_release_line();
        }
        Ok(Flow::Normal)
    }

    /// Écrit `text` dans la ligne de sortie courante à partir de la colonne
    /// `cursor` (0-based), en complétant de blancs si le curseur est au-delà
    /// de la longueur courante, et avance le curseur après le texte écrit.
    fn put_write_at(&mut self, text: &str) {
        let mut chars: Vec<char> = self.put.line.chars().collect();
        let start = self.put.cursor;
        // Compléter de blancs jusqu'à `start`.
        while chars.len() < start {
            chars.push(' ');
        }
        // Écrire (écrasement) à partir de `start`.
        for (i, c) in text.chars().enumerate() {
            let pos = start + i;
            if pos < chars.len() {
                chars[pos] = c;
            } else {
                chars.push(c);
            }
        }
        self.put.cursor = start + text.chars().count();
        self.put.line = chars.into_iter().collect();
    }

    /// Relâche (flush + clear) la ligne de sortie courante vers la
    /// destination active, et réinitialise l'état de ligne.
    fn put_release_line(&mut self) {
        let line = std::mem::take(&mut self.put.line);
        // SAS rogne les blancs de fin de la ligne PUT relâchée.
        let line = line.trim_end().to_string();
        let dest = self.put.dest.clone();
        self.put.out.push((dest, line));
        self.put.cursor = 0;
        self.put.started = false;
        self.put.hold = false;
        self.put.hold_double = false;
    }

    /// Flush de fin d'étape : une ligne encore maintenue (`@`/`@@`) ou en
    /// construction est relâchée.
    fn put_flush_at_step_end(&mut self) {
        if self.put.started || !self.put.line.is_empty() {
            self.put_release_line();
        }
    }

    /// Rejoue les lignes PUT produites vers leurs destinations (LOG, listing,
    /// fichiers externes). Les fichiers sont regroupés par chemin et écrits
    /// (création/troncature) en une fois.
    fn put_replay(&mut self, session: &mut Session) -> Result<()> {
        use std::collections::HashMap;
        // Tampon par fichier (ordre des lignes préservé).
        let mut files: HashMap<String, Vec<String>> = HashMap::new();
        let mut file_order: Vec<String> = Vec::new();
        for (dest, line) in std::mem::take(&mut self.put.out) {
            match dest {
                PutDestKind::Log => session.log.put_line(&line),
                PutDestKind::Print => session.listing.write_line(&line),
                PutDestKind::Path(path) => {
                    files
                        .entry(path.clone())
                        .or_insert_with(|| {
                            file_order.push(path.clone());
                            Vec::new()
                        })
                        .push(line);
                }
            }
        }
        for path in file_order {
            let lines = files.remove(&path).unwrap_or_default();
            let mut content = lines.join("\n");
            // Terminer le fichier par un saut de ligne (convention texte).
            if !content.is_empty() {
                content.push('\n');
            }
            // Chemin relatif résolu sous `base_dir` (cohérent avec LIBNAME et
            // INFILE) ; le message d'erreur garde le chemin source.
            let resolved = session.resolve_path(&path);
            std::fs::write(&resolved, content).map_err(|e| {
                SasError::runtime(format!("Unable to write the FILE '{path}': {e}"))
            })?;
        }
        Ok(())
    }

    /// Rend une variable PUT (par nom) en texte, avec son format explicite
    /// (`format`), ou son format d'affichage, ou le défaut BESTw./$w.
    fn render_put_var(&self, name: &str, format: Option<&str>) -> Result<String> {
        let slot = self.pdv.slot(name).ok_or_else(|| {
            SasError::runtime(format!("Variable {name} is not on the PUT statement."))
        })?;
        Ok(self.render_put_slot(slot, format))
    }

    /// Rend la valeur du slot PDV `slot` en texte pour un PUT. Ordre de
    /// résolution du format : format explicite de l'item > format d'affichage
    /// de la variable > défaut (BEST12. justifié à droite pour un numérique,
    /// valeur brute pour un caractère). Le résultat est rogné de ses blancs
    /// de bord (mode liste SAS : les valeurs formatées sont posées « left
    /// aligned » dans la ligne).
    fn render_put_slot(&self, slot: usize, format: Option<&str>) -> String {
        let value = self.pdv.get(slot).clone();
        // Format explicite, sinon format d'affichage de la variable.
        let fmt_tok = format
            .map(str::to_string)
            .or_else(|| self.pdv.vars()[slot].format.clone());
        if let Some(tok) = fmt_tok {
            if let Some(spec) = crate::formats::FormatSpec::parse(&tok) {
                return self.format_catalog.format(&value, &spec).trim().to_string();
            }
        }
        // Défaut : pas de format.
        match value {
            Value::Missing(kind) => kind.display(),
            Value::Num(f) => format_best(f, 12).trim().to_string(),
            Value::Char(s) => s,
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

    /// Appel de méthode d'objet hash (M17.1). Seules
    /// `defineKey`/`defineData`/`defineDone` sont exécutées ; les autres
    /// (find/add/replace/remove/output/...) parsent mais produisent une erreur
    /// runtime « not yet implemented » (M17.2).
    fn exec_hash_method(
        &mut self,
        object: &str,
        method: &str,
        args: &[crate::ast::Expr],
    ) -> Result<Flow> {
        let upper = object.to_uppercase();
        if !self.ctx.hashes.contains_key(&upper) {
            return Err(SasError::runtime(format!(
                "Hash object {upper} has not been declared."
            )));
        }
        let m = method.to_ascii_lowercase();
        match m.as_str() {
            "definekey" | "definedata" => {
                // Les arguments sont des littéraux chaîne nommant des variables
                // du PDV (validés à la compilation). On collecte les noms
                // UPPERCASE dans l'ordre.
                let mut names = Vec::with_capacity(args.len());
                for a in args {
                    let crate::ast::Expr::Str(varname) = a else {
                        return Err(SasError::runtime(format!(
                            "Argument of {upper}.{method} must be a quoted variable name."
                        )));
                    };
                    names.push(varname.to_uppercase());
                }
                let obj = self.ctx.hashes.get_mut(&upper).expect("checked above");
                if m == "definekey" {
                    obj.keys = names;
                } else {
                    obj.data_vars = names;
                }
                Ok(Flow::Normal)
            }
            "definedone" => {
                // Finalisation, idempotente. Le chargement éventuel d'un
                // `dataset:` est différé à M17.2 (le nom est conservé).
                let obj = self.ctx.hashes.get_mut(&upper).expect("checked above");
                obj.defined = true;
                Ok(Flow::Normal)
            }
            // find/check/add/replace/remove/clear/output/num_items/... : M17.2.
            other => Err(SasError::runtime(format!(
                "Hash method {}.{} is not yet implemented.",
                upper,
                other.to_uppercase()
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

    /// SET sans BY = CONCATÉNATION : le premier dataset en entier, puis le
    /// suivant. Boucle de skip interne : charge des lignes jusqu'à en
    /// trouver une qui passe le WHERE= (faux/missing → ligne suivante SANS
    /// exécuter le reste de l'itération). Les lignes rejetées ne comptent
    /// pas dans `rows_read`. Tous les datasets épuisés → l'étape se
    /// termine IMMÉDIATEMENT (EndStep). Au passage d'un dataset au
    /// suivant, les variables absentes du nouveau dataset GARDENT leur
    /// valeur (RETAIN implicite des variables de SET — règle SAS, pas de
    /// remise à missing).
    fn exec_set_concat(&mut self) -> Result<Flow> {
        loop {
            let Some(input) = &self.input else {
                return Err(SasError::runtime("SET statement without input data."));
            };
            let Some(ds) = input.datasets.get(self.cur_ds) else {
                // Fin de TOUS les inputs : fin d'étape immédiate.
                return Ok(Flow::EndStep);
            };
            let row = self.cursors[self.cur_ds];
            if row >= ds.n_rows {
                self.cur_ds += 1;
                continue;
            }
            for (col, slot) in ds.columns.iter().zip(&ds.var_slots) {
                self.pdv.set(*slot, col[row].clone());
            }
            self.cursors[self.cur_ds] += 1;
            let Some(w) = &ds.where_ else {
                self.rows_read[self.cur_ds] += 1;
                self.set_end_flag();
                return Ok(Flow::Normal);
            };
            // Évaluation inline (emprunts disjoints : `input` tient
            // self.input, eval n'utilise que pdv et ctx).
            let v = eval(w, &self.pdv, &mut self.ctx);
            if let Some(msg) = self.ctx.fatal.take() {
                let msg = msg.strip_prefix("ERROR: ").unwrap_or(&msg).to_string();
                return Err(SasError::runtime(msg));
            }
            if self.ctx.error_flag {
                self.pdv.error_ = true;
                self.ctx.error_flag = false;
            }
            if v.truthy() {
                self.rows_read[self.cur_ds] += 1;
                self.set_end_flag();
                return Ok(Flow::Normal);
            }
        }
    }

    /// Met à jour la variable END= (M16.4) après une lecture réussie en mode
    /// concaténation : 1 si AUCUNE observation ne reste à lire (en tenant
    /// compte du WHERE= de chaque dataset), 0 sinon. Sans END= déclaré,
    /// no-op. La détection se fait par un balayage en avant NON destructif
    /// (les curseurs ne sont pas modifiés).
    fn set_end_flag(&mut self) {
        if self.ctx.end_flag.is_none() {
            return;
        }
        let has_more = self.concat_has_more();
        if let Some((_, v)) = &mut self.ctx.end_flag {
            *v = if has_more { 0.0 } else { 1.0 };
        }
    }

    /// Balaye en avant (sans muter les curseurs) pour savoir s'il reste au
    /// moins une observation lisible APRÈS la position courante, en respectant
    /// le WHERE= de chaque dataset. Sert END= en mode concaténation.
    fn concat_has_more(&mut self) -> bool {
        let Some(input) = self.input.take() else {
            return false;
        };
        // Le balayage évalue éventuellement des WHERE= sur des lignes JAMAIS
        // réellement lues : il ne doit donc émettre AUCUNE NOTE/erreur. On
        // mémorise l'état des compteurs de l'évaluateur et on le restaure à la
        // fin (le vrai chargement, lui, comptabilise normalement).
        let saved_ctx = (
            self.ctx.missing_generated,
            self.ctx.division_by_zero,
            self.ctx.note_num_to_char,
            self.ctx.note_char_to_num,
            self.ctx.invalid_data,
            self.ctx.error_flag,
            self.ctx.fatal.take(),
        );
        let mut found = false;
        'outer: for d in self.cur_ds..input.datasets.len() {
            let ds = &input.datasets[d];
            let start = if d == self.cur_ds { self.cursors[d] } else { 0 };
            for row in start..ds.n_rows {
                match &ds.where_ {
                    None => {
                        found = true;
                        break 'outer;
                    }
                    Some(w) => {
                        // Évalue le WHERE= sur une COPIE des valeurs de la ligne
                        // chargées dans le PDV, puis restaure (le balayage ne
                        // doit pas laisser de trace). On sauvegarde/restaure les
                        // slots touchés.
                        let saved: Vec<(usize, Value)> = ds
                            .var_slots
                            .iter()
                            .map(|&s| (s, self.pdv.get(s).clone()))
                            .collect();
                        for (col, slot) in ds.columns.iter().zip(&ds.var_slots) {
                            self.pdv.set(*slot, col[row].clone());
                        }
                        let v = eval(w, &self.pdv, &mut self.ctx);
                        // Restaure les slots (le balayage ne laisse aucune
                        // trace sur le PDV).
                        for (slot, val) in saved {
                            self.pdv.set(slot, val);
                        }
                        if v.truthy() {
                            found = true;
                            break 'outer;
                        }
                    }
                }
            }
        }
        // Restaure intégralement les compteurs de l'évaluateur.
        (
            self.ctx.missing_generated,
            self.ctx.division_by_zero,
            self.ctx.note_num_to_char,
            self.ctx.note_char_to_num,
            self.ctx.invalid_data,
            self.ctx.error_flag,
            self.ctx.fatal,
        ) = saved_ctx;
        self.input = Some(input);
        found
    }

    /// SET ... POINT= (M16.4) : ACCÈS DIRECT. Lit la valeur de la variable
    /// d'index (slot `point_slot`), l'arrondit à l'entier (sémantique SAS),
    /// et charge l'observation correspondante (1-based). Avec plusieurs
    /// datasets en concaténation, l'index est GLOBAL (1..total, parcourant les
    /// datasets dans l'ordre du SET). Index missing / non entier valide /
    /// hors bornes [1, total] → ERROR "Error in variable p." (l'étape
    /// s'arrête). N'avance AUCUN curseur (l'utilisateur pilote l'itération) et
    /// ne compte pas dans les NOTEs "There were N observations read" au sens
    /// d'un balayage séquentiel — mais on incrémente `rows_read` du dataset
    /// servi pour rester cohérent avec le décompte SAS d'obs lues.
    fn exec_set_point(&mut self) -> Result<Flow> {
        let Some(input) = self.input.take() else {
            return Err(SasError::runtime("SET statement without input data."));
        };
        let point_slot = input.point_slot.expect("exec_set_point requires POINT=");
        let total: usize = input.datasets.iter().map(|d| d.n_rows).sum();
        let point_name = self.pdv.vars()[point_slot].name.clone();

        // Lecture + coercition de l'index. Une valeur missing ou non
        // convertible → erreur SAS sur la variable d'index.
        let idx_val = self.pdv.get(point_slot).clone();
        let idx = match coerce_num(&idx_val, &mut self.ctx) {
            Some(f) => f.round() as i64,
            None => {
                self.input = Some(input);
                self.pdv.error_ = true;
                return Err(SasError::runtime(format!(
                    "Error in variable {point_name}."
                )));
            }
        };
        if idx < 1 || (idx as usize) > total {
            self.input = Some(input);
            self.pdv.error_ = true;
            return Err(SasError::runtime(format!("Error in variable {point_name}.")));
        }

        // Localiser l'observation globale `idx` (1-based) dans la concaténation.
        let mut remaining = idx as usize - 1; // 0-based offset global
        let mut target: Option<(usize, usize)> = None;
        for (d, ds) in input.datasets.iter().enumerate() {
            if remaining < ds.n_rows {
                target = Some((d, remaining));
                break;
            }
            remaining -= ds.n_rows;
        }
        let (d, row) = target.expect("index validated against total");
        let ds = &input.datasets[d];
        for (col, slot) in ds.columns.iter().zip(&ds.var_slots) {
            self.pdv.set(*slot, col[row].clone());
        }
        self.rows_read[d] += 1;
        // END= avec POINT= : 1 si l'index pointe la DERNIÈRE observation.
        if let Some((_, v)) = &mut self.ctx.end_flag {
            *v = if (idx as usize) == total { 1.0 } else { 0.0 };
        }
        self.input = Some(input);
        Ok(Flow::Normal)
    }

    /// MODIFY+POINT= (M16.5) : au marqueur MODIFY, on CAPTURE la ligne
    /// précédemment chargée (les assignations qui l'ont suivie sont ses
    /// modifications), puis on CHARGE l'obs à l'index POINT= courant (1-based,
    /// arrondi). Index missing / hors bornes → erreur différée (relevée par la
    /// boucle externe). L'état partagé est `self.modify_state`.
    fn exec_modify_point(&mut self) -> Result<Flow> {
        // Capture de la ligne précédente.
        let mut state = self.modify_state.take().expect("modify_state present");
        capture_modify_state(&mut state, &self.pdv);
        // Index POINT= courant.
        let idx_val = self.pdv.get(state.point_slot).clone();
        let idx = match coerce_num(&idx_val, &mut self.ctx) {
            Some(f) => f.round() as i64,
            None => {
                state.error = Some(format!("Invalid POINT= value for the data set {}.", state.display));
                self.modify_state = Some(state);
                self.pdv.error_ = true;
                return Ok(Flow::EndStep);
            }
        };
        if idx < 1 || (idx as usize) > state.n_rows {
            state.error = Some(format!("Invalid POINT= value for the data set {}.", state.display));
            self.modify_state = Some(state);
            self.pdv.error_ = true;
            return Ok(Flow::EndStep);
        }
        let row = idx as usize - 1;
        // Charger la ligne `row` depuis le tampon (qui peut déjà porter des
        // modifications d'un tour précédent — fidèle à la réécriture en place).
        for (pos, &slot) in state.var_slots.iter().enumerate() {
            self.pdv.set(slot, state.cols[pos][row].clone());
        }
        state.touched[row] = true;
        state.cur_row = Some(row);
        self.modify_state = Some(state);
        Ok(Flow::Normal)
    }

    /// SET avec BY = INTERCLASSEMENT : parmi les datasets non épuisés,
    /// servir celui dont la prochaine observation (après pré-filtrage
    /// WHERE=) porte la PLUS PETITE clé BY (`sas_cmp`, DESCENDING par clé
    /// respecté) ; égalité → ordre du statement SET. Met à jour les flags
    /// FIRST./LAST. (comparaison des préfixes de clés avec l'observation
    /// précédente / suivante) et détecte le désordre (clé servie < clé
    /// précédente → ERROR, l'étape s'arrête).
    fn exec_set_interleave(&mut self) -> Result<Flow> {
        let Some(input) = &self.input else {
            return Err(SasError::runtime("SET statement without input data."));
        };
        let Some((d, cur_keys)) = choose_next(input, &self.filtered, &self.cursors) else {
            // Tous les datasets épuisés : fin d'étape immédiate.
            return Ok(Flow::EndStep);
        };
        let ds = &input.datasets[d];
        // Détection de désordre : l'interclassement choisit toujours la
        // plus petite clé disponible ; si elle régresse, c'est qu'un input
        // n'est pas trié selon le BY.
        if let Some(prev) = &self.prev_keys {
            if compare_keys(&cur_keys, prev, &input.by) == Ordering::Less {
                return Err(SasError::runtime(format!(
                    "BY variables are not properly sorted on data set {}.",
                    ds.display
                )));
            }
        }
        let row = self.filtered[d][self.cursors[d]];
        // Les variables absentes du dataset servi GARDENT leur valeur
        // précédente (RETAIN implicite des variables de SET — règle SAS,
        // pas de remise à missing).
        for (col, slot) in ds.columns.iter().zip(&ds.var_slots) {
            self.pdv.set(*slot, col[row].clone());
        }
        self.cursors[d] += 1;
        self.rows_read[d] += 1;
        // FIRST.var_i : première obs, ou clé j ≤ i différente de l'obs
        // précédente. LAST.var_i : dernière obs, ou clé j ≤ i différente
        // de l'obs SUIVANTE (la tête du prochain choix d'interclassement).
        let next_keys = choose_next(input, &self.filtered, &self.cursors).map(|(_, k)| k);
        for (i, flags) in self.ctx.by_flags.iter_mut().enumerate() {
            flags.1 = match &self.prev_keys {
                None => true,
                Some(prev) => prefix_changed(&cur_keys, prev, i),
            };
            flags.2 = match &next_keys {
                None => true,
                Some(next) => prefix_changed(&cur_keys, next, i),
            };
        }
        self.prev_keys = Some(cur_keys);
        // END= (M16.4) : 1 si plus aucune observation à interclasser.
        if let Some((_, v)) = &mut self.ctx.end_flag {
            *v = if next_keys.is_none() { 1.0 } else { 0.0 };
        }
        Ok(Flow::Normal)
    }

    /// MERGE (M3) : sert la prochaine observation pré-calculée du plan. À
    /// l'épuisement du plan → EndStep (fin d'étape immédiate, comme SET).
    /// Applique les MISSING des datasets absents (1re obs du groupe), les
    /// chargements (gauche→droite, le dernier contributeur écrase les
    /// variables partagées), puis met à jour les flags FIRST./LAST. et IN=.
    fn exec_merge(&mut self) -> Result<Flow> {
        let Some(input) = &self.input else {
            return Err(SasError::runtime("MERGE statement without input data."));
        };
        let Some(obs) = self.merge_plan.get(self.merge_cursor) else {
            return Ok(Flow::EndStep);
        };
        // Emprunts disjoints : on copie les petites données nécessaires.
        let blank_slots = obs.blank_slots.clone();
        let loads = obs.loads.clone();
        let in_active = obs.in_active.clone();
        let first = obs.first.clone();
        let last = obs.last.clone();

        // (1) Variables PROPRES des datasets absents → MISSING (persistées
        // ensuite tout le groupe, car from_input).
        for &slot in &blank_slots {
            let init = match self.pdv.vars()[slot].ty {
                VarType::Num => Value::missing(),
                VarType::Char => Value::Char(String::new()),
            };
            self.pdv.set(slot, init);
        }
        // (2) Chargements gauche→droite (les datasets non chargés PERSISTENT
        // leurs valeurs — c'est la « persistance du côté court »).
        for &(d, row) in &loads {
            let ds = &input.datasets[d];
            for (col, slot) in ds.columns.iter().zip(&ds.var_slots) {
                self.pdv.set(*slot, col[row].clone());
            }
        }
        // (3) Flags FIRST./LAST. (sur la clé de groupe).
        for (i, flags) in self.ctx.by_flags.iter_mut().enumerate() {
            flags.1 = first[i];
            flags.2 = last[i];
        }
        // (4) Flags IN= : 1 pour les datasets ayant participé au groupe.
        let input = self.input.as_ref().unwrap();
        for (name, ds_idx) in &input.in_flags {
            if let Some((_, flag)) = self.ctx.in_flags.iter_mut().find(|(n, _)| n == name) {
                *flag = in_active[*ds_idx];
            }
        }
        self.merge_cursor += 1;
        Ok(Flow::Normal)
    }

    /// Pré-calcule la séquence des observations de sortie d'un MERGE, groupe
    /// par groupe (cf. en-tête de fichier). Pour chaque clé de l'UNION triée
    /// des clés présentes dans au moins un dataset, le groupe produit
    /// `max_i(n_i)` observations. Détecte le désordre (clés non triées dans
    /// un dataset) → ERROR.
    fn build_merge_plan(&mut self) -> Result<Vec<MergeObs>> {
        let input = self.input.as_ref().unwrap();
        let n_ds = input.datasets.len();
        let n_by = input.by.len();

        // Groupes consécutifs par dataset : Vec<(clé, début, longueur)> sur
        // les lignes RETENUES (`filtered`). Détection de désordre intra-ds.
        let mut ds_groups: Vec<Vec<(Vec<Value>, usize, usize)>> = Vec::with_capacity(n_ds);
        for (d, ds) in input.datasets.iter().enumerate() {
            let rows = &self.filtered[d];
            let mut groups: Vec<(Vec<Value>, usize, usize)> = Vec::new();
            let mut prev_key: Option<Vec<Value>> = None;
            for (pos, &row) in rows.iter().enumerate() {
                let key = keys_at(ds, row);
                match groups.last_mut() {
                    Some((k, _, len)) if compare_keys(&key, k, &input.by) == Ordering::Equal => {
                        *len += 1;
                    }
                    _ => {
                        // Nouvelle clé : doit être STRICTEMENT supérieure à la
                        // précédente (sinon dataset non trié).
                        if let Some(prev) = &prev_key {
                            if compare_keys(&key, prev, &input.by) == Ordering::Less {
                                return Err(SasError::runtime(format!(
                                    "BY variables are not properly sorted on data set {}.",
                                    ds.display
                                )));
                            }
                        }
                        prev_key = Some(key.clone());
                        groups.push((key, pos, 1));
                    }
                }
            }
            ds_groups.push(groups);
        }

        // Curseurs de groupe par dataset.
        let mut g_cursors = vec![0usize; n_ds];
        let mut plan: Vec<MergeObs> = Vec::new();
        let mut prev_group_key: Option<Vec<Value>> = None;

        loop {
            // Plus petite clé de groupe parmi les datasets non épuisés.
            let mut best: Option<Vec<Value>> = None;
            for d in 0..n_ds {
                if let Some((key, _, _)) = ds_groups[d].get(g_cursors[d]) {
                    let better = match &best {
                        None => true,
                        Some(b) => compare_keys(key, b, &input.by) == Ordering::Less,
                    };
                    if better {
                        best = Some(key.clone());
                    }
                }
            }
            let Some(group_key) = best else { break };

            // Par dataset : participe-t-il à ce groupe ? Si oui, (début,
            // longueur) de ses lignes dans `filtered`.
            let mut participate: Vec<Option<(usize, usize)>> = vec![None; n_ds];
            let mut n = vec![0usize; n_ds];
            for d in 0..n_ds {
                if let Some((key, start, len)) = ds_groups[d].get(g_cursors[d]) {
                    if compare_keys(key, &group_key, &input.by) == Ordering::Equal {
                        participate[d] = Some((*start, *len));
                        n[d] = *len;
                        g_cursors[d] += 1;
                    }
                }
            }
            let in_active: Vec<bool> = n.iter().map(|&c| c > 0).collect();
            let max = n.iter().copied().max().unwrap_or(0);

            // Slots PROPRES des datasets absents (n_i == 0) à blanchir au
            // début du groupe : un slot d'un dataset absent n'est blanchi que
            // s'il n'appartient à AUCUN dataset participant (sinon le
            // participant l'écrit).
            let mut blank_slots: Vec<usize> = Vec::new();
            for d in 0..n_ds {
                if n[d] > 0 {
                    continue;
                }
                for &slot in &input.datasets[d].var_slots {
                    let owned_by_participant = (0..n_ds).any(|p| {
                        n[p] > 0 && input.datasets[p].var_slots.contains(&slot)
                    });
                    if !owned_by_participant && !blank_slots.contains(&slot) {
                        blank_slots.push(slot);
                    }
                }
            }

            // FIRST./LAST. du groupe vs groupes voisins (préfixe de clés).
            let first_flags: Vec<bool> = (0..n_by)
                .map(|i| match &prev_group_key {
                    None => true,
                    Some(prev) => prefix_changed(&group_key, prev, i),
                })
                .collect();

            // La clé du groupe SUIVANT (pour LAST.) : plus petite clé restante
            // après consommation de ce groupe.
            let mut next_group_key: Option<Vec<Value>> = None;
            for d in 0..n_ds {
                if let Some((key, _, _)) = ds_groups[d].get(g_cursors[d]) {
                    let better = match &next_group_key {
                        None => true,
                        Some(b) => compare_keys(key, b, &input.by) == Ordering::Less,
                    };
                    if better {
                        next_group_key = Some(key.clone());
                    }
                }
            }
            let last_flags: Vec<bool> = (0..n_by)
                .map(|i| match &next_group_key {
                    None => true,
                    Some(next) => prefix_changed(&group_key, next, i),
                })
                .collect();

            // `max` observations de sortie pour ce groupe. FIRST.x n'est vrai
            // qu'à la PREMIÈRE obs du groupe (j==0), LAST.x qu'à la DERNIÈRE
            // (j==max-1) — combiné au changement de préfixe vs groupe voisin.
            for j in 0..max {
                let mut loads: Vec<(usize, usize)> = Vec::new();
                for d in 0..n_ds {
                    if let Some((start, len)) = participate[d] {
                        if j < len {
                            // j-ème ligne du groupe dans `filtered`.
                            let row = self.filtered[d][start + j];
                            loads.push((d, row));
                        }
                        // j >= len : PERSISTANCE (pas de chargement).
                    }
                }
                let first: Vec<bool> = first_flags
                    .iter()
                    .map(|&f| f && j == 0)
                    .collect();
                let last: Vec<bool> = last_flags
                    .iter()
                    .map(|&l| l && j + 1 == max)
                    .collect();
                plan.push(MergeObs {
                    // Blanchiment seulement à la 1re obs du groupe.
                    blank_slots: if j == 0 { blank_slots.clone() } else { Vec::new() },
                    loads,
                    in_active: in_active.clone(),
                    first,
                    last,
                });
            }
            // Compte des lignes lues (toutes les obs participantes du groupe).
            for d in 0..n_ds {
                self.rows_read[d] += n[d];
            }
            prev_group_key = Some(group_key);
        }
        Ok(plan)
    }

    /// Pré-applique les WHERE= des datasets d'un SET avec BY : remplit
    /// `filtered` (indices des lignes retenues) en évaluant chaque ligne
    /// sur le PDV, puis remet les slots d'input à leur état initial
    /// (missing / chaîne vide). Un `_ERROR_` levé pendant ce pré-filtrage
    /// n'est pas reporté aux itérations (divergence mineure documentée) ;
    /// les compteurs de NOTEs (conversions, invalid data) sont conservés.
    fn prefilter(&mut self) -> Result<()> {
        let Some(input) = &self.input else {
            return Ok(());
        };
        for (d, ds) in input.datasets.iter().enumerate() {
            let Some(w) = &ds.where_ else {
                self.filtered[d] = (0..ds.n_rows).collect();
                continue;
            };
            let mut keep = Vec::new();
            for row in 0..ds.n_rows {
                for (col, slot) in ds.columns.iter().zip(&ds.var_slots) {
                    self.pdv.set(*slot, col[row].clone());
                }
                let v = eval(w, &self.pdv, &mut self.ctx);
                if let Some(msg) = self.ctx.fatal.take() {
                    let msg = msg.strip_prefix("ERROR: ").unwrap_or(&msg).to_string();
                    return Err(SasError::runtime(msg));
                }
                self.ctx.error_flag = false;
                if v.truthy() {
                    keep.push(row);
                }
            }
            self.filtered[d] = keep;
        }
        // Restaurer l'état initial des slots d'input touchés par le
        // pré-filtrage.
        for ds in &input.datasets {
            if ds.where_.is_none() {
                continue;
            }
            for &slot in &ds.var_slots {
                let init = match self.pdv.vars()[slot].ty {
                    VarType::Num => Value::missing(),
                    VarType::Char => Value::Char(String::new()),
                };
                self.pdv.set(slot, init);
            }
        }
        Ok(())
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

/// Clés BY de la ligne `row` d'un dataset (dans l'ordre du BY).
fn keys_at(ds: &InputDataset, row: usize) -> Vec<Value> {
    ds.by_cols.iter().map(|&c| ds.columns[c][row].clone()).collect()
}

/// Comparaison de deux jeux de clés BY : `sas_cmp` clé par clé (les
/// missings SONT ordonnés : `._ < . < .a < nombres`), inversée pour les
/// clés DESCENDING.
fn compare_keys(a: &[Value], b: &[Value], by: &[ByVar]) -> Ordering {
    for (i, bv) in by.iter().enumerate() {
        let mut ord = a[i].sas_cmp(&b[i]);
        if bv.descending {
            ord = ord.reverse();
        }
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

/// Le préfixe de clés `0..=i` diffère-t-il entre deux observations ?
/// (Égalité pure : DESCENDING est sans effet ici.)
fn prefix_changed(a: &[Value], b: &[Value], i: usize) -> bool {
    (0..=i).any(|j| a[j].sas_cmp(&b[j]) != Ordering::Equal)
}

/// Choisit la prochaine observation de l'interclassement : parmi les
/// datasets non épuisés (curseur dans `filtered`), celui dont la tête
/// porte la plus petite clé BY ; égalité stricte → le premier dans
/// l'ordre du SET. Renvoie (index du dataset, clés de sa tête).
fn choose_next(
    input: &InputData,
    filtered: &[Vec<usize>],
    cursors: &[usize],
) -> Option<(usize, Vec<Value>)> {
    let mut best: Option<(usize, Vec<Value>)> = None;
    for (d, ds) in input.datasets.iter().enumerate() {
        let Some(&row) = filtered[d].get(cursors[d]) else {
            continue;
        };
        let keys = keys_at(ds, row);
        let better = match &best {
            None => true,
            // Strictement plus petit seulement : à égalité le premier
            // dataset du SET gagne.
            Some((_, bk)) => compare_keys(&keys, bk, &input.by) == Ordering::Less,
        };
        if better {
            best = Some((d, keys));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datastep::compile;
    use crate::parser::StatementStream;
    use crate::source::SourceFile;
    use crate::value::MissingKind;
    use polars::df;
    use std::path::PathBuf;

    fn session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

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

    fn run(src: &str, session: &mut Session) -> Result<StepStats> {
        let file = SourceFile::new(src);
        let mut ts = StatementStream::new(&file).unwrap();
        assert!(ts.next().is_kw("data"));
        let ast = crate::parser::datastep::parse_data_step(&mut ts).unwrap();
        let prog = compile(&ast, session)?;
        execute(prog, session)
    }

    fn read_work(session: &Session, table: &str) -> SasDataset {
        session.libs.get("WORK").unwrap().read(table).unwrap().0
    }

    #[test]
    fn set_assign_implicit_output() {
        let mut s = session();
        write_class(&s, "inp");
        let stats = run("data out; set inp; x = age * 2; run;", &mut s).unwrap();
        assert_eq!(stats.read, vec![("WORK.INP".to_string(), 3)]);
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 3, 3)]);

        let ds = read_work(&s, "out");
        assert_eq!(ds.n_obs(), 3);
        let x = ds.df.column("x").unwrap().f64().unwrap();
        assert_eq!(x.get(0), Some(28.0));
        // age missing → x missing (propagation) + note.
        assert_eq!(x.get(1), None);
        assert_eq!(x.get(2), Some(26.0));

        let log = s.log.into_string();
        assert!(log.contains("There were 3 observations read from the data set WORK.INP."));
        assert!(log.contains("The data set WORK.OUT has 3 observations and 3 variables."));
        assert!(log.contains("Missing values were generated"));
        assert_eq!(s.last_dataset.as_deref(), Some("WORK.OUT"));
    }

    #[test]
    fn subsetting_if_filters() {
        let mut s = session();
        write_class(&s, "inp");
        let stats = run("data out; set inp; if age > 13; run;", &mut s).unwrap();
        // age > 13 : 14 vrai, missing faux (. < 14), 13 faux.
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 2)]);
        let ds = read_work(&s, "out");
        let name = ds.df.column("Name").unwrap().str().unwrap();
        assert_eq!(name.get(0), Some("Alfred"));
    }

    #[test]
    fn explicit_output_disables_implicit() {
        let mut s = session();
        write_class(&s, "inp");
        let stats = run("data out; set inp; output; output; run;", &mut s).unwrap();
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 6, 2)]);
    }

    #[test]
    fn stop_ends_step_without_output() {
        let mut s = session();
        write_class(&s, "inp");
        let stats = run("data out; set inp; output; stop; run;", &mut s).unwrap();
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 2)]);
        // STOP au milieu : une seule ligne lue.
        assert_eq!(stats.read, vec![("WORK.INP".to_string(), 1)]);
    }

    #[test]
    fn no_input_runs_single_iteration() {
        let mut s = session();
        let stats = run("data out; x = 1; y = 'ab'; run;", &mut s).unwrap();
        assert_eq!(stats.read, vec![]);
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 2)]);
        let ds = read_work(&s, "out");
        assert_eq!(ds.n_obs(), 1);
        assert_eq!(
            ds.df.column("y").unwrap().str().unwrap().get(0),
            Some("ab")
        );
    }

    #[test]
    fn data_null_writes_nothing() {
        let mut s = session();
        write_class(&s, "inp");
        let stats = run("data _null_; set inp; run;", &mut s).unwrap();
        assert!(stats.written.is_empty());
        assert!(!s.libs.get("WORK").unwrap().exists("_null_"));
        // _LAST_ inchangé.
        assert_eq!(s.last_dataset, None);
    }

    #[test]
    fn if_then_else_branches() {
        let mut s = session();
        write_class(&s, "inp");
        run(
            "data out; set inp; if age >= 14 then grp = 'old'; else grp = 'yng'; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        let grp = ds.df.column("grp").unwrap().str().unwrap();
        assert_eq!(grp.get(0), Some("old"));
        // age missing : . >= 14 faux → else.
        assert_eq!(grp.get(1), Some("yng"));
        assert_eq!(grp.get(2), Some("yng"));
    }

    #[test]
    fn uninitialized_note_and_missing_column() {
        let mut s = session();
        let stats = run("data out; y = x; run;", &mut s).unwrap();
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 2)]);
        let ds = read_work(&s, "out");
        assert_eq!(ds.df.column("x").unwrap().f64().unwrap().get(0), None);
        let log = s.log.into_string();
        assert!(log.contains("Variable x is uninitialized."));
    }

    #[test]
    fn assign_coercion_num_to_char_best12_right_justified() {
        let mut s = session();
        run("data out; c = 'init'; c = 7; run;", &mut s).unwrap();
        // c figée Char(4) par la 1re assignation ; 7 → BEST12 justifié
        // droite sur 12 ('           7') puis tronqué à 4 ('    ') → trim
        // stockage → "".
        let ds = read_work(&s, "out");
        assert_eq!(ds.df.column("c").unwrap().str().unwrap().get(0), Some(""));
        let log = s.log.into_string();
        assert!(log.contains("Numeric values have been converted to character values."));
    }

    #[test]
    fn special_missing_roundtrip_through_output() {
        let mut s = session();
        write_class(&s, "inp");
        // .a : missing spécial assigné puis écrit ; doit survivre au parquet.
        run("data out; set inp; m = .a; run;", &mut s).unwrap();
        let ds = read_work(&s, "out");
        // Relu : NaN payload → décodé par num_to_value au prochain SET ;
        // au niveau parquet brut c'est un NaN (pas un null).
        let m = ds.df.column("m").unwrap().f64().unwrap();
        let raw = m.get(0);
        assert!(raw.is_some_and(f64::is_nan));
        assert_eq!(
            crate::missing::num_to_value(raw),
            Value::Missing(MissingKind::Letter(0))
        );
    }

    // ── RETAIN / sum statement / LENGTH (M2) ─────────────────────────────

    #[test]
    fn sum_statement_counter_increments_per_obs() {
        let mut s = session();
        write_class(&s, "inp");
        run("data out; set inp; n + 1; run;", &mut s).unwrap();
        let ds = read_work(&s, "out");
        let n = ds.df.column("n").unwrap().f64().unwrap();
        assert_eq!(n.get(0), Some(1.0));
        assert_eq!(n.get(1), Some(2.0));
        assert_eq!(n.get(2), Some(3.0));
    }

    #[test]
    fn sum_statement_ignores_missing_increment() {
        let mut s = session();
        write_class(&s, "inp");
        // age = 14, ., 13 : le missing du milieu ajoute 0 (PAS propagé).
        run("data out; set inp; total + age; run;", &mut s).unwrap();
        let ds = read_work(&s, "out");
        let total = ds.df.column("total").unwrap().f64().unwrap();
        assert_eq!(total.get(0), Some(14.0));
        assert_eq!(total.get(1), Some(14.0));
        assert_eq!(total.get(2), Some(27.0));
        // Aucun missing généré par le sum statement.
        let log = s.log.into_string();
        assert!(
            !log.contains("Missing values were generated"),
            "log was: {log}"
        );
    }

    #[test]
    fn sum_statement_missing_accumulator_restarts_from_zero() {
        let mut s = session();
        write_class(&s, "inp");
        // total remis à `.` à chaque itération : total + age repart de 0.
        run("data out; set inp; total = .; total + age; run;", &mut s).unwrap();
        let ds = read_work(&s, "out");
        let total = ds.df.column("total").unwrap().f64().unwrap();
        assert_eq!(total.get(0), Some(14.0));
        assert_eq!(total.get(1), Some(0.0));
        assert_eq!(total.get(2), Some(13.0));
    }

    #[test]
    fn retain_with_init_accumulates_max() {
        let mut s = session();
        write_class(&s, "inp");
        run(
            "data out; set inp; retain maxage 0; if age > maxage then maxage = age; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        let maxage = ds.df.column("maxage").unwrap().f64().unwrap();
        // 14 dès la 1re obs, retenu ensuite (. > 14 faux, 13 > 14 faux).
        assert_eq!(maxage.get(0), Some(14.0));
        assert_eq!(maxage.get(1), Some(14.0));
        assert_eq!(maxage.get(2), Some(14.0));
    }

    #[test]
    fn retain_initial_value_wins_over_sum_zero() {
        let mut s = session();
        write_class(&s, "inp");
        run("data out; set inp; n + 1; retain n 100; run;", &mut s).unwrap();
        let ds = read_work(&s, "out");
        let n = ds.df.column("n").unwrap().f64().unwrap();
        assert_eq!(n.get(0), Some(101.0));
        assert_eq!(n.get(2), Some(103.0));
    }

    #[test]
    fn retain_without_init_keeps_value_across_iterations() {
        let mut s = session();
        write_class(&s, "inp");
        // prev : Name de l'observation précédente ('' à la 1re itération).
        run(
            "data out; set inp; retain prev; output; prev = name; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        let prev = ds.df.column("prev").unwrap().str().unwrap();
        assert_eq!(prev.get(0), Some(""));
        assert_eq!(prev.get(1), Some("Alfred"));
        assert_eq!(prev.get(2), Some("Alice"));
    }

    #[test]
    fn length_truncates_longer_assignment() {
        let mut s = session();
        let stats = run("data out; length c $ 3; c = 'abcdef'; run;", &mut s).unwrap();
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 1)]);
        let ds = read_work(&s, "out");
        assert_eq!(ds.df.column("c").unwrap().str().unwrap().get(0), Some("abc"));
        assert_eq!(ds.vars[0].length, 3);
    }

    #[test]
    fn retain_char_init_truncated_to_fixed_length() {
        let mut s = session();
        // c figée Char(3) par LENGTH ; l'init RETAIN 'abcdef' est tronquée
        // par le pdv.set normal au moment de poser les valeurs initiales.
        run("data out; length c $ 3; retain c 'abcdef'; run;", &mut s).unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(ds.df.column("c").unwrap().str().unwrap().get(0), Some("abc"));
    }

    // ── DO itératif / DELETE (M2) ────────────────────────────────────────

    /// Lit la valeur f64 de la colonne `col`, ligne 0, de WORK.`table`.
    fn num_at(session: &Session, table: &str, col: &str, row: usize) -> Option<f64> {
        read_work(session, table)
            .df
            .column(col)
            .unwrap()
            .f64()
            .unwrap()
            .get(row)
    }

    #[test]
    fn do_to_sums_one_to_ten() {
        let mut s = session();
        run("data out; s = 0; do i = 1 to 10; s = s + i; end; run;", &mut s).unwrap();
        assert_eq!(num_at(&s, "out", "s", 0), Some(55.0));
    }

    #[test]
    fn index_is_four_after_do_one_to_three() {
        let mut s = session();
        // Règle SAS célèbre : à la sortie par le test TO, i vaut la
        // PREMIÈRE valeur qui dépasse.
        run("data out; do i = 1 to 3; end; run;", &mut s).unwrap();
        assert_eq!(num_at(&s, "out", "i", 0), Some(4.0));
    }

    #[test]
    fn do_negative_by_runs_three_times() {
        let mut s = session();
        run("data out; do i = 3 to 1 by -1; n + 1; end; run;", &mut s).unwrap();
        assert_eq!(num_at(&s, "out", "n", 0), Some(3.0));
        // Sortie par le test TO : i a dépassé vers le bas.
        assert_eq!(num_at(&s, "out", "i", 0), Some(0.0));
    }

    #[test]
    fn do_fractional_by() {
        let mut s = session();
        // 1, 1.5, 2, 2.5, 3 → 5 tours ; i == 3.5 après.
        run("data out; do i = 1 to 3 by 0.5; n + 1; end; run;", &mut s).unwrap();
        assert_eq!(num_at(&s, "out", "n", 0), Some(5.0));
        assert_eq!(num_at(&s, "out", "i", 0), Some(3.5));
    }

    #[test]
    fn do_while_clause_cuts_iteration() {
        let mut s = session();
        // WHILE testé avant chaque tour : coupe à i = 4 (3 tours).
        run(
            "data out; do i = 1 to 10 while(i < 4); n + 1; end; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "n", 0), Some(3.0));
        assert_eq!(num_at(&s, "out", "i", 0), Some(4.0));
    }

    #[test]
    fn do_until_runs_at_least_once() {
        let mut s = session();
        run("data out; do until(1); n + 1; end; run;", &mut s).unwrap();
        assert_eq!(num_at(&s, "out", "n", 0), Some(1.0));
    }

    #[test]
    fn do_while_false_runs_zero_times() {
        let mut s = session();
        run("data out; n = 0; do while(0); n + 1; end; run;", &mut s).unwrap();
        assert_eq!(num_at(&s, "out", "n", 0), Some(0.0));
    }

    #[test]
    fn pure_do_while_loops_until_condition_false() {
        let mut s = session();
        run("data out; x = 0; do while(x < 3); x = x + 1; end; run;", &mut s).unwrap();
        assert_eq!(num_at(&s, "out", "x", 0), Some(3.0));
    }

    #[test]
    fn delete_filters_missing_age() {
        let mut s = session();
        write_class(&s, "inp");
        // 3 obs dont 1 age missing → 2 obs en sortie.
        let stats = run("data out; set inp; if age = . then delete; run;", &mut s).unwrap();
        assert_eq!(stats.read, vec![("WORK.INP".to_string(), 3)]);
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 2, 2)]);
        let ds = read_work(&s, "out");
        let name = ds.df.column("Name").unwrap().str().unwrap();
        assert_eq!(name.get(0), Some("Alfred"));
        assert_eq!(name.get(1), Some("Barbara"));
    }

    #[test]
    fn delete_inside_do_exits_loop_and_iteration() {
        let mut s = session();
        write_class(&s, "inp");
        // Chaque itération entre dans le DO et DELETE à i = 2 : le Flow
        // NextIter traverse la boucle → aucune obs en sortie, tout est lu.
        let stats = run(
            "data out; set inp; do i = 1 to 10; if i = 2 then delete; end; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(stats.read, vec![("WORK.INP".to_string(), 3)]);
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 0, 3)]);
    }

    #[test]
    fn nested_do_loops() {
        let mut s = session();
        run(
            "data out; do i = 1 to 3; do j = 1 to 2; n + 1; end; end; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "n", 0), Some(6.0));
        assert_eq!(num_at(&s, "out", "i", 0), Some(4.0));
        assert_eq!(num_at(&s, "out", "j", 0), Some(3.0));
    }

    #[test]
    fn do_bounds_are_evaluated_once_at_entry() {
        let mut s = session();
        // n modifié dans le corps : la borne TO reste celle de l'entrée
        // (3) — règle SAS, les bornes sont figées.
        run(
            "data out; n = 3; do i = 1 to n; n = 0; c + 1; end; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "c", 0), Some(3.0));
        assert_eq!(num_at(&s, "out", "i", 0), Some(4.0));
    }

    #[test]
    fn stop_inside_do_ends_step() {
        let mut s = session();
        write_class(&s, "inp");
        let stats = run(
            "data out; set inp; do i = 1 to 10; stop; end; run;",
            &mut s,
        )
        .unwrap();
        // STOP au premier tour de la première itération : rien d'écrit,
        // une seule ligne lue.
        assert_eq!(stats.read, vec![("WORK.INP".to_string(), 1)]);
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 0, 3)]);
    }

    #[test]
    fn set_exhausted_inside_do_ends_step() {
        let mut s = session();
        write_class(&s, "inp");
        // Le SET vit DANS la boucle : à l'épuisement de l'input (4e tour),
        // EndStep traverse le DO et termine l'étape. 3 outputs explicites.
        let stats = run(
            "data out; do i = 1 to 10; set inp; output; end; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(stats.read, vec![("WORK.INP".to_string(), 3)]);
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 3, 3)]);
    }

    #[test]
    fn missing_do_bound_is_runtime_error() {
        let mut s = session();
        // m jamais assignée → missing : erreur de contrôle de boucle
        // (divergence documentée : stoppe l'étape).
        let err = run("data out; do i = 1 to m; end; run;", &mut s)
            .err()
            .unwrap();
        assert_eq!(err.to_string(), "Invalid DO loop control information.");
    }

    #[test]
    fn modifying_index_in_body_affects_loop() {
        let mut s = session();
        // L'index est une variable normale du PDV : i = i + 1 dans le
        // corps saute une valeur sur deux → 5 tours (1,3,5,7,9).
        run(
            "data out; do i = 1 to 10; i = i + 1; n + 1; end; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "n", 0), Some(5.0));
        assert_eq!(num_at(&s, "out", "i", 0), Some(11.0));
    }

    #[test]
    fn infinite_do_loop_guard_trips() {
        let mut s = session();
        let err = run("data out; do while(1); end; run;", &mut s)
            .err()
            .unwrap();
        assert_eq!(
            err.to_string(),
            "DO loop exceeded 10000000 iterations; stopping (possible infinite loop)."
        );
    }

    // ── ARRAY 1-D + indexation (M2, lot 3) ───────────────────────────────

    #[test]
    fn array_fill_via_do_loop_braces() {
        let mut s = session();
        run(
            "data out; array a{3} x y z; do i = 1 to 3; a{i} = i * 10; end; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "x", 0), Some(10.0));
        assert_eq!(num_at(&s, "out", "y", 0), Some(20.0));
        assert_eq!(num_at(&s, "out", "z", 0), Some(30.0));
    }

    #[test]
    fn array_paren_form_lvalue_and_rvalue() {
        let mut s = session();
        // Lvalue `a(i) = ...` et rvalue `a(i)` (l'array masque la
        // fonction) ; lecture croisée via t = a(1) + a(3).
        run(
            "data out; array a(3) x y z; do i = 1 to 3; a(i) = i * 10; end; \
             t = a(1) + a(3); run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "x", 0), Some(10.0));
        assert_eq!(num_at(&s, "out", "y", 0), Some(20.0));
        assert_eq!(num_at(&s, "out", "z", 0), Some(30.0));
        assert_eq!(num_at(&s, "out", "t", 0), Some(40.0));
    }

    #[test]
    fn array_sum_via_dim() {
        let mut s = session();
        run(
            "data out; array a{3} x y z; do i = 1 to 3; a{i} = i; end; \
             do i = 1 to dim(a); s + a{i}; end; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "s", 0), Some(6.0));
    }

    #[test]
    fn array_index_rounds_to_nearest() {
        let mut s = session();
        // 1.4 → 1, 2.6 → 3 (arrondi au plus proche, comme SAS).
        run(
            "data out; array a{3} x y z; a{1.4} = 7; a{2.6} = 9; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "x", 0), Some(7.0));
        assert_eq!(num_at(&s, "out", "z", 0), Some(9.0));
        assert_eq!(num_at(&s, "out", "y", 0), None);
    }

    #[test]
    fn char_array_with_truncation() {
        let mut s = session();
        run(
            "data out; array c{2} $ 3 u v; c{1} = 'abcdef'; c{2} = 'xy'; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        // Longueur fixe 3 : troncature silencieuse à l'assignation.
        assert_eq!(ds.df.column("u").unwrap().str().unwrap().get(0), Some("abc"));
        assert_eq!(ds.df.column("v").unwrap().str().unwrap().get(0), Some("xy"));
        assert_eq!(ds.vars[0].length, 3);
    }

    #[test]
    fn char_array_default_length_is_8() {
        let mut s = session();
        run("data out; array c{1} $ u; c{1} = 'abcdefghij'; run;", &mut s).unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(
            ds.df.column("u").unwrap().str().unwrap().get(0),
            Some("abcdefgh")
        );
        assert_eq!(ds.vars[0].length, 8);
    }

    // ── M16.2 : arrays multi-dimensionnels, valeurs initiales, DIM/HBOUND/
    //    LBOUND, _TEMPORARY_/_NUMERIC_/_CHARACTER_/_ALL_ ─────────────────

    #[test]
    fn array_2d_creation_and_access_row_major() {
        let mut s = session();
        // 2×3 array sur 6 variables ; remplissage row-major v(i,j) = i*10+j.
        run(
            "data out; array m{2,3} v1-v6; do i = 1 to 2; do j = 1 to 3; \
             m{i,j} = i*10 + j; end; end; run;",
            &mut s,
        )
        .unwrap();
        // Ordre row-major : v1=m(1,1), v2=m(1,2), v3=m(1,3), v4=m(2,1)...
        assert_eq!(num_at(&s, "out", "v1", 0), Some(11.0));
        assert_eq!(num_at(&s, "out", "v2", 0), Some(12.0));
        assert_eq!(num_at(&s, "out", "v3", 0), Some(13.0));
        assert_eq!(num_at(&s, "out", "v4", 0), Some(21.0));
        assert_eq!(num_at(&s, "out", "v5", 0), Some(22.0));
        assert_eq!(num_at(&s, "out", "v6", 0), Some(23.0));
    }

    #[test]
    fn array_3d_creation_and_access() {
        let mut s = session();
        // 2×3×2 = 12 slots, éléments auto-nommés t1..t12.
        run(
            "data out; array t{2,3,2}; \
             t{1,1,1} = 1; t{1,1,2} = 2; t{2,3,2} = 99; \
             a = t{1,1,1}; b = t{1,1,2}; c = t{2,3,2}; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "a", 0), Some(1.0));
        assert_eq!(num_at(&s, "out", "b", 0), Some(2.0));
        assert_eq!(num_at(&s, "out", "c", 0), Some(99.0));
        // t1 = (1,1,1) ; t12 = (2,3,2).
        assert_eq!(num_at(&s, "out", "t1", 0), Some(1.0));
        assert_eq!(num_at(&s, "out", "t12", 0), Some(99.0));
    }

    #[test]
    fn array_linear_index_on_multidim() {
        let mut s = session();
        // Accès linéaire `m{n}` sur un array 2-D (interprétation row-major).
        run(
            "data out; array m{2,3} v1-v6; do n = 1 to 6; m{n} = n*n; end; \
             a = m{1,1}; f = m{2,3}; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "v1", 0), Some(1.0));
        assert_eq!(num_at(&s, "out", "v6", 0), Some(36.0));
        assert_eq!(num_at(&s, "out", "a", 0), Some(1.0));
        assert_eq!(num_at(&s, "out", "f", 0), Some(36.0));
    }

    #[test]
    fn array_initial_values_row_major() {
        let mut s = session();
        run(
            "data out; array a{2,2} (1, 2, 3, 4); \
             p = a{1,1}; q = a{1,2}; r = a{2,1}; t = a{2,2}; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "p", 0), Some(1.0));
        assert_eq!(num_at(&s, "out", "q", 0), Some(2.0));
        assert_eq!(num_at(&s, "out", "r", 0), Some(3.0));
        assert_eq!(num_at(&s, "out", "t", 0), Some(4.0));
    }

    #[test]
    fn array_initial_values_space_separated_1d() {
        let mut s = session();
        run(
            "data out; array a{3} x y z (10 20 30); run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "x", 0), Some(10.0));
        assert_eq!(num_at(&s, "out", "y", 0), Some(20.0));
        assert_eq!(num_at(&s, "out", "z", 0), Some(30.0));
    }

    #[test]
    fn array_dim_hbound_lbound_functions() {
        let mut s = session();
        run(
            "data out; array m{2,3} v1-v6; \
             nd = dim(m); n1 = dim(m, 1); n2 = dim(m, 2); \
             hb = hbound(m); hb2 = hbound(m, 2); \
             lb = lbound(m); lb2 = lbound(m, 2); run;",
            &mut s,
        )
        .unwrap();
        // dim(m) sans n = 1re dimension = 2 ; dim(m,2) = 3.
        assert_eq!(num_at(&s, "out", "nd", 0), Some(2.0));
        assert_eq!(num_at(&s, "out", "n1", 0), Some(2.0));
        assert_eq!(num_at(&s, "out", "n2", 0), Some(3.0));
        // hbound = borne supérieure (= dim, lbound=1).
        assert_eq!(num_at(&s, "out", "hb", 0), Some(2.0));
        assert_eq!(num_at(&s, "out", "hb2", 0), Some(3.0));
        // lbound toujours 1.
        assert_eq!(num_at(&s, "out", "lb", 0), Some(1.0));
        assert_eq!(num_at(&s, "out", "lb2", 0), Some(1.0));
    }

    #[test]
    fn array_dim_on_1d_array() {
        let mut s = session();
        run(
            "data out; array a{5} a1-a5; d = dim(a); h = hbound(a); l = lbound(a); run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "d", 0), Some(5.0));
        assert_eq!(num_at(&s, "out", "h", 0), Some(5.0));
        assert_eq!(num_at(&s, "out", "l", 0), Some(1.0));
    }

    #[test]
    fn array_temporary_elements_not_in_output() {
        let mut s = session();
        run(
            "data out; array t{3} _temporary_ (100 200 300); \
             total = t{1} + t{2} + t{3}; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "total", 0), Some(600.0));
        let ds = read_work(&s, "out");
        // Les éléments temporaires ne sont PAS des colonnes de sortie.
        let cols: Vec<&str> = ds.df.get_column_names().iter().map(|s| s.as_str()).collect();
        assert_eq!(cols, vec!["total"], "temporary elements must not be output");
    }

    #[test]
    fn array_temporary_retained_across_iterations() {
        let mut s = session();
        write_class(&s, "inp");
        // Les éléments _TEMPORARY_ sont retenus : un compteur accumule
        // (valeur initiale 0, puis +1 par itération).
        run(
            "data out; set inp; array acc{1} _temporary_ (0); \
             acc{1} = acc{1} + 1; n = acc{1}; run;",
            &mut s,
        )
        .unwrap();
        // 3 observations → n vaut 1, 2, 3 (retenu, pas remis à missing).
        assert_eq!(num_at(&s, "out", "n", 0), Some(1.0));
        assert_eq!(num_at(&s, "out", "n", 1), Some(2.0));
        assert_eq!(num_at(&s, "out", "n", 2), Some(3.0));
    }

    #[test]
    fn array_numeric_special_list() {
        let mut s = session();
        // _NUMERIC_ : toutes les variables numériques déjà connues.
        run(
            "data out; x = 1; y = 2; z = 3; array nums{*} _numeric_; \
             d = dim(nums); s = 0; do i = 1 to dim(nums); s = s + nums{i}; end; run;",
            &mut s,
        )
        .unwrap();
        // x, y, z sont les 3 numériques (i, d, s entrent APRÈS l'ARRAY).
        assert_eq!(num_at(&s, "out", "d", 0), Some(3.0));
        assert_eq!(num_at(&s, "out", "s", 0), Some(6.0));
    }

    #[test]
    fn array_character_special_list() {
        let mut s = session();
        run(
            "data out; a = 'foo'; b = 'bar'; array chs{*} $ _character_; \
             d = dim(chs); chs{1} = 'NEW'; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "d", 0), Some(2.0));
        let ds = read_work(&s, "out");
        // chs{1} pointe sur la 1re variable char (a).
        assert_eq!(ds.df.column("a").unwrap().str().unwrap().get(0), Some("NEW"));
    }

    #[test]
    fn array_mixing_1d_and_multidim() {
        let mut s = session();
        // Une étape avec un array 1-D et un array 2-D coexistants.
        run(
            "data out; array a{3} a1-a3; array m{2,2} m1-m4; \
             do i = 1 to 3; a{i} = i; end; \
             m{1,1} = 9; m{2,2} = 8; \
             da = dim(a); dm = dim(m); dm2 = dim(m,2); run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "a2", 0), Some(2.0));
        assert_eq!(num_at(&s, "out", "m1", 0), Some(9.0));
        assert_eq!(num_at(&s, "out", "m4", 0), Some(8.0));
        assert_eq!(num_at(&s, "out", "da", 0), Some(3.0));
        assert_eq!(num_at(&s, "out", "dm", 0), Some(2.0));
        assert_eq!(num_at(&s, "out", "dm2", 0), Some(2.0));
    }

    #[test]
    fn array_2d_out_of_bounds_stops_step() {
        // Indice de dimension hors bornes : arrêt avec ERROR.
        let out = crate::run(
            "data out; array m{2,3} v1-v6; m{3,1} = 1; run;",
            crate::RunOptions {
                work_dir: None,
                base_dir: None,
                deterministic: true,
                vectorize: false,
            },
        );
        assert_eq!(out.exit_code, 2, "log was:\n{}", out.log);
        assert!(
            out.log.contains("ERROR: Array subscript out of range."),
            "log was:\n{}",
            out.log
        );
    }

    #[test]
    fn array_2d_wrong_index_count_stops_step() {
        // 2 indices attendus, 3 fournis → hors bornes.
        let out = crate::run(
            "data out; array m{2,3} v1-v6; t = m{1,2,1}; run;",
            crate::RunOptions {
                work_dir: None,
                base_dir: None,
                deterministic: true,
                vectorize: false,
            },
        );
        assert_eq!(out.exit_code, 2, "log was:\n{}", out.log);
        assert!(
            out.log.contains("ERROR: Array subscript out of range."),
            "log was:\n{}",
            out.log
        );
    }

    #[test]
    fn array_initial_too_many_values_errors() {
        let mut s = session();
        match run("data out; array a{2} x y (1 2 3); run;", &mut s) {
            Err(e) => assert!(
                e.to_string().contains("Too many initial values"),
                "wrong error message: {e}"
            ),
            Ok(_) => panic!("expected too-many-initial-values error"),
        }
    }

    #[test]
    fn array_dim_count_mismatch_errors() {
        let mut s = session();
        // 2×3 = 6 attendus, 4 variables fournies.
        match run("data out; array m{2,3} a b c d; run;", &mut s) {
            Err(e) => assert!(
                e.to_string().contains("does not match"),
                "wrong error message: {e}"
            ),
            Ok(_) => panic!("expected dimension-mismatch error"),
        }
    }

    #[test]
    fn out_of_range_subscript_stops_step_with_error() {
        // Lvalue hors bornes : l'étape s'arrête avec ERROR (exit code 2).
        let out = crate::run(
            "data out; array a{3} x y z; a{4} = 1; run;",
            crate::RunOptions {
                work_dir: None,
                base_dir: None,
                deterministic: true,
                vectorize: false,
            },
        );
        assert_eq!(out.exit_code, 2, "log was:\n{}", out.log);
        assert!(
            out.log.contains("ERROR: Array subscript out of range."),
            "log was:\n{}",
            out.log
        );
        assert!(out
            .log
            .contains("The SAS System stopped processing this step because of errors."));

        // Rvalue hors bornes (y compris indice 0) : même arrêt.
        let out = crate::run(
            "data out; array a{3} x y z; t = a{0}; run;",
            crate::RunOptions {
                work_dir: None,
                base_dir: None,
                deterministic: true,
                vectorize: false,
            },
        );
        assert_eq!(out.exit_code, 2, "log was:\n{}", out.log);
        assert!(
            out.log.contains("ERROR: Array subscript out of range."),
            "log was:\n{}",
            out.log
        );
    }

    #[test]
    fn missing_subscript_stops_step_with_error() {
        let out = crate::run(
            "data out; array a{3} x y z; i = .; a{i} = 1; run;",
            crate::RunOptions {
                work_dir: None,
                base_dir: None,
                deterministic: true,
                vectorize: false,
            },
        );
        assert_eq!(out.exit_code, 2, "log was:\n{}", out.log);
        assert!(
            out.log.contains("ERROR: Array subscript out of range."),
            "log was:\n{}",
            out.log
        );
    }

    #[test]
    fn auto_named_elements_are_usable_as_variables() {
        let mut s = session();
        // a1 a2 a3 auto-nommés : adressables par indice ET par nom.
        run(
            "data out; array a{3}; do i = 1 to 3; a{i} = i; end; t = a1 + a2 + a3; \
             a2 = 20; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "a1", 0), Some(1.0));
        assert_eq!(num_at(&s, "out", "a2", 0), Some(20.0));
        assert_eq!(num_at(&s, "out", "a3", 0), Some(3.0));
        assert_eq!(num_at(&s, "out", "t", 0), Some(6.0));
    }

    #[test]
    fn array_over_input_variables_updates_them() {
        let mut s = session();
        write_class(&s, "inp");
        // Array sur une variable d'input : l'élément référence le slot
        // existant (type/longueur de l'input conservés).
        run(
            "data out; set inp; array nums{1} age; nums{1} = nums{1} * 2; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        let age = ds.df.column("Age").unwrap().f64().unwrap();
        assert_eq!(age.get(0), Some(28.0));
        assert_eq!(age.get(2), Some(26.0));
    }

    #[test]
    fn multiple_outputs_written() {
        let mut s = session();
        write_class(&s, "inp");
        let stats = run("data a b; set inp; run;", &mut s).unwrap();
        assert_eq!(stats.written.len(), 2);
        assert!(s.libs.get("WORK").unwrap().exists("a"));
        assert!(s.libs.get("WORK").unwrap().exists("b"));
        // _LAST_ = dernière sortie.
        assert_eq!(s.last_dataset.as_deref(), Some("WORK.B"));
    }

    // ── Options de dataset + OUTPUT ciblé (M2, lot 4) ────────────────────

    /// Mini-CLASS à trois variables (Name char, Sex char, Age num) pour les
    /// tests d'options de dataset et de sorties multiples.
    fn write_class_full(session: &Session, table: &str) {
        let df = df!(
            "Name" => ["Alfred", "Alice", "Barbara", "Henry"],
            "Sex" => ["M", "F", "F", "M"],
            "Age" => [Some(14.0), None, Some(13.0), Some(15.0)],
        )
        .unwrap();
        let vars = vec![
            VarMeta {
                name: "Name".into(),
                ty: VarType::Char,
                length: 7,
                format: None,
                label: None,
            },
            VarMeta {
                name: "Sex".into(),
                ty: VarType::Char,
                length: 1,
                format: None,
                label: None,
            },
            VarMeta {
                name: "Age".into(),
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
    fn set_keep_outputs_only_kept_variables() {
        let mut s = session();
        write_class_full(&s, "class");
        let stats = run("data out; set class(keep=name age); run;", &mut s).unwrap();
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 4, 2)]);
        let ds = read_work(&s, "out");
        let cols: Vec<&str> = ds.df.get_column_names_str();
        assert_eq!(cols, vec!["Name", "Age"]);
    }

    #[test]
    fn set_where_filters_rows_and_read_counter() {
        let mut s = session();
        write_class_full(&s, "class");
        let stats = run("data out; set class(where=(age > 13)); run;", &mut s).unwrap();
        // 14, ., 13, 15 : seuls 14 et 15 passent ; le compteur d'obs LUES
        // est réduit aux lignes qui passent (fidèle à la NOTE SAS).
        assert_eq!(stats.read, vec![("WORK.CLASS".to_string(), 2)]);
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 2, 3)]);
        let ds = read_work(&s, "out");
        let name = ds.df.column("Name").unwrap().str().unwrap();
        assert_eq!(name.get(0), Some("Alfred"));
        assert_eq!(name.get(1), Some("Henry"));
        let log = s.log.into_string();
        assert!(
            log.contains("There were 2 observations read from the data set WORK.CLASS."),
            "log was: {log}"
        );
    }

    #[test]
    fn set_rename_exposes_new_name_only() {
        let mut s = session();
        write_class_full(&s, "class");
        run(
            "data out; set class(rename=(age=years)); next = years + 1; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        let cols: Vec<&str> = ds.df.get_column_names_str();
        assert!(cols.contains(&"years"), "columns were: {cols:?}");
        assert!(!cols.contains(&"Age"), "columns were: {cols:?}");
        let years = ds.df.column("years").unwrap().f64().unwrap();
        assert_eq!(years.get(0), Some(14.0));
        let next = ds.df.column("next").unwrap().f64().unwrap();
        assert_eq!(next.get(0), Some(15.0));
    }

    #[test]
    fn output_drop_option_removes_work_variable() {
        let mut s = session();
        write_class_full(&s, "class");
        run(
            "data out(drop=tmp); set class; tmp = age * 2; final = tmp + 1; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        let cols: Vec<&str> = ds.df.get_column_names_str();
        assert!(!cols.contains(&"tmp"), "columns were: {cols:?}");
        let final_ = ds.df.column("final").unwrap().f64().unwrap();
        assert_eq!(final_.get(0), Some(29.0));
    }

    #[test]
    fn targeted_outputs_split_disjoint_datasets() {
        let mut s = session();
        write_class_full(&s, "class");
        let stats = run(
            "data m f; set class; if sex = 'M' then output m; else output f; run;",
            &mut s,
        )
        .unwrap();
        // Deux datasets disjoints, total = obs d'origine, comptes PAR
        // sortie indépendants.
        assert_eq!(
            stats.written,
            vec![
                ("WORK.M".to_string(), 2, 3),
                ("WORK.F".to_string(), 2, 3),
            ]
        );
        let m = read_work(&s, "m");
        let f = read_work(&s, "f");
        assert_eq!(m.n_obs() + f.n_obs(), 4);
        let m_names = m.df.column("Name").unwrap().str().unwrap();
        assert_eq!(m_names.get(0), Some("Alfred"));
        assert_eq!(m_names.get(1), Some("Henry"));
        let f_names = f.df.column("Name").unwrap().str().unwrap();
        assert_eq!(f_names.get(0), Some("Alice"));
        assert_eq!(f_names.get(1), Some("Barbara"));
        let log = s.log.into_string();
        assert!(log.contains("The data set WORK.M has 2 observations and 3 variables."));
        assert!(log.contains("The data set WORK.F has 2 observations and 3 variables."));
    }

    #[test]
    fn targeted_output_two_names_writes_both() {
        let mut s = session();
        write_class(&s, "inp");
        let stats = run("data a b; set inp; output a b; run;", &mut s).unwrap();
        assert_eq!(
            stats.written,
            vec![("WORK.A".to_string(), 3, 2), ("WORK.B".to_string(), 3, 2)]
        );
    }

    #[test]
    fn output_rename_option_writes_renamed_column() {
        let mut s = session();
        write_class_full(&s, "class");
        run("data out(rename=(age=years)); set class; run;", &mut s).unwrap();
        let ds = read_work(&s, "out");
        // La colonne du parquet (et son VarMeta) porte le nouveau nom.
        let cols: Vec<&str> = ds.df.get_column_names_str();
        assert_eq!(cols, vec!["Name", "Sex", "years"]);
        assert_eq!(ds.vars[2].name, "years");
        let years = ds.df.column("years").unwrap().f64().unwrap();
        assert_eq!(years.get(3), Some(15.0));
    }

    #[test]
    fn where_skip_does_not_run_rest_of_iteration() {
        let mut s = session();
        write_class_full(&s, "class");
        // n compte les itérations qui exécutent le corps : avec WHERE=,
        // seules les lignes qui passent y arrivent.
        run(
            "data out; set class(where=(sex = 'F')); n + 1; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        let n = ds.df.column("n").unwrap().f64().unwrap();
        assert_eq!(ds.n_obs(), 2);
        assert_eq!(n.get(0), Some(1.0));
        assert_eq!(n.get(1), Some(2.0));
    }

    // ── SET multi-datasets + BY + FIRST./LAST. (M3) ──────────────────────

    /// Écrit un dataset entièrement numérique : colonnes (nom, valeurs).
    fn write_num_ds(session: &Session, table: &str, cols: &[(&str, Vec<Option<f64>>)]) {
        let mut columns: Vec<Column> = Vec::new();
        let mut vars: Vec<VarMeta> = Vec::new();
        for (name, vals) in cols {
            columns.push(Series::new((*name).into(), vals.clone()).into());
            vars.push(VarMeta {
                name: (*name).to_string(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            });
        }
        let df = DataFrame::new(columns).unwrap();
        session
            .libs
            .get("WORK")
            .unwrap()
            .write(table, &SasDataset { df, vars })
            .unwrap();
    }

    fn some(vals: &[f64]) -> Vec<Option<f64>> {
        vals.iter().copied().map(Some).collect()
    }

    /// Colonne f64 complète de WORK.`table`.
    fn col(session: &Session, table: &str, col: &str) -> Vec<Option<f64>> {
        read_work(session, table)
            .df
            .column(col)
            .unwrap()
            .f64()
            .unwrap()
            .iter()
            .collect()
    }

    #[test]
    fn set_two_datasets_without_by_concatenates() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[1.0, 3.0, 5.0]))]);
        write_num_ds(&s, "b", &[("x", some(&[2.0, 3.0, 4.0]))]);
        let stats = run("data out; set a b; run;", &mut s).unwrap();
        // Tout a, puis tout b.
        assert_eq!(
            col(&s, "out", "x"),
            some(&[1.0, 3.0, 5.0, 2.0, 3.0, 4.0])
        );
        assert_eq!(
            stats.read,
            vec![("WORK.A".to_string(), 3), ("WORK.B".to_string(), 3)]
        );
        let log = s.log.into_string();
        assert!(log.contains("There were 3 observations read from the data set WORK.A."));
        assert!(log.contains("There were 3 observations read from the data set WORK.B."));
    }

    #[test]
    fn set_by_interleaves_sorted_datasets_with_union_and_retained_vars() {
        let mut s = session();
        // u n'existe que dans a, v que dans b.
        write_num_ds(
            &s,
            "a",
            &[("x", some(&[1.0, 3.0, 5.0])), ("u", some(&[10.0, 30.0, 50.0]))],
        );
        write_num_ds(
            &s,
            "b",
            &[("x", some(&[2.0, 3.0, 4.0])), ("v", some(&[200.0, 300.0, 400.0]))],
        );
        let stats = run(
            "data out; set a b; by x; f = first.x; l = last.x; run;",
            &mut s,
        )
        .unwrap();
        // Interclassement par x croissant ; égalité (x=3) → a (premier du
        // SET) avant b.
        assert_eq!(
            col(&s, "out", "x"),
            some(&[1.0, 2.0, 3.0, 3.0, 4.0, 5.0])
        );
        // u/v : RETAIN implicite des variables de SET — une variable
        // absente du dataset de l'obs courante GARDE sa valeur précédente
        // (et reste missing avant sa première lecture).
        assert_eq!(
            col(&s, "out", "u"),
            vec![Some(10.0), Some(10.0), Some(30.0), Some(30.0), Some(30.0), Some(50.0)]
        );
        assert_eq!(
            col(&s, "out", "v"),
            vec![None, Some(200.0), Some(200.0), Some(300.0), Some(400.0), Some(400.0)]
        );
        // FIRST.x / LAST.x : le groupe x=3 a deux obs ; LAST. de la
        // dernière obs globale vaut 1.
        assert_eq!(
            col(&s, "out", "f"),
            some(&[1.0, 1.0, 1.0, 0.0, 1.0, 1.0])
        );
        assert_eq!(
            col(&s, "out", "l"),
            some(&[1.0, 1.0, 0.0, 1.0, 1.0, 1.0])
        );
        assert_eq!(
            stats.read,
            vec![("WORK.A".to_string(), 3), ("WORK.B".to_string(), 3)]
        );
    }

    #[test]
    fn first_last_group_count_per_group() {
        let mut s = session();
        write_num_ds(
            &s,
            "g",
            &[
                ("grp", some(&[1.0, 1.0, 1.0, 2.0, 2.0])),
                ("val", some(&[5.0, 6.0, 7.0, 8.0, 9.0])),
            ],
        );
        // Idiome SAS canonique : compteur remis à zéro en tête de groupe,
        // une obs émise par groupe (subsetting IF sur last.grp).
        run(
            "data out; set g; by grp; if first.grp then n = 0; n + 1; if last.grp; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(col(&s, "out", "grp"), some(&[1.0, 2.0]));
        assert_eq!(col(&s, "out", "n"), some(&[3.0, 2.0]));
    }

    #[test]
    fn two_by_keys_first_last_prefix_rule() {
        let mut s = session();
        write_num_ds(
            &s,
            "g",
            &[
                ("a", some(&[1.0, 1.0, 2.0])),
                ("b", some(&[7.0, 8.0, 8.0])),
            ],
        );
        run(
            "data out; set g; by a b; fa = first.a; fb = first.b; la = last.a; lb = last.b; run;",
            &mut s,
        )
        .unwrap();
        // first.b = 1 dès que a OU b change (préfixe de clés).
        assert_eq!(col(&s, "out", "fa"), some(&[1.0, 0.0, 1.0]));
        assert_eq!(col(&s, "out", "fb"), some(&[1.0, 1.0, 1.0]));
        // last.b suit le même préfixe vers l'obs suivante ; b=8 ne forme
        // PAS un groupe à cheval sur a=1/a=2.
        assert_eq!(col(&s, "out", "la"), some(&[0.0, 1.0, 1.0]));
        assert_eq!(col(&s, "out", "lb"), some(&[1.0, 1.0, 1.0]));
    }

    #[test]
    fn missing_by_key_collates_first() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", vec![None, Some(2.0)])]);
        write_num_ds(&s, "b", &[("x", some(&[1.0]))]);
        run("data out; set a b; by x; f = first.x; run;", &mut s).unwrap();
        // `.` < 1 < 2 (les missings se collationnent en premier).
        assert_eq!(col(&s, "out", "x"), vec![None, Some(1.0), Some(2.0)]);
        assert_eq!(col(&s, "out", "f"), some(&[1.0, 1.0, 1.0]));
    }

    #[test]
    fn descending_by_interleaves_in_decreasing_order() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[3.0, 1.0]))]);
        write_num_ds(&s, "b", &[("x", some(&[2.0]))]);
        run("data out; set a b; by descending x; run;", &mut s).unwrap();
        assert_eq!(col(&s, "out", "x"), some(&[3.0, 2.0, 1.0]));
    }

    #[test]
    fn unsorted_by_data_stops_with_error() {
        let mut s = session();
        write_num_ds(&s, "d", &[("x", some(&[2.0, 1.0]))]);
        let err = run("data out; set d; by x; run;", &mut s).err().unwrap();
        assert_eq!(
            err.to_string(),
            "BY variables are not properly sorted on data set WORK.D."
        );
    }

    #[test]
    fn where_option_is_prefiltered_before_interleave() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0, 3.0]))]);
        write_num_ds(&s, "b", &[("x", some(&[2.0]))]);
        // a filtré sur x ne 2 : l'interclassement voit 1,3 côté a.
        let stats = run(
            "data out; set a(where=(x ne 2)) b; by x; l = last.x; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(col(&s, "out", "x"), some(&[1.0, 2.0, 3.0]));
        assert_eq!(col(&s, "out", "l"), some(&[1.0, 1.0, 1.0]));
        // Les lignes rejetées par WHERE= ne comptent pas comme lues.
        assert_eq!(
            stats.read,
            vec![("WORK.A".to_string(), 2), ("WORK.B".to_string(), 1)]
        );
    }

    #[test]
    fn single_dataset_by_groups_match_simple_set() {
        let mut s = session();
        write_class(&s, "inp"); // Age = 14, ., 13 — PAS trié.
        // Un SET simple sans BY reste inchangé (chemin M1/M2 intact).
        let stats = run("data out; set inp; run;", &mut s).unwrap();
        assert_eq!(stats.read, vec![("WORK.INP".to_string(), 3)]);
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 3, 2)]);
    }

    // ── MERGE avec BY : match-merge SAS (M3-3) ───────────────────────────
    //
    // Plusieurs sorties sont COMPARÉES À UNE SORTIE SAS CALCULÉE À LA MAIN
    // (indiqué dans le commentaire de chaque test).

    /// Colonne char complète de WORK.`table`.
    fn col_str(session: &Session, table: &str, col: &str) -> Vec<Option<String>> {
        read_work(session, table)
            .df
            .column(col)
            .unwrap()
            .str()
            .unwrap()
            .iter()
            .map(|o| o.map(str::to_string))
            .collect()
    }

    #[test]
    fn merge_one_to_one() {
        // Sortie SAS calculée à la main : a={(1,x=10),(2,x=20)},
        // b={(1,y=100),(2,y=200)} ; merge a b; by id; → (1,10,100),(2,20,200).
        let mut s = session();
        write_num_ds(&s, "a", &[("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))]);
        write_num_ds(&s, "b", &[("id", some(&[1.0, 2.0])), ("y", some(&[100.0, 200.0]))]);
        let stats = run("data out; merge a b; by id; run;", &mut s).unwrap();
        assert_eq!(col(&s, "out", "id"), some(&[1.0, 2.0]));
        assert_eq!(col(&s, "out", "x"), some(&[10.0, 20.0]));
        assert_eq!(col(&s, "out", "y"), some(&[100.0, 200.0]));
        assert_eq!(
            stats.read,
            vec![("WORK.A".to_string(), 2), ("WORK.B".to_string(), 2)]
        );
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 2, 3)]);
    }

    #[test]
    fn merge_one_to_many_short_side_persists() {
        // Sortie SAS calculée à la main : a={(1,x=10),(1,x=20)}, b={(1,y=100)}
        // ; merge a b; by id; → (1,10,100),(1,20,100). y PERSISTE à 100 sur
        // la 2e obs (persistance du côté court).
        let mut s = session();
        write_num_ds(&s, "a", &[("id", some(&[1.0, 1.0])), ("x", some(&[10.0, 20.0]))]);
        write_num_ds(&s, "b", &[("id", some(&[1.0])), ("y", some(&[100.0]))]);
        run("data out; merge a b; by id; run;", &mut s).unwrap();
        assert_eq!(col(&s, "out", "id"), some(&[1.0, 1.0]));
        assert_eq!(col(&s, "out", "x"), some(&[10.0, 20.0]));
        // VÉRIFICATION EXPLICITE : y == 100 sur la 2e obs (persistance).
        assert_eq!(col(&s, "out", "y"), some(&[100.0, 100.0]));
    }

    #[test]
    fn merge_unmatched_keys_with_in_and_missing() {
        // Sortie SAS calculée à la main : a={(1,x=10),(3,x=30)},
        // b={(2,y=20),(3,y=33)} ; merge a(in=ina) b(in=inb); by id; →
        //   id=1 : x=10, y=. , ina=1, inb=0
        //   id=2 : x=. , y=20, ina=0, inb=1
        //   id=3 : x=30, y=33, ina=1, inb=1
        let mut s = session();
        write_num_ds(&s, "a", &[("id", some(&[1.0, 3.0])), ("x", some(&[10.0, 30.0]))]);
        write_num_ds(&s, "b", &[("id", some(&[2.0, 3.0])), ("y", some(&[20.0, 33.0]))]);
        run(
            "data out; merge a(in=ina) b(in=inb); by id; a_in = ina; b_in = inb; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(col(&s, "out", "id"), some(&[1.0, 2.0, 3.0]));
        assert_eq!(col(&s, "out", "x"), vec![Some(10.0), None, Some(30.0)]);
        assert_eq!(col(&s, "out", "y"), vec![None, Some(20.0), Some(33.0)]);
        assert_eq!(col(&s, "out", "a_in"), some(&[1.0, 0.0, 1.0]));
        assert_eq!(col(&s, "out", "b_in"), some(&[0.0, 1.0, 1.0]));
        // ina/inb sont des automatiques : jamais écrites en sortie.
        let out_ds = read_work(&s, "out");
        let cols: Vec<&str> = out_ds.df.get_column_names_str();
        assert!(!cols.contains(&"ina"), "cols: {cols:?}");
        assert!(!cols.contains(&"inb"), "cols: {cols:?}");
    }

    #[test]
    fn merge_inner_join_via_in() {
        // Idiome SAS : `if ina and inb;` = inner join. Mêmes données que le
        // test précédent → 1 obs (id=3). Sortie calculée à la main.
        let mut s = session();
        write_num_ds(&s, "a", &[("id", some(&[1.0, 3.0])), ("x", some(&[10.0, 30.0]))]);
        write_num_ds(&s, "b", &[("id", some(&[2.0, 3.0])), ("y", some(&[20.0, 33.0]))]);
        run(
            "data out; merge a(in=ina) b(in=inb); by id; if ina and inb; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(col(&s, "out", "id"), some(&[3.0]));
        assert_eq!(col(&s, "out", "x"), some(&[30.0]));
        assert_eq!(col(&s, "out", "y"), some(&[33.0]));
    }

    #[test]
    fn merge_variable_overlap_rightmost_wins() {
        // a={(1,v='A')}, b={(1,v='B')} ; merge a b; by id; → v='B' (le dernier
        // dataset du MERGE écrase). Sortie calculée à la main.
        let mut s = session();
        let id_a = Series::new("id".into(), &[Some(1.0)]);
        let v_a = Series::new("v".into(), &["A"]);
        let df_a = DataFrame::new(vec![id_a.into(), v_a.into()]).unwrap();
        let vars = vec![
            VarMeta { name: "id".into(), ty: VarType::Num, length: 8, format: None, label: None },
            VarMeta { name: "v".into(), ty: VarType::Char, length: 8, format: None, label: None },
        ];
        s.libs.get("WORK").unwrap().write("a", &SasDataset { df: df_a, vars: vars.clone() }).unwrap();
        let id_b = Series::new("id".into(), &[Some(1.0)]);
        let v_b = Series::new("v".into(), &["B"]);
        let df_b = DataFrame::new(vec![id_b.into(), v_b.into()]).unwrap();
        s.libs.get("WORK").unwrap().write("b", &SasDataset { df: df_b, vars }).unwrap();
        run("data out; merge a b; by id; run;", &mut s).unwrap();
        assert_eq!(col(&s, "out", "id"), some(&[1.0]));
        assert_eq!(col_str(&s, "out", "v"), vec![Some("B".to_string())]);
    }

    #[test]
    fn merge_first_last_on_one_to_many_group() {
        // FIRST./LAST. avec MERGE sur un groupe one-to-many. a a deux obs
        // id=1 et une id=2 ; b une obs id=1 et une id=2. Groupe id=1 → 2
        // obs : first=1/0, last=0/1 ; groupe id=2 → 1 obs : first=1, last=1.
        // Sortie calculée à la main.
        let mut s = session();
        write_num_ds(&s, "a", &[("id", some(&[1.0, 1.0, 2.0])), ("x", some(&[10.0, 11.0, 20.0]))]);
        write_num_ds(&s, "b", &[("id", some(&[1.0, 2.0])), ("y", some(&[100.0, 200.0]))]);
        run(
            "data out; merge a b; by id; f = first.id; l = last.id; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(col(&s, "out", "id"), some(&[1.0, 1.0, 2.0]));
        assert_eq!(col(&s, "out", "f"), some(&[1.0, 0.0, 1.0]));
        assert_eq!(col(&s, "out", "l"), some(&[0.0, 1.0, 1.0]));
        // y persiste sur la 2e obs du groupe id=1.
        assert_eq!(col(&s, "out", "y"), some(&[100.0, 100.0, 200.0]));
    }

    #[test]
    fn merge_unsorted_data_stops_with_error() {
        // Un dataset non trié selon le BY → ERROR de désordre.
        let mut s = session();
        write_num_ds(&s, "a", &[("id", some(&[2.0, 1.0])), ("x", some(&[1.0, 2.0]))]);
        write_num_ds(&s, "b", &[("id", some(&[1.0, 2.0])), ("y", some(&[1.0, 2.0]))]);
        let err = run("data out; merge a b; by id; run;", &mut s).err().unwrap();
        assert_eq!(
            err.to_string(),
            "BY variables are not properly sorted on data set WORK.A."
        );
    }

    #[test]
    fn merge_without_by_is_compile_error() {
        let mut s = session();
        write_num_ds(&s, "a", &[("id", some(&[1.0]))]);
        write_num_ds(&s, "b", &[("id", some(&[1.0]))]);
        let err = run("data out; merge a b; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("BY"), "got: {err}");
    }

    #[test]
    fn merge_after_set_is_error() {
        let mut s = session();
        write_num_ds(&s, "a", &[("id", some(&[1.0]))]);
        write_num_ds(&s, "b", &[("id", some(&[1.0]))]);
        let err = run("data out; set a; merge a b; by id; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("not allowed"), "got: {err}");
    }

    #[test]
    fn set_in_option_is_error() {
        let mut s = session();
        write_num_ds(&s, "a", &[("id", some(&[1.0]))]);
        let err = run("data out; set a(in=ina); run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("IN="), "got: {err}");
    }

    #[test]
    fn output_in_option_is_error() {
        let mut s = session();
        write_num_ds(&s, "a", &[("id", some(&[1.0]))]);
        let err = run("data out(in=foo); set a; run;", &mut s).err().unwrap();
        assert!(err.to_string().to_uppercase().contains("IN"), "got: {err}");
    }

    // ── Missings spéciaux bout en bout + _ERROR_ + NOTEs (M2, lot 5) ─────

    #[test]
    fn special_missings_keep_identity_through_parquet_roundtrip() {
        use crate::missing::num_to_value;
        let mut s = session();
        // Étape 1 : assigne les trois familles de missing et écrit en
        // parquet (WORK est une DirLibrary : écriture/lecture RÉELLES).
        run("data a; x = .a; y = ._; z = .; run;", &mut s).unwrap();

        // Le parquet de A relu directement : x/y sont des NaN (PAS des
        // nulls), z est un null (`.` ordinaire ⇔ null Polars).
        let a = read_work(&s, "a");
        let at = |c: &str| a.df.column(c).unwrap().f64().unwrap().get(0);
        assert!(at("x").is_some_and(f64::is_nan));
        assert!(at("y").is_some_and(f64::is_nan));
        assert_eq!(at("z"), None);
        // Et chaque missing garde SON IDENTITÉ au décodage.
        assert_eq!(num_to_value(at("x")), Value::Missing(MissingKind::Letter(0)));
        assert_eq!(num_to_value(at("y")), Value::Missing(MissingKind::Underscore));
        assert_eq!(num_to_value(at("z")), Value::missing());
        // x ≠ y ≠ z par sas_cmp (ordre total : ._ < . < .a).
        let (x, y, z) = (num_to_value(at("x")), num_to_value(at("y")), num_to_value(at("z")));
        assert_ne!(x.sas_cmp(&y), std::cmp::Ordering::Equal);
        assert_ne!(x.sas_cmp(&z), std::cmp::Ordering::Equal);
        assert_ne!(y.sas_cmp(&z), std::cmp::Ordering::Equal);

        // Étape 2 : relecture via SET — `.a` relu == `.a`, distinct de
        // `.b` et de `.`.
        run(
            "data b; set a; xa = (x = .a); xb = (x = .b); xd = (x = .); \
             xy = (x = y); yu = (y = ._); run;",
            &mut s,
        )
        .unwrap();
        let b = read_work(&s, "b");
        let bt = |c: &str| b.df.column(c).unwrap().f64().unwrap().get(0);
        assert_eq!(bt("xa"), Some(1.0), ".a relu doit valoir .a");
        assert_eq!(bt("xb"), Some(0.0), ".a relu doit rester distinct de .b");
        assert_eq!(bt("xd"), Some(0.0), ".a relu doit rester distinct de .");
        assert_eq!(bt("xy"), Some(0.0), ".a et ._ doivent rester distincts");
        assert_eq!(bt("yu"), Some(1.0), "._ relu doit valoir ._");
    }

    #[test]
    fn division_by_zero_sets_error_only_for_nonmissing_numerator() {
        let mut s = session();
        write_class(&s, "inp"); // age = 14, ., 13
        run("data out; set inp; r = age / 0; e = _error_; run;", &mut s).unwrap();
        let ds = read_work(&s, "out");
        let e = ds.df.column("e").unwrap().f64().unwrap();
        // 14/0 → division par zéro → _ERROR_=1 ; ./0 → missing propagé
        // (opérande missing AVANT le test du diviseur) SANS _ERROR_ — ce
        // 0 prouve aussi le RESET de _ERROR_ entre itérations ; 13/0 → 1.
        assert_eq!(e.get(0), Some(1.0));
        assert_eq!(e.get(1), Some(0.0));
        assert_eq!(e.get(2), Some(1.0));
        // Le résultat est `.` ordinaire dans tous les cas → nulls.
        let r = ds.df.column("r").unwrap().f64().unwrap();
        assert_eq!(r.null_count(), 3);
        // NOTE émise UNE fois malgré deux divisions par zéro, plus la
        // NOTE de missing généré (./0).
        let log = s.log.into_string();
        assert_eq!(
            log.matches("NOTE: Division by zero detected.").count(),
            1,
            "log was: {log}"
        );
        assert_eq!(
            log.matches(
                "NOTE: Missing values were generated as a result of \
                 performing an operation on missing values."
            )
            .count(),
            1,
            "log was: {log}"
        );
    }

    #[test]
    fn missing_over_zero_does_not_emit_division_note() {
        let mut s = session();
        run("data out; m = .; r = m / 0; e = _error_; run;", &mut s).unwrap();
        // missing/0 : propagation missing, PAS une division par zéro.
        assert_eq!(num_at(&s, "out", "e", 0), Some(0.0));
        assert_eq!(num_at(&s, "out", "r", 0), None);
        let log = s.log.into_string();
        assert!(!log.contains("Division by zero"), "log was: {log}");
        assert!(log.contains("Missing values were generated"), "log was: {log}");
    }

    #[test]
    fn automatic_variables_readable_but_never_output_columns() {
        let mut s = session();
        write_class(&s, "inp");
        let stats = run("data out; set inp; n = _n_; e = _error_; run;", &mut s).unwrap();
        // 4 colonnes seulement : ni _N_ ni _ERROR_ ne sont écrites.
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 3, 4)]);
        let ds = read_work(&s, "out");
        let cols: Vec<&str> = ds.df.get_column_names_str();
        assert_eq!(cols, vec!["Age", "Name", "n", "e"]);
        // _N_ == numéro d'observation avec un simple SET.
        let n = ds.df.column("n").unwrap().f64().unwrap();
        assert_eq!(n.get(0), Some(1.0));
        assert_eq!(n.get(1), Some(2.0));
        assert_eq!(n.get(2), Some(3.0));
        // Pas d'erreur : _ERROR_ = 0 partout.
        let e = ds.df.column("e").unwrap().f64().unwrap();
        assert!(e.iter().all(|v| v == Some(0.0)));
        // Et surtout pas de NOTE "uninitialized" parasite pour les
        // variables automatiques.
        let log = s.log.into_string();
        assert!(!log.contains("uninitialized"), "log was: {log}");
    }

    #[test]
    fn invalid_numeric_data_sets_error_with_single_note() {
        let mut s = session();
        write_class(&s, "inp");
        // name + 0 : conversion char→num invalide à chaque ligne →
        // _ERROR_=1 partout, mais chaque NOTE n'apparaît qu'UNE fois.
        run("data out; set inp; v = name + 0; e = _error_; run;", &mut s).unwrap();
        let ds = read_work(&s, "out");
        let e = ds.df.column("e").unwrap().f64().unwrap();
        assert!(e.iter().all(|v| v == Some(1.0)));
        let log = s.log.into_string();
        assert_eq!(
            log.matches("NOTE: Invalid numeric data.").count(),
            1,
            "log was: {log}"
        );
        assert_eq!(
            log.matches("NOTE: Character values have been converted to numeric values.")
                .count(),
            1,
            "log was: {log}"
        );
    }

    // ── INFILE / INPUT / DATALINES (M14) ─────────────────────────────────

    #[test]
    fn input_list_mode_basic() {
        let mut s = session();
        let stats = run(
            "data out; input name $ age; datalines;\nAlice 14\nBob 16\n;\nrun;",
            &mut s,
        )
        .unwrap();
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 2, 2)]);
        let ds = read_work(&s, "out");
        let name = ds.df.column("name").unwrap().str().unwrap();
        let age = ds.df.column("age").unwrap().f64().unwrap();
        assert_eq!(name.get(0), Some("Alice"));
        assert_eq!(age.get(0), Some(14.0));
        assert_eq!(name.get(1), Some("Bob"));
        assert_eq!(age.get(1), Some(16.0));
        // Données instream : SAS n'émet PAS de NOTE "records were read from
        // the infile DATALINES" (réservée aux fichiers externes) — seule la
        // NOTE du data set apparaît.
        let log = s.log.into_string();
        assert!(
            !log.contains("records were read from the infile"),
            "instream DATALINES must not emit an infile-records NOTE; log was: {log}"
        );
        assert!(
            log.contains("The data set WORK.OUT has 2 observations and 2 variables."),
            "log was: {log}"
        );
    }

    #[test]
    fn input_column_mode() {
        let mut s = session();
        // Colonnes fixes : name = 1-10, age = 11-12.
        run(
            "data out; input name $ 1-10 age 11-12; datalines;\nAlice     14\nBob       16\n;\nrun;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        let name = ds.df.column("name").unwrap().str().unwrap();
        let age = ds.df.column("age").unwrap().f64().unwrap();
        assert_eq!(name.get(0), Some("Alice"));
        assert_eq!(age.get(0), Some(14.0));
        assert_eq!(name.get(1), Some("Bob"));
        assert_eq!(age.get(1), Some(16.0));
    }

    #[test]
    fn input_formatted_informat_decimal() {
        let mut s = session();
        // Informat 5.2 : sans point décimal dans le champ, divise par 100.
        run(
            "data out; input x 5.2; datalines;\n12345\n6.78\n;\nrun;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        let x = ds.df.column("x").unwrap().f64().unwrap();
        // "12345" sans point → 123.45 ; "6.78" avec point → 6.78 (d ignoré).
        assert_eq!(x.get(0), Some(123.45));
        assert_eq!(x.get(1), Some(6.78));
    }

    #[test]
    fn input_char_truncation_at_pdv() {
        let mut s = session();
        // $char4. : la longueur du PDV est 4 → troncature à l'assignation.
        run(
            "data out; input name $char4.; datalines;\nAlexander\n;\nrun;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        let name = ds.df.column("name").unwrap().str().unwrap();
        assert_eq!(name.get(0), Some("Alex"));
    }

    #[test]
    fn input_dsd_consecutive_delimiters_are_missing() {
        let mut s = session();
        run(
            "data out; infile datalines dsd; input a b c; datalines;\n1,,3\n;\nrun;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        let a = ds.df.column("a").unwrap().f64().unwrap();
        let b = ds.df.column("b").unwrap().f64().unwrap();
        let c = ds.df.column("c").unwrap().f64().unwrap();
        assert_eq!(a.get(0), Some(1.0));
        assert_eq!(b.get(0), None); // champ vide → missing
        assert_eq!(c.get(0), Some(3.0));
    }

    #[test]
    fn input_dsd_quoted_field_with_comma() {
        let mut s = session();
        // `$20.` informat → longueur 20 (le défaut liste serait 8 et
        // tronquerait "Smith, John").
        run(
            "data out; infile datalines dsd; input name $20. x; datalines;\n\"Smith, John\",5\n;\nrun;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        let name = ds.df.column("name").unwrap().str().unwrap();
        let x = ds.df.column("x").unwrap().f64().unwrap();
        assert_eq!(name.get(0), Some("Smith, John"));
        assert_eq!(x.get(0), Some(5.0));
    }

    #[test]
    fn input_delimiter_option() {
        let mut s = session();
        run(
            "data out; infile datalines dlm='|'; input a b; datalines;\n10|20\n;\nrun;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(ds.df.column("a").unwrap().f64().unwrap().get(0), Some(10.0));
        assert_eq!(ds.df.column("b").unwrap().f64().unwrap().get(0), Some(20.0));
    }

    #[test]
    fn input_missover_short_record() {
        let mut s = session();
        // MISSOVER : la 2e ligne n'a qu'une valeur → b reste missing.
        run(
            "data out; infile datalines missover; input a b; datalines;\n1 2\n3\n;\nrun;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        let b = ds.df.column("b").unwrap().f64().unwrap();
        assert_eq!(b.get(0), Some(2.0));
        assert_eq!(b.get(1), None);
        assert_eq!(ds.n_obs(), 2);
    }

    #[test]
    fn input_truncover_partial_field() {
        let mut s = session();
        // TRUNCOVER : champ formaté partiel en fin de ligne lu tel quel.
        run(
            "data out; infile datalines truncover; input x 5.; datalines;\n12\n;\nrun;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(ds.df.column("x").unwrap().f64().unwrap().get(0), Some(12.0));
    }

    #[test]
    fn input_stopover_errors() {
        let mut s = session();
        let err = run(
            "data out; infile datalines stopover; input a b c; datalines;\n1 2\n;\nrun;",
            &mut s,
        );
        assert!(err.is_err(), "expected STOPOVER error");
    }

    #[test]
    fn input_double_hold_multiple_obs_per_line() {
        let mut s = session();
        // `@@` : plusieurs observations par ligne.
        run(
            "data out; input x @@; datalines;\n1 2 3 4 5\n;\nrun;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(ds.n_obs(), 5);
        let x = ds.df.column("x").unwrap().f64().unwrap();
        assert_eq!(x.get(0), Some(1.0));
        assert_eq!(x.get(4), Some(5.0));
    }

    #[test]
    fn input_single_hold_then_release() {
        let mut s = session();
        // `@` : maintient l'enregistrement pour un second INPUT de la même
        // itération — ici un seul INPUT lit deux variables avec hold, l'autre
        // est relâché à l'itération suivante.
        run(
            "data out; input a @; input b; datalines;\n1 2\n3 4\n;\nrun;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(ds.n_obs(), 2);
        let a = ds.df.column("a").unwrap().f64().unwrap();
        let b = ds.df.column("b").unwrap().f64().unwrap();
        assert_eq!(a.get(0), Some(1.0));
        assert_eq!(b.get(0), Some(2.0));
        assert_eq!(a.get(1), Some(3.0));
        assert_eq!(b.get(1), Some(4.0));
    }

    #[test]
    fn input_column_pointer_at() {
        let mut s = session();
        run(
            "data out; input @3 x 2.; datalines;\nXX42\n;\nrun;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(ds.df.column("x").unwrap().f64().unwrap().get(0), Some(42.0));
    }

    #[test]
    fn input_firstobs_obs_options() {
        let mut s = session();
        // FIRSTOBS=2, OBS=3 : lignes 2 et 3 seulement.
        run(
            "data out; infile datalines firstobs=2 obs=3; input x; datalines;\n1\n2\n3\n4\n;\nrun;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(ds.n_obs(), 2);
        let x = ds.df.column("x").unwrap().f64().unwrap();
        assert_eq!(x.get(0), Some(2.0));
        assert_eq!(x.get(1), Some(3.0));
    }

    #[test]
    fn input_informat_date9() {
        let mut s = session();
        run(
            "data out; input d date9.; datalines;\n01JAN1960\n02JAN1960\n;\nrun;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        let d = ds.df.column("d").unwrap().f64().unwrap();
        // epoch SAS 1960-01-01 = 0.
        assert_eq!(d.get(0), Some(0.0));
        assert_eq!(d.get(1), Some(1.0));
    }

    #[test]
    fn input_list_modifier_colon_informat() {
        let mut s = session();
        // `:date9.` lit un jeton délimité puis applique l'informat.
        run(
            "data out; infile datalines; input name $ x :date9.; datalines;\nAlice 01JAN1960\n;\nrun;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(ds.df.column("x").unwrap().f64().unwrap().get(0), Some(0.0));
        assert_eq!(
            ds.df.column("name").unwrap().str().unwrap().get(0),
            Some("Alice")
        );
    }

    #[test]
    fn datalines_without_infile_is_implicit_source() {
        let mut s = session();
        // Pas de `infile datalines;` : `input` utilise quand même le bloc.
        run(
            "data out; input x y; datalines;\n1 2\n3 4\n;\nrun;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(ds.n_obs(), 2);
    }

    // ── FILE / PUT (M14.2) ───────────────────────────────────────────────

    /// Extrait les lignes PUT du log (celles qui ne sont ni vides, ni un
    /// écho de source numéroté, ni une NOTE/WARNING/ERROR).
    fn put_log_lines(log: &str) -> Vec<String> {
        // L'écho de source SAS est de la forme "<num>     <texte>" : un nombre
        // suivi d'AU MOINS deux espaces (padding à la colonne 6) puis du texte.
        // Une ligne PUT purement numérique ("42") n'a pas ce padding.
        fn is_source_echo(l: &str) -> bool {
            let mut it = l.char_indices();
            let mut end = 0;
            for (i, c) in it.by_ref() {
                if c.is_ascii_digit() {
                    end = i + 1;
                } else {
                    break;
                }
            }
            if end == 0 {
                return false;
            }
            // Au moins deux espaces après le nombre.
            l[end..].starts_with("  ")
        }
        log.lines()
            .filter(|l| {
                let t = l.trim_start();
                !t.is_empty()
                    && !t.starts_with("NOTE:")
                    && !t.starts_with("WARNING:")
                    && !t.starts_with("ERROR:")
                    && !is_source_echo(l)
                    // Les continuations de NOTE timing ("real time...").
                    && !t.starts_with("real time")
                    && !t.starts_with("cpu time")
            })
            .map(|l| l.to_string())
            .collect()
    }

    #[test]
    fn put_list_mode_to_log() {
        let mut s = session();
        write_class(&s, "inp");
        // `data _null_` : sortie PUT seulement, aucun dataset écrit.
        run("data _null_; set inp; put name age; run;", &mut s).unwrap();
        let log = s.log.into_string();
        let lines = put_log_lines(&log);
        // Age missing (Alice) → "." ; format BEST par défaut.
        assert_eq!(lines, vec!["Alfred 14", "Alice .", "Barbara 13"]);
    }

    #[test]
    fn put_named_form() {
        let mut s = session();
        write_class(&s, "inp");
        run(
            "data _null_; set inp; if name='Alfred'; put name= age=; run;",
            &mut s,
        )
        .unwrap();
        let lines = put_log_lines(&s.log.into_string());
        assert_eq!(lines, vec!["name=Alfred age=14"]);
    }

    #[test]
    fn put_literal_and_var() {
        let mut s = session();
        write_class(&s, "inp");
        run(
            "data _null_; set inp; if name='Alfred'; put 'Report for' name; run;",
            &mut s,
        )
        .unwrap();
        let lines = put_log_lines(&s.log.into_string());
        assert_eq!(lines, vec!["Report for Alfred"]);
    }

    #[test]
    fn put_formatted_numeric() {
        let mut s = session();
        run("data _null_; x = 3.14159; put x 8.2; run;", &mut s).unwrap();
        let lines = put_log_lines(&s.log.into_string());
        // 8.2 → "    3.14" justifié, puis trim de fin (les blancs de tête
        // restent mais sont rognés par render_put_slot via .trim()).
        assert_eq!(lines, vec!["3.14"]);
    }

    #[test]
    fn put_formatted_date9() {
        let mut s = session();
        // 0 = 01JAN1960 (epoch SAS).
        run("data _null_; d = 0; put d date9.; run;", &mut s).unwrap();
        let lines = put_log_lines(&s.log.into_string());
        assert_eq!(lines, vec!["01JAN1960"]);
    }

    #[test]
    fn put_column_pointer_and_skip() {
        let mut s = session();
        run(
            "data _null_; x = 1; y = 2; put @5 x +3 y; run;",
            &mut s,
        )
        .unwrap();
        let lines = put_log_lines(&s.log.into_string());
        // @5 → "1" en colonne 5 (index 4) ; le curseur passe à la colonne 6
        // (index 5), +3 l'avance à la colonne 9 (index 8) où s'écrit "2".
        assert_eq!(lines, vec!["    1    2"]);
    }

    #[test]
    fn put_slash_newline_within_one_put() {
        let mut s = session();
        run(
            "data _null_; x = 1; y = 2; put x / y; run;",
            &mut s,
        )
        .unwrap();
        let lines = put_log_lines(&s.log.into_string());
        assert_eq!(lines, vec!["1", "2"]);
    }

    #[test]
    fn put_single_hold_joins_one_line() {
        let mut s = session();
        write_class(&s, "inp");
        // `put name @;` maintient la ligne ; le PUT suivant (même itération)
        // la continue, puis la relâche.
        run(
            "data _null_; set inp; put name @; put age; run;",
            &mut s,
        )
        .unwrap();
        let lines = put_log_lines(&s.log.into_string());
        // Une ligne par observation (hold simple relâché en fin d'itération).
        assert_eq!(lines, vec!["Alfred 14", "Alice .", "Barbara 13"]);
    }

    #[test]
    fn put_double_hold_joins_across_iterations() {
        let mut s = session();
        write_class(&s, "inp");
        // `put name @@;` maintient la ligne À TRAVERS les itérations : les
        // trois noms s'accumulent sur une seule ligne, relâchée en fin d'étape.
        run("data _null_; set inp; put name @@; run;", &mut s).unwrap();
        let lines = put_log_lines(&s.log.into_string());
        assert_eq!(lines, vec!["Alfred Alice Barbara"]);
    }

    #[test]
    fn put_all_writes_every_pdv_var() {
        let mut s = session();
        write_class(&s, "inp");
        run(
            "data _null_; set inp; if name='Alfred'; put _all_; run;",
            &mut s,
        )
        .unwrap();
        let lines = put_log_lines(&s.log.into_string());
        // Ordre PDV : Age (num) puis Name (char) — l'ordre des colonnes de
        // l'input.
        assert_eq!(lines, vec!["Age=14 Name=Alfred"]);
    }

    #[test]
    fn file_print_routes_to_listing() {
        let mut s = session();
        write_class(&s, "inp");
        run(
            "data _null_; set inp; if name='Alfred'; file print; put 'in listing' name; run;",
            &mut s,
        )
        .unwrap();
        let listing = s.listing.into_string();
        assert!(
            listing.contains("in listing Alfred"),
            "listing was: {listing}"
        );
        // Rien dans le log côté PUT.
        let log = s.log.into_string();
        assert!(!log.contains("in listing"), "log was: {log}");
    }

    #[test]
    fn file_log_explicit_routes_to_log() {
        let mut s = session();
        run(
            "data _null_; x = 7; file log; put 'val' x; run;",
            &mut s,
        )
        .unwrap();
        let lines = put_log_lines(&s.log.into_string());
        assert_eq!(lines, vec!["val 7"]);
    }

    #[test]
    fn file_path_writes_external_file() {
        let mut s = session();
        write_class(&s, "inp");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.txt");
        let path_str = path.to_str().unwrap();
        let src = format!(
            "data _null_; set inp; file '{path_str}'; put name age; run;"
        );
        run(&src, &mut s).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "Alfred 14\nAlice .\nBarbara 13\n");
    }

    #[test]
    fn put_unknown_variable_errors() {
        let mut s = session();
        let res = run("data _null_; x = 1; put nosuchvar; run;", &mut s);
        let err = res.err().expect("expected an error for an unknown PUT variable");
        assert!(
            err.to_string().contains("nosuchvar is not on the PUT statement"),
            "got: {err}"
        );
    }

    #[test]
    fn put_default_destination_is_log() {
        let mut s = session();
        // Sans FILE, un PUT écrit dans le LOG (défaut SAS).
        run("data _null_; x = 42; put x; run;", &mut s).unwrap();
        let lines = put_log_lines(&s.log.into_string());
        assert_eq!(lines, vec!["42"]);
        // Rien dans le listing.
        assert!(!s.listing.into_string().contains("42"));
    }

    // =====================================================================
    // M15.6 — CALL routines
    // =====================================================================

    fn num_col(ds: &SasDataset, name: &str) -> Vec<Option<f64>> {
        let c = ds.df.column(name).unwrap().f64().unwrap();
        (0..ds.n_obs()).map(|i| c.get(i)).collect()
    }
    fn str_col(ds: &SasDataset, name: &str) -> Vec<String> {
        let c = ds.df.column(name).unwrap().str().unwrap();
        (0..ds.n_obs()).map(|i| c.get(i).unwrap_or("").to_string()).collect()
    }

    // ---- CALL MISSING ---------------------------------------------------

    #[test]
    fn call_missing_sets_numeric_to_missing() {
        let mut s = session();
        run(
            "data out; x = 5; y = 10; call missing(x); output; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(num_col(&ds, "x"), vec![None]);
        assert_eq!(num_col(&ds, "y"), vec![Some(10.0)]);
    }

    #[test]
    fn call_missing_sets_char_to_empty() {
        let mut s = session();
        run(
            "data out; length name $10; name = 'Alice'; call missing(name); output; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(str_col(&ds, "name"), vec![String::new()]);
    }

    #[test]
    fn call_missing_multiple_vars_mixed_types() {
        let mut s = session();
        run(
            "data out; length c $5; a = 1; b = 2; c = 'hi'; call missing(a, b, c); output; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(num_col(&ds, "a"), vec![None]);
        assert_eq!(num_col(&ds, "b"), vec![None]);
        assert_eq!(str_col(&ds, "c"), vec![String::new()]);
    }

    // ---- CALL EXECUTE ---------------------------------------------------

    #[test]
    fn call_execute_queues_literal_code() {
        let mut s = session();
        run(
            "data _null_; call execute('data q; v = 7; run;'); run;",
            &mut s,
        )
        .unwrap();
        // L'étape elle-même ne fait que mettre en file (rejeu = exécuteur).
        assert_eq!(
            s.call_execute_queue,
            vec!["data q; v = 7; run;".to_string()]
        );
    }

    #[test]
    fn call_execute_queues_per_row_in_order() {
        let mut s = session();
        write_class(&s, "inp");
        run(
            "data _null_; set inp; call execute('proc print; '||name); run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(s.call_execute_queue.len(), 3);
        assert!(s.call_execute_queue[0].contains("Alfred"));
        assert!(s.call_execute_queue[2].contains("Barbara"));
    }

    #[test]
    fn call_execute_requires_one_argument() {
        let mut s = session();
        let res = run("data _null_; call execute('a', 'b'); run;", &mut s);
        assert!(res.is_err());
    }

    // ---- CALL SORTN / SORTC --------------------------------------------

    #[test]
    fn call_sortn_sorts_array_ascending() {
        let mut s = session();
        run(
            "data out; array a{3} a1-a3; a1=3; a2=1; a3=2; call sortn(a); output; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(num_col(&ds, "a1"), vec![Some(1.0)]);
        assert_eq!(num_col(&ds, "a2"), vec![Some(2.0)]);
        assert_eq!(num_col(&ds, "a3"), vec![Some(3.0)]);
    }

    #[test]
    fn call_sortn_missing_sorts_first() {
        let mut s = session();
        // SAS collation: missing (.) is smaller than any number.
        run(
            "data out; array a{3} a1-a3; a1=5; a2=.; a3=1; call sortn(a); output; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(num_col(&ds, "a1"), vec![None]);
        assert_eq!(num_col(&ds, "a2"), vec![Some(1.0)]);
        assert_eq!(num_col(&ds, "a3"), vec![Some(5.0)]);
    }

    #[test]
    fn call_sortc_sorts_char_array_ascending() {
        let mut s = session();
        run(
            "data out; array c{3} $5 c1-c3; c1='pear'; c2='apple'; c3='kiwi'; \
             call sortc(c); output; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(str_col(&ds, "c1"), vec!["apple".to_string()]);
        assert_eq!(str_col(&ds, "c2"), vec!["kiwi".to_string()]);
        assert_eq!(str_col(&ds, "c3"), vec!["pear".to_string()]);
    }

    #[test]
    fn call_sortn_explicit_var_list() {
        let mut s = session();
        run(
            "data out; x=9; y=2; z=5; call sortn(x, y, z); output; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(num_col(&ds, "x"), vec![Some(2.0)]);
        assert_eq!(num_col(&ds, "y"), vec![Some(5.0)]);
        assert_eq!(num_col(&ds, "z"), vec![Some(9.0)]);
    }

    // ---- CALL SYMPUTX ---------------------------------------------------

    #[test]
    fn call_symputx_trims_value() {
        let mut s = session();
        run(
            "data _null_; length v $20; v = '   hi   '; call symputx('a', v); run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(s.macro_engine.get_symbol("a").as_deref(), Some("hi"));
    }

    #[test]
    fn call_symputx_numeric_no_blanks() {
        let mut s = session();
        run("data _null_; call symputx('n', 42); run;", &mut s).unwrap();
        assert_eq!(s.macro_engine.get_symbol("n").as_deref(), Some("42"));
    }

    #[test]
    fn call_symput_vs_symputx_value_trimming() {
        // SYMPUT keeps leading blanks of a char value; SYMPUTX trims them.
        let mut s = session();
        run(
            "data _null_; length v $10; v = '  x'; call symput('a', v); call symputx('b', v); run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(s.macro_engine.get_symbol("a").as_deref(), Some("  x"));
        assert_eq!(s.macro_engine.get_symbol("b").as_deref(), Some("x"));
    }

    // ---- CALL CATS ------------------------------------------------------

    #[test]
    fn call_cats_concatenates_stripped() {
        let mut s = session();
        run(
            "data out; length r $20; a='  foo '; b=' bar'; call cats(r, a, b); output; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(str_col(&ds, "r"), vec!["foobar".to_string()]);
    }

    #[test]
    fn call_cats_mixed_num_and_char() {
        let mut s = session();
        run(
            "data out; length r $20; call cats(r, 'x', 12, 'y'); output; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(str_col(&ds, "r"), vec!["x12y".to_string()]);
    }

    #[test]
    fn call_cats_truncates_to_result_length() {
        let mut s = session();
        run(
            "data out; length r $3; call cats(r, 'abcdef'); output; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(str_col(&ds, "r"), vec!["abc".to_string()]);
    }

    // ---- CALL SCAN ------------------------------------------------------

    #[test]
    fn call_scan_extracts_nth_word() {
        let mut s = session();
        run(
            "data out; length w $10; call scan('alpha beta gamma', 2, w); output; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(str_col(&ds, "w"), vec!["beta".to_string()]);
    }

    #[test]
    fn call_scan_negative_index_from_end() {
        let mut s = session();
        run(
            "data out; length w $10; call scan('a b c', -1, w); output; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(str_col(&ds, "w"), vec!["c".to_string()]);
    }

    #[test]
    fn call_scan_custom_delimiter() {
        let mut s = session();
        run(
            "data out; length w $10; call scan('a,b,c', 2, w, ','); output; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(str_col(&ds, "w"), vec!["b".to_string()]);
    }

    // ---- CALL LABEL -----------------------------------------------------

    #[test]
    fn call_label_returns_label() {
        let mut s = session();
        run(
            "data out; length lbl $40; x = 1; label x = 'My X Variable'; \
             call label(x, lbl); output; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(str_col(&ds, "lbl"), vec!["My X Variable".to_string()]);
    }

    #[test]
    fn call_label_falls_back_to_name_when_no_label() {
        let mut s = session();
        run(
            "data out; length lbl $40; weight = 1; call label(weight, lbl); output; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        // No label declared → SAS returns the variable name.
        assert_eq!(str_col(&ds, "lbl"), vec!["weight".to_string()]);
    }

    // ---- CALL VNAME -----------------------------------------------------

    #[test]
    fn call_vname_returns_variable_name() {
        let mut s = session();
        run(
            "data out; length nm $32; Height = 1; call vname(Height, nm); output; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        // Name preserved with first-reference casing.
        assert_eq!(str_col(&ds, "nm"), vec!["Height".to_string()]);
    }

    #[test]
    fn call_vname_on_array_element() {
        let mut s = session();
        run(
            "data out; length nm $32; array a{3} a1-a3; call vname(a{2}, nm); output; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(str_col(&ds, "nm"), vec!["a2".to_string()]);
    }

    #[test]
    fn unknown_call_routine_errors() {
        let mut s = session();
        let res = run("data _null_; call frobnicate(1); run;", &mut s);
        let err = res.err().expect("expected error for unknown CALL routine");
        assert!(err.to_string().contains("not yet implemented"), "got: {err}");
    }

    // ── SELECT / WHEN / OTHERWISE (M16.1) ────────────────────────────────

    #[test]
    fn select_selector_form_matches_first_value() {
        // Sélecteur numérique : age 14 → "teen", . → autre, 13 → "kid".
        let mut s = session();
        write_class(&s, "inp");
        run(
            "data out; set inp; length grp $8; \
             select (age); \
               when (13) grp='kid'; \
               when (14, 15) grp='teen'; \
               otherwise grp='other'; \
             end; \
             run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        // age = 14, missing, 13.
        assert_eq!(
            str_col(&ds, "grp"),
            vec!["teen".to_string(), "other".to_string(), "kid".to_string()]
        );
    }

    #[test]
    fn select_selector_multiple_values_in_one_when() {
        // Une seule clause liste plusieurs valeurs ; n'importe laquelle suffit.
        let mut s = session();
        run(
            "data out; \
             do x = 1 to 4; \
               select (x); \
                 when (1, 3) flag = 1; \
                 otherwise flag = 0; \
               end; \
               output; \
             end; \
             run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "flag", 0), Some(1.0)); // x=1
        assert_eq!(num_at(&s, "out", "flag", 1), Some(0.0)); // x=2
        assert_eq!(num_at(&s, "out", "flag", 2), Some(1.0)); // x=3
        assert_eq!(num_at(&s, "out", "flag", 3), Some(0.0)); // x=4
    }

    #[test]
    fn select_selector_char_form() {
        // Sélecteur caractère ; comparaison ignore les blancs finaux (sas_cmp).
        let mut s = session();
        run(
            "data out; length sex $1 desc $8; \
             sex = 'F'; \
             select (sex); \
               when ('M') desc = 'male'; \
               when ('F') desc = 'female'; \
               otherwise desc = 'unknown'; \
             end; \
             output; \
             run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(str_col(&ds, "desc"), vec!["female".to_string()]);
    }

    #[test]
    fn select_boolean_form_first_true_wins() {
        // Forme booléenne : conditions évaluées dans l'ordre, première vraie.
        let mut s = session();
        run(
            "data out; length band $8; \
             do x = 5 to 25 by 10; \
               select; \
                 when (x < 10) band = 'low'; \
                 when (x < 20) band = 'mid'; \
                 otherwise band = 'high'; \
               end; \
               output; \
             end; \
             run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        // x = 5, 15, 25.
        assert_eq!(
            str_col(&ds, "band"),
            vec!["low".to_string(), "mid".to_string(), "high".to_string()]
        );
    }

    #[test]
    fn select_boolean_form_range_condition() {
        // Plage exprimée par une condition booléenne 1 <= x <= 10.
        let mut s = session();
        run(
            "data out; length r $8; \
             do x = 0 to 15 by 5; \
               select; \
                 when (x >= 1 and x <= 10) r = 'in'; \
                 otherwise r = 'out'; \
               end; \
               output; \
             end; \
             run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        // x = 0(out), 5(in), 10(in), 15(out).
        assert_eq!(
            str_col(&ds, "r"),
            vec![
                "out".to_string(),
                "in".to_string(),
                "in".to_string(),
                "out".to_string()
            ]
        );
    }

    #[test]
    fn select_when_do_block_runs_all_statements() {
        // Le corps d'un WHEN peut être un do; ... end; (plusieurs statements).
        let mut s = session();
        run(
            "data out; x = 2; \
             select (x); \
               when (2) do; a = 10; b = 20; end; \
               otherwise do; a = 0; b = 0; end; \
             end; \
             output; \
             run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "a", 0), Some(10.0));
        assert_eq!(num_at(&s, "out", "b", 0), Some(20.0));
    }

    #[test]
    fn select_no_fall_through() {
        // Pas de fall-through : seule la PREMIÈRE clause vraie s'exécute,
        // même si une clause suivante correspondrait aussi.
        let mut s = session();
        run(
            "data out; x = 1; n = 0; \
             select (x); \
               when (1) n = n + 1; \
               when (1) n = n + 100; \
               otherwise n = n + 1000; \
             end; \
             output; \
             run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "n", 0), Some(1.0));
    }

    #[test]
    fn select_missing_value_matches_dot() {
        // `. = .` est vrai en SAS : un WHEN (.) capture le sélecteur missing.
        let mut s = session();
        write_class(&s, "inp");
        run(
            "data out; set inp; length tag $8; \
             select (age); \
               when (.) tag = 'na'; \
               otherwise tag = 'ok'; \
             end; \
             run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        // age = 14, missing, 13.
        assert_eq!(
            str_col(&ds, "tag"),
            vec!["ok".to_string(), "na".to_string(), "ok".to_string()]
        );
    }

    #[test]
    fn select_no_otherwise_no_match_is_runtime_error() {
        // Sans OTHERWISE et sans WHEN correspondant : erreur runtime (SAS).
        let mut s = session();
        let err = run(
            "data out; x = 99; select (x); when (1) y = 1; end; run;",
            &mut s,
        )
        .err()
        .unwrap();
        assert!(
            err.to_string().contains("does not match any clause"),
            "got: {err}"
        );
    }

    #[test]
    fn select_no_otherwise_with_match_is_ok() {
        // Sans OTHERWISE mais avec un WHEN correspondant : pas d'erreur.
        let mut s = session();
        run(
            "data out; x = 1; select (x); when (1) y = 7; end; output; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "y", 0), Some(7.0));
    }

    #[test]
    fn select_empty_when_body_is_noop() {
        // `when (1) ;` corps vide : la clause est prise mais ne fait rien
        // (pas de fall-through vers OTHERWISE).
        let mut s = session();
        run(
            "data out; x = 1; y = 5; \
             select (x); \
               when (1) ; \
               otherwise y = 0; \
             end; \
             output; \
             run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "y", 0), Some(5.0));
    }

    #[test]
    fn select_selector_evaluated_once_via_subsetting() {
        // Le sélecteur est une expression : 2*x. x=3 → 6 → "six".
        let mut s = session();
        run(
            "data out; length w $8; x = 3; \
             select (2 * x); \
               when (6) w = 'six'; \
               otherwise w = 'no'; \
             end; \
             output; \
             run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(str_col(&ds, "w"), vec!["six".to_string()]);
    }

    // ── M16.3 : DO sur liste de valeurs, DO OVER, RETAIN littéraux date ───

    #[test]
    fn do_list_numeric_explicit_values() {
        // `do i = 1, 3, 5, 7;` — somme et dernière valeur.
        let mut s = session();
        run(
            "data out; s = 0; do i = 1, 3, 5, 7; s = s + i; end; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "s", 0), Some(16.0));
        // À la sortie d'une liste, l'index garde la DERNIÈRE valeur (≠ TO).
        assert_eq!(num_at(&s, "out", "i", 0), Some(7.0));
    }

    #[test]
    fn do_list_unordered_values() {
        // Ordre quelconque honoré tel quel : 5, 1, 9.
        let mut s = session();
        run(
            "data out; n = 0; do i = 5, 1, 9; n + 1; last = i; end; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "n", 0), Some(3.0));
        assert_eq!(num_at(&s, "out", "last", 0), Some(9.0));
    }

    #[test]
    fn do_list_single_value() {
        // `do i = 42;` — liste à un élément (boucle une fois).
        let mut s = session();
        run("data out; c = 0; do i = 42; c + 1; end; run;", &mut s).unwrap();
        assert_eq!(num_at(&s, "out", "c", 0), Some(1.0));
        assert_eq!(num_at(&s, "out", "i", 0), Some(42.0));
    }

    #[test]
    fn do_list_character_values() {
        // `do color = 'red', 'blue', 'green';` — char.
        let mut s = session();
        run(
            "data out; length color $5; n = 0; \
             do color = 'red', 'blue', 'green'; n + 1; end; run;",
            &mut s,
        )
        .unwrap();
        // Dernière valeur conservée ; n = 3.
        assert_eq!(num_at(&s, "out", "n", 0), Some(3.0));
        let ds = read_work(&s, "out");
        assert_eq!(str_col(&ds, "color"), vec!["green".to_string()]);
    }

    #[test]
    fn do_list_mixed_range_and_explicit() {
        // `do i = 1 to 5 by 2, 10, 20 to 22;` → 1,3,5,10,20,21,22 (7 valeurs).
        let mut s = session();
        run(
            "data out; n = 0; s = 0; \
             do i = 1 to 5 by 2, 10, 20 to 22; n + 1; s = s + i; end; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "n", 0), Some(7.0));
        // 1+3+5+10+20+21+22 = 82.
        assert_eq!(num_at(&s, "out", "s", 0), Some(82.0));
        // Dernière valeur = 22.
        assert_eq!(num_at(&s, "out", "i", 0), Some(22.0));
    }

    #[test]
    fn do_list_range_first_then_values() {
        // `1 to 12 by 2, 0` : c'est une LISTE (à cause de la virgule) → le
        // range énumère 1,3,5,7,9,11 puis la valeur 0 ; 7 tours, index final 0.
        let mut s = session();
        run(
            "data out; n = 0; do month = 1 to 12 by 2, 0; n + 1; end; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "n", 0), Some(7.0));
        // En LISTE, l'index garde la dernière valeur (≠ TO classique).
        assert_eq!(num_at(&s, "out", "month", 0), Some(0.0));
    }

    #[test]
    fn do_over_1d_iterates_all_elements() {
        // DO OVER 1-D : `arr` nu = élément courant ; on double chaque élément.
        let mut s = session();
        run(
            "data out; array a{5} v1-v5; \
             do i = 1 to 5; a{i} = i; end; \
             do over a; a = a * 10; end; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "v1", 0), Some(10.0));
        assert_eq!(num_at(&s, "out", "v2", 0), Some(20.0));
        assert_eq!(num_at(&s, "out", "v3", 0), Some(30.0));
        assert_eq!(num_at(&s, "out", "v4", 0), Some(40.0));
        assert_eq!(num_at(&s, "out", "v5", 0), Some(50.0));
    }

    #[test]
    fn do_over_1d_reads_current_element_into_accumulator() {
        // `arr` en lecture nue dans une accumulation.
        let mut s = session();
        run(
            "data out; array a{4} v1-v4 (3 6 9 12); \
             tot = 0; do over a; tot = tot + a; end; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "tot", 0), Some(30.0));
    }

    #[test]
    fn do_over_static_indexed_access_inside_loop() {
        // Accès indexé `a{1}` reste STATIQUE même dans DO OVER : on lit le
        // premier élément à chaque tour.
        let mut s = session();
        run(
            "data out; array a{3} v1-v3 (5 6 7); \
             firstsum = 0; do over a; firstsum = firstsum + a{1}; end; run;",
            &mut s,
        )
        .unwrap();
        // a{1}=5 lu 3 fois → 15.
        assert_eq!(num_at(&s, "out", "firstsum", 0), Some(15.0));
    }

    #[test]
    fn do_over_multidim_row_major_order() {
        // DO OVER sur un array 2×3 : itération row-major (= ordre des slots).
        // On affecte des valeurs croissantes par tour pour vérifier l'ordre.
        let mut s = session();
        run(
            "data out; array m{2,3} v1-v6; \
             k = 0; do over m; k + 1; m = k; end; run;",
            &mut s,
        )
        .unwrap();
        // Row-major : v1=1, v2=2, ..., v6=6.
        assert_eq!(num_at(&s, "out", "v1", 0), Some(1.0));
        assert_eq!(num_at(&s, "out", "v2", 0), Some(2.0));
        assert_eq!(num_at(&s, "out", "v3", 0), Some(3.0));
        assert_eq!(num_at(&s, "out", "v4", 0), Some(4.0));
        assert_eq!(num_at(&s, "out", "v5", 0), Some(5.0));
        assert_eq!(num_at(&s, "out", "v6", 0), Some(6.0));
    }

    #[test]
    fn do_over_char_array() {
        // DO OVER sur array caractère : uppercase de chaque élément.
        let mut s = session();
        run(
            "data out; array c{3} $3 a b cc; \
             a = 'foo'; b = 'bar'; cc = 'baz'; \
             do over c; c = upcase(c); end; run;",
            &mut s,
        )
        .unwrap();
        let ds = read_work(&s, "out");
        assert_eq!(str_col(&ds, "a"), vec!["FOO".to_string()]);
        assert_eq!(str_col(&ds, "b"), vec!["BAR".to_string()]);
        assert_eq!(str_col(&ds, "cc"), vec!["BAZ".to_string()]);
    }

    #[test]
    fn retain_date_literal_bare_suffix() {
        // `retain d 21710d;` — 21710 est la valeur SAS date (2019-06-14).
        let mut s = session();
        run("data out; retain d 21710d; run;", &mut s).unwrap();
        assert_eq!(num_at(&s, "out", "d", 0), Some(21710.0));
    }

    #[test]
    fn retain_date_literal_quoted() {
        // `retain d '01JAN1960'd;` — l'époque SAS = 0.
        let mut s = session();
        run("data out; retain d '01JAN1960'd; run;", &mut s).unwrap();
        assert_eq!(num_at(&s, "out", "d", 0), Some(0.0));
        // '02JAN1960'd = 1.
        let mut s2 = session();
        run("data out; retain e '02JAN1960'd; run;", &mut s2).unwrap();
        assert_eq!(num_at(&s2, "out", "e", 0), Some(1.0));
    }

    #[test]
    fn retain_datetime_literal() {
        // `retain dt '01JAN1960 00:00:00'dt;` = 0 secondes depuis l'époque.
        let mut s = session();
        run(
            "data out; retain dt '01JAN1960 00:00:00'dt; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "dt", 0), Some(0.0));
        // '01JAN1960 00:01:00'dt = 60 secondes.
        let mut s2 = session();
        run(
            "data out; retain dt '01JAN1960 00:01:00'dt; run;",
            &mut s2,
        )
        .unwrap();
        assert_eq!(num_at(&s2, "out", "dt", 0), Some(60.0));
    }

    #[test]
    fn retain_date_literal_is_retained_across_iterations() {
        // La valeur initiale issue d'un littéral date est bien RETENUE :
        // on l'incrémente à chaque obs lue.
        let mut s = session();
        write_class(&s, "inp");
        run(
            "data out; set inp; retain d 100d; d = d + 1; run;",
            &mut s,
        )
        .unwrap();
        // 100 (initial) +1 par obs : 101, 102, 103.
        assert_eq!(num_at(&s, "out", "d", 0), Some(101.0));
        assert_eq!(num_at(&s, "out", "d", 1), Some(102.0));
        assert_eq!(num_at(&s, "out", "d", 2), Some(103.0));
    }

    #[test]
    fn do_over_then_index_value_independent() {
        // Intégration M16.2 : DO OVER puis accès indexé hors boucle.
        let mut s = session();
        run(
            "data out; array a{3} x y z (1 2 3); \
             do over a; a = a + 100; end; \
             p = a{2}; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(num_at(&s, "out", "x", 0), Some(101.0));
        assert_eq!(num_at(&s, "out", "y", 0), Some(102.0));
        assert_eq!(num_at(&s, "out", "z", 0), Some(103.0));
        assert_eq!(num_at(&s, "out", "p", 0), Some(102.0));
    }

    // ── M16.4 : SET options END= / NOBS= / POINT= + multi-datasets ────────

    fn run_err(src: &str, session: &mut Session) -> String {
        let file = SourceFile::new(src);
        let mut ts = StatementStream::new(&file).unwrap();
        assert!(ts.next().is_kw("data"));
        let ast = crate::parser::datastep::parse_data_step(&mut ts).unwrap();
        match compile(&ast, session).and_then(|p| execute(p, session)) {
            Ok(_) => panic!("expected an error, got Ok"),
            Err(e) => e.to_string(),
        }
    }

    /// SET de 3 datasets : concaténation dans l'ordre, comptes par dataset.
    #[test]
    fn set_three_datasets_concatenates_in_order() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0]))]);
        write_num_ds(&s, "b", &[("x", some(&[3.0]))]);
        write_num_ds(&s, "c", &[("x", some(&[4.0, 5.0]))]);
        let stats = run("data out; set a b c; run;", &mut s).unwrap();
        assert_eq!(col(&s, "out", "x"), some(&[1.0, 2.0, 3.0, 4.0, 5.0]));
        assert_eq!(
            stats.read,
            vec![
                ("WORK.A".to_string(), 2),
                ("WORK.B".to_string(), 1),
                ("WORK.C".to_string(), 2),
            ]
        );
    }

    /// END= sur un seul dataset : 0 sauf la dernière obs.
    #[test]
    fn end_option_single_dataset() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[10.0, 20.0, 30.0]))]);
        run("data out; set a end=eof; flag = eof; run;", &mut s).unwrap();
        assert_eq!(col(&s, "out", "flag"), some(&[0.0, 0.0, 1.0]));
        // eof n'est PAS écrite en sortie (variable automatique).
        assert!(read_work(&s, "out").df.column("eof").is_err());
    }

    /// END= permet une logique « dernière observation » (totaux).
    #[test]
    fn end_option_drives_last_obs_logic() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0, 3.0, 4.0]))]);
        run(
            "data out; set a end=eof; retain total 0; total + x; \
             if eof then output; run;",
            &mut s,
        )
        .unwrap();
        // Une seule obs sortie : le total final.
        assert_eq!(read_work(&s, "out").n_obs(), 1);
        assert_eq!(num_at(&s, "out", "total", 0), Some(10.0));
    }

    /// END= avec plusieurs datasets : 1 seulement après la dernière obs du
    /// DERNIER dataset.
    #[test]
    fn end_option_multiple_datasets() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0]))]);
        write_num_ds(&s, "b", &[("x", some(&[3.0]))]);
        run("data out; set a b end=eof; flag = eof; run;", &mut s).unwrap();
        assert_eq!(col(&s, "out", "flag"), some(&[0.0, 0.0, 1.0]));
    }

    /// END= avec WHERE= : la dernière obs RETENUE porte eof=1.
    #[test]
    fn end_option_with_where() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0, 3.0, 4.0]))]);
        run(
            "data out; set a(where=(x <= 2)) end=eof; flag = eof; run;",
            &mut s,
        )
        .unwrap();
        // Seules x=1 et x=2 passent ; eof=1 sur x=2.
        assert_eq!(col(&s, "out", "x"), some(&[1.0, 2.0]));
        assert_eq!(col(&s, "out", "flag"), some(&[0.0, 1.0]));
    }

    /// END= avec BY (interclassement) : 1 sur la toute dernière obs servie.
    #[test]
    fn end_option_with_by() {
        let mut s = session();
        write_num_ds(&s, "a", &[("k", some(&[1.0, 3.0]))]);
        write_num_ds(&s, "b", &[("k", some(&[2.0, 4.0]))]);
        run(
            "data out; set a b end=eof; by k; flag = eof; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(col(&s, "out", "k"), some(&[1.0, 2.0, 3.0, 4.0]));
        assert_eq!(col(&s, "out", "flag"), some(&[0.0, 0.0, 0.0, 1.0]));
    }

    /// NOBS= : disponible AVANT la boucle (somme d'observations).
    #[test]
    fn nobs_option_available_before_loop() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[10.0, 20.0, 30.0]))]);
        run("data out; set a nobs=n; cnt = n; run;", &mut s).unwrap();
        // n est constant = 3 pour chaque obs.
        assert_eq!(col(&s, "out", "cnt"), some(&[3.0, 3.0, 3.0]));
        assert_eq!(col(&s, "out", "n"), some(&[3.0, 3.0, 3.0]));
    }

    /// NOBS= total sur plusieurs datasets.
    #[test]
    fn nobs_option_total_across_datasets() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0]))]);
        write_num_ds(&s, "b", &[("x", some(&[3.0, 4.0, 5.0]))]);
        run("data out; set a b nobs=n; cnt = n; run;", &mut s).unwrap();
        assert_eq!(col(&s, "out", "cnt"), some(&[5.0, 5.0, 5.0, 5.0, 5.0]));
    }

    /// NOBS= utilisable pour une initialisation AVANT toute lecture (le test
    /// le plus parlant : un `_N_ = 1` avec `if _n_ = 1` initialise un tableau
    /// dimensionné par n). Ici, on vérifie juste l'accès dès la 1re itération.
    #[test]
    fn nobs_usable_for_initialization() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[7.0, 8.0]))]);
        run(
            "data out; set a nobs=n; if _n_ = 1 then half = n / 2; \
             retain half; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(col(&s, "out", "half"), some(&[1.0, 1.0]));
    }

    /// POINT= : accès direct via une boucle DO 1..NOBS + OUTPUT explicite.
    #[test]
    fn point_option_direct_access_loop() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0, 33.0]))]);
        run(
            "data out; do i = 1 to n; set a point=i nobs=n; output; end; stop; run;",
            &mut s,
        )
        .unwrap();
        // Toutes les obs, dans l'ordre de l'index.
        assert_eq!(col(&s, "out", "x"), some(&[11.0, 22.0, 33.0]));
    }

    /// POINT= : lecture inverse (index décroissant).
    #[test]
    fn point_option_reverse_order() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0, 33.0]))]);
        run(
            "data out; do i = n to 1 by -1; set a point=i nobs=n; output; end; stop; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(col(&s, "out", "x"), some(&[33.0, 22.0, 11.0]));
    }

    /// POINT= : accès à UNE obs précise (1-based).
    #[test]
    fn point_option_single_index() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0, 33.0]))]);
        run(
            "data out; p = 2; set a point=p; output; stop; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(read_work(&s, "out").n_obs(), 1);
        assert_eq!(num_at(&s, "out", "x", 0), Some(22.0));
    }

    /// POINT= désactive l'output implicite : sans OUTPUT, rien n'est écrit.
    #[test]
    fn point_option_disables_implicit_output() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0]))]);
        // OUTPUT absent → 0 obs écrite (et STOP évite la boucle infinie).
        run("data out; p = 1; set a point=p; stop; run;", &mut s).unwrap();
        assert_eq!(read_work(&s, "out").n_obs(), 0);
    }

    /// POINT= index missing → erreur runtime « Error in variable ».
    #[test]
    fn point_option_missing_index_errors() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0]))]);
        // p jamais affecté → missing.
        let e = run_err("data out; set a point=p; output; stop; run;", &mut s);
        assert!(e.contains("Error in variable"), "got: {e}");
    }

    /// POINT= index hors bornes (0) → erreur runtime.
    #[test]
    fn point_option_zero_index_errors() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0]))]);
        let e = run_err("data out; p = 0; set a point=p; output; stop; run;", &mut s);
        assert!(e.contains("Error in variable"), "got: {e}");
    }

    /// POINT= index trop grand → erreur runtime.
    #[test]
    fn point_option_out_of_bounds_errors() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0]))]);
        let e = run_err("data out; p = 99; set a point=p; output; stop; run;", &mut s);
        assert!(e.contains("Error in variable"), "got: {e}");
    }

    /// POINT= avec plusieurs datasets : index GLOBAL sur la concaténation.
    #[test]
    fn point_option_multiple_datasets_global_index() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0]))]);
        write_num_ds(&s, "b", &[("x", some(&[33.0, 44.0]))]);
        run(
            "data out; do i = 1 to n; set a b point=i nobs=n; output; end; stop; run;",
            &mut s,
        )
        .unwrap();
        // n = 4 (total), index 1..4 parcourt a puis b.
        assert_eq!(col(&s, "out", "x"), some(&[11.0, 22.0, 33.0, 44.0]));
    }

    /// POINT= incompatible avec BY → erreur de compilation/exécution.
    #[test]
    fn point_option_with_by_errors() {
        let mut s = session();
        write_num_ds(&s, "a", &[("k", some(&[1.0, 2.0]))]);
        let e = run_err(
            "data out; set a point=p; by k; output; stop; run;",
            &mut s,
        );
        assert!(e.contains("POINT="), "got: {e}");
    }

    /// POINT= + END= : eof=1 quand l'index pointe la dernière obs.
    #[test]
    fn point_option_with_end() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0, 33.0]))]);
        run(
            "data out; do i = 1 to n; set a point=i nobs=n end=eof; \
             flag = eof; output; end; stop; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(col(&s, "out", "flag"), some(&[0.0, 0.0, 1.0]));
    }

    /// POINT= : re-lecture de la même obs (contrôle d'itération manuel).
    #[test]
    fn point_option_reread_same_obs() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0, 33.0]))]);
        run(
            "data out; do i = 1, 1, 3; set a point=i; output; end; stop; run;",
            &mut s,
        )
        .unwrap();
        // Index 1, 1, 3 → re-lecture autorisée.
        assert_eq!(col(&s, "out", "x"), some(&[11.0, 11.0, 33.0]));
    }

    /// Plusieurs SET *statements* restent refusés (hors périmètre M16.4).
    #[test]
    fn multiple_set_statements_still_error() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[1.0]))]);
        write_num_ds(&s, "b", &[("x", some(&[2.0]))]);
        let e = run_err("data out; set a; set b; run;", &mut s);
        assert!(e.contains("Multiple SET statements"), "got: {e}");
    }

    /// END=/NOBS= combinés : compteur de fin + total.
    #[test]
    fn end_and_nobs_combined() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[5.0, 6.0, 7.0]))]);
        run(
            "data out; set a end=eof nobs=n; \
             if eof then last_total = n; retain last_total; run;",
            &mut s,
        )
        .unwrap();
        // n est connu partout ; last_total posé sur eof.
        assert_eq!(num_at(&s, "out", "last_total", 2), Some(3.0));
    }

    // ── M16.5 : UPDATE / MODIFY ──────────────────────────────────────────

    /// Écrit un dataset avec une colonne char `key` et des colonnes num.
    /// `keys` = valeurs de la clé char ; `cols` = (nom, valeurs num).
    fn write_keyed_ds(
        session: &Session,
        table: &str,
        key: &str,
        keys: &[&str],
        cols: &[(&str, Vec<Option<f64>>)],
    ) {
        let mut columns: Vec<Column> = Vec::new();
        let mut vars: Vec<VarMeta> = Vec::new();
        columns.push(Series::new(key.into(), keys.to_vec()).into());
        vars.push(VarMeta {
            name: key.to_string(),
            ty: VarType::Char,
            length: 8,
            format: None,
            label: None,
        });
        for (name, vals) in cols {
            columns.push(Series::new((*name).into(), vals.clone()).into());
            vars.push(VarMeta {
                name: (*name).to_string(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            });
        }
        let df = DataFrame::new(columns).unwrap();
        session
            .libs
            .get("WORK")
            .unwrap()
            .write(table, &SasDataset { df, vars })
            .unwrap();
    }

    // ----- UPDATE -----

    /// UPDATE de base : transaction superpose le maître par clé (match).
    #[test]
    fn update_basic_overlay() {
        let mut s = session();
        // maître : id=1,2,3 ; x=10,20,30
        write_num_ds(&s, "mas", &[("id", some(&[1.0, 2.0, 3.0])), ("x", some(&[10.0, 20.0, 30.0]))]);
        // transaction : id=2 ; x=99
        write_num_ds(&s, "tra", &[("id", some(&[2.0])), ("x", some(&[99.0]))]);
        let stats = run("data mas; update mas tra key=id; run;", &mut s).unwrap();
        assert_eq!(col(&s, "mas", "id"), some(&[1.0, 2.0, 3.0]));
        // id=2 mis à jour à 99 ; les autres inchangés.
        assert_eq!(col(&s, "mas", "x"), some(&[10.0, 99.0, 30.0]));
        assert_eq!(stats.written, vec![("WORK.MAS".to_string(), 3, 2)]);
        // Deux NOTEs de lecture (maître + transaction).
        assert_eq!(
            stats.read,
            vec![("WORK.MAS".to_string(), 3), ("WORK.TRA".to_string(), 1)]
        );
    }

    /// UPDATE : une clé maître sans transaction correspondante reste inchangée.
    #[test]
    fn update_no_match_unchanged() {
        let mut s = session();
        write_num_ds(&s, "mas", &[("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))]);
        write_num_ds(&s, "tra", &[("id", some(&[9.0])), ("x", some(&[99.0]))]);
        run("data mas; update mas tra key=id; run;", &mut s).unwrap();
        assert_eq!(col(&s, "mas", "x"), some(&[10.0, 20.0]));
    }

    /// UPDATE : une valeur transaction MANQUANTE ne superpose pas (no-update).
    #[test]
    fn update_missing_transaction_skips_overlay() {
        let mut s = session();
        write_num_ds(
            &s,
            "mas",
            &[("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0])), ("y", some(&[1.0, 2.0]))],
        );
        // transaction id=1 : x=. (manquant → pas de MAJ), y=77 (MAJ).
        write_num_ds(
            &s,
            "tra",
            &[("id", some(&[1.0])), ("x", vec![None]), ("y", some(&[77.0]))],
        );
        run("data mas; update mas tra key=id; run;", &mut s).unwrap();
        // x inchangé (transaction manquante) ; y mis à jour.
        assert_eq!(col(&s, "mas", "x"), some(&[10.0, 20.0]));
        assert_eq!(col(&s, "mas", "y"), some(&[77.0, 2.0]));
    }

    /// UPDATE : la variable clé n'est jamais écrasée par la transaction.
    #[test]
    fn update_key_not_overwritten() {
        let mut s = session();
        write_num_ds(&s, "mas", &[("id", some(&[5.0])), ("x", some(&[1.0]))]);
        // La transaction porte la même clé 5 ; x=42.
        write_num_ds(&s, "tra", &[("id", some(&[5.0])), ("x", some(&[42.0]))]);
        run("data mas; update mas tra key=id; run;", &mut s).unwrap();
        assert_eq!(col(&s, "mas", "id"), some(&[5.0]));
        assert_eq!(col(&s, "mas", "x"), some(&[42.0]));
    }

    /// UPDATE : plusieurs transactions pour une clé → seule la PREMIÈRE compte.
    #[test]
    fn update_multiple_transactions_first_wins() {
        let mut s = session();
        write_num_ds(&s, "mas", &[("id", some(&[1.0])), ("x", some(&[10.0]))]);
        write_num_ds(&s, "tra", &[("id", some(&[1.0, 1.0])), ("x", some(&[20.0, 30.0]))]);
        run("data mas; update mas tra key=id; run;", &mut s).unwrap();
        // Première transaction (20) appliquée, la seconde (30) ignorée.
        assert_eq!(col(&s, "mas", "x"), some(&[20.0]));
    }

    /// UPDATE : une transaction sans maître correspondant est IGNORÉE (v1).
    #[test]
    fn update_unmatched_transaction_ignored() {
        let mut s = session();
        write_num_ds(&s, "mas", &[("id", some(&[1.0])), ("x", some(&[10.0]))]);
        write_num_ds(&s, "tra", &[("id", some(&[1.0, 2.0])), ("x", some(&[11.0, 22.0]))]);
        let stats = run("data mas; update mas tra key=id; run;", &mut s).unwrap();
        // id=2 (sans maître) n'est PAS inséré : 1 obs en sortie.
        assert_eq!(col(&s, "mas", "id"), some(&[1.0]));
        assert_eq!(col(&s, "mas", "x"), some(&[11.0]));
        assert_eq!(stats.written, vec![("WORK.MAS".to_string(), 1, 2)]);
    }

    /// UPDATE avec WHERE= sur le maître : les obs filtrées ne sont ni mises à
    /// jour ni sorties.
    #[test]
    fn update_master_where() {
        let mut s = session();
        write_num_ds(&s, "mas", &[("id", some(&[1.0, 2.0, 3.0])), ("x", some(&[10.0, 20.0, 30.0]))]);
        write_num_ds(&s, "tra", &[("id", some(&[2.0])), ("x", some(&[99.0]))]);
        let stats = run(
            "data out; update mas(where=(id>=2)) tra key=id; run;",
            &mut s,
        )
        .unwrap();
        // id=1 filtré ; id=2 mis à jour, id=3 inchangé.
        assert_eq!(col(&s, "out", "id"), some(&[2.0, 3.0]));
        assert_eq!(col(&s, "out", "x"), some(&[99.0, 30.0]));
        // 2 obs maître lues (id=1 rejeté).
        assert_eq!(stats.read[0], ("WORK.MAS".to_string(), 2));
    }

    /// UPDATE avec plusieurs variables clé.
    #[test]
    fn update_multiple_keys() {
        let mut s = session();
        write_num_ds(
            &s,
            "mas",
            &[("k1", some(&[1.0, 1.0])), ("k2", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))],
        );
        // Met à jour seulement (1,2).
        write_num_ds(
            &s,
            "tra",
            &[("k1", some(&[1.0])), ("k2", some(&[2.0])), ("x", some(&[99.0]))],
        );
        run("data mas; update mas tra key=k1 k2; run;", &mut s).unwrap();
        assert_eq!(col(&s, "mas", "x"), some(&[10.0, 99.0]));
    }

    /// UPDATE avec clé CARACTÈRE (insensible aux blancs finaux).
    #[test]
    fn update_char_key() {
        let mut s = session();
        write_keyed_ds(&s, "mas", "name", &["a", "b", "c"], &[("x", some(&[1.0, 2.0, 3.0]))]);
        write_keyed_ds(&s, "tra", "name", &["b"], &[("x", some(&[20.0]))]);
        run("data mas; update mas tra key=name; run;", &mut s).unwrap();
        assert_eq!(col(&s, "mas", "x"), some(&[1.0, 20.0, 3.0]));
    }

    /// UPDATE : la transaction apporte une NOUVELLE variable absente du maître.
    #[test]
    fn update_new_variable_from_transaction() {
        let mut s = session();
        write_num_ds(&s, "mas", &[("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))]);
        write_num_ds(&s, "tra", &[("id", some(&[1.0])), ("z", some(&[5.0]))]);
        run("data mas; update mas tra key=id; run;", &mut s).unwrap();
        // z existe (du maître absent → missing), posée pour id=1.
        assert_eq!(col(&s, "mas", "z"), vec![Some(5.0), None]);
    }

    /// UPDATE : KEY= absente d'un dataset → erreur de compilation.
    #[test]
    fn update_key_not_on_transaction_errors() {
        let mut s = session();
        write_num_ds(&s, "mas", &[("id", some(&[1.0])), ("x", some(&[10.0]))]);
        write_num_ds(&s, "tra", &[("other", some(&[1.0])), ("x", some(&[20.0]))]);
        let e = run_err("data mas; update mas tra key=id; run;", &mut s);
        assert!(e.contains("KEY variable id"), "got: {e}");
    }

    /// UPDATE : KEY= obligatoire (erreur de parsing si absente).
    #[test]
    fn update_requires_key_option() {
        // Parsing seul : KEY= absente → erreur de parsing (pas d'exécution).
        let file = SourceFile::new("data mas; update mas tra; run;");
        let mut ts = StatementStream::new(&file).unwrap();
        assert!(ts.next().is_kw("data"));
        let err = crate::parser::datastep::parse_data_step(&mut ts).unwrap_err();
        assert!(err.to_string().to_uppercase().contains("KEY"), "got: {err}");
    }

    /// UPDATE avec BY : FIRST./LAST. exposés sur les groupes BY du maître.
    #[test]
    fn update_with_by_first_last() {
        let mut s = session();
        // maître trié par g : g=1,1,2 ; x=10,20,30.
        write_num_ds(
            &s,
            "mas",
            &[("g", some(&[1.0, 1.0, 2.0])), ("id", some(&[1.0, 2.0, 3.0])), ("x", some(&[10.0, 20.0, 30.0]))],
        );
        write_num_ds(&s, "tra", &[("id", some(&[2.0])), ("x", some(&[99.0]))]);
        run(
            "data out; update mas tra key=id; by g; \
             f = first.g; l = last.g; run;",
            &mut s,
        )
        .unwrap();
        // id=2 mis à jour ; FIRST.g sur les 1res obs de chaque groupe g.
        assert_eq!(col(&s, "out", "x"), some(&[10.0, 99.0, 30.0]));
        assert_eq!(col(&s, "out", "f"), some(&[1.0, 0.0, 1.0]));
        assert_eq!(col(&s, "out", "l"), some(&[0.0, 1.0, 1.0]));
    }

    /// UPDATE avec BY : la mise à jour reste pilotée par KEY= au sein des
    /// groupes BY (chaque obs maître conserve son comportement).
    #[test]
    fn update_with_by_groups_update() {
        let mut s = session();
        write_num_ds(
            &s,
            "mas",
            &[("g", some(&[1.0, 2.0])), ("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))],
        );
        write_num_ds(&s, "tra", &[("id", some(&[1.0, 2.0])), ("x", some(&[100.0, 200.0]))]);
        run("data out; update mas tra key=id; by g; run;", &mut s).unwrap();
        assert_eq!(col(&s, "out", "x"), some(&[100.0, 200.0]));
    }

    /// UPDATE : le corps peut calculer des variables dérivées.
    #[test]
    fn update_with_derived_body_statement() {
        let mut s = session();
        write_num_ds(&s, "mas", &[("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))]);
        write_num_ds(&s, "tra", &[("id", some(&[1.0])), ("x", some(&[100.0]))]);
        run("data out; update mas tra key=id; d = x * 2; run;", &mut s).unwrap();
        // x après MAJ : 100, 20 ; d = 200, 40.
        assert_eq!(col(&s, "out", "x"), some(&[100.0, 20.0]));
        assert_eq!(col(&s, "out", "d"), some(&[200.0, 40.0]));
    }

    // ----- MODIFY -----

    /// MODIFY de base : une modification par assignation persiste en place.
    #[test]
    fn modify_basic_assign_persists() {
        let mut s = session();
        write_num_ds(&s, "d", &[("id", some(&[1.0, 2.0, 3.0])), ("x", some(&[10.0, 20.0, 30.0]))]);
        let stats = run("data d; modify d; x = x + 1; run;", &mut s).unwrap();
        assert_eq!(col(&s, "d", "x"), some(&[11.0, 21.0, 31.0]));
        // Réécriture en place : même nombre d'obs/variables.
        assert_eq!(stats.written, vec![("WORK.D".to_string(), 3, 2)]);
        assert_eq!(stats.read, vec![("WORK.D".to_string(), 3)]);
        assert_eq!(s.last_dataset.as_deref(), Some("WORK.D"));
    }

    /// MODIFY : conditionnel (modifie seulement certaines obs).
    #[test]
    fn modify_conditional_update() {
        let mut s = session();
        write_num_ds(&s, "d", &[("id", some(&[1.0, 2.0, 3.0])), ("x", some(&[10.0, 20.0, 30.0]))]);
        run("data d; modify d; if id = 2 then x = 999; run;", &mut s).unwrap();
        assert_eq!(col(&s, "d", "x"), some(&[10.0, 999.0, 30.0]));
    }

    /// MODIFY : OUTPUT explicite est INTERDIT (erreur de compilation).
    #[test]
    fn modify_output_not_allowed() {
        let mut s = session();
        write_num_ds(&s, "d", &[("x", some(&[1.0]))]);
        let e = run_err("data d; modify d; output; run;", &mut s);
        assert!(e.contains("OUTPUT statement is not allowed"), "got: {e}");
    }

    /// MODIFY avec KEY= (lecture séquentielle, clés présentes).
    #[test]
    fn modify_with_key() {
        let mut s = session();
        write_num_ds(&s, "d", &[("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))]);
        run("data d; modify d key=id; x = x * 10; run;", &mut s).unwrap();
        assert_eq!(col(&s, "d", "x"), some(&[100.0, 200.0]));
    }

    /// MODIFY + NOBS= : le total est disponible avant la boucle.
    #[test]
    fn modify_nobs_available() {
        let mut s = session();
        write_num_ds(&s, "d", &[("x", some(&[5.0, 6.0, 7.0]))]);
        run("data d; modify d nobs=n; x = n; run;", &mut s).unwrap();
        // Chaque obs reçoit le total = 3.
        assert_eq!(col(&s, "d", "x"), some(&[3.0, 3.0, 3.0]));
    }

    /// MODIFY + POINT= : accès direct piloté par un DO, modifie toutes les obs.
    #[test]
    fn modify_point_loop_all() {
        let mut s = session();
        write_num_ds(&s, "d", &[("x", some(&[10.0, 20.0, 30.0]))]);
        let stats = run(
            "data d; do p = 1 to 3; modify d point=p; x = x + 100; end; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(col(&s, "d", "x"), some(&[110.0, 120.0, 130.0]));
        // 3 obs traitées, 3 réécrites.
        assert_eq!(stats.read, vec![("WORK.D".to_string(), 3)]);
        assert_eq!(stats.written, vec![("WORK.D".to_string(), 3, 1)]);
    }

    /// MODIFY + POINT= : accès direct ciblé (une seule obs modifiée).
    #[test]
    fn modify_point_single_row() {
        let mut s = session();
        write_num_ds(&s, "d", &[("x", some(&[10.0, 20.0, 30.0]))]);
        run(
            "data d; do p = 2 to 2; modify d point=p; x = 999; end; run;",
            &mut s,
        )
        .unwrap();
        // Seule la 2e obs change.
        assert_eq!(col(&s, "d", "x"), some(&[10.0, 999.0, 30.0]));
    }

    /// MODIFY : char + num, modification d'une colonne char persiste.
    #[test]
    fn modify_char_column() {
        let mut s = session();
        write_keyed_ds(&s, "d", "grp", &["a", "a", "b"], &[("x", some(&[1.0, 2.0, 3.0]))]);
        run("data d; modify d; if x >= 2 then grp = 'z'; run;", &mut s).unwrap();
        assert_eq!(
            col_str(&s, "d", "grp"),
            vec![Some("a".into()), Some("z".into()), Some("z".into())]
        );
    }

    /// MODIFY : KEY= absente du dataset → erreur de compilation.
    #[test]
    fn modify_key_not_present_errors() {
        let mut s = session();
        write_num_ds(&s, "d", &[("x", some(&[1.0]))]);
        let e = run_err("data d; modify d key=nope; run;", &mut s);
        assert!(e.contains("KEY variable nope"), "got: {e}");
    }

    /// UPDATE/MODIFY exclusif : pas plus d'une source par étape.
    #[test]
    fn update_after_set_is_error() {
        let mut s = session();
        write_num_ds(&s, "a", &[("id", some(&[1.0]))]);
        write_num_ds(&s, "b", &[("id", some(&[1.0]))]);
        let e = run_err("data out; set a; update a b key=id; run;", &mut s);
        assert!(e.contains("Only one SET, MERGE, UPDATE, or MODIFY"), "got: {e}");
    }

    /// MODIFY après MODIFY → erreur.
    #[test]
    fn modify_twice_is_error() {
        let mut s = session();
        write_num_ds(&s, "a", &[("x", some(&[1.0]))]);
        write_num_ds(&s, "b", &[("x", some(&[1.0]))]);
        let e = run_err("data a; modify a; modify b; run;", &mut s);
        assert!(e.contains("Only one SET, MERGE, UPDATE, or MODIFY"), "got: {e}");
    }

    // ── M16.6 : LINK / RETURN / GOTO / labels / RETAIN _ALL_ ─────────────

    /// LINK/RETURN de base : appel d'une sous-routine étiquetée, retour après.
    /// Structure SAS idiomatique : la ligne principale se termine par un RETURN
    /// (output implicite), puis les sous-routines suivent.
    #[test]
    fn link_basic_call_and_return() {
        let mut s = session();
        run(
            "data out; x = 1; link sub; y = x; return; \
             sub: x = 10; return; \
             run;",
            &mut s,
        )
        .unwrap();
        // x=1, LINK sub → x=10, RETURN → reprise : y=x=10, RETURN principal →
        // output implicite (la sous-routine n'est pas exécutée en chute).
        assert_eq!(col(&s, "out", "x"), vec![Some(10.0)]);
        assert_eq!(col(&s, "out", "y"), vec![Some(10.0)]);
    }

    /// LINK imbriqué : la pile d'adresses de retour est correcte.
    #[test]
    fn link_nested_stack() {
        let mut s = session();
        run(
            "data out; a = 0; link one; a = a + 1; return; \
             one: a = a + 10; link two; a = a + 100; return; \
             two: a = a + 1000; return; \
             run;",
            &mut s,
        )
        .unwrap();
        // a: 0 → link one → +10 (10) → link two → +1000 (1010) → return one
        // → +100 (1110) → return main → +1 (1111). stop.
        assert_eq!(col(&s, "out", "a"), vec![Some(1111.0)]);
    }

    /// GOTO : saut inconditionnel (les statements entre le GOTO et la cible
    /// sont ignorés).
    #[test]
    fn goto_unconditional_jump() {
        let mut s = session();
        run(
            "data out; x = 1; goto skip; x = 999; skip: y = x; run;",
            &mut s,
        )
        .unwrap();
        // x=1, GOTO skip → x=999 sauté, y=x=1 ; chute en fin → output implicite.
        assert_eq!(col(&s, "out", "x"), vec![Some(1.0)]);
        assert_eq!(col(&s, "out", "y"), vec![Some(1.0)]);
    }

    /// GOTO qui sort d'une boucle DO (termine la boucle prématurément).
    #[test]
    fn goto_breaks_out_of_do_loop() {
        let mut s = session();
        run(
            "data out; total = 0; \
             do i = 1 to 100; total = total + i; if i = 5 then goto done; end; \
             done: ; run;",
            &mut s,
        )
        .unwrap();
        // 1+2+3+4+5 = 15 ; la boucle est terminée par le GOTO à i=5.
        assert_eq!(col(&s, "out", "total"), vec![Some(15.0)]);
        assert_eq!(col(&s, "out", "i"), vec![Some(5.0)]);
    }

    /// GOTO avec plusieurs étiquettes : ciblage correct.
    #[test]
    fn goto_multiple_labels_targets_correctly() {
        let mut s = session();
        run(
            "data out; x = 1; goto third; \
             first: r = 1; goto fin; \
             second: r = 2; goto fin; \
             third: r = 3; goto fin; \
             fin: ; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(col(&s, "out", "r"), vec![Some(3.0)]);
    }

    /// Étiquette sur divers statements (ici un bloc DO et une assignation).
    #[test]
    fn label_on_various_statements() {
        let mut s = session();
        run(
            "data out; goto blk; a = 1; \
             blk: do; a = 7; b = 8; end; \
             after: c = 9; run;",
            &mut s,
        )
        .unwrap();
        // GOTO blk saute `a = 1` ; le bloc DO étiqueté pose a=7,b=8 ; puis c=9.
        assert_eq!(col(&s, "out", "a"), vec![Some(7.0)]);
        assert_eq!(col(&s, "out", "b"), vec![Some(8.0)]);
        assert_eq!(col(&s, "out", "c"), vec![Some(9.0)]);
    }

    /// RETURN sans LINK actif : termine l'itération (output implicite), pas une
    /// erreur. La variable assignée APRÈS le RETURN n'est pas affectée.
    #[test]
    fn return_without_link_ends_iteration() {
        let mut s = session();
        let stats = run(
            "data out; x = 1; return; x = 2; run;",
            &mut s,
        )
        .unwrap();
        // RETURN sans LINK → fin d'itération avec output implicite : x=1.
        assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 1)]);
        assert_eq!(col(&s, "out", "x"), vec![Some(1.0)]);
    }

    /// GOTO vers une étiquette inexistante : erreur de compilation.
    #[test]
    fn goto_undefined_label_compile_error() {
        let mut s = session();
        let e = run_err("data out; x = 1; goto nowhere; run;", &mut s);
        assert!(
            e.contains("NOWHERE") && e.contains("not defined"),
            "got: {e}"
        );
    }

    /// LINK vers une étiquette inexistante : erreur de compilation.
    #[test]
    fn link_undefined_label_compile_error() {
        let mut s = session();
        let e = run_err("data out; x = 1; link nowhere; run;", &mut s);
        assert!(
            e.contains("NOWHERE") && e.contains("not defined"),
            "got: {e}"
        );
    }

    /// Étiquette définie deux fois : erreur de compilation.
    #[test]
    fn duplicate_label_compile_error() {
        let mut s = session();
        let e = run_err("data out; lbl: x = 1; lbl: x = 2; goto lbl; run;", &mut s);
        assert!(e.contains("LBL") && e.contains("more than once"), "got: {e}");
    }

    /// GOTO vers une étiquette imbriquée dans un bloc DO : non supporté (erreur).
    #[test]
    fn goto_into_nested_block_compile_error() {
        let mut s = session();
        let e = run_err(
            "data out; goto inner; do; inner: x = 1; end; stop; run;",
            &mut s,
        );
        assert!(e.contains("INNER") && e.contains("nested"), "got: {e}");
    }

    /// RETAIN _ALL_ : toutes les variables connues sont retenues à travers les
    /// itérations.
    #[test]
    fn retain_all_retains_every_variable() {
        let mut s = session();
        write_num_ds(&s, "inp", &[("x", some(&[1.0, 2.0, 3.0]))]);
        run(
            "data out; set inp; retain a b; a = 0; b = 0; retain _all_; \
             a = a + x; b = b + 1; run;",
            &mut s,
        )
        .unwrap();
        // a et b retenus (cumul) : a = 1,3,6 ; b = 1,2,3. Mais `a=0;b=0;` les
        // remet à 0 AVANT le cumul, à CHAQUE itération → a=x, b=1. On teste donc
        // que la retenue n'empêche pas la ré-assignation explicite : a=1,2,3.
        assert_eq!(col(&s, "out", "a"), vec![Some(1.0), Some(2.0), Some(3.0)]);
        assert_eq!(col(&s, "out", "b"), vec![Some(1.0), Some(1.0), Some(1.0)]);
    }

    /// RETAIN _ALL_ : effet cumulatif réel (pas de remise à missing entre
    /// itérations) pour une variable jamais ré-initialisée. `t` est initialisé
    /// à 0 à la 1re itération seulement, puis RETAIN _ALL_ le préserve.
    #[test]
    fn retain_all_accumulates() {
        let mut s = session();
        write_num_ds(&s, "inp", &[("x", some(&[1.0, 2.0, 3.0, 4.0]))]);
        run(
            "data out; set inp; if _n_ = 1 then t = 0; retain _all_; t = t + x; run;",
            &mut s,
        )
        .unwrap();
        // t=0 à la 1re obs, retenu ensuite (jamais remis à missing) → cumul :
        // 1, 3, 6, 10.
        assert_eq!(
            col(&s, "out", "t"),
            vec![Some(1.0), Some(3.0), Some(6.0), Some(10.0)]
        );
    }

    /// RETAIN _ALL_ mélangé à un RETAIN explicite avec valeur initiale : la
    /// valeur initiale du RETAIN explicite est honorée.
    #[test]
    fn retain_all_mixed_with_explicit_retain() {
        let mut s = session();
        write_num_ds(&s, "inp", &[("x", some(&[1.0, 2.0, 3.0]))]);
        run(
            "data out; set inp; retain base 100; if _n_ = 1 then sum = 0; \
             retain _all_; sum = sum + x; run;",
            &mut s,
        )
        .unwrap();
        // base retenu avec init 100 (jamais réassigné) ; sum cumulé.
        assert_eq!(
            col(&s, "out", "base"),
            vec![Some(100.0), Some(100.0), Some(100.0)]
        );
        assert_eq!(col(&s, "out", "sum"), vec![Some(1.0), Some(3.0), Some(6.0)]);
    }

    /// Variable créée APRÈS RETAIN _ALL_ : NON retenue automatiquement (remise à
    /// missing à chaque itération).
    #[test]
    fn variable_created_after_retain_all_not_retained() {
        let mut s = session();
        write_num_ds(&s, "inp", &[("x", some(&[5.0, 6.0, 7.0]))]);
        run(
            "data out; set inp; retain _all_; later = later + x; run;",
            &mut s,
        )
        .unwrap();
        // `later` n'existe PAS au point du RETAIN _ALL_ (créée par sa 1re
        // référence ensuite) → non retenue → remise à missing chaque itération
        // → later = . + x = . (missing propagé).
        assert_eq!(col(&s, "out", "later"), vec![None, None, None]);
    }

    /// LINK dans une boucle : l'itération de la boucle reprend après le retour.
    #[test]
    fn link_inside_do_loop_continues_iteration() {
        let mut s = session();
        run(
            "data out; total = 0; \
             do i = 1 to 4; link addit; end; \
             return; \
             addit: total = total + i; return; \
             run;",
            &mut s,
        )
        .unwrap();
        // total = 1+2+3+4 = 10 ; la boucle continue après chaque RETURN.
        assert_eq!(col(&s, "out", "total"), vec![Some(10.0)]);
        assert_eq!(col(&s, "out", "i"), vec![Some(5.0)]);
    }

    /// Modifications de variables dans le code LINKé : persistance (PDV partagé).
    #[test]
    fn link_modifications_persist() {
        let mut s = session();
        run(
            "data out; x = 5; y = 0; link doit; z = x + y; return; \
             doit: x = x * 2; y = 100; return; run;",
            &mut s,
        )
        .unwrap();
        // doit : x=10, y=100 (persistants) ; z = 10 + 100 = 110.
        assert_eq!(col(&s, "out", "x"), vec![Some(10.0)]);
        assert_eq!(col(&s, "out", "y"), vec![Some(100.0)]);
        assert_eq!(col(&s, "out", "z"), vec![Some(110.0)]);
    }

    /// Entrées multiples vers la même étiquette via LINK (réutilisation).
    #[test]
    fn multiple_link_entries_same_label() {
        let mut s = session();
        run(
            "data out; c = 0; link bump; link bump; link bump; return; \
             bump: c = c + 1; return; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(col(&s, "out", "c"), vec![Some(3.0)]);
    }

    /// GOTO en arrière formant une boucle, terminée par une condition.
    #[test]
    fn goto_backward_forms_loop() {
        let mut s = session();
        run(
            "data out; n = 0; \
             loop: n = n + 1; if n < 5 then goto loop; \
             run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(col(&s, "out", "n"), vec![Some(5.0)]);
    }

    /// LINK depuis une sous-routine LINK vers une troisième : pile à 2 niveaux,
    /// chaque RETURN reprend au bon endroit.
    #[test]
    fn link_chain_returns_in_order() {
        let mut s = session();
        run(
            "data out; trace = 0; link a; trace = trace * 10 + 9; return; \
             a: trace = trace * 10 + 1; link b; trace = trace * 10 + 2; return; \
             b: trace = trace * 10 + 3; return; \
             run;",
            &mut s,
        )
        .unwrap();
        // 0 → a: *10+1 = 1 → b: *10+3 = 13 → ret a: *10+2 = 132 → ret main:
        // *10+9 = 1329.
        assert_eq!(col(&s, "out", "trace"), vec![Some(1329.0)]);
    }

    /// `go to label;` (forme en deux mots) équivalente à `goto label;`.
    #[test]
    fn go_to_two_word_form() {
        let mut s = session();
        run(
            "data out; x = 1; go to skip; x = 999; skip: ; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(col(&s, "out", "x"), vec![Some(1.0)]);
    }

    /// GOTO/LINK fonctionnent sur plusieurs itérations d'un SET (la pile de
    /// retour est ré-initialisée à chaque itération).
    #[test]
    fn link_resets_per_iteration() {
        let mut s = session();
        write_num_ds(&s, "inp", &[("x", some(&[1.0, 2.0, 3.0]))]);
        run(
            "data out; set inp; link dbl; stop_marker: ; goto past; \
             dbl: d = x * 2; return; \
             past: ; run;",
            &mut s,
        )
        .unwrap();
        // Chaque itération : link dbl (d=2x), retour, goto past (saute rien),
        // output implicite. d = 2,4,6 sur les 3 obs.
        assert_eq!(
            col(&s, "out", "d"),
            vec![Some(2.0), Some(4.0), Some(6.0)]
        );
    }

    /// RETAIN _ALL_ n'autorise pas de valeur initiale.
    #[test]
    fn retain_all_rejects_initial_value() {
        let mut s = session();
        let e = run_err("data out; x = 1; retain _all_ 5; run;", &mut s);
        assert!(e.contains("initial value"), "got: {e}");
    }

    // ── M17.1 : DECLARE HASH + defineKey/defineData/defineDone ───────────

    /// DECLARE HASH sans option crée l'objet (options par défaut).
    #[test]
    fn hash_declare_no_options() {
        let mut s = session();
        run("data _null_; declare hash h(); run;", &mut s).unwrap();
        let h = s.debug_hashes.get("H").expect("hash H exists");
        assert!(h.ordered.is_none());
        assert!(h.duplicate.is_none());
        assert!(!h.multidata);
        assert!(h.dataset.is_none());
        assert!(h.keys.is_empty());
        assert!(h.data_vars.is_empty());
        assert!(!h.defined);
    }

    /// Options ordered/duplicate/multidata parsées et stockées (minuscules).
    #[test]
    fn hash_declare_options_parsed() {
        let mut s = session();
        run(
            "data _null_; declare hash h(ordered:'YES', duplicate:'replace', multidata:'yes'); run;",
            &mut s,
        )
        .unwrap();
        let h = s.debug_hashes.get("H").unwrap();
        assert_eq!(h.ordered.as_deref(), Some("yes"));
        assert_eq!(h.duplicate.as_deref(), Some("replace"));
        assert!(h.multidata);
    }

    /// multidata:'no' (ou absent) → false.
    #[test]
    fn hash_declare_multidata_no() {
        let mut s = session();
        run(
            "data _null_; declare hash h(multidata:'no'); run;",
            &mut s,
        )
        .unwrap();
        assert!(!s.debug_hashes.get("H").unwrap().multidata);
    }

    /// Option dataset: conservée (chargement différé à M17.2).
    #[test]
    fn hash_declare_dataset_option() {
        let mut s = session();
        run(
            "data _null_; declare hash h(dataset:'work.lookup'); run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(
            s.debug_hashes.get("H").unwrap().dataset.as_deref(),
            Some("work.lookup")
        );
    }

    /// Option inconnue → erreur de compilation.
    #[test]
    fn hash_declare_unknown_option_errors() {
        let mut s = session();
        let e = run_err("data _null_; declare hash h(bogus:'x'); run;", &mut s);
        assert!(e.to_uppercase().contains("BOGUS"), "got: {e}");
    }

    /// defineKey avec une seule variable.
    #[test]
    fn hash_define_key_single() {
        let mut s = session();
        run(
            "data _null_; k = 1; declare hash h(); h.defineKey('k'); h.defineDone(); run;",
            &mut s,
        )
        .unwrap();
        let h = s.debug_hashes.get("H").unwrap();
        assert_eq!(h.keys, vec!["K".to_string()]);
        assert!(h.defined);
    }

    /// defineKey avec plusieurs variables (ordre préservé, UPPERCASE).
    #[test]
    fn hash_define_key_multiple() {
        let mut s = session();
        run(
            "data _null_; k1 = 1; k2 = 'a'; declare hash h(); h.defineKey('k1', 'k2'); run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(
            s.debug_hashes.get("H").unwrap().keys,
            vec!["K1".to_string(), "K2".to_string()]
        );
    }

    /// defineData simple et multiple.
    #[test]
    fn hash_define_data_single_and_multiple() {
        let mut s = session();
        run(
            "data _null_; k = 1; v1 = 2; v2 = 3; declare hash h(); h.defineKey('k'); h.defineData('v1', 'v2'); run;",
            &mut s,
        )
        .unwrap();
        let h = s.debug_hashes.get("H").unwrap();
        assert_eq!(h.keys, vec!["K".to_string()]);
        assert_eq!(h.data_vars, vec!["V1".to_string(), "V2".to_string()]);
    }

    /// defineDone est idempotent (deux appels → toujours defined, pas d'erreur).
    #[test]
    fn hash_define_done_idempotent() {
        let mut s = session();
        run(
            "data _null_; k = 1; declare hash h(); h.defineKey('k'); h.defineDone(); h.defineDone(); run;",
            &mut s,
        )
        .unwrap();
        assert!(s.debug_hashes.get("H").unwrap().defined);
    }

    /// Plusieurs objets hash dans la même étape, indépendants.
    #[test]
    fn hash_multiple_objects() {
        let mut s = session();
        run(
            "data _null_; a = 1; b = 2; \
             declare hash h1(); h1.defineKey('a'); h1.defineDone(); \
             declare hash h2(multidata:'yes'); h2.defineKey('b'); h2.defineData('a'); h2.defineDone(); \
             run;",
            &mut s,
        )
        .unwrap();
        let h1 = s.debug_hashes.get("H1").unwrap();
        let h2 = s.debug_hashes.get("H2").unwrap();
        assert_eq!(h1.keys, vec!["A".to_string()]);
        assert!(h1.data_vars.is_empty());
        assert!(!h1.multidata);
        assert_eq!(h2.keys, vec!["B".to_string()]);
        assert_eq!(h2.data_vars, vec!["A".to_string()]);
        assert!(h2.multidata);
    }

    /// defineKey sur une variable inconnue → erreur de compilation.
    #[test]
    fn hash_define_key_unknown_variable_errors() {
        let mut s = session();
        let e = run_err(
            "data _null_; declare hash h(); h.defineKey('nosuchvar'); run;",
            &mut s,
        );
        assert!(e.to_uppercase().contains("NOSUCHVAR"), "got: {e}");
    }

    /// Méthode sur un objet non déclaré → erreur de compilation.
    #[test]
    fn hash_method_on_undeclared_object_errors() {
        let mut s = session();
        let e = run_err(
            "data _null_; k = 1; ghost.defineKey('k'); run;",
            &mut s,
        );
        assert!(e.to_uppercase().contains("GHOST"), "got: {e}");
    }

    /// Méthode non implémentée (find) → erreur runtime « not yet implemented »
    /// (M17.2). L'objet est déclaré et défini ; find n'est pas câblé.
    #[test]
    fn hash_unimplemented_method_errors() {
        let mut s = session();
        let e = run_err(
            "data _null_; k = 1; declare hash h(); h.defineKey('k'); h.defineDone(); h.find(); run;",
            &mut s,
        );
        assert!(
            e.to_uppercase().contains("NOT YET IMPLEMENTED") && e.to_uppercase().contains("FIND"),
            "got: {e}"
        );
    }
}
