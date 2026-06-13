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

use super::eval::{coerce_num, eval, EvalCtx};
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

pub struct StepStats {
    /// (display, lignes lues) par input.
    pub read: Vec<(String, usize)>,
    /// (display, obs, vars) par output écrit.
    pub written: Vec<(String, usize, usize)>,
}

#[derive(PartialEq)]
enum Flow {
    Normal,
    NextIter,
    EndStep,
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

    let StepProgram {
        pdv,
        stmts,
        input,
        text_input,
        outputs,
        has_explicit_output,
        uninitialized,
        initial_values,
        arrays,
        labels,
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
        cur_ds: 0,
        cursors: vec![0; n_datasets],
        filtered: vec![Vec::new(); n_datasets],
        prev_keys: None,
        rows_read: vec![0; n_datasets],
        ctx: EvalCtx {
            arrays,
            by_flags,
            in_flags,
            macro_symbols,
            ..EvalCtx::default()
        },
        outputs,
        builders,
        out_rows: vec![0; n_outputs],
        merge_plan: Vec::new(),
        merge_cursor: 0,
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

        let mut flow = Flow::Normal;
        for stmt in &stmts {
            flow = r.exec_stmt(stmt)?;
            if flow != Flow::Normal {
                break;
            }
        }
        if flow == Flow::EndStep {
            break;
        }
        if flow != Flow::NextIter && !has_explicit_output {
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

    // CALL SYMPUT (M11.5) : drain des écritures différées vers la table
    // macro APRÈS le RUN de l'étape (règle de visibilité SAS — le symbole
    // n'est pas visible dans la même étape). Sous le build par défaut,
    // `set_symbol_global` est un no-op (l'engine identité n'a pas de table) :
    // `call symput` parse et s'exécute mais n'a aucun effet macro.
    for (name, value) in std::mem::take(&mut r.ctx.symput_writes) {
        session.macro_engine.set_symbol_global(&name, value);
    }

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
    // Source texte (M14) : NOTE "N records were read from the infile ...".
    if let Some(text) = &r.text {
        session.log.note(&format!(
            "{} records were read from {}.",
            r.text_read, text.display
        ));
        stats.read.push((text.display.clone(), r.text_read));
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
            let label = labels.get(&v.name.to_uppercase()).cloned();
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

impl Runner {
    fn exec_stmt(&mut self, stmt: &DsStmt) -> Result<Flow> {
        match stmt {
            DsStmt::Set(_) => {
                let Some(input) = &self.input else {
                    // Impossible après compile() ; garde-fou.
                    return Err(SasError::runtime("SET statement without input data."));
                };
                if input.by.is_empty() {
                    self.exec_set_concat()
                } else {
                    self.exec_set_interleave()
                }
            }
            DsStmt::Merge(_) => self.exec_merge(),
            DsStmt::Assign { var, expr } => {
                let value = self.eval_checked(expr)?;
                let Some(slot) = self.pdv.slot(var) else {
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
            DsStmt::AssignIndexed { array, index, expr } => {
                // Indice évalué avec les MÊMES règles que les rvalues
                // (coercition num + arrondi ; missing/hors bornes → l'étape
                // s'arrête), puis coercition vers le type de l'élément.
                let idx_val = self.eval_checked(index)?;
                let slot = self.resolve_subscript(array, idx_val)?;
                let value = self.eval_checked(expr)?;
                let coerced = self.coerce_assign(value, self.pdv.vars()[slot].ty);
                self.pdv.set(slot, coerced);
                Ok(Flow::Normal)
            }
            DsStmt::CallRoutine { name, args } => self.exec_call_routine(name, args),
            // INPUT (M14) : lit le PROCHAIN enregistrement de la source texte
            // dans le PDV. Comme SET, l'épuisement de la source termine
            // l'étape IMMÉDIATEMENT (au milieu de l'itération).
            DsStmt::Input(items) => self.exec_input(items),
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

    /// Exécute une CALL routine (M11.5). v1 : seule `SYMPUT` est supportée.
    ///
    /// `call symput(name, value);` évalue les deux arguments, convertit le
    /// nom et la valeur en chaîne (un numérique est formaté en BEST12.
    /// cadré à gauche, conformément à SAS), et POUSSE la paire dans
    /// `ctx.symput_writes`. La table macro n'est PAS touchée pendant
    /// l'étape : le symbole n'est visible qu'APRÈS le RUN (règle SAS) ; le
    /// drain effectif est fait par `execute` après la boucle implicite.
    /// Toute autre routine → erreur runtime « not yet implemented ».
    fn exec_call_routine(&mut self, name: &str, args: &[crate::ast::Expr]) -> Result<Flow> {
        if !name.eq_ignore_ascii_case("symput") {
            return Err(SasError::runtime(format!(
                "CALL routine {} is not yet implemented.",
                name.to_uppercase()
            )));
        }
        if args.len() != 2 {
            return Err(SasError::runtime(
                "CALL SYMPUT requires exactly two arguments (name, value).",
            ));
        }
        let name_val = self.eval_checked(&args[0])?;
        let value_val = self.eval_checked(&args[1])?;
        // Le nom macro est trimé (SAS rogne les blancs de bord du nom).
        let sym_name = symput_string(name_val);
        let sym_value = symput_string(value_val);
        self.ctx
            .symput_writes
            .push((sym_name.trim().to_string(), sym_value));
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
                return Ok(Flow::Normal);
            }
        }
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

    /// Résout l'indice d'une assignation indexée en slot PDV : coercition
    /// numérique (mêmes règles que `eval::coerce_num`), arrondi au plus
    /// proche ; missing ou hors 1..=dim → erreur qui stoppe l'étape.
    fn resolve_subscript(&mut self, array: &str, idx_val: Value) -> Result<usize> {
        let idx = coerce_num(&idx_val, &mut self.ctx).map(f64::round);
        if self.ctx.error_flag {
            self.pdv.error_ = true;
            self.ctx.error_flag = false;
        }
        let Some(slots) = self.ctx.arrays.get(&array.to_uppercase()) else {
            // Impossible après compile() ; garde-fou.
            return Err(SasError::runtime(format!(
                "Undeclared array referenced: {array}."
            )));
        };
        match idx {
            Some(i) if i >= 1.0 && i <= slots.len() as f64 => Ok(slots[i as usize - 1]),
            _ => Err(SasError::runtime("Array subscript out of range.")),
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
        let log = s.log.into_string();
        assert!(
            log.contains("2 records were read from the infile DATALINES."),
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
}
