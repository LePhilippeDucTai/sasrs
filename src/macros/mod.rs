//! Text-transformation stage that runs BEFORE the lexer.
//!
//! This is the architectural seam where the SAS macro processor will live
//! (phase M8+): `%let`, `&var` resolution, `%macro`/`%mend` expansion.
//! Today only the identity stage exists. The contract is incremental by
//! design: the executor feeds source to the stage and lexes the result
//! block by block, because macro execution can affect downstream source
//! (`%let` evaluated mid-program, `CALL SYMPUT`).

mod control;
mod define;
mod error;
mod eval;
mod expand;
mod functions;
mod include;
mod quoting;
mod scan;
mod symbols;

use quoting::MaskSet;

pub use define::{MacroDef, MacroParam};
pub use error::MacroError;

mod segmenter;

pub use segmenter::RawSegmenter;

pub trait TextStage {
    /// Transform submitted source text before lexing.
    fn process(&mut self, source: &str) -> String;
}

/// Processeur macro de la session (M11).
///
/// `MacroEngine` porte la table des symboles macro (`%let`/`&var`) et est
/// stocké dans `Session` (cf. `Session::macro_engine`). C'est la couture
/// d'état du futur processeur macro : la table vit pour toute la session et
/// l'expansion est désormais pilotée depuis l'`executor`, plus depuis `lib.rs`.
///
/// # Invariant de bascule (byte-identical)
/// `expand_open_code` DOIT être l'identité stricte pour tout segment sans
/// déclencheur macro résolu (ni `%` ni `&name`) : son fast-path renvoie alors
/// l'entrée inchangée. C'est ce qui garantit l'octet-identité de tout source
/// macro-free (M1..M10), désormais que le processeur macro est TOUJOURS actif.
///
/// # M11.1 — périmètre
/// Cette unité établit seulement la couture : l'état macro vit dans `Session`
/// et l'expansion est appelée depuis l'`executor` (sur le source ENTIER, une
/// fois, en tête de `run_program`). Le découpage en segments bruts
/// (`RawSegmenter`) et l'expansion interfoliée bloc-par-bloc — nécessaires à
/// `CALL SYMPUT` (M11.5) — sont DÉFÉRÉS pour préserver la garantie
/// byte-identical (la segmentation per-bloc risquerait de changer le lexing et
/// l'écho de numéros de ligne).
///
/// # M11.2 — `%macro`/`%mend` + invocation `%name(args)` + `%local`/`%global`
/// L'expanseur gère désormais :
/// - **Définition** : `%macro name[(p1, p2, kw=def, ...)] ; <body> %mend [name];`
///   capture le corps VERBATIM et l'enregistre dans `macros` ; n'émet RIEN.
///   Un `%macro` imbriqué dans un corps n'est PAS traité spécialement à la
///   capture (le `%mend` suivant ferme le corps courant) : la définition
///   imbriquée n'est enregistrée qu'à l'invocation de la macro englobante,
///   lorsque le corps est ré-expansé. C'est une simplification (les corps
///   imbriqués sans invocation de l'englobante restent inertes).
/// - **Invocation** : `%name` ou `%name(args)` en code ouvert. Liaison des
///   arguments (positionnels d'abord, puis `clé=valeur`), empilement d'une
///   portée locale (`scopes`), ré-expansion récursive du corps (donc `&param`
///   et `%name` imbriqués se résolvent), insertion du texte expansé à la place
///   de l'appel, dépilement de la portée.
/// - **`%local v1 v2;`** : crée les variables (vides) dans la portée du haut.
///   **`%global v1 v2;`** : crée les variables (vides) dans `table`.
/// - **Résolution `&name`** : pile de portées (plus interne d'abord) puis
///   `table`.
///
/// ## Règle d'affectation (`%let`/affectation nue dans une macro)
/// `%let v = ...;` met à jour la variable `v` LÀ OÙ ELLE EST DÉJÀ DÉFINIE en
/// remontant la pile (plus interne → plus externe → `table`). Si `v` n'existe
/// nulle part, elle est créée dans `table` (global), conformément au principe
/// SAS : un `%let` non précédé d'un `%local v;` crée un symbole global. Donc un
/// `%local v;` AVANT le `%let v=...;` confine la modification à la portée locale
/// et l'empêche de fuiter en open code.
///
/// ## Garde de récursion
/// `depth` est incrémenté à chaque invocation et plafonné à `MAX_MACRO_DEPTH`.
/// Au-delà, l'invocation n'est PAS expansée : un commentaire de note SAS-like
/// `/* ... */` est émis à la place et le scan continue — aucun `panic`.
///
/// ## Différé (non interprété ici)
/// `%if/%do` (M11.3), `%eval` (M11.4), `%sysfunc`/vars auto (M11.6), fonctions
/// de quoting (`%str`/`%nrstr`). Un corps contenant `%if`/`%do` est stocké tel
/// quel et ré-émis verbatim à l'invocation (non interprété).
#[derive(Default)]
pub struct MacroEngine {
    /// Table globale des symboles macro (`%let`/`&var` en open code, `%global`).
    table: std::collections::HashMap<String, String>,
    /// Table des définitions `%macro name(params); body %mend;`.
    macros: std::collections::HashMap<String, MacroDef>,
    /// Pile de portées locales empilée à chaque invocation de macro. La portée
    /// du haut est la plus interne. Vide en open code. `%local` crée dans le
    /// haut de pile ; `&name` consulte la pile du plus interne au plus externe
    /// avant de retomber sur `table`.
    scopes: Vec<std::collections::HashMap<String, String>>,
    /// Profondeur d'invocation courante (garde anti-récursion infinie).
    depth: usize,
    /// M19.2 — répertoire de base pour résoudre les chemins relatifs de
    /// `%include 'fichier';` (calé sur `Session::base_dir`). Vide par défaut
    /// (chemins relatifs résolus au CWD).
    include_base_dir: std::path::PathBuf,
    /// M19.2 — chemins de bibliothèques autocall (`SASAUTOS`). Pour
    /// `%nomMacro(...)` non défini, on cherche `nommacro.sas` dans ces
    /// répertoires (premier trouvé gagne), on le compile (= `process_impl` du
    /// fichier qui enregistre la `%macro`) puis on invoque. Vide par défaut.
    sasautos_path: Vec<std::path::PathBuf>,
    /// M19.2 — profondeur d'imbrication courante des `%include` (garde contre
    /// les inclusions cycliques). Plafonnée à `MAX_INCLUDE_DEPTH`.
    include_depth: usize,
    /// M35.2 — registre minimal des `fileref` posés par le statement global
    /// `FILENAME ref 'chemin';`. Clé = nom du fileref EN MAJUSCULES, valeur =
    /// chemin déjà résolu (absolu ou relatif à la base). Consulté par
    /// `%include fileref;` pour résoudre un fileref nu en chemin de fichier.
    filerefs: std::collections::HashMap<String, std::path::PathBuf>,
    /// M38.5 — registre des `fileref` assignés à un DEVICE non-fichier
    /// (`FILENAME ref PIPE|URL|TEMP|… ;`). Clé = nom EN MAJUSCULES, valeur =
    /// device EN MAJUSCULES (`PIPE`, `URL`, …). Consulté par `%include fileref;`
    /// pour émettre une NOTE de déferrement propre au lieu d'un « cannot read »
    /// trompeur. Disjoint de `filerefs` (dernier `FILENAME` gagne).
    fileref_devices: std::collections::HashMap<String, String>,
    /// M19.2 — noms (MAJUSCULES) de macros dont la recherche autocall a déjà
    /// été TENTÉE (trouvée ou non), pour éviter de relire/recompiler le disque
    /// à chaque invocation. Une fois compilée, la macro vit dans `macros`.
    autocall_tried: std::collections::HashSet<String>,
    /// M19.3 — options de trace du processeur macro (`MPRINT`/`MLOGIC`/
    /// `SYMBOLGEN`). Voir [`TraceOptions`].
    trace: TraceOptions,
    /// M19.3 — tampons de sortie vers la session (lignes de log, fragments
    /// `%call execute`), drainés par l'exécuteur. Voir [`PendingOutputs`].
    pending: PendingOutputs,
    /// M19.3 — pile des noms de macros en cours d'expansion, pour étiqueter les
    /// lignes `MPRINT(nom):` / `MLOGIC(nom):` et détecter qu'on est dans un
    /// corps de macro (`%return`/`%goto`). La macro la plus interne est en fin
    /// de pile. Vide en code ouvert. Laissée à plat à côté de `depth` : c'est
    /// de l'état d'imbrication d'invocation, consulté à la fois par la trace et
    /// par le contrôle de flux.
    macro_stack: Vec<String>,
    /// M35.4 — état de contrôle de flux macro (`%return`/`%abort`/`%goto`),
    /// dont le cycle de vie est couplé. Voir [`ControlFlow`].
    flow: ControlFlow,
}

/// M19.3 — options de trace du processeur macro (statement global `OPTIONS`).
/// Toutes OFF par défaut.
#[derive(Default)]
struct TraceOptions {
    /// Option `MPRINT` : si vrai, chaque ligne de code produite par
    /// l'expansion d'une macro est écho­tée au log (préfixe `MPRINT(nom):`).
    mprint: bool,
    /// Option `MLOGIC` : si vrai, les décisions d'exécution du processeur
    /// macro (entrée/sortie de macro, conditions `%if`, itérations `%do`) sont
    /// écho­tées au log (préfixe `MLOGIC(nom):`).
    mlogic: bool,
    /// Option `SYMBOLGEN` : si vrai, chaque résolution `&symbol` est écho­tée
    /// au log (`SYMBOLGEN:  Macro variable X resolves to ...`).
    symbolgen: bool,
}

/// M19.3 — tampons de sortie de l'engine vers la session. L'engine n'a pas
/// accès au `LogWriter` (emprunté ailleurs) : il accumule ici et l'exécuteur
/// draine après chaque `expand_open_code`.
#[derive(Default)]
struct PendingOutputs {
    /// Lignes de log produites pendant l'expansion (écho MPRINT/MLOGIC/
    /// SYMBOLGEN et sortie de `%put`). Drainées via `take_pending_log_lines`.
    log_lines: Vec<String>,
    /// File de fragments de code SAS produits par `%call execute(...)` en code
    /// macro, à exécuter APRÈS l'étape/segment courant (même sémantique que le
    /// `CALL EXECUTE` côté DATA step). Drainée via `take_pending_call_execute`.
    call_execute: Vec<String>,
}

/// M35.4 — drapeaux de contrôle de flux du processeur macro
/// (`%return`/`%abort`/`%goto`). Leur cycle de vie est couplé : posés pendant
/// l'expansion d'un corps, testés en tête de `process_impl`, puis
/// réinitialisés/drainés ensemble (cf. docs de champ).
#[derive(Default)]
struct ControlFlow {
    /// `%return` demandé : interrompt l'expansion du corps de macro COURANT
    /// (revient à l'appelant). Posé par `consume_return`, testé en tête de la
    /// boucle `process_impl`, et RÉINITIALISÉ par `expand_invocation` après
    /// l'expansion du corps (ré-entrance : ne fuit jamais vers l'appelant ni
    /// l'open code).
    return_requested: bool,
    /// `%abort` demandé : interrompt l'expansion comme `%return` mais se
    /// PROPAGE vers le haut (l'appelant l'observe). `expand_invocation` ne le
    /// réinitialise PAS ; il est drainé par l'exécuteur via
    /// `take_abort_request` après l'expansion d'un segment d'open code.
    abort_requested: bool,
    /// Variante du `%abort` en cours (forme/option et code retour).
    abort_kind: Option<AbortKind>,
    /// Saut `%goto` en attente : nom (MAJUSCULES) de l'étiquette cible.
    /// Posé par `consume_goto`, il « remonte » : chaque niveau de `process_impl`
    /// (corps de macro, action de `%if`/`%do`) tente de trouver `%label:` dans
    /// SON propre texte ; s'il y parvient il réinitialise le drapeau et saute,
    /// sinon il laisse remonter au niveau parent. Au niveau le PLUS externe du
    /// corps, un drapeau encore posé = étiquette introuvable → NOTE.
    goto_requested: Option<String>,
    /// Budget de sauts `%goto` partagé sur toute l'expansion d'un corps
    /// (garde anti-boucle ; voir `MAX_GOTO_JUMPS`). `None` = pas d'expansion de
    /// corps en cours (réinitialisé à l'entrée de l'invocation).
    goto_budget: i64,
}

/// M35.4 — variante d'un `%abort` macro rencontré pendant l'expansion.
///
/// Le processeur macro ne pouvant pas réellement terminer le process (pas de
/// `process::exit`), on enregistre l'INTENTION : l'exécuteur peut la draîner via
/// `take_abort_request` et arrêter proprement la soumission s'il le souhaite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AbortKind {
    /// `%abort;` — arrêt simple.
    Plain,
    /// `%abort abend [n];` — arrêt « anormal » (code retour optionnel).
    Abend(Option<i64>),
    /// `%abort cancel;` — annulation de la soumission courante.
    Cancel,
    /// `%abort return [n];` — arrêt avec code retour optionnel.
    Return(Option<i64>),
}

impl MacroEngine {
    /// Construit l'engine de session.
    ///
    /// # M11.6 — variables automatiques
    /// On amorce la table globale avec un sous-ensemble des variables
    /// automatiques SAS, résolues ensuite par un `&SYSDATE9` normal. Le flag
    /// `deterministic` choisit entre valeurs FIGÉES (pour des snapshots stables)
    /// et valeurs dérivées de l'horloge réelle.
    ///
    /// Valeurs FIGÉES (`deterministic == true`) :
    /// - `SYSDATE9` = `01JAN1960`
    /// - `SYSDATE`  = `01JAN60`
    /// - `SYSTIME`  = `00:00`
    /// - `SYSDAY`   = `Friday`
    /// - `SYSVER`   = `9.4`
    /// - `SYSSCP`   = `LIN X64`
    pub fn new(deterministic: bool) -> Self {
        let mut engine = Self::default();
        engine.seed_automatic_vars(deterministic);
        engine
    }

    /// M19.2 — fixe le répertoire de base servant à résoudre les chemins
    /// relatifs de `%include 'fichier';` (cf. `Session::base_dir`).
    pub fn set_include_base_dir(&mut self, dir: std::path::PathBuf) {
        self.include_base_dir = dir;
    }

    /// M19.2 — fixe les répertoires de bibliothèques autocall (`SASAUTOS`).
    /// Une macro `%nom` non définie sera cherchée comme `nom.sas` dans ces
    /// répertoires, dans l'ordre (premier trouvé gagne).
    pub fn set_sasautos_path(&mut self, path: Vec<std::path::PathBuf>) {
        self.sasautos_path = path;
    }

    /// M35.2 — enregistre un `fileref` (statement global `FILENAME ref 'chemin';`).
    /// Le nom est stocké en MAJUSCULES (recherche insensible à la casse) ; le
    /// chemin doit être DÉJÀ résolu (cf. `Session::resolve_path`). Un
    /// ré-enregistrement écrase l'ancien chemin (dernier `FILENAME` gagne).
    pub fn set_fileref(&mut self, name: &str, path: std::path::PathBuf) {
        let key = name.to_uppercase();
        // Dernier `FILENAME` gagne : une assignation chemin remplace une
        // éventuelle assignation device antérieure du même nom (M38.5).
        self.fileref_devices.remove(&key);
        self.filerefs.insert(key, path);
    }

    /// M38.5 — enregistre un `fileref` assigné à un DEVICE non-fichier
    /// (statement global `FILENAME ref PIPE|URL|TEMP|… ;`). Le nom et le device
    /// sont stockés en MAJUSCULES ; une assignation device remplace une
    /// éventuelle assignation chemin antérieure du même nom (dernier `FILENAME`
    /// gagne). Consulté par `%include fileref;` pour un diagnostic fidèle.
    pub fn set_fileref_device(&mut self, name: &str, device: &str) {
        let key = name.to_uppercase();
        self.filerefs.remove(&key);
        self.fileref_devices.insert(key, device.to_uppercase());
    }

    /// M38.5 — device associé à un `fileref` (recherche insensible à la casse),
    /// ou `None` si le fileref n'est pas assigné à un device. Consulté par
    /// `%include fileref;` AVANT la résolution en chemin.
    pub(super) fn fileref_device(&self, name: &str) -> Option<&str> {
        self.fileref_devices
            .get(&name.to_uppercase())
            .map(String::as_str)
    }

    /// M35.2 — chemin associé à un `fileref` (recherche insensible à la casse),
    /// ou `None` si le fileref n'est pas enregistré. Consulté par
    /// `%include fileref;`.
    pub(super) fn fileref_path(&self, name: &str) -> Option<&std::path::PathBuf> {
        self.filerefs.get(&name.to_uppercase())
    }

    /// M19.3 — active/désactive l'option de trace `MPRINT` (écho du code
    /// produit par l'expansion macro). OFF par défaut.
    pub fn set_mprint(&mut self, on: bool) {
        self.trace.mprint = on;
    }

    /// M19.3 — active/désactive l'option de trace `MLOGIC` (écho des décisions
    /// d'exécution du processeur macro). OFF par défaut.
    pub fn set_mlogic(&mut self, on: bool) {
        self.trace.mlogic = on;
    }

    /// M19.3 — active/désactive l'option de trace `SYMBOLGEN` (écho de chaque
    /// résolution `&symbol`). OFF par défaut.
    pub fn set_symbolgen(&mut self, on: bool) {
        self.trace.symbolgen = on;
    }

    /// M19.3 — état courant des options de trace (lecture).
    pub fn mprint(&self) -> bool {
        self.trace.mprint
    }
    pub fn mlogic(&self) -> bool {
        self.trace.mlogic
    }
    pub fn symbolgen(&self) -> bool {
        self.trace.symbolgen
    }

    /// M19.3 — draine les lignes de log accumulées pendant l'expansion (écho
    /// MPRINT/MLOGIC/SYMBOLGEN et sortie de `%put`). L'exécuteur les transfère
    /// vers le `LogWriter` après chaque `expand_open_code`.
    pub fn take_pending_log_lines(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending.log_lines)
    }

    /// M19.3 — draine les fragments de code mis en file par `%call execute(...)`
    /// en code macro, à exécuter après le segment courant.
    pub fn take_pending_call_execute(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending.call_execute)
    }

    /// M35.4 — draine une éventuelle demande d'`%abort` rencontrée pendant
    /// l'expansion du dernier segment, et remet à zéro l'état d'abort. L'exécuteur
    /// peut consulter ce résultat après chaque `expand_open_code` pour arrêter
    /// proprement la soumission. Rend `None` si aucun `%abort` n'a été vu.
    pub fn take_abort_request(&mut self) -> Option<AbortKind> {
        self.flow.abort_requested = false;
        self.flow.abort_kind.take()
    }

    /// M19.3 — écho d'une ligne de log (helper interne). On la pousse dans le
    /// tampon ; l'exécuteur la relaiera au `LogWriter`.
    fn log_line(&mut self, line: impl Into<String>) {
        self.pending.log_lines.push(line.into());
    }

    /// M19.3 — étiquette de macro courante pour MPRINT/MLOGIC : nom de la macro
    /// la plus interne en cours d'expansion, ou chaîne vide en code ouvert.
    fn current_macro_label(&self) -> String {
        self.macro_stack.last().cloned().unwrap_or_default()
    }

    /// Expanse un segment de "open code" (texte SAS hors corps de `%macro`).
    ///
    /// Applique le `%let`/`&var`/`%macro`/… Pour un segment SANS déclencheur
    /// macro (`%`/`&`) le fast-path renvoie l'entrée inchangée — c'est
    /// l'invariant byte-identical pour le source macro-free.
    pub fn expand_open_code(&mut self, raw: &str) -> String {
        // Fast-path identité : sans déclencheur macro, rien à expanser. Garantit
        // l'invariant byte-identical pour le source sans tokens macro.
        if !raw.contains('%') && !raw.contains('&') {
            return raw.to_string();
        }
        let expanded = self.process(raw);
        // Passe finale d'« unmask » : les sentinelles posées par `%str`/`%nrstr`
        // sont retransformées en leurs caractères littéraux d'origine.
        Self::unmask(&expanded)
    }
}

/// SPIKE M8 (feature `macros`) : processeur macro minimal `%let` / `&var`.
///
/// But : valider que la couture `TextStage` peut héberger le futur processeur
/// macro. Il ne s'agit PAS d'une implémentation complète — pas de
/// `%macro`/`%mend`, `%if`, `CALL SYMPUT`, fonctions macro ni quoting.
///
/// Comportement (une seule passe avant gauche→droite sur tout le source) :
/// - `%let <name> = <value>;` (insensible à la casse, espaces optionnels) :
///   la `value` va jusqu'au prochain `;` (les valeurs ne contiennent pas de
///   `;` dans ce spike). Les `&ref` du RHS sont résolus avec la table
///   COURANTE (SAS résout le RHS au moment du %let), puis on stocke
///   `name.to_uppercase() -> value.trim()` (SAS rogne les blancs de bord,
///   garde les blancs internes). Le `%let ...;` est consommé, y compris les
///   blancs en ligne (espaces/tabs) qui le suivent ; un éventuel `\n` final
///   juste après est préservé pour ne pas décaler la numérotation des lignes.
/// - `&name` ou `&name.` ailleurs : on cherche le nom EN MAJUSCULES ; si
///   trouvé on émet sa valeur (résolue itérativement, garde de récursion à
///   10 itérations) ; sinon on laisse `&name` verbatim (SAS warne et laisse).
///   Un `.` juste après le nom est le terminateur SAS et est CONSOMMÉ (UN
///   seul point) : `&lib.x` avec lib=work → `workx`. `&&` → un seul `&`.
/// - Tout autre caractère est émis tel quel. Un `&` non suivi d'un début de
///   nom (ex. ` & ` opérateur booléen) reste intact.
/// - Chaînes : ce spike résout `&x` PARTOUT (y compris dans les littéraux
///   simple/double quote). SAS ne résout pas dans `'...'`, mais on documente
///   ici qu'on simplifie — la résolution s'applique partout.
///
/// NB (M11.1) : la logique `%let`/`&var` du spike vit désormais sur
/// `MacroEngine` (cf. ci-dessus). `MacroStage` est conservé comme alias mince
/// implémentant `TextStage` afin que les tests de spike existants restent
/// inchangés ; il n'est plus utilisé par `lib.rs` / l'`executor`.
pub type MacroStage = MacroEngine;

impl TextStage for MacroEngine {
    fn process(&mut self, source: &str) -> String {
        self.process_impl(source)
    }
}

impl MacroEngine {
    /// Nombre maximal d'itérations de résolution d'une valeur contenant
    /// elle-même des `&refs` (garde contre les cycles).
    const MAX_RESOLVE_ITERS: usize = 10;
}

#[cfg(test)]
mod tests;
