//! Text-transformation stage that runs BEFORE the lexer.
//!
//! This is the architectural seam where the SAS macro processor will live
//! (phase M8+): `%let`, `&var` resolution, `%macro`/`%mend` expansion.
//! Today only the identity stage exists. The contract is incremental by
//! design: the executor feeds source to the stage and lexes the result
//! block by block, because macro execution can affect downstream source
//! (`%let` evaluated mid-program, `CALL SYMPUT`).

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
    /// M19.2 — noms (MAJUSCULES) de macros dont la recherche autocall a déjà
    /// été TENTÉE (trouvée ou non), pour éviter de relire/recompiler le disque
    /// à chaque invocation. Une fois compilée, la macro vit dans `macros`.
    autocall_tried: std::collections::HashSet<String>,
    /// M19.3 — option `MPRINT` : si vrai, chaque ligne de code produite par
    /// l'expansion d'une macro est écho­tée au log (préfixe `MPRINT(nom):`).
    /// OFF par défaut.
    mprint: bool,
    /// M19.3 — option `MLOGIC` : si vrai, les décisions d'exécution du
    /// processeur macro (entrée/sortie de macro, conditions `%if`, itérations
    /// `%do`) sont écho­tées au log (préfixe `MLOGIC(nom):`). OFF par défaut.
    mlogic: bool,
    /// M19.3 — option `SYMBOLGEN` : si vrai, chaque résolution `&symbol` est
    /// écho­tée au log (`SYMBOLGEN:  Macro variable X resolves to ...`). OFF par
    /// défaut.
    symbolgen: bool,
    /// M19.3 — tampon de lignes de log produites pendant l'expansion (écho
    /// MPRINT/MLOGIC/SYMBOLGEN et sortie de `%put`). L'engine n'a pas accès au
    /// `LogWriter` (emprunté ailleurs) ; il accumule ici et l'exécuteur draine
    /// après chaque `expand_open_code` via `take_pending_log_lines`.
    pending_log_lines: Vec<String>,
    /// M19.3 — file de fragments de code SAS produits par `%call execute(...)`
    /// en code macro, à exécuter APRÈS l'étape/segment courant (même sémantique
    /// que le `CALL EXECUTE` côté DATA step). Drainé par l'exécuteur via
    /// `take_pending_call_execute`.
    pending_call_execute: Vec<String>,
    /// M19.3 — pile des noms de macros en cours d'expansion, pour étiqueter les
    /// lignes `MPRINT(nom):` / `MLOGIC(nom):`. La macro la plus interne est en
    /// fin de pile. Vide en code ouvert.
    macro_stack: Vec<String>,
}

/// Définition d'une macro capturée par `%macro name(params); <body> %mend;`.
///
/// `body` est le texte VERBATIM entre le `;` qui clôt la liste de paramètres et
/// le `%mend` correspondant. Il n'est PAS expansé à la définition ; il l'est à
/// chaque invocation, dans la portée locale créée pour cet appel.
#[derive(Clone, Debug)]
pub struct MacroDef {
    /// Nom de la macro, stocké tel quel (la recherche se fait en MAJUSCULES).
    pub name: String,
    /// Paramètres déclarés, dans l'ordre (positionnels puis mots-clés en SAS,
    /// mais on stocke l'ordre déclaré tel quel).
    pub params: Vec<MacroParam>,
    /// Corps verbatim (non expansé) de la macro.
    pub body: String,
}

/// Un paramètre formel de macro.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MacroParam {
    /// Paramètre positionnel `p` (sans valeur par défaut ; défaut = chaîne vide).
    Positional(String),
    /// Paramètre mot-clé `kw=default`.
    Keyword { name: String, default: String },
}

/// Erreur d'évaluation d'une expression macro (`%eval`, conditions `%if`,
/// bornes `%to`/`%by`). Portée par la feature `macros`. On ne `panic` jamais
/// sur une entrée macro invalide : l'expanseur transforme cette erreur en une
/// note SAS-like émise dans le flux de sortie et poursuit le scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MacroError {
    /// Message lisible (proche du libellé SAS quand pertinent).
    pub message: String,
}

impl MacroError {
    fn new(msg: impl Into<String>) -> Self {
        MacroError { message: msg.into() }
    }
}

impl std::fmt::Display for MacroError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
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

    /// M19.3 — active/désactive l'option de trace `MPRINT` (écho du code
    /// produit par l'expansion macro). OFF par défaut.
    pub fn set_mprint(&mut self, on: bool) {
        self.mprint = on;
    }

    /// M19.3 — active/désactive l'option de trace `MLOGIC` (écho des décisions
    /// d'exécution du processeur macro). OFF par défaut.
    pub fn set_mlogic(&mut self, on: bool) {
        self.mlogic = on;
    }

    /// M19.3 — active/désactive l'option de trace `SYMBOLGEN` (écho de chaque
    /// résolution `&symbol`). OFF par défaut.
    pub fn set_symbolgen(&mut self, on: bool) {
        self.symbolgen = on;
    }

    /// M19.3 — état courant des options de trace (lecture).
    pub fn mprint(&self) -> bool {
        self.mprint
    }
    pub fn mlogic(&self) -> bool {
        self.mlogic
    }
    pub fn symbolgen(&self) -> bool {
        self.symbolgen
    }

    /// M19.3 — draine les lignes de log accumulées pendant l'expansion (écho
    /// MPRINT/MLOGIC/SYMBOLGEN et sortie de `%put`). L'exécuteur les transfère
    /// vers le `LogWriter` après chaque `expand_open_code`.
    pub fn take_pending_log_lines(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_log_lines)
    }

    /// M19.3 — draine les fragments de code mis en file par `%call execute(...)`
    /// en code macro, à exécuter après le segment courant.
    pub fn take_pending_call_execute(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_call_execute)
    }

    /// M19.3 — écho d'une ligne de log (helper interne). On la pousse dans le
    /// tampon ; l'exécuteur la relaiera au `LogWriter`.
    fn log_line(&mut self, line: impl Into<String>) {
        self.pending_log_lines.push(line.into());
    }

    /// M19.3 — étiquette de macro courante pour MPRINT/MLOGIC : nom de la macro
    /// la plus interne en cours d'expansion, ou chaîne vide en code ouvert.
    fn current_macro_label(&self) -> String {
        self.macro_stack
            .last()
            .cloned()
            .unwrap_or_default()
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

    /// Pose un symbole macro GLOBAL (sémantique `CALL SYMPUT` — M11.5) : le
    /// symbole est créé/écrasé dans la table globale, insensible casse.
    pub fn set_symbol_global(&mut self, name: &str, value: String) {
        self.table.insert(name.to_uppercase(), value);
    }

    /// Lit la valeur d'un symbole macro (pile de portées puis table globale,
    /// comme `&var`). `None` si indéfini.
    pub fn get_symbol(&self, name: &str) -> Option<String> {
        self.lookup(name)
    }

    /// Instantané (clés MAJUSCULES → valeur) de la table macro VISIBLE en
    /// open code, pour alimenter `SYMGET` (M11.5). On aplatit la pile de
    /// portées (plus interne d'abord) puis la table globale ; en open code la
    /// pile est vide, donc seule `table` contribue.
    /// Variables macro GLOBALES (table globale uniquement, hors portées
    /// locales), pour `DICTIONARY.MACROS` / `sashelp.vmacro` (M20.3). Clés en
    /// MAJUSCULES → valeur. Le classement scope GLOBAL/AUTOMATIC est laissé à
    /// l'appelant (cf. `sql::dictionary`).
    pub fn global_symbols(&self) -> std::collections::HashMap<String, String> {
        self.table.clone()
    }

    pub fn symbols_snapshot(&self) -> std::collections::HashMap<String, String> {
        let mut snap = self.table.clone();
        // La table globale est la base ; les portées locales (s'il y en a)
        // l'emportent. En open code, `scopes` est vide.
        for scope in &self.scopes {
            for (k, v) in scope {
                snap.insert(k.clone(), v.clone());
            }
        }
        snap
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

    /// Lit un nom SAS (lettre/`_` puis alnum/`_`) à partir de `chars[i]`.
    /// Rend `(nom, index après le nom)`, ou `None` si pas un nom valide.
    fn read_name(chars: &[char], i: usize) -> Option<(String, usize)> {
        let first = *chars.get(i)?;
        if !(first.is_ascii_alphabetic() || first == '_') {
            return None;
        }
        let mut j = i + 1;
        while let Some(&c) = chars.get(j) {
            if c.is_ascii_alphanumeric() || c == '_' {
                j += 1;
            } else {
                break;
            }
        }
        Some((chars[i..j].iter().collect(), j))
    }

    /// Profondeur maximale d'invocation de macro (garde anti-récursion).
    const MAX_MACRO_DEPTH: usize = 100;

    /// M19.2 — profondeur maximale d'imbrication des `%include` (garde contre
    /// les inclusions cycliques : un fichier qui s'inclut lui-même, ou un cycle
    /// A→B→A). Au-delà, l'inclusion est refusée avec une note SAS-like.
    const MAX_INCLUDE_DEPTH: usize = 50;

    /// Cherche un symbole macro par nom (insensible casse) : pile de portées du
    /// plus interne au plus externe, puis table globale. Rend la valeur si
    /// trouvée.
    fn lookup(&self, name: &str) -> Option<String> {
        let key = name.to_uppercase();
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(&key) {
                return Some(v.clone());
            }
        }
        self.table.get(&key).cloned()
    }

    /// Affecte une valeur à un symbole (sémantique `%let`) : met à jour la
    /// variable là où elle est DÉJÀ définie (pile du plus interne au plus
    /// externe, puis table) ; sinon la crée en global (`table`). Cf. la règle
    /// documentée dans l'en-tête de `MacroEngine`.
    fn assign(&mut self, name: &str, value: String) {
        let key = name.to_uppercase();
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(&key) {
                scope.insert(key, value);
                return;
            }
        }
        self.table.insert(key, value);
    }

    /// Résout récursivement (itérativement) les `&ref` d'une valeur en
    /// utilisant la table courante. Garde de récursion `MAX_RESOLVE_ITERS`.
    fn resolve_value(&self, value: &str) -> String {
        let mut current = value.to_string();
        for _ in 0..Self::MAX_RESOLVE_ITERS {
            if !current.contains('&') {
                break;
            }
            let next = self.resolve_refs_once(&current);
            if next == current {
                break;
            }
            current = next;
        }
        current
    }

    /// M19.3 — produit les lignes SYMBOLGEN pour un token `&...` (potentiellement
    /// indirect `&&v&i`). On résout l'indirection jusqu'à obtenir un (ou
    /// plusieurs) `&name` direct(s), puis on émet une ligne par variable
    /// effectivement consultée, façon SAS :
    /// `SYMBOLGEN:  Macro variable NAME resolves to VALUE`.
    /// Les variables indéfinies ne produisent pas de ligne (SAS warne ailleurs).
    fn symbolgen_trace(&mut self, run: &str) {
        // Réduit l'indirection : tant qu'il reste des `&&`, on résout une passe
        // (qui transforme `&&`→`&` et substitue les `&name` directs internes).
        let mut current = run.to_string();
        for _ in 0..Self::MAX_RESOLVE_ITERS {
            if !current.contains("&&") {
                break;
            }
            let next = self.resolve_refs_once(&current);
            if next == current {
                break;
            }
            current = next;
        }
        // À ce stade `current` ne contient plus que des `&name` directs.
        let chars: Vec<char> = current.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '&' {
                if let Some((name, after)) = Self::read_name(&chars, i + 1) {
                    if let Some(v) = self.lookup(&name) {
                        self.log_line(format!(
                            "SYMBOLGEN:  Macro variable {} resolves to {}",
                            name.to_uppercase(),
                            v
                        ));
                    }
                    i = after;
                    continue;
                }
            }
            i += 1;
        }
    }

    /// Une passe de résolution des `&ref` sur une chaîne, sans réinjection.
    fn resolve_refs_once(&self, text: &str) -> String {
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::with_capacity(text.len());
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '&' {
                // `&&` -> un seul `&`.
                if chars.get(i + 1) == Some(&'&') {
                    out.push('&');
                    i += 2;
                    continue;
                }
                if let Some((name, after)) = Self::read_name(&chars, i + 1) {
                    let mut next = after;
                    // Terminateur point : consommé qu'on résolve ou non.
                    if chars.get(next) == Some(&'.') {
                        next += 1;
                    }
                    match self.lookup(&name) {
                        Some(v) => out.push_str(&v),
                        None => {
                            // Non défini : on laisse `&name` verbatim. Le
                            // point terminateur a déjà été consommé.
                            out.push('&');
                            out.push_str(&name);
                        }
                    }
                    i = next;
                    continue;
                }
            }
            out.push(c);
            i += 1;
        }
        out
    }
}

impl MacroEngine {
    /// Coeur de l'expansion `%let`/`&var` (une passe gauche→droite). Met à jour
    /// la table de l'engine (état conservé entre appels — donc entre segments).
    fn process_impl(&mut self, source: &str) -> String {
        let chars: Vec<char> = source.chars().collect();
        let mut out = String::with_capacity(source.len());
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];

            // `%let` insensible à la casse.
            if c == '%' && Self::matches_let(&chars, i) {
                if let Some(next) = self.consume_let(&chars, i, &mut out) {
                    i = next;
                    continue;
                }
            }

            // `%put <texte>;` (M19.3) — écrit son argument (résolu) au log,
            // n'émet RIEN dans le flux de code.
            if c == '%' && Self::matches_kw(&chars, i, "put") {
                if let Some(next) = self.consume_put(&chars, i, &mut out) {
                    i = next;
                    continue;
                }
            }

            // `%call execute(text);` (M19.3) — met en file un fragment de code
            // SAS à exécuter APRÈS le segment courant (sémantique CALL EXECUTE).
            if c == '%' && Self::matches_kw(&chars, i, "call") {
                if let Some(next) = self.consume_macro_call(&chars, i, &mut out) {
                    i = next;
                    continue;
                }
            }

            // `%include 'chemin';` (M19.2) — charge le fichier, l'expanse
            // récursivement et splice le résultat À LA PLACE du statement,
            // AVANT de poursuivre le scan du segment courant.
            if c == '%' && Self::matches_kw(&chars, i, "include") {
                if let Some(next) = self.consume_include(&chars, i, &mut out) {
                    i = next;
                    continue;
                }
            }

            // `%macro name(params); body %mend;` — capture, n'émet rien.
            if c == '%' && Self::matches_kw(&chars, i, "macro") {
                if let Some(next) = self.consume_macro_def(&chars, i, &mut out) {
                    i = next;
                    continue;
                }
            }

            // `%local v1 v2;` / `%global v1 v2;`
            if c == '%' && Self::matches_kw(&chars, i, "local") {
                if let Some(next) = self.consume_scope_decl(&chars, i, true, &mut out) {
                    i = next;
                    continue;
                }
            }
            if c == '%' && Self::matches_kw(&chars, i, "global") {
                if let Some(next) = self.consume_scope_decl(&chars, i, false, &mut out) {
                    i = next;
                    continue;
                }
            }

            // `%eval(expr)` — évalue et splice le résultat entier.
            if c == '%' && Self::matches_kw_paren(&chars, i, "eval") {
                if let Some(next) = self.consume_eval(&chars, i, &mut out) {
                    i = next;
                    continue;
                }
            }

            // `%sysfunc(func(args))` / `%qsysfunc(...)` — appelle la fonction
            // DATA-step et splice le résultat formaté en texte.
            if c == '%' && Self::matches_kw_paren(&chars, i, "sysfunc") {
                if let Some(next) = self.consume_sysfunc(&chars, "sysfunc", i, &mut out) {
                    i = next;
                    continue;
                }
            }
            if c == '%' && Self::matches_kw_paren(&chars, i, "qsysfunc") {
                if let Some(next) = self.consume_sysfunc(&chars, "qsysfunc", i, &mut out) {
                    i = next;
                    continue;
                }
            }

            // `%sysevalf(expr [, conv])` — évaluation FLOTTANTE (M19.1).
            if c == '%' && Self::matches_kw_paren(&chars, i, "sysevalf") {
                if let Some(next) = self.consume_sysevalf(&chars, i, &mut out) {
                    i = next;
                    continue;
                }
            }

            // `%cmpres(text)` / `%qcmpres(text)` — compression des blancs (M19.1).
            if c == '%' && Self::matches_kw_paren(&chars, i, "qcmpres") {
                if let Some(next) = self.consume_cmpres(&chars, "qcmpres", true, i, &mut out) {
                    i = next;
                    continue;
                }
            }
            if c == '%' && Self::matches_kw_paren(&chars, i, "cmpres") {
                if let Some(next) = self.consume_cmpres(&chars, "cmpres", false, i, &mut out) {
                    i = next;
                    continue;
                }
            }

            // `%symexist(name)` / `%sysmexist(name)` / `%sysget(name)` (M19.1).
            if c == '%' && Self::matches_kw_paren(&chars, i, "symexist") {
                if let Some(next) = self.consume_symexist(&chars, i, &mut out) {
                    i = next;
                    continue;
                }
            }
            if c == '%' && Self::matches_kw_paren(&chars, i, "sysmexist") {
                if let Some(next) = self.consume_sysmexist(&chars, i, &mut out) {
                    i = next;
                    continue;
                }
            }
            if c == '%' && Self::matches_kw_paren(&chars, i, "sysget") {
                if let Some(next) = self.consume_sysget(&chars, i, &mut out) {
                    i = next;
                    continue;
                }
            }

            // `%unquote(text)` — ré-active la résolution `&`/`%` masquée (M19.1).
            if c == '%' && Self::matches_kw_paren(&chars, i, "unquote") {
                if let Some(next) = self.consume_unquote(&chars, i, &mut out) {
                    i = next;
                    continue;
                }
            }

            // `%superq(name)` — valeur d'une variable SANS résolution, masquée.
            if c == '%' && Self::matches_kw_paren(&chars, i, "superq") {
                if let Some(next) = self.consume_superq(&chars, i, &mut out) {
                    i = next;
                    continue;
                }
            }

            // `%bquote(text)` / `%nrbquote(text)` — résout puis masque.
            if c == '%' && Self::matches_kw_paren(&chars, i, "nrbquote") {
                if let Some(next) = self.consume_bquote(&chars, i, "nrbquote", true, &mut out) {
                    i = next;
                    continue;
                }
            }
            if c == '%' && Self::matches_kw_paren(&chars, i, "bquote") {
                if let Some(next) = self.consume_bquote(&chars, i, "bquote", false, &mut out) {
                    i = next;
                    continue;
                }
            }

            // Fonctions chaîne macro simples et leurs variantes `%q*` (résultat
            // masqué). On teste les `%q*` AVANT leurs versions nues : grâce à la
            // frontière de mot de `matches_kw_paren` il n'y a pas de collision,
            // mais l'ordre garde l'intention explicite. `consumed_macro_fn`
            // permet de relancer la boucle `while` externe via `continue`.
            if c == '%' {
                let mut handled = false;
                for (kw, masked) in [
                    ("qupcase", true),
                    ("upcase", false),
                    ("qlowcase", true),
                    ("lowcase", false),
                    ("qsubstr", true),
                    ("substr", false),
                    ("qscan", true),
                    ("scan", false),
                    ("index", false),
                    ("length", false),
                ] {
                    if Self::matches_kw_paren(&chars, i, kw) {
                        if let Some(next) = self.consume_macro_fn(&chars, i, kw, masked, &mut out) {
                            i = next;
                            handled = true;
                            break;
                        }
                    }
                }
                if handled {
                    continue;
                }
            }

            // `%str(...)` / `%nrstr(...)` — masquage des caractères spéciaux.
            if c == '%' && Self::matches_kw_paren(&chars, i, "str") {
                if let Some(next) = self.consume_quote(&chars, i, "str", false, &mut out) {
                    i = next;
                    continue;
                }
            }
            if c == '%' && Self::matches_kw_paren(&chars, i, "nrstr") {
                if let Some(next) = self.consume_quote(&chars, i, "nrstr", true, &mut out) {
                    i = next;
                    continue;
                }
            }

            // `%if <cond> %then <action>; [%else <action>;]`
            if c == '%' && Self::matches_kw(&chars, i, "if") {
                if let Some(next) = self.consume_if(&chars, i, &mut out) {
                    i = next;
                    continue;
                }
            }

            // `%do ...` (plain group ou itératif `%do i=a %to b`).
            if c == '%' && Self::matches_kw(&chars, i, "do") {
                if let Some(next) = self.consume_do(&chars, i, &mut out) {
                    i = next;
                    continue;
                }
            }

            // Invocation `%name` ou `%name(args)` d'une macro DÉFINIE — ou, à
            // défaut, chargée paresseusement depuis une bibliothèque autocall
            // (`SASAUTOS`, M19.2). `try_autocall` compile `nom.sas` au premier
            // appel (idempotent via `autocall_tried`).
            if c == '%' {
                if let Some((name, after)) = Self::read_name(&chars, i + 1) {
                    let key = name.to_uppercase();
                    if !self.macros.contains_key(&key) {
                        self.try_autocall(&name);
                    }
                    if self.macros.contains_key(&key) {
                        let next = self.expand_invocation(&chars, i + 1, &name, after, &mut out);
                        i = next;
                        continue;
                    }
                }
            }

            if c == '&' {
                // Indirection imbriquée `&&&x` / `&&var&i` (M11.6).
                //
                // On capture le run complet de `&` en tête, puis le nom et un
                // unique point terminateur, et on confie la résolution à
                // `resolve_value` (multi-passes vers point fixe : chaque passe
                // transforme `&&`→`&` et résout `&name`, jusqu'à `MAX_RESOLVE_ITERS`).
                let amp_start = i;
                let mut k = i;
                while chars.get(k) == Some(&'&') {
                    k += 1;
                }
                if let Some((_name, after)) = Self::read_name(&chars, k) {
                    // Étendre le token tant qu'on enchaîne `&`/nom sans rupture
                    // (`&&v&i` est UN seul token d'indirection). On consomme
                    // aussi un unique point terminateur à la toute fin.
                    let mut next = after;
                    loop {
                        if chars.get(next) == Some(&'&') {
                            let mut m = next;
                            while chars.get(m) == Some(&'&') {
                                m += 1;
                            }
                            if let Some((_, a2)) = Self::read_name(&chars, m) {
                                next = a2;
                                continue;
                            }
                        }
                        break;
                    }
                    if chars.get(next) == Some(&'.') {
                        next += 1;
                    }
                    let run: String = chars[amp_start..next].iter().collect();
                    // M19.3 — SYMBOLGEN : écho de chaque résolution `&symbol`.
                    // On trace la résolution finale au point fixe de la chaîne
                    // (pour `&&v&i` l'indirection est résolue avant l'écho :
                    // SAS trace alors la variable réellement consultée).
                    if self.symbolgen {
                        self.symbolgen_trace(&run);
                    }
                    let resolved = self.resolve_value(&run);
                    out.push_str(&resolved);
                    i = next;
                    continue;
                }
                // `&` non suivi (in fine) d'un nom : `&&` seul -> un `&` ; sinon
                // `&` brut (opérateur booléen) laissé tel quel.
                if chars.get(i + 1) == Some(&'&') {
                    out.push('&');
                    i += 2;
                    continue;
                }
            }

            out.push(c);
            i += 1;
        }
        out
    }
}

impl MacroEngine {
    /// Vrai si `chars[i..]` commence par `%let` (insensible casse) suivi d'un
    /// blanc (pour ne pas matcher `%letx`).
    fn matches_let(chars: &[char], i: usize) -> bool {
        let kw = ['%', 'l', 'e', 't'];
        for (k, &kc) in kw.iter().enumerate() {
            match chars.get(i + k) {
                Some(c) if c.to_ascii_lowercase() == kc => {}
                _ => return false,
            }
        }
        matches!(chars.get(i + 4), Some(c) if c.is_whitespace())
    }

    /// Consomme un `%let name = value ;` complet à partir de `i` et met la
    /// table à jour. Rend l'index après le `;` (et préserve un `\n` final).
    /// Rend `None` si la syntaxe ne tient pas (on laisse alors le `%` brut).
    fn consume_let(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let mut j = i + 4; // après `%let`
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        let (name, after_name) = Self::read_name(chars, j)?;
        j = after_name;
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) != Some(&'=') {
            return None;
        }
        j += 1;
        // Valeur = tout jusqu'au prochain `;`, mais un `%str(...)`/`%nrstr(...)`
        // masque son `;` interne (M11.6) : on saute ces régions à parenthèses
        // équilibrées pour ne pas terminer le `%let` prématurément.
        let val_start = j;
        while j < chars.len() && chars[j] != ';' {
            if chars[j] == '%'
                && (Self::matches_kw_paren(chars, j, "str")
                    || Self::matches_kw_paren(chars, j, "nrstr")
                    || Self::matches_kw_paren(chars, j, "bquote")
                    || Self::matches_kw_paren(chars, j, "nrbquote")
                    || Self::matches_kw_paren(chars, j, "superq")
                    || Self::matches_kw_paren(chars, j, "cmpres")
                    || Self::matches_kw_paren(chars, j, "qcmpres")
                    || Self::matches_kw_paren(chars, j, "unquote")
                    || Self::matches_kw_paren(chars, j, "sysevalf"))
            {
                // Avancer jusqu'à la `(` puis sauter la région équilibrée.
                let mut p = j + 1;
                while matches!(chars.get(p), Some(ch) if *ch != '(') {
                    p += 1;
                }
                if let Some((_, after)) = Self::read_balanced_parens(chars, p) {
                    j = after;
                    continue;
                }
            }
            j += 1;
        }
        if chars.get(j) != Some(&';') {
            return None; // pas de `;` terminal : abandon, on n'avale rien.
        }
        let raw_value: String = chars[val_start..j].iter().collect();
        // Si la valeur contient un déclencheur de fonction macro (`%`), la
        // ré-expanser entièrement (gère `%str`/`%nrstr`/`%sysfunc`/`%eval`) ;
        // sinon, simple résolution des `&refs` (comportement historique).
        let resolved = if raw_value.contains('%') {
            self.process_impl(&raw_value)
        } else {
            self.resolve_value(&raw_value)
        };
        self.assign(&name, resolved.trim().to_string());
        j += 1; // après le `;`
        // Le `%let ...;` est consommé entièrement, y compris les blancs
        // *en ligne* (espaces/tabs) qui le suivent, pour ne pas laisser de
        // résidu entre deux instructions sur la même ligne. Un éventuel
        // `\n` final est préservé afin de garder la numérotation des lignes.
        while matches!(chars.get(j), Some(c) if *c == ' ' || *c == '\t') {
            j += 1;
        }
        if chars.get(j) == Some(&'\n') {
            out.push('\n');
            j += 1;
        }
        Some(j)
    }
}

impl MacroEngine {
    /// M19.2 — consomme un `%include 'chemin';` à partir de `i` (qui pointe sur
    /// le `%`), charge le fichier référencé, l'expanse RÉCURSIVEMENT (les
    /// `%macro` qu'il définit s'enregistrent dans l'état vivant de l'engine, et
    /// son code ouvert est émis) et splice le résultat dans `out` À LA PLACE du
    /// statement. Rend l'index APRÈS le `;` (un `\n` final est préservé pour la
    /// numérotation), ou `None` si la syntaxe ne tient pas (le `%` est alors
    /// laissé brut).
    ///
    /// # Formes reconnues
    /// - `%include 'chemin';` / `%include "chemin";` : littéral entre guillemets.
    ///   Le chemin est résolu via `include_base_dir` (relatif) ou tel quel
    ///   (absolu).
    ///
    /// # Cas d'erreur (jamais de `panic`)
    /// - profondeur d'inclusion > `MAX_INCLUDE_DEPTH` (cycle présumé) → un
    ///   commentaire de note SAS-like est émis, le statement est consommé ;
    /// - fichier illisible/absent → idem (commentaire d'erreur) ;
    /// - `%include` sans guillemets (ex. `%include fileref;` ou `*` / stdin) →
    ///   non supporté ici : un commentaire de note est émis et le statement
    ///   consommé jusqu'au `;`.
    fn consume_include(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let mut j = i + "%include".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        // Chemin entre guillemets simples ou doubles.
        let quote = match chars.get(j) {
            Some(&q @ ('\'' | '"')) => q,
            _ => {
                // Forme non supportée (fileref nu, `*` stdin) : consommer jusqu'au
                // `;` et émettre une note plutôt que de laisser un résidu.
                let mut k = j;
                while k < chars.len() && chars[k] != ';' {
                    k += 1;
                }
                if chars.get(k) != Some(&';') {
                    return None;
                }
                out.push_str(
                    "/* %include: only quoted file paths are supported (fileref/stdin deferred) */",
                );
                return Some(Self::skip_trailing_newline(chars, k + 1, out));
            }
        };
        j += 1; // après le guillemet ouvrant
        let path_start = j;
        while j < chars.len() && chars[j] != quote {
            j += 1;
        }
        if chars.get(j) != Some(&quote) {
            return None; // guillemet non fermé
        }
        let path: String = chars[path_start..j].iter().collect();
        j += 1; // après le guillemet fermant
                // Consommer le reste jusqu'au `;` terminal (options éventuelles ignorées).
        while j < chars.len() && chars[j] != ';' {
            j += 1;
        }
        if chars.get(j) != Some(&';') {
            return None;
        }
        let resume = Self::skip_trailing_newline(chars, j + 1, out);

        // Garde contre les inclusions cycliques (profondeur max).
        if self.include_depth >= Self::MAX_INCLUDE_DEPTH {
            out.push_str(&format!(
                "/* %include nesting limit ({}) reached for '{}' */",
                Self::MAX_INCLUDE_DEPTH,
                path
            ));
            return Some(resume);
        }

        // Résolution du chemin : absolu → tel quel ; relatif → joint à la base.
        let resolved = {
            let p = std::path::PathBuf::from(&path);
            if p.is_absolute() {
                p
            } else {
                self.include_base_dir.join(p)
            }
        };
        let contents = match std::fs::read_to_string(&resolved) {
            Ok(text) => text,
            Err(e) => {
                out.push_str(&format!(
                    "/* %include: cannot read '{}': {} */",
                    path, e
                ));
                return Some(resume);
            }
        };

        // Expansion récursive du fichier inclus avec l'état VIVANT de l'engine.
        self.include_depth += 1;
        let expanded = self.process_impl(&contents);
        self.include_depth -= 1;
        out.push_str(&expanded);
        Some(resume)
    }

    /// M19.3 — consomme un `%put <texte>;` à partir de `i` (sur le `%`). Le
    /// texte va jusqu'au prochain `;` de niveau supérieur (en sautant les
    /// régions à parenthèses équilibrées des fonctions macro, pour ne pas
    /// couper sur un `;` interne). Il est résolu (`&var` + `%function`) puis
    /// écrit AU LOG via le tampon `pending_log_lines` — `%put` n'émet RIEN dans
    /// le flux de code. Rend l'index après le `;` (un `\n` final préservé).
    ///
    /// Conformément à SAS, `%put;` (sans argument) écrit une ligne vide.
    fn consume_put(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let mut j = i + "%put".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        let arg_start = j;
        // Texte jusqu'au `;` de niveau 0 (parenthèses équilibrées sautées).
        let mut paren = 0i32;
        while j < chars.len() {
            let ch = chars[j];
            if ch == '(' {
                paren += 1;
            } else if ch == ')' {
                if paren > 0 {
                    paren -= 1;
                }
            } else if ch == ';' && paren == 0 {
                break;
            }
            j += 1;
        }
        if chars.get(j) != Some(&';') {
            return None; // pas de `;` terminal : abandon, on ne consomme rien.
        }
        let raw: String = chars[arg_start..j].iter().collect();
        j += 1; // après le `;`
        // Résolution immédiate (interprétation des `&var` et `%function`).
        let resolved = if raw.contains('%') {
            // Ré-expansion complète (gère `%upcase`, `%sysfunc`, etc.), puis
            // dé-masquage des sentinelles `%str`/`%nrstr`.
            Self::unmask(&self.process_impl(&raw))
        } else if raw.contains('&') {
            Self::unmask(&self.resolve_value(&raw))
        } else {
            Self::unmask(&raw)
        };
        // SAS rogne le blanc de tête laissé après `%put` ; le reste est verbatim.
        self.log_line(resolved.trim_end().to_string());
        Some(Self::skip_trailing_newline(chars, j, out))
    }

    /// M19.3 — consomme un `%call <routine>(args);` à partir de `i` (sur le
    /// `%`). Seul `%call execute(text)` est interprété : le texte (résolu) est
    /// mis en file dans `pending_call_execute` pour exécution APRÈS le segment
    /// courant, comme le `CALL EXECUTE` côté DATA step. Les autres routines
    /// sont consommées sans effet (note SAS-like). N'émet RIEN dans le flux.
    /// Rend l'index après le `;` (un `\n` final préservé), ou `None` si la
    /// syntaxe ne tient pas.
    fn consume_macro_call(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let mut j = i + "%call".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        let (routine, after_name) = Self::read_name(chars, j)?;
        j = after_name;
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) != Some(&'(') {
            return None;
        }
        let (inner, after_paren) = Self::read_balanced_parens(chars, j)?;
        j = after_paren;
        // Consommer un `;` terminal optionnel.
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) == Some(&';') {
            j += 1;
        }
        if routine.eq_ignore_ascii_case("execute") {
            // L'argument est résolu (macro + symboles) puis mis en file.
            let code = if inner.contains('%') {
                Self::unmask(&self.process_impl(&inner))
            } else if inner.contains('&') {
                Self::unmask(&self.resolve_value(&inner))
            } else {
                Self::unmask(&inner)
            };
            self.pending_call_execute.push(code);
        } else {
            out.push_str(&format!(
                "/* %call {}: only EXECUTE is supported in macro code */",
                routine
            ));
        }
        Some(Self::skip_trailing_newline(chars, j, out))
    }

    /// Préserve un éventuel `\n` immédiatement après l'index `j` (poussé dans
    /// `out`) afin de conserver la numérotation des lignes, comme le font les
    /// autres `consume_*`. Rend l'index après ce `\n` (ou `j` inchangé).
    fn skip_trailing_newline(chars: &[char], mut j: usize, out: &mut String) -> usize {
        while matches!(chars.get(j), Some(c) if *c == ' ' || *c == '\t') {
            j += 1;
        }
        if chars.get(j) == Some(&'\n') {
            out.push('\n');
            j += 1;
        }
        j
    }

    /// M19.2 — chargement paresseux d'une macro autocall (`SASAUTOS`).
    ///
    /// Appelé à l'expansion de `%nom(...)` lorsque `nom` n'est PAS encore défini
    /// dans `self.macros`. Cherche `nom.sas` (nom en minuscules) dans chaque
    /// répertoire de `sasautos_path` (premier trouvé gagne), lit le fichier et
    /// l'expanse via `process_impl` (ce qui ENREGISTRE la `%macro` qu'il
    /// contient ; toute sortie de code ouvert du fichier est ignorée — un
    /// fichier autocall ne doit définir que la macro). La tentative est mémoïsée
    /// dans `autocall_tried` (trouvée ou non), pour ne pas relire le disque à
    /// chaque appel suivant.
    fn try_autocall(&mut self, name: &str) {
        let key = name.to_uppercase();
        if self.autocall_tried.contains(&key) {
            return;
        }
        self.autocall_tried.insert(key);
        if self.sasautos_path.is_empty() {
            return;
        }
        let filename = format!("{}.sas", name.to_lowercase());
        // Premier fichier lisible gagne. On lit AVANT d'appeler `process_impl`
        // pour ne pas garder `self.sasautos_path` emprunté pendant l'expansion
        // (qui prend `&mut self`).
        let mut found: Option<String> = None;
        for dir in &self.sasautos_path {
            let candidate = dir.join(&filename);
            if let Ok(contents) = std::fs::read_to_string(&candidate) {
                found = Some(contents);
                break;
            }
        }
        if let Some(contents) = found {
            // Compilation : on expanse le fichier (qui enregistre la `%macro`)
            // et on jette la sortie de code ouvert.
            let _ = self.process_impl(&contents);
        }
    }
}

impl MacroEngine {
    /// Vrai si `chars[i..]` commence par `%<kw>` (insensible casse) suivi d'un
    /// non-identifiant (blanc, `(`, `;` ...). Évite de matcher `%macrox`.
    fn matches_kw(chars: &[char], i: usize, kw: &str) -> bool {
        if chars.get(i) != Some(&'%') {
            return false;
        }
        let kwc: Vec<char> = kw.chars().collect();
        for (k, &kc) in kwc.iter().enumerate() {
            match chars.get(i + 1 + k) {
                Some(c) if c.to_ascii_lowercase() == kc => {}
                _ => return false,
            }
        }
        // Le caractère suivant ne doit pas continuer un identifiant.
        match chars.get(i + 1 + kwc.len()) {
            Some(c) if c.is_ascii_alphanumeric() || *c == '_' => false,
            _ => true,
        }
    }

    /// Consomme une définition `%macro name[(params)] ; <body> %mend [name];`.
    /// Enregistre la définition et n'émet RIEN. Rend l'index après le `;` du
    /// `%mend` (un `\n` final juste après est préservé pour la numérotation),
    /// ou `None` si la syntaxe ne tient pas (le `%` est alors laissé brut).
    fn consume_macro_def(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let mut j = i + "%macro".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        let (name, after_name) = Self::read_name(chars, j)?;
        j = after_name;
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        // Liste de paramètres optionnelle.
        let mut params = Vec::new();
        if chars.get(j) == Some(&'(') {
            let (parsed, after_paren) = Self::parse_param_list(chars, j)?;
            params = parsed;
            j = after_paren;
            while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
                j += 1;
            }
        }
        // `;` qui clôt l'en-tête.
        if chars.get(j) != Some(&';') {
            return None;
        }
        j += 1;
        // Corps verbatim jusqu'au `%mend` (au niveau courant). On scanne en
        // suivant les `%macro`/`%mend` imbriqués pour équilibrer.
        let body_start = j;
        let mut depth = 1usize;
        let mut body_end = None;
        while j < chars.len() {
            if chars[j] == '%' {
                if Self::matches_kw(chars, j, "macro") {
                    depth += 1;
                    j += "%macro".len();
                    continue;
                }
                if Self::matches_kw(chars, j, "mend") {
                    depth -= 1;
                    if depth == 0 {
                        body_end = Some(j);
                        break;
                    }
                    j += "%mend".len();
                    continue;
                }
            }
            j += 1;
        }
        let body_end = body_end?; // pas de `%mend` : abandon.
        // Corps capturé verbatim, puis trim des blancs de bord. SAS conserve
        // les blancs internes du corps ; on rogne uniquement les bords pour que
        // `%macro p; x=1; %mend; %p` n'introduise pas d'espaces parasites.
        let body: String = chars[body_start..body_end]
            .iter()
            .collect::<String>()
            .trim()
            .to_string();
        // Avancer après `%mend [name] ;`.
        j = body_end + "%mend".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        // Nom optionnel après %mend.
        if let Some((_, after)) = Self::read_name(chars, j) {
            j = after;
        }
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) == Some(&';') {
            j += 1;
        }
        // Consommer les blancs en ligne suivants ; préserver un `\n` final.
        while matches!(chars.get(j), Some(c) if *c == ' ' || *c == '\t') {
            j += 1;
        }
        if chars.get(j) == Some(&'\n') {
            out.push('\n');
            j += 1;
        }
        self.macros.insert(
            name.to_uppercase(),
            MacroDef {
                name,
                params,
                body,
            },
        );
        Some(j)
    }

    /// Parse une liste de paramètres `(p1, p2, kw=def, ...)` à partir de `(`.
    /// Rend `(params, index après `)`)`. `None` si pas de `)` fermant.
    fn parse_param_list(chars: &[char], lparen: usize) -> Option<(Vec<MacroParam>, usize)> {
        let mut j = lparen + 1;
        let mut params = Vec::new();
        loop {
            while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
                j += 1;
            }
            if chars.get(j) == Some(&')') {
                return Some((params, j + 1));
            }
            // Nom du paramètre.
            let (name, after) = Self::read_name(chars, j)?;
            j = after;
            while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
                j += 1;
            }
            if chars.get(j) == Some(&'=') {
                // Mot-clé avec défaut jusqu'à `,` ou `)`.
                j += 1;
                let start = j;
                while j < chars.len() && chars[j] != ',' && chars[j] != ')' {
                    j += 1;
                }
                let default: String = chars[start..j].iter().collect();
                params.push(MacroParam::Keyword {
                    name,
                    default: default.trim().to_string(),
                });
            } else {
                params.push(MacroParam::Positional(name));
            }
            while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
                j += 1;
            }
            match chars.get(j) {
                Some(&',') => {
                    j += 1;
                    continue;
                }
                Some(&')') => return Some((params, j + 1)),
                _ => return None,
            }
        }
    }

    /// Consomme `%local v1 v2 ...;` (si `is_local`) ou `%global v1 v2 ...;`.
    /// Crée les variables (vides) dans la portée appropriée. Rend l'index après
    /// le `;`, ou `None` si pas de `;`.
    fn consume_scope_decl(
        &mut self,
        chars: &[char],
        i: usize,
        is_local: bool,
        out: &mut String,
    ) -> Option<usize> {
        let kwlen = if is_local { "%local".len() } else { "%global".len() };
        let mut j = i + kwlen;
        let mut names = Vec::new();
        loop {
            while matches!(chars.get(j), Some(c) if c.is_whitespace() || *c == ',') {
                j += 1;
            }
            if chars.get(j) == Some(&';') {
                j += 1;
                break;
            }
            match Self::read_name(chars, j) {
                Some((name, after)) => {
                    names.push(name);
                    j = after;
                }
                None => return None, // syntaxe inattendue : laisser brut.
            }
        }
        // Comme `%let` : absorber les blancs en ligne suivants, préserver un
        // `\n` final pour la numérotation des lignes.
        while matches!(chars.get(j), Some(c) if *c == ' ' || *c == '\t') {
            j += 1;
        }
        if chars.get(j) == Some(&'\n') {
            out.push('\n');
            j += 1;
        }
        for name in names {
            let key = name.to_uppercase();
            if is_local {
                if let Some(scope) = self.scopes.last_mut() {
                    scope.entry(key).or_default();
                } else {
                    // `%local` hors d'une macro : SAS l'ignore ~ ; on retombe
                    // sur global pour rester simple.
                    self.table.entry(key).or_default();
                }
            } else {
                self.table.entry(key).or_default();
            }
        }
        Some(j)
    }

    /// Expanse une invocation `%name[(args)]`. `name_start` pointe sur le nom
    /// (après le `%`), `after_name` sur le premier caractère après le nom.
    /// Lie les arguments, empile une portée, ré-expanse le corps, insère le
    /// résultat dans `out`, dépile. Rend l'index de reprise du scan.
    fn expand_invocation(
        &mut self,
        chars: &[char],
        _name_start: usize,
        name: &str,
        after_name: usize,
        out: &mut String,
    ) -> usize {
        // Parse des arguments éventuels.
        let mut j = after_name;
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        let (pos_args, kw_args, resume) = if chars.get(j) == Some(&'(') {
            match Self::parse_arg_list(chars, j) {
                Some(v) => v,
                None => {
                    // Parenthèse non fermée : émettre `%name` verbatim, ne rien
                    // consommer de plus.
                    out.push('%');
                    out.push_str(name);
                    return after_name;
                }
            }
        } else {
            // Appel sans parenthèses : consommer un `;` immédiat optionnel
            // (SAS termine l'appel macro par `;` qui n'est pas réémis).
            let mut r = after_name;
            if chars.get(r) == Some(&';') {
                r += 1;
            }
            (Vec::new(), Vec::new(), r)
        };

        // Garde de récursion.
        if self.depth >= Self::MAX_MACRO_DEPTH {
            out.push_str(&format!(
                "/* macro recursion limit ({}) reached for %{} */",
                Self::MAX_MACRO_DEPTH,
                name
            ));
            return resume;
        }

        let def = self.macros.get(&name.to_uppercase()).cloned();
        let def = match def {
            Some(d) => d,
            None => {
                // Ne devrait pas arriver (vérifié par l'appelant) ; sûreté.
                out.push('%');
                out.push_str(name);
                return after_name;
            }
        };

        // Les valeurs d'arguments sont résolues dans la portée APPELANTE (SAS
        // évalue les arguments au moment de l'appel) avant la liaison.
        let pos_args: Vec<String> = pos_args.iter().map(|a| self.resolve_value(a)).collect();
        let kw_args: Vec<(String, String)> = kw_args
            .iter()
            .map(|(k, v)| (k.clone(), self.resolve_value(v)))
            .collect();

        // Liaison des paramètres -> portée locale.
        let scope = Self::bind_params(&def.params, &pos_args, &kw_args);
        let label = name.to_uppercase();
        // M19.3 — MLOGIC : décision d'entrée de macro.
        if self.mlogic {
            self.log_line(format!("MLOGIC({label}):  Beginning execution."));
            // Écho de la valeur reçue par chaque paramètre (façon SAS).
            for param in &def.params {
                let pname = match param {
                    MacroParam::Positional(n) => n,
                    MacroParam::Keyword { name, .. } => name,
                };
                let val = scope.get(&pname.to_uppercase()).cloned().unwrap_or_default();
                self.log_line(format!(
                    "MLOGIC({label}):  Parameter {} has value {}",
                    pname.to_uppercase(),
                    val
                ));
            }
        }
        self.scopes.push(scope);
        self.macro_stack.push(label.clone());
        self.depth += 1;
        let expanded = self.process_impl(&def.body);
        self.depth -= 1;
        self.macro_stack.pop();
        self.scopes.pop();
        // M19.3 — MPRINT : écho du code produit par la macro, ligne à ligne.
        // Chaque ligne NON VIDE (après trim) du texte expansé est écho­tée
        // avec le préfixe `MPRINT(nom):`.
        if self.mprint {
            for line in expanded.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    self.log_line(format!("MPRINT({label}):   {trimmed}"));
                }
            }
        }
        // M19.3 — MLOGIC : décision de sortie de macro.
        if self.mlogic {
            self.log_line(format!("MLOGIC({label}):  Ending execution."));
        }
        out.push_str(&expanded);
        resume
    }

    /// Parse une liste d'arguments d'appel `(a, b, key=val, ...)` à partir de
    /// `(`. Rend `(positionnels, mots-clés, index après `)`)`. Les valeurs sont
    /// prises telles quelles (trim des bords) jusqu'au `,`/`)` de même niveau ;
    /// les parenthèses imbriquées sont équilibrées.
    fn parse_arg_list(
        chars: &[char],
        lparen: usize,
    ) -> Option<(Vec<String>, Vec<(String, String)>, usize)> {
        let mut j = lparen + 1;
        let mut positional = Vec::new();
        let mut keyword = Vec::new();
        // Cas liste vide `()`.
        {
            let mut k = j;
            while matches!(chars.get(k), Some(c) if c.is_whitespace()) {
                k += 1;
            }
            if chars.get(k) == Some(&')') {
                return Some((positional, keyword, k + 1));
            }
        }
        loop {
            // Détecter `name=` : un identifiant suivi de `=` (avant tout `,`/`(`).
            let seg_start = j;
            let mut paren = 0i32;
            let mut eq_pos: Option<usize> = None;
            let mut k = j;
            while k < chars.len() {
                let c = chars[k];
                if c == '(' {
                    paren += 1;
                } else if c == ')' {
                    if paren == 0 {
                        break;
                    }
                    paren -= 1;
                } else if c == ',' && paren == 0 {
                    break;
                } else if c == '=' && paren == 0 && eq_pos.is_none() {
                    eq_pos = Some(k);
                }
                k += 1;
            }
            let seg_end = k; // pointe sur `,` ou `)` ou fin.
            // Déterminer si c'est `key=value` : la partie avant `=` doit être un
            // identifiant pur (trim).
            let mut is_keyword = false;
            if let Some(eq) = eq_pos {
                let key: String = chars[seg_start..eq].iter().collect();
                let key_trim = key.trim();
                if !key_trim.is_empty()
                    && key_trim
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_alphabetic() || c == '_')
                        .unwrap_or(false)
                    && key_trim.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    is_keyword = true;
                    let val: String = chars[eq + 1..seg_end].iter().collect();
                    keyword.push((key_trim.to_string(), val.trim().to_string()));
                }
            }
            if !is_keyword {
                let val: String = chars[seg_start..seg_end].iter().collect();
                positional.push(val.trim().to_string());
            }
            match chars.get(seg_end) {
                Some(&',') => {
                    j = seg_end + 1;
                    continue;
                }
                Some(&')') => return Some((positional, keyword, seg_end + 1)),
                _ => return None, // parenthèse non fermée.
            }
        }
    }

    /// Lie les paramètres formels aux arguments fournis et rend la portée
    /// locale (toutes les variables des paramètres y sont présentes).
    ///
    /// Règles (documentées) :
    /// - Les arguments positionnels remplissent les paramètres dans l'ordre de
    ///   déclaration (positionnels comme mots-clés peuvent recevoir une valeur
    ///   positionnelle, fidèle à SAS où l'ordre prime).
    /// - Les `clé=valeur` écrasent ensuite le paramètre nommé correspondant.
    /// - Paramètres non fournis : `Keyword` prend son défaut, `Positional`
    ///   prend la chaîne vide.
    /// - Trop d'arguments positionnels : les excédentaires sont IGNORÉS
    ///   (SAS émet une erreur ; on choisit la tolérance — documenté).
    /// - `clé=valeur` pour une clé inconnue : IGNORÉ (SAS erreur ; toléré ici).
    fn bind_params(
        params: &[MacroParam],
        pos_args: &[String],
        kw_args: &[(String, String)],
    ) -> std::collections::HashMap<String, String> {
        let mut scope = std::collections::HashMap::new();
        // Valeurs par défaut.
        for p in params {
            match p {
                MacroParam::Positional(n) => {
                    scope.insert(n.to_uppercase(), String::new());
                }
                MacroParam::Keyword { name, default } => {
                    scope.insert(name.to_uppercase(), default.clone());
                }
            }
        }
        // Positionnels dans l'ordre déclaré.
        for (p, arg) in params.iter().zip(pos_args.iter()) {
            let key = match p {
                MacroParam::Positional(n) => n.to_uppercase(),
                MacroParam::Keyword { name, .. } => name.to_uppercase(),
            };
            scope.insert(key, arg.clone());
        }
        // Mots-clés : écrasent le paramètre nommé s'il existe.
        for (k, v) in kw_args {
            let key = k.to_uppercase();
            if scope.contains_key(&key) {
                scope.insert(key, v.clone());
            }
            // sinon : clé inconnue, ignorée.
        }
        scope
    }
}

impl MacroEngine {
    /// Garde anti-boucle-folle pour les `%do` itératifs.
    const MAX_LOOP_ITERS: i64 = 1_000_000;

    /// Vrai si `chars[i..]` commence par `%<kw>` (insensible casse) suivi
    /// éventuellement de blancs puis d'une `(` — pour les fonctions macro comme
    /// `%eval(...)`. Évite de matcher un identifiant plus long (`%evalx`).
    fn matches_kw_paren(chars: &[char], i: usize, kw: &str) -> bool {
        if chars.get(i) != Some(&'%') {
            return false;
        }
        let kwc: Vec<char> = kw.chars().collect();
        for (k, &kc) in kwc.iter().enumerate() {
            match chars.get(i + 1 + k) {
                Some(c) if c.to_ascii_lowercase() == kc => {}
                _ => return false,
            }
        }
        let mut j = i + 1 + kwc.len();
        // Le caractère juste après le mot-clé ne doit pas continuer un identifiant.
        if matches!(chars.get(j), Some(c) if c.is_ascii_alphanumeric() || *c == '_') {
            return false;
        }
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        chars.get(j) == Some(&'(')
    }

    /// Émet une note d'erreur macro SAS-like dans le flux et poursuit. Jamais
    /// de `panic` sur entrée invalide.
    fn emit_error(out: &mut String, err: &MacroError) {
        out.push_str(&format!("/* {} */", err.message));
    }

    /// Consomme `%eval ( expr )` : résout les `&refs` de `expr`, évalue, et
    /// splice le résultat entier. Rend l'index après la `)`, ou `None` si la
    /// parenthèse n'est pas trouvée (laisse alors le `%` brut).
    fn consume_eval(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let mut j = i + 1 + "eval".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) != Some(&'(') {
            return None;
        }
        let (inner, after) = Self::read_balanced_parens(chars, j)?;
        // Résoudre d'abord les &refs, puis (récursivement) tout `%eval`/macro
        // imbriqué dans l'argument avant d'évaluer.
        let resolved = self.resolve_value(&inner);
        let expanded = self.process_impl(&resolved);
        match self.macro_eval(&expanded) {
            Ok(v) => out.push_str(&v.to_string()),
            Err(e) => Self::emit_error(out, &e),
        }
        Some(after)
    }

    /// Lit le contenu entre `(` (à l'index `lparen`) et sa `)` équilibrée.
    /// Rend `(contenu_sans_parenthèses, index_après_la_parenthèse_fermante)`.
    fn read_balanced_parens(chars: &[char], lparen: usize) -> Option<(String, usize)> {
        let mut depth = 0i32;
        let mut j = lparen;
        let start = lparen + 1;
        while j < chars.len() {
            match chars[j] {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        let inner: String = chars[start..j].iter().collect();
                        return Some((inner, j + 1));
                    }
                }
                _ => {}
            }
            j += 1;
        }
        None
    }

    // ── M11.6 : variables automatiques ──────────────────────────────────────

    /// Amorce un sous-ensemble des variables automatiques SAS dans `table`.
    /// Sous `deterministic`, valeurs FIGÉES (snapshots stables) ; sinon dérivées
    /// de l'horloge réelle. Cf. la doc de [`MacroEngine::new`].
    fn seed_automatic_vars(&mut self, deterministic: bool) {
        let vars: [(&str, String); 6] = if deterministic {
            [
                ("SYSDATE9", "01JAN1960".to_string()),
                ("SYSDATE", "01JAN60".to_string()),
                ("SYSTIME", "00:00".to_string()),
                ("SYSDAY", "Friday".to_string()),
                ("SYSVER", "9.4".to_string()),
                ("SYSSCP", "LIN X64".to_string()),
            ]
        } else {
            use chrono::{Datelike, Local, Timelike};
            let now = Local::now();
            const MONTHS: [&str; 12] = [
                "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
            ];
            const DAYS: [&str; 7] = [
                "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday",
            ];
            let mon = MONTHS[(now.month0()) as usize];
            let day = now.day();
            let year4 = now.year();
            let year2 = (year4 % 100).abs();
            let sysdate9 = format!("{day:02}{mon}{year4:04}");
            let sysdate = format!("{day:02}{mon}{year2:02}");
            let systime = format!("{:02}:{:02}", now.hour(), now.minute());
            let sysday = DAYS[now.weekday().num_days_from_monday() as usize].to_string();
            [
                ("SYSDATE9", sysdate9),
                ("SYSDATE", sysdate),
                ("SYSTIME", systime),
                ("SYSDAY", sysday),
                ("SYSVER", "9.4".to_string()),
                ("SYSSCP", "LIN X64".to_string()),
            ]
        };
        for (k, v) in vars {
            self.table.insert(k.to_string(), v);
        }
    }

    // ── M11.6 : %sysfunc ─────────────────────────────────────────────────────

    /// Consomme `%sysfunc ( func(args) )` (ou `%qsysfunc`). Résout les `&refs`,
    /// parse `func(arg1, arg2, ...)`, appelle `functions::call` avec les args
    /// typés (numérique si l'argument parse en nombre, sinon `Char`), puis
    /// splice le résultat formaté en texte. Fonction inconnue / non whitelistée
    /// → note d'erreur propre (pas de panic). Rend l'index après la `)`, ou
    /// `None` si la parenthèse externe n'est pas trouvée.
    fn consume_sysfunc(
        &mut self,
        chars: &[char],
        kw: &str,
        i: usize,
        out: &mut String,
    ) -> Option<usize> {
        let mut j = i + 1 + kw.len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) != Some(&'(') {
            return None;
        }
        let (inner, after) = Self::read_balanced_parens(chars, j)?;
        // Résoudre les &refs (et les sentinelles déjà posées restent inertes).
        let resolved = self.resolve_value(&inner);
        // `%qsysfunc` masque son résultat (ponctuation + déclencheurs), comme
        // les autres variantes `%q*` ; `%sysfunc` l'émet en clair.
        let q = kw == "qsysfunc";
        match self.eval_sysfunc(&resolved) {
            Ok(text) => {
                if q {
                    out.push_str(&Self::mask_special(&text, true));
                } else {
                    out.push_str(&text);
                }
            }
            Err(e) => Self::emit_error(out, &e),
        }
        Some(after)
    }

    /// Liste blanche des fonctions DATA-step appelables via `%sysfunc`. On
    /// délègue à `functions::call`, mais on filtre explicitement pour éviter
    /// d'exposer des fonctions sans signification en contexte macro (texte).
    const SYSFUNC_WHITELIST: &'static [&'static str] = &[
        "UPCASE", "LOWCASE", "SUBSTR", "TRIM", "STRIP", "LEFT", "COMPRESS", "INDEX", "SCAN",
        "LENGTH", "CAT", "CATS", "CATX", "TRANWRD", "SUM", "MAX", "MIN", "ABS", "INT", "MDY",
        "YEAR", "MONTH", "DAY", "TODAY", "DATE", "WEEKDAY",
    ];

    /// Parse `func(a, b, ...)` et évalue la fonction. Le contenu a déjà ses
    /// `&refs` résolus. Rend le texte du résultat ou une `MacroError`.
    fn eval_sysfunc(&self, content: &str) -> Result<String, MacroError> {
        let content = content.trim();
        let chars: Vec<char> = content.chars().collect();
        // Nom de fonction.
        let (name, after) = Self::read_name(&chars, 0)
            .ok_or_else(|| MacroError::new("ERROR: %SYSFUNC requires a function call"))?;
        let mut j = after;
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        // Arguments : `(...)` optionnel (fonctions sans argument : TODAY()).
        let arg_strings: Vec<String> = if chars.get(j) == Some(&'(') {
            let (args_inner, _) = Self::read_balanced_parens(&chars, j)
                .ok_or_else(|| MacroError::new("ERROR: unbalanced parentheses in %SYSFUNC"))?;
            Self::split_top_level_commas(&args_inner)
        } else {
            Vec::new()
        };
        let upper = name.to_uppercase();
        if !Self::SYSFUNC_WHITELIST.contains(&upper.as_str()) {
            return Err(MacroError::new(format!(
                "ERROR: Function {} not supported by %SYSFUNC in this interpreter",
                name
            )));
        }
        // Typage des arguments : un argument qui parse en nombre devient
        // `Value::Num`, sinon `Value::Char` (le trim de bord est appliqué, comme
        // SAS pour les arguments macro).
        let args: Vec<crate::value::Value> = arg_strings
            .iter()
            .map(|a| {
                let t = a.trim();
                match t.parse::<f64>() {
                    Ok(n) => crate::value::Value::Num(n),
                    Err(_) => crate::value::Value::Char(t.to_string()),
                }
            })
            .collect();
        // EvalCtx minimal jetable : `Default` suffit (aucune dépendance PDV pour
        // les fonctions whitelistées).
        let mut ctx = crate::datastep::eval::EvalCtx::default();
        match crate::datastep::functions::call(&upper, &args, &mut ctx) {
            Some(v) => Ok(Self::value_to_text(&v)),
            None => Err(MacroError::new(format!(
                "ERROR: Function {} is unknown to %SYSFUNC",
                name
            ))),
        }
    }

    /// Formate une `Value` en texte pour l'insertion macro : `Char` tel quel
    /// (trim de droite), `Num` via le format BEST (entier sans décimales),
    /// missing → chaîne vide.
    fn value_to_text(v: &crate::value::Value) -> String {
        match v {
            crate::value::Value::Char(s) => s.trim_end().to_string(),
            crate::value::Value::Num(n) => crate::value::format_best(*n, 12),
            crate::value::Value::Missing(_) => String::new(),
        }
    }

    /// Découpe une chaîne d'arguments sur les `,` de niveau supérieur (les
    /// parenthèses imbriquées sont équilibrées). Chaîne vide → aucun argument.
    fn split_top_level_commas(s: &str) -> Vec<String> {
        let chars: Vec<char> = s.chars().collect();
        let mut parts = Vec::new();
        let mut depth = 0i32;
        let mut start = 0usize;
        let mut any = false;
        for (k, &c) in chars.iter().enumerate() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                ',' if depth == 0 => {
                    parts.push(chars[start..k].iter().collect());
                    start = k + 1;
                    any = true;
                }
                _ => {}
            }
        }
        let last: String = chars[start..].iter().collect();
        if any || !last.trim().is_empty() {
            parts.push(last);
        }
        parts
    }

    // ── M11.6 : %str / %nrstr (quoting par sentinelles) ─────────────────────

    /// Sentinelle de base (zone privée Unicode). Chaque caractère spécial masqué
    /// est remplacé par `MASK_BASE + offset`, où `offset` est un petit index
    /// stable. La passe `unmask` finale rétablit les littéraux. Ces points de
    /// code n'apparaissent jamais dans un source SAS normal.
    const MASK_BASE: u32 = 0xE000;

    /// Caractères masqués par `%str` (et `%nrstr`), dans l'ordre des offsets.
    /// `%str` masque la ponctuation/opérateurs pour qu'un `;` ou `,` interne
    /// soit littéral ; `&` et `%` ne sont masqués QUE par `%nrstr`.
    const STR_MASKED: &'static [char] = &[
        ';', '+', '-', '*', '/', '<', '>', '=', '|', '~', ',', '(', ')', '\'', '"',
    ];
    /// Caractères additionnels masqués UNIQUEMENT par `%nrstr` (déclencheurs).
    const NRSTR_EXTRA: &'static [char] = &['&', '%'];

    /// Masque un caractère vers sa sentinelle si présent dans la table donnée.
    /// Rend `Some(sentinelle)` ou `None` si le caractère n'est pas masqué.
    fn mask_char(c: char) -> Option<char> {
        // Index global stable sur la concaténation STR_MASKED ++ NRSTR_EXTRA.
        if let Some(idx) = Self::STR_MASKED.iter().position(|&m| m == c) {
            return char::from_u32(Self::MASK_BASE + idx as u32);
        }
        if let Some(idx) = Self::NRSTR_EXTRA.iter().position(|&m| m == c) {
            return char::from_u32(Self::MASK_BASE + Self::STR_MASKED.len() as u32 + idx as u32);
        }
        None
    }

    /// Passe finale d'« unmask » : rétablit chaque sentinelle en son littéral.
    fn unmask(s: &str) -> String {
        if !s.chars().any(|c| {
            let v = c as u32;
            (Self::MASK_BASE..Self::MASK_BASE + 0x100).contains(&v)
        }) {
            return s.to_string();
        }
        let total = Self::STR_MASKED.len() + Self::NRSTR_EXTRA.len();
        s.chars()
            .map(|c| {
                let v = c as u32;
                if v >= Self::MASK_BASE && v < Self::MASK_BASE + total as u32 {
                    let idx = (v - Self::MASK_BASE) as usize;
                    if idx < Self::STR_MASKED.len() {
                        Self::STR_MASKED[idx]
                    } else {
                        Self::NRSTR_EXTRA[idx - Self::STR_MASKED.len()]
                    }
                } else {
                    c
                }
            })
            .collect()
    }

    /// Consomme `%str ( ... )` (si `!nrstr`) ou `%nrstr ( ... )`. Masque les
    /// caractères spéciaux du contenu (pour `%str`, `&`/`%` restent ACTIFS et
    /// sont donc résolus ; pour `%nrstr` ils sont AUSSI masqués → inertes). Pour
    /// `%str`, on ré-expanse le contenu masqué afin de résoudre les `&x`/`%m`
    /// éventuels ; pour `%nrstr`, on émet le contenu masqué tel quel. Rend
    /// l'index après la `)`, ou `None` si la parenthèse n'est pas trouvée.
    fn consume_quote(
        &mut self,
        chars: &[char],
        i: usize,
        kw: &str,
        nrstr: bool,
        out: &mut String,
    ) -> Option<usize> {
        let mut j = i + 1 + kw.len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) != Some(&'(') {
            return None;
        }
        let (inner, after) = Self::read_balanced_parens(chars, j)?;
        let masked: String = inner
            .chars()
            .map(|c| {
                // `&` et `%` ne sont masqués que par `%nrstr`.
                if (c == '&' || c == '%') && !nrstr {
                    return c;
                }
                Self::mask_char(c).unwrap_or(c)
            })
            .collect();
        if nrstr {
            // Contenu inerte : émis tel quel (déclencheurs masqués).
            out.push_str(&masked);
        } else {
            // `%str` : `&`/`%` restent actifs → ré-expansion.
            let expanded = self.process_impl(&masked);
            out.push_str(&expanded);
        }
        Some(after)
    }

    // ── M19.1 : fonctions macro différées ───────────────────────────────────

    /// Consomme `%unquote ( text )`. C'est l'INVERSE des fonctions de quoting
    /// (`%str`/`%nrstr`/`%bquote`/`%superq`/`%q*`) : il « dé-masque » le texte et
    /// RÉ-ACTIVE la résolution des déclencheurs `&`/`%` qui avaient été rendus
    /// inertes par le schéma de sentinelles.
    ///
    /// Interaction avec le schéma de sentinelles (point délicat) : les fonctions
    /// de quoting remplacent `&`/`%`/ponctuation par des sentinelles `MASK_BASE+k`.
    /// `%unquote` procède en trois temps :
    ///   1. résoudre les `&refs` ENCORE actifs de l'argument (texte non masqué) ;
    ///   2. `unmask` → rétablir les littéraux d'origine, ce qui ressuscite tout
    ///      `&`/`%` précédemment masqué ;
    ///   3. ré-`process_impl` le texte dé-masqué → les `&`/`%` ressuscités sont
    ///      maintenant résolus comme des déclencheurs normaux.
    /// La passe `unmask` finale de `expand_open_code` ne fait alors plus rien sur
    /// ce fragment (déjà dé-masqué). Rend l'index après la `)`.
    fn consume_unquote(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let mut j = i + 1 + "unquote".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) != Some(&'(') {
            return None;
        }
        let (inner, after) = Self::read_balanced_parens(chars, j)?;
        // 1. expanser l'argument tel quel : tout `%str`/`%nrstr`/`%q*` imbriqué
        //    s'exécute et POSE ses sentinelles (déclencheurs `&`/`%` masqués) ;
        // 2. `unmask` → rétablit les littéraux, ce qui RESSUSCITE `&`/`%` ;
        // 3. ré-`process_impl` → ces déclencheurs ressuscités sont maintenant
        //    résolus comme des déclencheurs normaux. La passe `unmask` finale de
        //    `expand_open_code` ne fait plus rien sur ce fragment.
        let expanded = self.process_impl(&inner);
        let unmasked = Self::unmask(&expanded);
        let reexpanded = self.process_impl(&unmasked);
        out.push_str(&reexpanded);
        Some(after)
    }

    /// Consomme `%cmpres ( text )` / `%qcmpres ( text )`. Résout les `&refs`,
    /// puis COMPRESSE les blancs : rogne les blancs de bord et réduit toute
    /// suite de blancs interne à UN seul espace (fidèle à SAS CMPRES). La
    /// variante `q` masque le résultat (ponctuation + déclencheurs) comme les
    /// autres `%q*`. Rend l'index après la `)`.
    fn consume_cmpres(
        &mut self,
        chars: &[char],
        kw: &str,
        masked: bool,
        i: usize,
        out: &mut String,
    ) -> Option<usize> {
        let mut j = i + 1 + kw.len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) != Some(&'(') {
            return None;
        }
        let (inner, after) = Self::read_balanced_parens(chars, j)?;
        let resolved = self.resolve_value(&inner);
        let compressed = Self::compress_blanks(&resolved);
        if masked {
            out.push_str(&Self::mask_special(&compressed, true));
        } else {
            out.push_str(&compressed);
        }
        Some(after)
    }

    /// Rogne les blancs de bord et réduit chaque suite de blancs interne à un
    /// unique espace. Helper de `%cmpres`/`%qcmpres`.
    fn compress_blanks(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut prev_blank = false;
        for c in s.trim().chars() {
            if c.is_whitespace() {
                if !prev_blank {
                    out.push(' ');
                    prev_blank = true;
                }
            } else {
                out.push(c);
                prev_blank = false;
            }
        }
        out
    }

    /// Lit l'argument NOM d'une fonction `%kw ( name )` (commune à `%symexist`,
    /// `%sysmexist`, `%sysget`). Résout les `&refs` de l'argument puis rogne les
    /// blancs et un éventuel `&` de tête (SAS accepte `%symexist(&x)`). Rend
    /// `(nom, index après la `)`)`, ou `None` si la parenthèse manque.
    fn read_name_arg(&mut self, chars: &[char], kw: &str, i: usize) -> Option<(String, usize)> {
        let mut j = i + 1 + kw.len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) != Some(&'(') {
            return None;
        }
        let (inner, after) = Self::read_balanced_parens(chars, j)?;
        let resolved = self.resolve_value(&inner);
        let name = resolved.trim().trim_start_matches('&').trim().to_string();
        Some((name, after))
    }

    /// Consomme `%symexist ( name )`. Rend `1` si la variable macro existe (dans
    /// une portée locale OU globale), `0` sinon. Rend l'index après la `)`.
    fn consume_symexist(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let (name, after) = self.read_name_arg(chars, "symexist", i)?;
        let exists = self.lookup(&name).is_some();
        out.push_str(if exists { "1" } else { "0" });
        Some(after)
    }

    /// Consomme `%sysmexist ( name )`. Rend `1` si la macro (définie via
    /// `%macro`) existe, `0` sinon. Rend l'index après la `)`.
    fn consume_sysmexist(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let (name, after) = self.read_name_arg(chars, "sysmexist", i)?;
        let exists = self.macros.contains_key(&name.to_uppercase());
        out.push_str(if exists { "1" } else { "0" });
        Some(after)
    }

    /// Consomme `%sysget ( name )`. Rend la valeur de la variable d'environnement
    /// nommée. Une variable inexistante rend la CHAÎNE VIDE (SAS émet un WARNING ;
    /// on se contente de produire vide pour rester déterministe). Cf. la note de
    /// déterminisme dans l'en-tête du module. Rend l'index après la `)`.
    fn consume_sysget(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let (name, after) = self.read_name_arg(chars, "sysget", i)?;
        if let Ok(v) = std::env::var(&name) {
            out.push_str(&v);
        }
        Some(after)
    }

    /// Consomme `%sysevalf ( expr [, conv] )` : évaluation FLOTTANTE de `expr`
    /// (contrairement à `%eval` qui est entier seulement). Le résultat brut est
    /// un `f64` ; un éventuel deuxième argument `conv` le convertit :
    /// - `BOOLEAN` → `1` si non nul (et non missing), `0` sinon ;
    /// - `CEIL`    → plafond, formaté en entier ;
    /// - `FLOOR`   → plancher, formaté en entier ;
    /// - `INTEGER` → troncature vers zéro, formaté en entier ;
    /// - absent    → le flottant formaté (entier sans décimales si exact).
    /// `&refs`/macros imbriquées dans `expr` sont résolues d'abord. Erreur de
    /// syntaxe → note d'erreur (pas de panic). Rend l'index après la `)`.
    fn consume_sysevalf(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let mut j = i + 1 + "sysevalf".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) != Some(&'(') {
            return None;
        }
        let (inner, after) = Self::read_balanced_parens(chars, j)?;
        // L'argument peut contenir des &refs/macros : résoudre AVANT de découper
        // les virgules (les nombres ne contiennent pas de virgule de niveau sup.).
        let resolved = self.resolve_value(&inner);
        let expanded = self.process_impl(&resolved);
        let parts = Self::split_top_level_commas(&expanded);
        let expr = parts.first().map(String::as_str).unwrap_or("").trim();
        let conv = parts.get(1).map(|s| s.trim().to_ascii_uppercase());
        match Self::eval_float(expr) {
            Ok(v) => out.push_str(&Self::format_sysevalf(v, conv.as_deref())),
            Err(e) => Self::emit_error(out, &e),
        }
        Some(after)
    }

    /// Formate le résultat flottant de `%sysevalf` selon la conversion demandée.
    fn format_sysevalf(v: f64, conv: Option<&str>) -> String {
        match conv {
            Some("BOOLEAN") => {
                if v != 0.0 && !v.is_nan() {
                    "1".to_string()
                } else {
                    "0".to_string()
                }
            }
            Some("CEIL") => Self::format_float(v.ceil()),
            Some("FLOOR") => Self::format_float(v.floor()),
            Some("INTEGER") => Self::format_float(v.trunc()),
            _ => Self::format_float(v),
        }
    }

    /// Formate un `f64` en texte façon SAS : un entier exact perd ses décimales
    /// (`3.0` → `"3"`), sinon on emploie une représentation compacte sans zéros
    /// finaux superflus.
    fn format_float(v: f64) -> String {
        if v.is_nan() {
            return String::new();
        }
        if v == v.trunc() && v.abs() < 1e15 {
            return format!("{}", v as i64);
        }
        // Représentation compacte : `{}` sur f64 rend déjà la plus courte forme
        // fidèle sans zéros finaux superflus.
        format!("{v}")
    }

    /// Évalue une expression arithmétique FLOTTANTE (pour `%sysevalf`). Supporte
    /// `+ - * / **`, parenthèses, comparaisons (`= ne < <= > >= eq …` → 1/0),
    /// logique (`and or not & | ^`) et l'unaire `+`/`-`. Tout est calculé en
    /// `f64` (division réelle, `**` réelle). Un opérande non numérique → erreur.
    fn eval_float(expr: &str) -> Result<f64, MacroError> {
        let toks = Self::tokenize_eval(expr)?;
        let mut p = FloatParser { toks: &toks, pos: 0 };
        let v = p.parse_expr()?;
        if p.pos != p.toks.len() {
            return Err(MacroError::new(format!(
                "ERROR: A syntax error was detected in the %SYSEVALF expression: {expr}"
            )));
        }
        Ok(v)
    }

    // ── M12.2 : quoting étendu (%superq, %bquote, %nrbquote) ─────────────────

    /// Masque TOUS les caractères « spéciaux » d'une chaîne via le schéma de
    /// sentinelles partagé avec `%str`/`%nrstr`. Si `triggers` est vrai, masque
    /// aussi `&` et `%` (déclencheurs) — sinon ils restent actifs en aval. Les
    /// caractères non listés (lettres, chiffres, blancs, `.`) passent inchangés.
    fn mask_special(s: &str, triggers: bool) -> String {
        s.chars()
            .map(|c| {
                if (c == '&' || c == '%') && !triggers {
                    return c;
                }
                Self::mask_char(c).unwrap_or(c)
            })
            .collect()
    }

    /// Consomme `%superq ( name )`. Prend un NOM de variable (pas `&name`),
    /// lit sa valeur SANS résoudre aucun `&`/`%` qu'elle contient, et masque
    /// TOUT (y compris `&`/`%`) afin que le résultat soit littéral et inerte en
    /// aval — l'outil idéal pour des valeurs contenant des `&`/`%` parasites.
    /// L'argument peut lui-même être un `&ref` désignant le nom (SAS résout
    /// l'argument en un nom). Variable indéfinie → chaîne vide (SAS émet un
    /// WARNING ; on se contente d'émettre vide). Rend l'index après la `)`.
    fn consume_superq(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let mut j = i + 1 + "superq".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) != Some(&'(') {
            return None;
        }
        let (inner, after) = Self::read_balanced_parens(chars, j)?;
        // L'argument désigne un nom : on résout d'éventuels `&ref` puis on rogne
        // les blancs et un éventuel `&` de tête (SAS accepte `%superq(&x)` =
        // nom dans x, comme `%superq(x)`). On ne touche PAS à la valeur lue.
        let name_arg = self.resolve_value(&inner);
        let name = name_arg.trim().trim_start_matches('&').trim();
        match self.lookup(name) {
            Some(v) => out.push_str(&Self::mask_special(&v, true)),
            None => { /* indéfini → vide (SAS warne) */ }
        }
        Some(after)
    }

    /// Consomme `%bquote ( text )` (si `!nr`) ou `%nrbquote ( text )`. Résout
    /// d'abord les `&`/`%` du texte (expansion normale), PUIS masque le
    /// résultat pour le rendre littéral en aval :
    /// - `%bquote` masque la ponctuation/opérateurs (`; , ( ) ' " + - * / < >
    ///   = | ~`) mais laisse `&`/`%` ACTIFS (ils ont déjà été résolus ; un `&`
    ///   résiduel non défini reste tel quel) ;
    /// - `%nrbquote` masque EN PLUS `&`/`%` du résultat (empêche toute
    ///   résolution ultérieure).
    /// Les quotes/parenthèses NON APPARIÉES de l'entrée ne posent pas de
    /// problème : on ne fait pas d'analyse appariée du contenu — `read_balanced_parens`
    /// borne sur la `)` de `%bquote(...)` et tout `'`/`(` interne est traité
    /// comme un caractère ordinaire (puis masqué). Rend l'index après la `)`.
    fn consume_bquote(
        &mut self,
        chars: &[char],
        i: usize,
        kw: &str,
        nr: bool,
        out: &mut String,
    ) -> Option<usize> {
        let mut j = i + 1 + kw.len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) != Some(&'(') {
            return None;
        }
        let (inner, after) = Self::read_balanced_parens(chars, j)?;
        // Expansion normale du texte (résout `&x`/`%m`), puis masquage du
        // résultat. `%nrbquote` masque aussi `&`/`%`.
        let expanded = self.process_impl(&inner);
        out.push_str(&Self::mask_special(&expanded, nr));
        Some(after)
    }

    // ── M12.2 : fonctions chaîne macro (%upcase/%substr/... et %q*) ──────────

    /// Consomme une fonction chaîne macro `%kw ( args )` parmi
    /// `upcase`/`lowcase`/`substr`/`scan`/`index`/`length` et leurs variantes
    /// `q*` (`masked == true` → résultat masqué par sentinelles, comme
    /// `%bquote`). Les arguments sont d'abord résolus (`&refs`) puis découpés
    /// sur les `,` de niveau supérieur. Positions 1-basées (convention SAS).
    /// Le résultat texte n'est PAS masqué pour les variantes nues. Rend l'index
    /// après la `)`, ou `None` si la parenthèse manque / arité invalide.
    fn consume_macro_fn(
        &mut self,
        chars: &[char],
        i: usize,
        kw: &str,
        masked: bool,
        out: &mut String,
    ) -> Option<usize> {
        let mut j = i + 1 + kw.len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) != Some(&'(') {
            return None;
        }
        let (inner, after) = Self::read_balanced_parens(chars, j)?;
        // Résoudre les `&refs` des arguments avant découpe (les sentinelles
        // déjà posées restent inertes et seront ré-émises telles quelles).
        let resolved = self.resolve_value(&inner);
        let args = Self::split_top_level_commas(&resolved);
        // Le nom de fonction « logique » est `kw` sans le préfixe `q` éventuel.
        let logical = kw.strip_prefix('q').unwrap_or(kw);
        let result = Self::eval_macro_fn(logical, &args)?;
        if masked {
            // Variantes `%q*` : masque la ponctuation ET les déclencheurs
            // résiduels (`&`/`%`) du résultat, afin qu'il soit totalement inerte
            // en aval (cf. `%qupcase(a&x)` qui masque le `&`).
            out.push_str(&Self::mask_special(&result, true));
        } else {
            out.push_str(&result);
        }
        Some(after)
    }

    /// Calcule le résultat d'une fonction chaîne macro à partir d'arguments
    /// (texte déjà résolu, non rognés sauf indication). Conventions SAS :
    /// - `upcase(t)` / `lowcase(t)` : casse (sur tout l'argument, blancs inclus) ;
    /// - `substr(t, pos[, len])` : sous-chaîne 1-basée ; `pos`/`len` rognés et
    ///   parsés en entier ; `pos` borné à `[1, len(t)+1]` ; `len` par défaut
    ///   jusqu'à la fin ; bornes clampées (pas de panic hors limites) ;
    /// - `scan(t, n[, delims])` : n-ième mot (1-basé), délimiteurs par défaut
    ///   = blanc et quelques ponctuations SAS ; `n` négatif compte depuis la
    ///   fin ; hors borne → chaîne vide ;
    /// - `index(t, sub)` : position 1-basée de `sub` dans `t` (0 si absent) ;
    /// - `length(t)` : longueur (au moins 1 pour une chaîne vide, comme SAS qui
    ///   rend 1 pour une chaîne vide ; ici on rend 0 pour vide et documente
    ///   l'écart — voir NB). Rend `None` si l'arité est invalide.
    fn eval_macro_fn(name: &str, args: &[String]) -> Option<String> {
        match name {
            "upcase" => {
                let t = args.first().map(String::as_str).unwrap_or("");
                Some(t.to_uppercase())
            }
            "lowcase" => {
                let t = args.first().map(String::as_str).unwrap_or("");
                Some(t.to_lowercase())
            }
            "substr" => {
                if args.len() < 2 || args.len() > 3 {
                    return None;
                }
                let t: Vec<char> = args[0].chars().collect();
                let pos = args[1].trim().parse::<i64>().ok()?;
                // SAS : pos 1-basé. On clamp dans [1, len+1].
                let start = pos.clamp(1, t.len() as i64 + 1) as usize - 1;
                let remaining = t.len() - start;
                let take = if args.len() == 3 {
                    let l = args[2].trim().parse::<i64>().ok()?;
                    (l.max(0) as usize).min(remaining)
                } else {
                    remaining
                };
                Some(t[start..start + take].iter().collect())
            }
            "scan" => {
                if args.len() < 2 || args.len() > 3 {
                    return None;
                }
                let t = &args[0];
                let n = args[1].trim().parse::<i64>().ok()?;
                // Délimiteurs par défaut SAS (sous-ensemble usuel) ; sinon ceux
                // fournis tels quels (sans rogner, un blanc compte).
                let delims: Vec<char> = if args.len() == 3 {
                    args[2].chars().collect()
                } else {
                    " \t\n.<(+|&!$*);^-/,%>".chars().collect()
                };
                let mut words: Vec<String> = Vec::new();
                let mut cur = String::new();
                for c in t.chars() {
                    if delims.contains(&c) {
                        if !cur.is_empty() {
                            words.push(std::mem::take(&mut cur));
                        }
                    } else {
                        cur.push(c);
                    }
                }
                if !cur.is_empty() {
                    words.push(cur);
                }
                let idx = if n >= 0 {
                    (n as usize).checked_sub(1)
                } else {
                    let from_end = (-n) as usize;
                    words.len().checked_sub(from_end)
                };
                Some(idx.and_then(|k| words.get(k)).cloned().unwrap_or_default())
            }
            "index" => {
                if args.len() != 2 {
                    return None;
                }
                let t = &args[0];
                let sub = &args[1];
                // Position 1-basée en CARACTÈRES (pas octets) ; 0 si absent.
                let pos = if sub.is_empty() {
                    0
                } else {
                    match t.find(sub.as_str()) {
                        Some(byte_off) => t[..byte_off].chars().count() + 1,
                        None => 0,
                    }
                };
                Some(pos.to_string())
            }
            "length" => {
                let t = args.first().map(String::as_str).unwrap_or("");
                Some(t.chars().count().to_string())
            }
            _ => None,
        }
    }

    /// Évalue une condition `%if` : résout d'abord les `&refs` et tout
    /// `%eval`/macro imbriqué, puis applique `macro_eval`. Truthy = non nul.
    fn eval_condition(&mut self, cond: &str) -> Result<bool, MacroError> {
        let resolved = self.resolve_value(cond);
        let expanded = self.process_impl(&resolved);
        Ok(self.macro_eval(expanded.trim())? != 0)
    }

    /// Consomme `%if <cond> %then <action> [; %else <action> ;]`.
    ///
    /// `<cond>` court jusqu'au `%then` (insensible casse). `<action>` est soit
    /// un groupe `%do; ... %end;`, soit un fragment de texte jusqu'au `;` de fin
    /// d'action (le `;` est inclus dans le texte émis, comme une instruction
    /// SAS). On émet la branche prise EXPANSÉE et rien pour l'autre. Rend
    /// l'index de reprise, ou `None` si la structure ne tient pas.
    fn consume_if(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let cond_start = i + 1 + "if".len();
        // Trouver le `%then`.
        let then_pos = Self::find_kw(chars, cond_start, "then")?;
        let cond: String = chars[cond_start..then_pos].iter().collect();
        let mut j = then_pos + 1 + "then".len();

        // Évaluer la condition.
        let take_then = match self.eval_condition(&cond) {
            Ok(b) => b,
            Err(e) => {
                Self::emit_error(out, &e);
                // En cas d'erreur, on consomme tout de même la structure pour ne
                // pas réémettre du texte macro brut. On parse les actions sans
                // les exécuter.
                false
            }
        };
        // M19.3 — MLOGIC : décision de la condition `%if`.
        if self.mlogic {
            let label = self.current_macro_label();
            self.log_line(format!(
                "MLOGIC({}):  %IF condition {} is {}",
                label,
                cond.trim(),
                if take_then { "TRUE" } else { "FALSE" }
            ));
        }

        // Parser l'action du THEN (group ou fragment) -> (texte, index_après).
        let (then_text, after_then) = Self::scan_action(chars, j)?;
        j = after_then;

        // %else optionnel.
        let mut else_text: Option<String> = None;
        let mut after_else = j;
        {
            let mut k = j;
            while matches!(chars.get(k), Some(c) if c.is_whitespace()) {
                k += 1;
            }
            if Self::matches_kw(chars, k, "else") {
                let astart = k + 1 + "else".len();
                let (etext, ae) = Self::scan_action(chars, astart)?;
                else_text = Some(etext);
                after_else = ae;
            }
        }

        // Émettre la branche prise, expansée.
        let chosen = if take_then {
            Some(then_text)
        } else {
            else_text
        };
        if let Some(text) = chosen {
            let expanded = self.process_impl(&text);
            out.push_str(&expanded);
        }
        Some(after_else)
    }

    /// Scanne une "action" de `%if`/`%then`/`%else` à partir de `start` :
    /// soit un groupe `%do; ... %end;` (le texte retourné est le corps interne
    /// du `%do`, sans le `%do;`/`%end;`), soit un fragment de texte jusqu'au
    /// `;` terminal inclus. Rend `(texte_action, index_après)`.
    fn scan_action(chars: &[char], start: usize) -> Option<(String, usize)> {
        let mut k = start;
        while matches!(chars.get(k), Some(c) if c.is_whitespace()) {
            k += 1;
        }
        if Self::matches_kw(chars, k, "do") {
            // Réutiliser le scan de `%do` complet : on renvoie le texte
            // `%do ... %end;` tel quel pour le laisser ré-expanser (gère donc
            // `%do;`, itératif, imbriqué). Le `process_impl` rappellera
            // `consume_do` dessus.
            let (text, after) = Self::scan_do_block(chars, k)?;
            Some((text, after))
        } else {
            // Fragment jusqu'au `;` terminal (inclus). On respecte les `%do`
            // imbriqués éventuels au cas où, mais le cas nominal est un texte
            // simple. On s'arrête au premier `;` de niveau 0.
            let frag_start = k;
            while k < chars.len() && chars[k] != ';' {
                k += 1;
            }
            if chars.get(k) != Some(&';') {
                // Pas de `;` : prendre jusqu'à la fin.
                let frag: String = chars[frag_start..k].iter().collect();
                return Some((frag, k));
            }
            k += 1; // inclure le `;`
            let frag: String = chars[frag_start..k].iter().collect();
            Some((frag, k))
        }
    }

    /// Scanne un bloc `%do ... %end;` complet à partir de `start` (qui pointe
    /// sur `%do`). Rend `(texte_complet_incluant_%do_et_%end;, index_après)`.
    /// Équilibre les `%do`/`%end` imbriqués.
    fn scan_do_block(chars: &[char], start: usize) -> Option<(String, usize)> {
        let mut j = start + 1 + "do".len();
        let mut depth = 1usize;
        while j < chars.len() && depth > 0 {
            if chars[j] == '%' {
                if Self::matches_kw(chars, j, "do") {
                    depth += 1;
                    j += 1 + "do".len();
                    continue;
                }
                if Self::matches_kw(chars, j, "end") {
                    depth -= 1;
                    j += 1 + "end".len();
                    if depth == 0 {
                        // Avaler un `;` terminal optionnel après `%end`.
                        let mut k = j;
                        while matches!(chars.get(k), Some(c) if c.is_whitespace()) {
                            k += 1;
                        }
                        if chars.get(k) == Some(&';') {
                            j = k + 1;
                        }
                        let text: String = chars[start..j].iter().collect();
                        return Some((text, j));
                    }
                    continue;
                }
            }
            j += 1;
        }
        None
    }

    /// Consomme un `%do` : soit `%do; ... %end;` (groupe), soit
    /// `%do i=a %to b [%by c]; ... %end;` (itératif). Émet le contenu expansé.
    fn consume_do(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let mut j = i + 1 + "do".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        // Forme itérative : `%do <name> = ...`.
        if let Some((var, after_var)) = Self::read_name(chars, j) {
            let mut k = after_var;
            while matches!(chars.get(k), Some(c) if c.is_whitespace()) {
                k += 1;
            }
            if chars.get(k) == Some(&'=') {
                return self.consume_iterative_do(chars, i, &var, k + 1, out);
            }
            // `%do %while`/`%until` : non implémenté (déféré). On émet une note
            // et on consomme le bloc pour ne pas réémettre du texte brut.
        }
        // Formes conditionnelles `%do %while(<cond>)` / `%do %until(<cond>)`.
        if Self::matches_kw_paren(chars, j, "while") {
            return self.consume_conditional_do(chars, i, j, "while", true, out);
        }
        if Self::matches_kw_paren(chars, j, "until") {
            return self.consume_conditional_do(chars, i, j, "until", false, out);
        }

        // Forme groupe `%do; ... %end;` : le caractère courant doit être `;`.
        if chars.get(j) != Some(&';') {
            return None;
        }
        let body_start = j + 1;
        // Trouver le `%end` équilibré.
        let (body_end, after) = Self::find_matching_end(chars, body_start)?;
        let body: String = chars[body_start..body_end].iter().collect();
        // Voir la note de `consume_iterative_do` sur le rognage des blancs de
        // bord (fidélité SAS simplifiée) : on rogne le bord gauche du corps puis
        // le bord droit de la contribution du bloc.
        let expanded = self.process_impl(body.trim_start());
        out.push_str(expanded.trim_end());
        Some(after)
    }

    /// Consomme la forme itérative `%do i = <start> %to <stop> [%by <step>]; body %end;`.
    /// `expr_start` pointe juste après le `=`. Itère `&i` de start à stop.
    ///
    /// # Rognage des blancs (fidélité SAS simplifiée)
    /// SAS conserve verbatim le texte entre `%do...;` et `%end`, blancs de bord
    /// inclus. On simplifie : on rogne le bord GAUCHE du corps (avant chaque
    /// expansion) et le bord DROIT de la contribution totale du bloc. Les blancs
    /// internes (entre instructions/itérations) sont préservés, d'où
    /// `%do i=1 %to 5 %by 2; [&i] %end;` -> `[1] [3] [5]` (un espace par
    /// séparateur, sans blanc de bord parasite).
    fn consume_iterative_do(
        &mut self,
        chars: &[char],
        _do_start: usize,
        var: &str,
        expr_start: usize,
        out: &mut String,
    ) -> Option<usize> {
        // `<start>` court jusqu'au `%to`.
        let to_pos = Self::find_kw(chars, expr_start, "to")?;
        let start_expr: String = chars[expr_start..to_pos].iter().collect();
        let after_to = to_pos + 1 + "to".len();
        // `<stop>` court jusqu'au `%by` ou au `;`.
        let by_pos = Self::find_kw_before_semicolon(chars, after_to, "by");
        let (stop_expr, after_stop, step_expr) = match by_pos {
            Some(bp) => {
                let stop: String = chars[after_to..bp].iter().collect();
                let after_by = bp + 1 + "by".len();
                // step jusqu'au `;`.
                let semi = Self::find_semicolon(chars, after_by)?;
                let step: String = chars[after_by..semi].iter().collect();
                (stop, semi, Some(step))
            }
            None => {
                let semi = Self::find_semicolon(chars, after_to)?;
                let stop: String = chars[after_to..semi].iter().collect();
                (stop, semi, None)
            }
        };
        // `after_stop` pointe sur le `;` terminant l'en-tête du %do.
        let body_start = after_stop + 1;
        let (body_end, after) = Self::find_matching_end(chars, body_start)?;
        let body: String = chars[body_start..body_end].iter().collect();

        // Évaluer bornes/step.
        let start = match self.eval_condition_int(&start_expr) {
            Ok(v) => v,
            Err(e) => {
                Self::emit_error(out, &e);
                return Some(after);
            }
        };
        let stop = match self.eval_condition_int(&stop_expr) {
            Ok(v) => v,
            Err(e) => {
                Self::emit_error(out, &e);
                return Some(after);
            }
        };
        let step = match &step_expr {
            Some(s) => match self.eval_condition_int(s) {
                Ok(v) => v,
                Err(e) => {
                    Self::emit_error(out, &e);
                    return Some(after);
                }
            },
            None => 1,
        };
        if step == 0 {
            Self::emit_error(
                out,
                &MacroError::new("ERROR: %DO loop step is zero (non-terminating)"),
            );
            return Some(after);
        }

        // Itérer. Garde anti-boucle-folle. On accumule dans un buffer local pour
        // pouvoir rogner le bord droit de la contribution complète (cf. note de
        // rognage ci-dessous). Chaque itération expanse `body.trim_start()`.
        let body_trimmed = body.trim_start();
        let mut buf = String::new();
        let mut value = start;
        let mut iters: i64 = 0;
        loop {
            let cont = if step > 0 { value <= stop } else { value >= stop };
            if !cont {
                break;
            }
            iters += 1;
            if iters > Self::MAX_LOOP_ITERS {
                Self::emit_error(
                    &mut buf,
                    &MacroError::new(format!(
                        "ERROR: %DO loop exceeded {} iterations (runaway guard)",
                        Self::MAX_LOOP_ITERS
                    )),
                );
                break;
            }
            // Affecter &i dans la portée courante (haut de pile, ou table en
            // open code) puis expanser le corps.
            self.set_loop_var(var, value);
            let expanded = self.process_impl(body_trimmed);
            buf.push_str(&expanded);
            // Avancer en gardant contre l'overflow.
            match value.checked_add(step) {
                Some(v) => value = v,
                None => break,
            }
        }
        out.push_str(buf.trim_end());
        Some(after)
    }

    /// Consomme les formes conditionnelles `%do %while(<cond>); body %end;` et
    /// `%do %until(<cond>); body %end;`.
    ///
    /// `kw_start` pointe sur le `%` de `%while`/`%until`. `is_while` distingue la
    /// sémantique :
    /// - `%while` : test AVANT chaque itération (0 itération si faux d'emblée) ;
    /// - `%until` : test APRÈS chaque itération (≥1 itération, on s'arrête dès
    ///   que la condition devient vraie).
    ///
    /// La condition est ré-évaluée fraîchement à chaque tour (résolution des
    /// `&refs` + `macro_eval`), si bien qu'un `%let` dans le corps influe sur la
    /// terminaison. Réutilise `MAX_LOOP_ITERS` comme garde anti-boucle-folle. Le
    /// rognage des blancs suit la convention de `consume_iterative_do`.
    fn consume_conditional_do(
        &mut self,
        chars: &[char],
        _do_start: usize,
        kw_start: usize,
        kw: &str,
        is_while: bool,
        out: &mut String,
    ) -> Option<usize> {
        // La `(` suit le mot-clé (éventuellement après des blancs).
        let mut p = kw_start + 1 + kw.len();
        while matches!(chars.get(p), Some(c) if c.is_whitespace()) {
            p += 1;
        }
        if chars.get(p) != Some(&'(') {
            return None;
        }
        let (cond, after_cond) = Self::read_balanced_parens(chars, p)?;
        // L'en-tête se termine par le `;` suivant la `)` de la condition.
        let semi = Self::find_semicolon(chars, after_cond)?;
        let body_start = semi + 1;
        let (body_end, after) = Self::find_matching_end(chars, body_start)?;
        let body: String = chars[body_start..body_end].iter().collect();
        let body_trimmed = body.trim_start();

        let mut buf = String::new();
        let mut iters: i64 = 0;
        loop {
            // `%while` teste avant le corps ; `%until` après.
            if is_while {
                match self.eval_condition(&cond) {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(e) => {
                        Self::emit_error(&mut buf, &e);
                        break;
                    }
                }
            }
            iters += 1;
            if iters > Self::MAX_LOOP_ITERS {
                Self::emit_error(
                    &mut buf,
                    &MacroError::new(format!(
                        "ERROR: %DO loop exceeded {} iterations (runaway guard)",
                        Self::MAX_LOOP_ITERS
                    )),
                );
                break;
            }
            let expanded = self.process_impl(body_trimmed);
            buf.push_str(&expanded);
            if !is_while {
                // `%until` : on s'arrête quand la condition devient vraie.
                match self.eval_condition(&cond) {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(e) => {
                        Self::emit_error(&mut buf, &e);
                        break;
                    }
                }
            }
        }
        out.push_str(buf.trim_end());
        Some(after)
    }

    /// Affecte la variable d'itération dans la portée courante (haut de la pile
    /// de portées si on est dans une macro, sinon la table globale).
    fn set_loop_var(&mut self, var: &str, value: i64) {
        let key = var.to_uppercase();
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(key, value.to_string());
        } else {
            self.table.insert(key, value.to_string());
        }
    }

    /// Comme `eval_condition` mais rend l'entier (pour les bornes `%to`/`%by`).
    fn eval_condition_int(&mut self, expr: &str) -> Result<i64, MacroError> {
        let resolved = self.resolve_value(expr);
        let expanded = self.process_impl(&resolved);
        self.macro_eval(expanded.trim())
    }

    /// Trouve le mot-clé `%<kw>` (insensible casse) à partir de `from`, au
    /// niveau de `%do` 0 (ne descend pas dans un `%do ... %end` imbriqué). Rend
    /// l'index du `%` du mot-clé. Utilisé pour `%then`.
    fn find_kw(chars: &[char], from: usize, kw: &str) -> Option<usize> {
        let mut j = from;
        let mut do_depth = 0usize;
        while j < chars.len() {
            if chars[j] == '%' {
                if Self::matches_kw(chars, j, "do") {
                    do_depth += 1;
                    j += 1 + "do".len();
                    continue;
                }
                if Self::matches_kw(chars, j, "end") {
                    do_depth = do_depth.saturating_sub(1);
                    j += 1 + "end".len();
                    continue;
                }
                if do_depth == 0 && Self::matches_kw(chars, j, kw) {
                    return Some(j);
                }
            }
            j += 1;
        }
        None
    }

    /// Trouve `%<kw>` à partir de `from` mais s'arrête au premier `;` de niveau
    /// 0 (utilisé pour `%by`, qui doit précéder le `;` de l'en-tête de boucle).
    fn find_kw_before_semicolon(chars: &[char], from: usize, kw: &str) -> Option<usize> {
        let mut j = from;
        while j < chars.len() {
            if chars[j] == ';' {
                return None;
            }
            if chars[j] == '%' && Self::matches_kw(chars, j, kw) {
                return Some(j);
            }
            j += 1;
        }
        None
    }

    /// Trouve le prochain `;` de niveau 0 à partir de `from`.
    fn find_semicolon(chars: &[char], from: usize) -> Option<usize> {
        let mut j = from;
        while j < chars.len() {
            if chars[j] == ';' {
                return Some(j);
            }
            j += 1;
        }
        None
    }

    /// À partir de `body_start` (juste après le `;` de l'en-tête du `%do`),
    /// trouve le `%end` équilibré. Rend `(index_du_%end, index_après_%end;)`.
    fn find_matching_end(chars: &[char], body_start: usize) -> Option<(usize, usize)> {
        let mut j = body_start;
        let mut depth = 1usize;
        while j < chars.len() {
            if chars[j] == '%' {
                if Self::matches_kw(chars, j, "do") {
                    depth += 1;
                    j += 1 + "do".len();
                    continue;
                }
                if Self::matches_kw(chars, j, "end") {
                    depth -= 1;
                    if depth == 0 {
                        let end_at = j;
                        let mut k = j + 1 + "end".len();
                        // Avaler un `;` terminal optionnel après `%end`.
                        let mut m = k;
                        while matches!(chars.get(m), Some(c) if *c == ' ' || *c == '\t') {
                            m += 1;
                        }
                        if chars.get(m) == Some(&';') {
                            k = m + 1;
                        }
                        return Some((end_at, k));
                    }
                    j += 1 + "end".len();
                    continue;
                }
            }
            j += 1;
        }
        None
    }
}

/// Jeton de l'expression macro pour `%eval`.
#[derive(Clone, Debug, PartialEq, Eq)]
enum EvalTok {
    Int(i64),
    /// Opérande non entier (rencontré tel quel) : déclenche l'erreur SAS
    /// "A character operand was found..." si utilisé dans un contexte
    /// arithmétique. Conservé pour égalité textuelle dans les comparaisons.
    Word(String),
    Plus,
    Minus,
    Star,
    Slash,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Not,
    LParen,
    RParen,
}

impl MacroEngine {
    /// Évalue une expression macro `%eval` selon la sémantique ENTIÈRE de SAS.
    /// Le texte fourni doit déjà avoir ses `&vars` résolus (l'appelant le fait).
    ///
    /// Grammaire (par précédence croissante, récursive-descente) :
    /// ```text
    /// expr        := or_expr
    /// or_expr     := and_expr ( ('|' | 'or') and_expr )*
    /// and_expr    := not_expr ( ('&' | 'and') not_expr )*
    /// not_expr    := ('^' | '~' | 'not')* cmp_expr
    /// cmp_expr    := add_expr ( cmp_op add_expr )?
    /// add_expr    := mul_expr ( ('+' | '-') mul_expr )*
    /// mul_expr    := pow_expr ( ('*' | '/') pow_expr )*
    /// pow_expr    := unary ( '**' pow_expr )?         // associatif à droite
    /// unary       := ('+' | '-')* primary
    /// primary     := INT | '(' expr ')'
    /// ```
    /// Sémantique : opérandes entiers ; division ENTIÈRE tronquée vers zéro ;
    /// `**` puissance entière ; comparaisons → 1/0 ; logiques → 1/0 (vrai =
    /// non nul). Un opérande non entier dans un contexte arithmétique est une
    /// erreur ("A character operand was found in the %EVAL function...").
    fn macro_eval(&self, expr: &str) -> Result<i64, MacroError> {
        let toks = Self::tokenize_eval(expr)?;
        let mut p = EvalParser { toks: &toks, pos: 0 };
        let v = p.parse_expr()?;
        if p.pos != p.toks.len() {
            return Err(MacroError::new(format!(
                "ERROR: A syntax error was detected in the %EVAL expression: {expr}"
            )));
        }
        Ok(v)
    }

    /// Découpe l'expression en jetons. Les espaces séparent ; les mots
    /// alphabétiques sont reconnus comme opérateurs textuels (`eq`, `and`,
    /// `not`, ...) sinon conservés comme `Word` (opérande non entier).
    fn tokenize_eval(expr: &str) -> Result<Vec<EvalTok>, MacroError> {
        let chars: Vec<char> = expr.chars().collect();
        let mut toks = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c.is_whitespace() {
                i += 1;
                continue;
            }
            match c {
                '+' => {
                    toks.push(EvalTok::Plus);
                    i += 1;
                }
                '-' => {
                    toks.push(EvalTok::Minus);
                    i += 1;
                }
                '*' => {
                    if chars.get(i + 1) == Some(&'*') {
                        toks.push(EvalTok::Pow);
                        i += 2;
                    } else {
                        toks.push(EvalTok::Star);
                        i += 1;
                    }
                }
                '/' => {
                    toks.push(EvalTok::Slash);
                    i += 1;
                }
                '(' => {
                    toks.push(EvalTok::LParen);
                    i += 1;
                }
                ')' => {
                    toks.push(EvalTok::RParen);
                    i += 1;
                }
                '=' => {
                    toks.push(EvalTok::Eq);
                    i += 1;
                }
                '&' => {
                    // `&&` ou `&` -> AND logique.
                    if chars.get(i + 1) == Some(&'&') {
                        i += 2;
                    } else {
                        i += 1;
                    }
                    toks.push(EvalTok::And);
                }
                '|' => {
                    if chars.get(i + 1) == Some(&'|') {
                        i += 2;
                    } else {
                        i += 1;
                    }
                    toks.push(EvalTok::Or);
                }
                '<' => {
                    if chars.get(i + 1) == Some(&'=') {
                        toks.push(EvalTok::Le);
                        i += 2;
                    } else if chars.get(i + 1) == Some(&'>') {
                        // `<>` = NE en contexte de comparaison macro SAS.
                        toks.push(EvalTok::Ne);
                        i += 2;
                    } else {
                        toks.push(EvalTok::Lt);
                        i += 1;
                    }
                }
                '>' => {
                    if chars.get(i + 1) == Some(&'=') {
                        toks.push(EvalTok::Ge);
                        i += 2;
                    } else {
                        toks.push(EvalTok::Gt);
                        i += 1;
                    }
                }
                '^' | '~' => {
                    if chars.get(i + 1) == Some(&'=') {
                        toks.push(EvalTok::Ne);
                        i += 2;
                    } else {
                        toks.push(EvalTok::Not);
                        i += 1;
                    }
                }
                _ if c.is_ascii_digit() || c == '.' => {
                    let start = i;
                    while matches!(chars.get(i), Some(d) if d.is_ascii_digit()) {
                        i += 1;
                    }
                    // Partie fractionnaire / exposant : marque un littéral FLOTTANT
                    // (`7.5`, `.5`, `1e3`). `%eval` (entier) le verra comme un
                    // `Word` et émettra l'erreur « character operand » ; `%sysevalf`
                    // (flottant) le parse en `f64`.
                    let mut is_float = false;
                    if chars.get(i) == Some(&'.') {
                        is_float = true;
                        i += 1;
                        while matches!(chars.get(i), Some(d) if d.is_ascii_digit()) {
                            i += 1;
                        }
                    }
                    if matches!(chars.get(i), Some('e' | 'E'))
                        && matches!(
                            chars.get(i + 1),
                            Some(d) if d.is_ascii_digit()
                                || ((*d == '+' || *d == '-')
                                    && matches!(chars.get(i + 2), Some(e) if e.is_ascii_digit()))
                        )
                    {
                        is_float = true;
                        i += 1; // 'e'
                        if matches!(chars.get(i), Some('+' | '-')) {
                            i += 1;
                        }
                        while matches!(chars.get(i), Some(d) if d.is_ascii_digit()) {
                            i += 1;
                        }
                    }
                    // Un opérande alphanumérique mixte (ex. `3a`) est un mot.
                    if matches!(chars.get(i), Some(d) if d.is_ascii_alphabetic() || *d == '_') {
                        let wstart = start;
                        while matches!(chars.get(i), Some(d) if d.is_ascii_alphanumeric() || *d == '_') {
                            i += 1;
                        }
                        let w: String = chars[wstart..i].iter().collect();
                        toks.push(EvalTok::Word(w));
                    } else if is_float {
                        // Littéral flottant : porté comme `Word` (entier le rejette).
                        toks.push(EvalTok::Word(chars[start..i].iter().collect()));
                    } else {
                        let s: String = chars[start..i].iter().collect();
                        match s.parse::<i64>() {
                            Ok(n) => toks.push(EvalTok::Int(n)),
                            Err(_) => {
                                return Err(MacroError::new(format!(
                                    "ERROR: Overflow in the %EVAL function: {s}"
                                )))
                            }
                        }
                    }
                }
                _ if c.is_ascii_alphabetic() || c == '_' => {
                    let start = i;
                    while matches!(chars.get(i), Some(d) if d.is_ascii_alphanumeric() || *d == '_') {
                        i += 1;
                    }
                    let w: String = chars[start..i].iter().collect();
                    match w.to_ascii_lowercase().as_str() {
                        "eq" => toks.push(EvalTok::Eq),
                        "ne" => toks.push(EvalTok::Ne),
                        "lt" => toks.push(EvalTok::Lt),
                        "le" => toks.push(EvalTok::Le),
                        "gt" => toks.push(EvalTok::Gt),
                        "ge" => toks.push(EvalTok::Ge),
                        "and" => toks.push(EvalTok::And),
                        "or" => toks.push(EvalTok::Or),
                        "not" => toks.push(EvalTok::Not),
                        _ => toks.push(EvalTok::Word(w)),
                    }
                }
                other => {
                    return Err(MacroError::new(format!(
                        "ERROR: A syntax error was detected in the %EVAL expression near '{other}'"
                    )))
                }
            }
        }
        Ok(toks)
    }
}

/// Analyseur récursif-descendant pour l'expression `%eval`.
struct EvalParser<'a> {
    toks: &'a [EvalTok],
    pos: usize,
}

impl<'a> EvalParser<'a> {
    fn peek(&self) -> Option<&EvalTok> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<&EvalTok> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn parse_expr(&mut self) -> Result<i64, MacroError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<i64, MacroError> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Some(EvalTok::Or)) {
            self.bump();
            let right = self.parse_and()?;
            left = ((left != 0) || (right != 0)) as i64;
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<i64, MacroError> {
        let mut left = self.parse_not()?;
        while matches!(self.peek(), Some(EvalTok::And)) {
            self.bump();
            let right = self.parse_not()?;
            left = ((left != 0) && (right != 0)) as i64;
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<i64, MacroError> {
        let mut negs = 0;
        while matches!(self.peek(), Some(EvalTok::Not)) {
            self.bump();
            negs += 1;
        }
        let v = self.parse_cmp()?;
        if negs % 2 == 1 {
            Ok((v == 0) as i64)
        } else {
            Ok(v)
        }
    }

    fn parse_cmp(&mut self) -> Result<i64, MacroError> {
        let left = self.parse_add()?;
        if let Some(op) = self.peek().cloned() {
            let is_cmp = matches!(
                op,
                EvalTok::Eq
                    | EvalTok::Ne
                    | EvalTok::Lt
                    | EvalTok::Le
                    | EvalTok::Gt
                    | EvalTok::Ge
            );
            if is_cmp {
                self.bump();
                let right = self.parse_add()?;
                let r = match op {
                    EvalTok::Eq => left == right,
                    EvalTok::Ne => left != right,
                    EvalTok::Lt => left < right,
                    EvalTok::Le => left <= right,
                    EvalTok::Gt => left > right,
                    EvalTok::Ge => left >= right,
                    _ => unreachable!(),
                };
                return Ok(r as i64);
            }
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<i64, MacroError> {
        let mut left = self.parse_mul()?;
        loop {
            match self.peek() {
                Some(EvalTok::Plus) => {
                    self.bump();
                    left = left.wrapping_add(self.parse_mul()?);
                }
                Some(EvalTok::Minus) => {
                    self.bump();
                    left = left.wrapping_sub(self.parse_mul()?);
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<i64, MacroError> {
        let mut left = self.parse_pow()?;
        loop {
            match self.peek() {
                Some(EvalTok::Star) => {
                    self.bump();
                    left = left.wrapping_mul(self.parse_pow()?);
                }
                Some(EvalTok::Slash) => {
                    self.bump();
                    let right = self.parse_pow()?;
                    if right == 0 {
                        return Err(MacroError::new(
                            "ERROR: Division by zero detected in the %EVAL expression",
                        ));
                    }
                    // Division entière tronquée vers zéro (sémantique Rust `/`).
                    left /= right;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_pow(&mut self) -> Result<i64, MacroError> {
        let base = self.parse_unary()?;
        if matches!(self.peek(), Some(EvalTok::Pow)) {
            self.bump();
            // Associatif à droite.
            let exp = self.parse_pow()?;
            return Ok(Self::ipow(base, exp));
        }
        Ok(base)
    }

    /// Puissance entière ; exposant négatif -> 0 (sémantique entière, comme SAS
    /// qui tronque le résultat fractionnaire vers 0 sauf base ±1).
    fn ipow(base: i64, exp: i64) -> i64 {
        if exp < 0 {
            return match base {
                1 => 1,
                -1 => {
                    if (-exp) % 2 == 0 {
                        1
                    } else {
                        -1
                    }
                }
                _ => 0,
            };
        }
        let mut result: i64 = 1;
        let mut b = base;
        let mut e = exp;
        while e > 0 {
            if e & 1 == 1 {
                result = result.wrapping_mul(b);
            }
            e >>= 1;
            if e > 0 {
                b = b.wrapping_mul(b);
            }
        }
        result
    }

    fn parse_unary(&mut self) -> Result<i64, MacroError> {
        match self.peek() {
            Some(EvalTok::Plus) => {
                self.bump();
                self.parse_unary()
            }
            Some(EvalTok::Minus) => {
                self.bump();
                Ok(self.parse_unary()?.wrapping_neg())
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<i64, MacroError> {
        match self.bump() {
            Some(EvalTok::Int(n)) => Ok(*n),
            Some(EvalTok::LParen) => {
                let v = self.parse_expr()?;
                match self.bump() {
                    Some(EvalTok::RParen) => Ok(v),
                    _ => Err(MacroError::new(
                        "ERROR: A syntax error was detected in the %EVAL expression: expected ')'",
                    )),
                }
            }
            Some(EvalTok::Word(w)) => Err(MacroError::new(format!(
                "ERROR: A character operand was found in the %EVAL function or %IF condition where a numeric operand is required. The condition was: {w}"
            ))),
            other => Err(MacroError::new(format!(
                "ERROR: A syntax error was detected in the %EVAL expression near {other:?}"
            ))),
        }
    }
}

/// Analyseur récursif-descendant FLOTTANT pour `%sysevalf` (M19.1). Même
/// grammaire que [`EvalParser`] mais en `f64` : division réelle, `**` réelle,
/// comparaisons/logique rendant `1.0`/`0.0`. Réutilise les `EvalTok` produits
/// par `MacroEngine::tokenize_eval` ; un littéral flottant arrive comme
/// `EvalTok::Word` (que cet analyseur parse en nombre, contrairement à
/// l'analyseur entier qui le rejette).
struct FloatParser<'a> {
    toks: &'a [EvalTok],
    pos: usize,
}

impl FloatParser<'_> {
    fn peek(&self) -> Option<&EvalTok> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<&EvalTok> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn parse_expr(&mut self) -> Result<f64, MacroError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<f64, MacroError> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Some(EvalTok::Or)) {
            self.bump();
            let right = self.parse_and()?;
            left = ((left != 0.0) || (right != 0.0)) as i64 as f64;
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<f64, MacroError> {
        let mut left = self.parse_not()?;
        while matches!(self.peek(), Some(EvalTok::And)) {
            self.bump();
            let right = self.parse_not()?;
            left = ((left != 0.0) && (right != 0.0)) as i64 as f64;
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<f64, MacroError> {
        let mut negs = 0;
        while matches!(self.peek(), Some(EvalTok::Not)) {
            self.bump();
            negs += 1;
        }
        let v = self.parse_cmp()?;
        if negs % 2 == 1 {
            Ok((v == 0.0) as i64 as f64)
        } else {
            Ok(v)
        }
    }

    fn parse_cmp(&mut self) -> Result<f64, MacroError> {
        let left = self.parse_add()?;
        if let Some(op) = self.peek().cloned() {
            let is_cmp = matches!(
                op,
                EvalTok::Eq
                    | EvalTok::Ne
                    | EvalTok::Lt
                    | EvalTok::Le
                    | EvalTok::Gt
                    | EvalTok::Ge
            );
            if is_cmp {
                self.bump();
                let right = self.parse_add()?;
                let r = match op {
                    EvalTok::Eq => left == right,
                    EvalTok::Ne => left != right,
                    EvalTok::Lt => left < right,
                    EvalTok::Le => left <= right,
                    EvalTok::Gt => left > right,
                    EvalTok::Ge => left >= right,
                    _ => unreachable!(),
                };
                return Ok(r as i64 as f64);
            }
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<f64, MacroError> {
        let mut left = self.parse_mul()?;
        loop {
            match self.peek() {
                Some(EvalTok::Plus) => {
                    self.bump();
                    left += self.parse_mul()?;
                }
                Some(EvalTok::Minus) => {
                    self.bump();
                    left -= self.parse_mul()?;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<f64, MacroError> {
        let mut left = self.parse_pow()?;
        loop {
            match self.peek() {
                Some(EvalTok::Star) => {
                    self.bump();
                    left *= self.parse_pow()?;
                }
                Some(EvalTok::Slash) => {
                    self.bump();
                    let right = self.parse_pow()?;
                    if right == 0.0 {
                        return Err(MacroError::new(
                            "ERROR: Division by zero detected in the %SYSEVALF expression",
                        ));
                    }
                    // Division RÉELLE (≠ %eval qui tronque).
                    left /= right;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_pow(&mut self) -> Result<f64, MacroError> {
        let base = self.parse_unary()?;
        if matches!(self.peek(), Some(EvalTok::Pow)) {
            self.bump();
            // Associatif à droite.
            let exp = self.parse_pow()?;
            return Ok(base.powf(exp));
        }
        Ok(base)
    }

    fn parse_unary(&mut self) -> Result<f64, MacroError> {
        match self.peek() {
            Some(EvalTok::Plus) => {
                self.bump();
                self.parse_unary()
            }
            Some(EvalTok::Minus) => {
                self.bump();
                Ok(-self.parse_unary()?)
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<f64, MacroError> {
        match self.bump() {
            Some(EvalTok::Int(n)) => Ok(*n as f64),
            Some(EvalTok::Word(w)) => w.parse::<f64>().map_err(|_| {
                MacroError::new(format!(
                    "ERROR: A character operand was found in the %SYSEVALF function where a numeric operand is required: {w}"
                ))
            }),
            Some(EvalTok::LParen) => {
                let v = self.parse_expr()?;
                match self.bump() {
                    Some(EvalTok::RParen) => Ok(v),
                    _ => Err(MacroError::new(
                        "ERROR: A syntax error was detected in the %SYSEVALF expression: expected ')'",
                    )),
                }
            }
            other => Err(MacroError::new(format!(
                "ERROR: A syntax error was detected in the %SYSEVALF expression near {other:?}"
            ))),
        }
    }
}

/// Découpeur de segments bruts d'open code (M11.5, feature `macros`).
///
/// `run_program` (sous `macros`) traite le source ORIGINAL segment par
/// segment : chaque segment est expansé par `expand_open_code` avec l'état
/// VIVANT de l'engine, puis lexé/parsé/exécuté, AVANT de passer au suivant.
/// C'est ce qui rend `CALL SYMPUT` visible dans le segment suivant (le drain
/// du symput a lieu à la fin de l'étape, donc avant l'expansion du segment
/// d'après).
///
/// # Règle de segmentation (volontairement GROSSIÈRE mais correcte)
/// On découpe le source brut en unités de haut niveau terminées par un
/// `run;`/`quit;` de niveau supérieur. Concrètement, le scanner avance
/// caractère par caractère en :
/// - ignorant l'intérieur des chaînes `'...'` / `"..."` (les `;` y sont
///   inertes) ;
/// - suivant la profondeur `%macro … %mend` (un `;` à l'intérieur d'une
///   définition de macro NE termine PAS un segment) ;
/// - coupant un segment juste APRÈS le `;` qui suit un mot-clé `run` ou
///   `quit` de niveau supérieur (fin d'étape DATA/PROC).
/// Le reliquat après le dernier `run;` (open code final : `%put`, `%let`,
/// etc.) forme le dernier segment. Les instructions d'open code situées
/// AVANT une étape sont donc regroupées avec cette étape dans un même
/// segment — sans incidence : l'expansion est gauche→droite et l'état macro
/// persiste de toute façon entre segments.
///
/// Renvoie des PLAGES d'octets `[start, end)` dans le source d'origine, afin
/// que l'executor puisse à la fois (a) ré-expanser le texte brut du segment
/// et (b) écho­ter les lignes ORIGINALES correspondantes (numérotation
/// préservée).
pub struct RawSegmenter<'a> {
    chars: Vec<char>,
    /// Décalages OCTETS cumulés, `byte_offset[i]` = offset du i-ème char.
    byte_offsets: Vec<usize>,
    total_bytes: usize,
    pos: usize,
    _src: std::marker::PhantomData<&'a str>,
}

impl<'a> RawSegmenter<'a> {
    pub fn new(src: &'a str) -> Self {
        let chars: Vec<char> = src.chars().collect();
        let mut byte_offsets = Vec::with_capacity(chars.len() + 1);
        let mut off = 0usize;
        for c in &chars {
            byte_offsets.push(off);
            off += c.len_utf8();
        }
        byte_offsets.push(off);
        RawSegmenter {
            chars,
            byte_offsets,
            total_bytes: src.len(),
            pos: 0,
            _src: std::marker::PhantomData,
        }
    }

    /// Vrai si `chars[i..]` commence par le mot-clé `kw` (insensible casse)
    /// PRÉCÉDÉ d'une frontière de mot (début, blanc, `;`) et SUIVI d'un
    /// non-identifiant. Sert à reconnaître `run`/`quit` de niveau supérieur.
    fn word_at(chars: &[char], i: usize, kw: &str) -> bool {
        // Frontière gauche.
        if i > 0 {
            let p = chars[i - 1];
            if p.is_ascii_alphanumeric() || p == '_' {
                return false;
            }
        }
        let kwc: Vec<char> = kw.chars().collect();
        for (k, &kc) in kwc.iter().enumerate() {
            match chars.get(i + k) {
                Some(c) if c.to_ascii_lowercase() == kc => {}
                _ => return false,
            }
        }
        match chars.get(i + kwc.len()) {
            Some(c) if c.is_ascii_alphanumeric() || *c == '_' => false,
            _ => true,
        }
    }

    /// Renvoie la prochaine plage d'octets `[start, end)`, ou `None` à la fin.
    pub fn next_segment(&mut self) -> Option<(usize, usize)> {
        if self.pos >= self.chars.len() {
            return None;
        }
        let start_char = self.pos;
        let mut i = self.pos;
        let mut macro_depth = 0usize;
        // Mot-clé run/quit vu et en attente de son `;` terminal.
        let mut pending_boundary = false;
        while i < self.chars.len() {
            let c = self.chars[i];
            // Chaînes : sauter jusqu'au guillemet fermant (mêmes guillemets).
            if c == '\'' || c == '"' {
                let quote = c;
                i += 1;
                while i < self.chars.len() && self.chars[i] != quote {
                    i += 1;
                }
                if i < self.chars.len() {
                    i += 1; // guillemet fermant
                }
                continue;
            }
            // Profondeur %macro / %mend (un `;` interne ne coupe pas).
            if c == '%' {
                if MacroEngine::matches_kw(&self.chars, i, "macro") {
                    macro_depth += 1;
                    i += "%macro".len();
                    continue;
                }
                if MacroEngine::matches_kw(&self.chars, i, "mend") {
                    macro_depth = macro_depth.saturating_sub(1);
                    i += "%mend".len();
                    continue;
                }
            }
            if macro_depth == 0 {
                if Self::word_at(&self.chars, i, "run") {
                    pending_boundary = true;
                    i += "run".len();
                    continue;
                }
                if Self::word_at(&self.chars, i, "quit") {
                    pending_boundary = true;
                    i += "quit".len();
                    continue;
                }
                if c == ';' && pending_boundary {
                    // Fin de segment juste après ce `;`.
                    i += 1;
                    self.pos = i;
                    return Some((self.byte_offsets[start_char], self.byte_offsets[i]));
                }
            }
            i += 1;
        }
        // Reliquat : tout le reste forme le dernier segment.
        self.pos = self.chars.len();
        Some((self.byte_offsets[start_char], self.total_bytes))
    }
}

#[cfg(test)]
mod macro_tests {
    use super::*;

    fn run(input: &str) -> String {
        MacroStage::default().process(input)
    }

    /// Comme `run` mais passe par `expand_open_code` (applique la passe finale
    /// d'unmask des sentinelles `%str`/`%nrstr`). Engine déterministe pour les
    /// variables automatiques figées.
    fn expand(input: &str) -> String {
        MacroEngine::new(true).expand_open_code(input)
    }

    fn segments(src: &str) -> Vec<String> {
        let mut seg = RawSegmenter::new(src);
        let mut out = Vec::new();
        while let Some((s, e)) = seg.next_segment() {
            out.push(src[s..e].to_string());
        }
        out
    }

    #[test]
    fn segmenter_splits_on_run() {
        let segs = segments("data a; x=1; run; %put &x;");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0], "data a; x=1; run;");
        assert_eq!(segs[1], " %put &x;");
    }

    #[test]
    fn segmenter_ignores_semicolon_in_macro_def() {
        let segs = segments("%macro m; x=1; %mend; data a; run;");
        // Un seul segment : pas de run; avant la fin de la def, puis run;.
        assert_eq!(segs.len(), 1);
    }

    #[test]
    fn segmenter_ignores_semicolon_in_string() {
        let segs = segments("data a; t='x;y'; run; data b; run;");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0], "data a; t='x;y'; run;");
    }

    #[test]
    fn segmenter_trailing_open_code() {
        let segs = segments("%let x=1; %put &x;");
        // Pas de run; → un seul segment.
        assert_eq!(segs.len(), 1);
    }

    #[test]
    fn let_then_ref() {
        assert_eq!(run("%let x = 5; y = &x;"), "y = 5;");
    }

    #[test]
    fn dot_terminator() {
        // Un seul point consommé : &lib. -> work, puis `.a` reste.
        assert_eq!(run("%let lib = work; data &lib..a;"), "data work.a;");
    }

    #[test]
    fn rhs_resolution() {
        assert_eq!(run("%let a = 1; %let b = &a; z = &b;"), "z = 1;");
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(run("%let Foo = 7; w = &FOO;"), "w = 7;");
    }

    #[test]
    fn undefined_left_verbatim() {
        assert_eq!(run("x = &zzz;"), "x = &zzz;");
    }

    #[test]
    fn bare_ampersand_untouched() {
        assert_eq!(run("if a & b;"), "if a & b;");
    }

    #[test]
    fn newline_after_let_preserved() {
        assert_eq!(run("%let x = 5;\ny = &x;"), "\ny = 5;");
    }

    // --- M11.2 : %macro / invocation / %local / %global ---

    #[test]
    fn macro_positional_and_default() {
        assert_eq!(run("%macro p(a,b=2); x=&a+&b; %mend; %p(1)"), "x=1+2;");
    }

    #[test]
    fn macro_keyword_override() {
        assert_eq!(run("%macro p(a,b=2); x=&a+&b; %mend; %p(1,b=9)"), "x=1+9;");
    }

    #[test]
    fn macro_positional_default_mix() {
        // a fourni positionnellement, b prend son défaut.
        assert_eq!(run("%macro p(a,b=2,c=3); &a-&b-&c; %mend; %p(7)"), "7-2-3;");
    }

    #[test]
    fn macro_too_few_args_uses_empty_for_positional() {
        // a non fourni -> chaîne vide ; b défaut. (Corps trimé aux bords.)
        assert_eq!(run("%macro p(a,b=2); [&a][&b] %mend; %p()"), "[][2]");
    }

    #[test]
    fn macro_too_many_positional_args_ignored() {
        // 2e positionnel excédentaire ignoré (un seul paramètre).
        assert_eq!(run("%macro p(a); val=&a; %mend; %p(1,2,3)"), "val=1;");
    }

    #[test]
    fn macro_definition_emits_nothing() {
        // La seule définition ne produit aucune sortie (hormis nl préservé).
        assert_eq!(run("%macro q(x); y=&x; %mend;"), "");
        assert_eq!(run("%macro q(x); y=&x; %mend;\n"), "\n");
    }

    #[test]
    fn macro_nested_calls_another_macro() {
        let src = "%macro inner(z); [&z] %mend; \
                   %macro outer(a); pre %inner(&a) post %mend; \
                   %outer(7)";
        assert_eq!(run(src), "pre [7] post");
    }

    #[test]
    fn macro_call_without_parens() {
        // Appel sans parenthèses : le `;` qui suit termine l'appel (consommé),
        // corps `hello` trimé puis collé à `world`.
        assert_eq!(run("%macro hi; hello %mend; %hi;world"), "helloworld");
    }

    #[test]
    fn macro_mend_with_name() {
        assert_eq!(run("%macro p(a); &a %mend p; %p(ok)"), "ok");
    }

    #[test]
    fn local_does_not_leak() {
        // %local v confine le %let à la macro : v reste indéfini en open code.
        let src = "%macro m; %local v; %let v = inside; got=&v; %mend; \
                   %m got2=&v;";
        // Dans la macro &v -> inside ; après, &v indéfini -> verbatim.
        assert_eq!(run(src), "got=inside; got2=&v;");
    }

    #[test]
    fn global_let_in_macro_leaks() {
        // Sans %local, le %let crée un global visible après l'appel.
        let src = "%macro m; %let g = out; in=&g; %mend; %m after=&g;";
        assert_eq!(run(src), "in=out; after=out;");
    }

    #[test]
    fn global_decl_creates_symbol() {
        // %global puis %let global, lu en open code.
        assert_eq!(run("%global gg; %let gg = 5; v=&gg;"), "v=5;");
    }

    #[test]
    fn undefined_macro_call_left_verbatim() {
        // %zzz non défini : laissé verbatim.
        assert_eq!(run("a %zzz b"), "a %zzz b");
        assert_eq!(run("a %zzz(1) b"), "a %zzz(1) b");
    }

    #[test]
    fn recursion_guard_does_not_panic() {
        // Auto-appel infini : la garde coupe sans paniquer.
        let out = run("%macro r; %r %mend; %r");
        assert!(out.contains("recursion limit"), "got: {out}");
    }

    #[test]
    fn arg_with_macro_ref_resolved_in_caller() {
        // &x défini en open code, passé à la macro.
        let src = "%macro p(a); v=&a; %mend; %let x = 9; %p(&x)";
        assert_eq!(run(src), "v=9;");
    }

    // --- M11.4 : %eval (évaluateur d'expression entière) ---

    fn eval(expr: &str) -> Result<i64, MacroError> {
        MacroStage::default().macro_eval(expr)
    }

    #[test]
    fn eval_precedence() {
        assert_eq!(eval("3+4*2").unwrap(), 11);
    }

    #[test]
    fn eval_integer_division_truncates() {
        assert_eq!(eval("7/2").unwrap(), 3);
        assert_eq!(eval("-7/2").unwrap(), -3); // tronqué vers zéro
    }

    #[test]
    fn eval_power() {
        assert_eq!(eval("2**10").unwrap(), 1024);
        // Associatif à droite : 2**3**2 = 2**9 = 512.
        assert_eq!(eval("2**3**2").unwrap(), 512);
    }

    #[test]
    fn eval_logical_and() {
        assert_eq!(eval("1 and 0").unwrap(), 0);
        assert_eq!(eval("1 & 1").unwrap(), 1);
    }

    #[test]
    fn eval_comparison() {
        assert_eq!(eval("5 ge 5").unwrap(), 1);
        assert_eq!(eval("5 > 6").unwrap(), 0);
        assert_eq!(eval("3 = 3").unwrap(), 1);
        assert_eq!(eval("3 ne 4").unwrap(), 1);
        assert_eq!(eval("2 <> 2").unwrap(), 0); // <> = NE
    }

    #[test]
    fn eval_parens() {
        assert_eq!(eval("(1+2)*3").unwrap(), 9);
    }

    #[test]
    fn eval_unary_minus() {
        assert_eq!(eval("-3 + 5").unwrap(), 2);
        assert_eq!(eval("- -4").unwrap(), 4);
    }

    #[test]
    fn eval_not() {
        assert_eq!(eval("not 0").unwrap(), 1);
        assert_eq!(eval("^0").unwrap(), 1);
        assert_eq!(eval("not 5").unwrap(), 0);
    }

    #[test]
    fn eval_or() {
        assert_eq!(eval("0 or 0").unwrap(), 0);
        assert_eq!(eval("0 | 1").unwrap(), 1);
    }

    #[test]
    fn eval_non_integer_operand_errors() {
        let e = eval("abc + 1").unwrap_err();
        assert!(e.message.contains("character operand"), "got: {}", e.message);
    }

    #[test]
    fn eval_division_by_zero_errors() {
        let e = eval("1/0").unwrap_err();
        assert!(e.message.contains("Division by zero"), "got: {}", e.message);
    }

    #[test]
    fn eval_function_splices_in_open_code() {
        assert_eq!(run("x = %eval(3+4*2);"), "x = 11;");
        assert_eq!(run("x = %eval((1+2)*3);"), "x = 9;");
    }

    #[test]
    fn eval_function_with_macro_var() {
        assert_eq!(run("%let n = 4; x = %eval(&n*2);"), "x = 8;");
    }

    // --- M11.3 : %if / %then / %else ---

    #[test]
    fn if_simple_then_else() {
        assert_eq!(run("%if 1 %then a; %else b;"), "a;");
        assert_eq!(run("%if 0 %then a; %else b;"), "b;");
    }

    #[test]
    fn if_then_no_else_false_emits_nothing() {
        assert_eq!(run("%if 0 %then x;"), "");
    }

    #[test]
    fn if_with_do_groups() {
        assert_eq!(
            run("%if 0 %then %do; a=1; %end; %else %do; a=2; %end;"),
            "a=2;"
        );
        assert_eq!(
            run("%if 1 %then %do; a=1; %end; %else %do; a=2; %end;"),
            "a=1;"
        );
    }

    #[test]
    fn if_condition_uses_macro_var() {
        assert_eq!(run("%let n = 5; %if &n ge 5 %then big; %else small;"), "big;");
        assert_eq!(run("%let n = 1; %if &n ge 5 %then big; %else small;"), "small;");
    }

    #[test]
    fn if_condition_uses_eval_expression() {
        assert_eq!(run("%if 3+4 gt 5 %then yes; %else no;"), "yes;");
    }

    // --- M11.3 : %do / %end (groupe) et itératif ---

    #[test]
    fn do_group_plain() {
        assert_eq!(run("%do; a=1; b=2; %end;"), "a=1; b=2;");
    }

    #[test]
    fn iterative_do_basic() {
        let src = "%macro g(n); %do i=1 %to &n; v&i=&i; %end; %mend; %g(3)";
        assert_eq!(run(src), "v1=1; v2=2; v3=3;");
    }

    #[test]
    fn iterative_do_with_by() {
        let src = "%macro g; %do i=1 %to 5 %by 2; [&i] %end; %mend; %g";
        assert_eq!(run(src), "[1] [3] [5]");
    }

    #[test]
    fn iterative_do_zero_iterations() {
        // start > stop avec pas positif -> aucune itération.
        let src = "%macro g; pre%do i=5 %to 1; x%end;post %mend; %g";
        assert_eq!(run(src), "prepost");
    }

    #[test]
    fn iterative_do_negative_step() {
        let src = "%macro g; %do i=3 %to 1 %by -1; [&i] %end; %mend; %g";
        assert_eq!(run(src), "[3] [2] [1]");
    }

    #[test]
    fn iterative_do_in_open_code() {
        assert_eq!(run("%do i=1 %to 3; n&i; %end;"), "n1; n2; n3;");
    }

    #[test]
    fn if_do_nested_in_macro_body() {
        let src = "%macro m(n); \
                   %do i=1 %to &n; \
                   %if &i ge 2 %then big&i; %else small&i; \
                   %end; \
                   %mend; %m(3)";
        // i=1 -> small1 ; i=2 -> big2 ; i=3 -> big3.
        assert_eq!(run(src), "small1; big2; big3;");
    }

    #[test]
    fn runaway_loop_guard_does_not_hang() {
        // step négatif avec start<stop et pas négatif s'arrête tout de suite ;
        // ici on teste le pas nul -> erreur propre, pas de hang.
        let out = run("%do i=1 %to 10 %by 0; x %end;");
        assert!(out.contains("step is zero"), "got: {out}");
    }

    // --- M12.1 : %do %while / %do %until ---

    #[test]
    fn do_while_counter_loop() {
        let out = run("%let i=1; %do %while(&i <= 3); v&i=&i; %let i=%eval(&i+1); %end;");
        assert_eq!(out, "v1=1; v2=2; v3=3;");
    }

    #[test]
    fn do_until_counter_loop() {
        let out = run("%let i=1; %do %until(&i > 3); v&i=&i; %let i=%eval(&i+1); %end;");
        assert_eq!(out, "v1=1; v2=2; v3=3;");
    }

    #[test]
    fn do_while_zero_iterations() {
        // Condition fausse d'emblée -> aucune itération, sortie vide.
        let out = run("%let i=5; %do %while(&i < 3); v&i=&i; %end;");
        assert_eq!(out, "");
    }

    #[test]
    fn do_until_runs_at_least_once() {
        // Condition déjà vraie à l'entrée : `%until` itère quand même une fois.
        let out = run("%let i=5; %do %until(&i > 3); hit; %end;");
        assert_eq!(out, "hit;");
    }

    #[test]
    fn do_while_inside_macro_body() {
        let src = "%macro m; %let i=1; %do %while(&i <= 3); v&i=&i; %let i=%eval(&i+1); %end; %mend; %m";
        let out = run(src);
        assert_eq!(out.trim(), "v1=1; v2=2; v3=3;");
    }

    #[test]
    fn do_while_runaway_guard() {
        // Condition toujours vraie, jamais mise à jour -> garde anti-runaway.
        let out = run("%do %while(1); x %end;");
        assert!(out.contains("runaway guard"), "got: {out}");
    }

    // --- M11.6 : &&& / &&var&i nested indirection ---

    #[test]
    fn triple_ampersand_indirection() {
        // &&&y : y -> x, &x -> ab.
        assert_eq!(run("%let x=ab; %let y=x; &&&y"), "ab");
    }

    #[test]
    fn double_ampersand_with_index() {
        // &&v&i : i -> 2, v2 -> hit.
        assert_eq!(run("%let i=2; %let v2=hit; &&v&i"), "hit");
    }

    // --- M11.6 : %sysfunc ---

    #[test]
    fn sysfunc_upcase() {
        assert_eq!(run("%sysfunc(upcase(abc))"), "ABC");
    }

    #[test]
    fn sysfunc_substr() {
        assert_eq!(run("%sysfunc(substr(abcdef,2,3))"), "bcd");
    }

    #[test]
    fn sysfunc_with_macro_var_arg() {
        assert_eq!(run("%let w=hello; %sysfunc(upcase(&w))"), "HELLO");
    }

    #[test]
    fn sysfunc_numeric_function() {
        assert_eq!(run("%sysfunc(length(abcd))"), "4");
    }

    #[test]
    fn sysfunc_unknown_function_errors_no_panic() {
        let out = run("%sysfunc(nosuchfn(1))");
        assert!(out.contains("not supported") || out.contains("unknown"), "got: {out}");
    }

    // --- M11.6 : automatic macro variables (deterministic frozen) ---

    #[test]
    fn auto_var_sysdate9() {
        assert_eq!(expand("&sysdate9"), "01JAN1960");
    }

    #[test]
    fn auto_var_sysver() {
        assert_eq!(expand("&sysver"), "9.4");
    }

    #[test]
    fn auto_var_systime_and_sysday() {
        assert_eq!(expand("&systime &sysday"), "00:00 Friday");
    }

    // --- M11.6 : %str / %nrstr quoting ---

    #[test]
    fn str_masks_semicolon_and_comma() {
        // Le `;` et le `,` internes sont littéraux (non terminateurs).
        assert_eq!(expand("%str(a;b,c)"), "a;b,c");
    }

    #[test]
    fn str_semicolon_does_not_terminate() {
        // Sans %str, `;` terminerait ; avec %str il reste dans la valeur.
        assert_eq!(expand("%let v=%str(a;b); &v"), "a;b");
    }

    #[test]
    fn str_resolves_ampersand() {
        // %str masque la ponctuation mais &x reste résolu.
        assert_eq!(expand("%let x=Z; %str(&x;y)"), "Z;y");
    }

    #[test]
    fn nrstr_leaves_triggers_unresolved() {
        // %nrstr masque & et % : %macro et &x ne sont pas résolus.
        assert_eq!(expand("%nrstr(%macro &x)"), "%macro &x");
    }

    // --- M12.2 : fonctions chaîne macro simples ---

    #[test]
    fn macro_fn_upcase_lowcase() {
        assert_eq!(expand("%upcase(abc)"), "ABC");
        assert_eq!(expand("%lowcase(ABC)"), "abc");
    }

    #[test]
    fn macro_fn_substr() {
        assert_eq!(expand("%substr(abcdef,2,3)"), "bcd");
        // Sans longueur : jusqu'à la fin.
        assert_eq!(expand("%substr(abcdef,4)"), "def");
    }

    #[test]
    fn macro_fn_scan() {
        assert_eq!(expand("%scan(a.b.c,2,.)"), "b");
        // Délimiteurs par défaut (blanc).
        assert_eq!(expand("%scan(one two three,3)"), "three");
        // Index depuis la fin.
        assert_eq!(expand("%scan(a.b.c,-1,.)"), "c");
    }

    #[test]
    fn macro_fn_index_length() {
        assert_eq!(expand("%index(abcdef,cd)"), "3");
        assert_eq!(expand("%index(abcdef,zz)"), "0");
        assert_eq!(expand("%length(abcd)"), "4");
    }

    #[test]
    fn macro_fn_resolves_refs_in_args() {
        // Les `&refs` des arguments sont résolus avant calcul.
        assert_eq!(expand("%let w=hello; %upcase(&w)"), "HELLO");
    }

    // --- M12.2 : %superq ---

    #[test]
    fn superq_returns_value_without_resolving() {
        // v = "a&b" avec b indéfini ; %superq(v) rend "a&b" littéral, sans
        // tenter de résoudre &b (donc pas d'expansion). On utilise %nrstr pour
        // stocker la valeur sans déclencher la résolution au moment du %let.
        assert_eq!(expand("%let v=%nrstr(a&b); %superq(v)"), "a&b");
    }

    #[test]
    fn superq_undefined_is_empty() {
        assert_eq!(expand("[%superq(nope)]"), "[]");
    }

    // --- M12.2 : %bquote / %nrbquote ---

    #[test]
    fn bquote_masks_comma_and_semicolon() {
        // `,` et `;` restent littéraux dans la sortie finale.
        assert_eq!(expand("%bquote(a,b;c)"), "a,b;c");
    }

    #[test]
    fn bquote_semicolon_does_not_terminate_let() {
        assert_eq!(expand("%let v=%bquote(a;b); &v"), "a;b");
    }

    #[test]
    fn bquote_unmatched_quote_ok() {
        // Une quote non appariée dans l'argument ne fait pas planter : elle est
        // traitée comme un caractère ordinaire puis masquée (littérale en sortie).
        assert_eq!(expand("%bquote(it's a test)"), "it's a test");
    }

    #[test]
    fn bquote_unmatched_paren_stays_verbatim() {
        // Parenthèse non appariée → l'appel `%bquote` n'est pas reconnu comme
        // équilibré ; on ne plante pas, le texte reste verbatim (pas d'erreur).
        let s = expand("%bquote(a (b)");
        assert!(s.contains("%bquote"));
    }

    #[test]
    fn bquote_resolves_then_masks() {
        // &x est résolu, puis le `;` reste littéral.
        assert_eq!(expand("%let x=Z; %bquote(&x;y)"), "Z;y");
    }

    #[test]
    fn nrbquote_masks_triggers_in_result() {
        // nrbquote masque les `&`/`%` résiduels : &z (indéfini) reste littéral
        // et inerte. (Après résolution &z est inchangé, puis masqué.)
        assert_eq!(expand("%nrbquote(a&z b)"), "a&z b");
    }

    // --- M12.2 : variantes %q* (résultat masqué) ---

    #[test]
    fn qsysfunc_upcase_masked() {
        assert_eq!(expand("%qsysfunc(upcase(abc))"), "ABC");
    }

    #[test]
    fn qupcase_qlowcase() {
        assert_eq!(expand("%qupcase(abc)"), "ABC");
        assert_eq!(expand("%qlowcase(ABC)"), "abc");
    }

    #[test]
    fn qsubstr_qscan() {
        assert_eq!(expand("%qsubstr(abcdef,2,3)"), "bcd");
        assert_eq!(expand("%qscan(a.b.c,2,.)"), "b");
    }

    #[test]
    fn qupcase_masks_residual_ampersand() {
        // x indéfini : &x reste, est mis en MAJ (inchangé), puis masqué donc
        // inerte ; la sortie finale (unmask) montre `&X` littéral.
        assert_eq!(expand("%qupcase(a&x)"), "A&X");
    }

    // --- M19.1 : %unquote ---

    #[test]
    fn unquote_reenables_resolution_after_nrstr() {
        // %nrstr masque le `&` : sans %unquote, `&x` reste littéral. %unquote
        // ré-active la résolution → la valeur de x est splicée.
        assert_eq!(expand("%let x=hi; %unquote(%nrstr(&x))"), "hi");
    }

    #[test]
    fn unquote_roundtrip_plain_text() {
        // Texte sans déclencheur : %unquote est l'identité.
        assert_eq!(expand("%unquote(abc)"), "abc");
    }

    #[test]
    fn unquote_reenables_macro_call() {
        // %nrstr masque le `%` d'un appel ; %unquote le ré-active → la macro
        // s'exécute et émet son corps.
        assert_eq!(expand("%macro m; got %mend; %unquote(%nrstr(%m))"), "got");
    }

    // --- M19.1 : %cmpres / %qcmpres ---

    #[test]
    fn cmpres_compresses_internal_blanks() {
        assert_eq!(expand("%cmpres(a    b     c)"), "a b c");
    }

    #[test]
    fn cmpres_trims_edges() {
        assert_eq!(expand("%cmpres(   hello   world   )"), "hello world");
    }

    #[test]
    fn cmpres_resolves_refs() {
        assert_eq!(expand("%let v=  x   y  ; %cmpres(&v)"), "x y");
    }

    #[test]
    fn qcmpres_masks_result() {
        // Le résultat de %qcmpres est masqué : un `;` interne ne termine pas le
        // %let. La valeur stockée (puis ré-émise) garde le `;` littéral.
        assert_eq!(expand("%let v=%qcmpres(a ;  b); &v"), "a ; b");
    }

    // --- M19.1 : %symexist ---

    #[test]
    fn symexist_found() {
        assert_eq!(expand("%let a=1; %symexist(a)"), "1");
    }

    #[test]
    fn symexist_not_found() {
        assert_eq!(expand("%symexist(nope)"), "0");
    }

    #[test]
    fn symexist_accepts_ampersand_name() {
        // %symexist(&which) : &which désigne le NOM à tester.
        assert_eq!(expand("%let a=1; %let which=a; %symexist(&which)"), "1");
    }

    // --- M19.1 : %sysmexist ---

    #[test]
    fn sysmexist_defined_macro() {
        assert_eq!(expand("%macro foo; %mend; %sysmexist(foo)"), "1");
    }

    #[test]
    fn sysmexist_undefined_macro() {
        assert_eq!(expand("%sysmexist(bar)"), "0");
    }

    // --- M19.1 : %sysget (env var posée en mémoire dans le test) ---

    #[test]
    fn sysget_reads_env_var() {
        // SAFETY: test mono-thread sur une variable d'env dédiée à ce test ;
        // posée puis retirée localement.
        unsafe {
            std::env::set_var("SASRS_TEST_VAR_M19", "hello_env");
        }
        assert_eq!(expand("%sysget(SASRS_TEST_VAR_M19)"), "hello_env");
        unsafe {
            std::env::remove_var("SASRS_TEST_VAR_M19");
        }
    }

    #[test]
    fn sysget_unset_is_empty() {
        // SAFETY: variable d'env dédiée, jamais posée ailleurs.
        unsafe {
            std::env::remove_var("SASRS_DEFINITELY_UNSET_M19");
        }
        assert_eq!(expand("%sysget(SASRS_DEFINITELY_UNSET_M19)"), "");
    }

    // --- M19.1 : %sysevalf (évaluation flottante) ---

    #[test]
    fn sysevalf_float_division() {
        assert_eq!(expand("%sysevalf(7/2)"), "3.5");
    }

    #[test]
    fn sysevalf_vs_eval_integer_division() {
        // %eval tronque (entier) ; %sysevalf est réel.
        assert_eq!(expand("%eval(7/2)"), "3");
        assert_eq!(expand("%sysevalf(7/2)"), "3.5");
    }

    #[test]
    fn sysevalf_decimal_literals() {
        assert_eq!(expand("%sysevalf(0.5 + 0.25)"), "0.75");
    }

    #[test]
    fn sysevalf_integer_result_has_no_decimals() {
        assert_eq!(expand("%sysevalf(4/2)"), "2");
    }

    #[test]
    fn sysevalf_conv_boolean() {
        assert_eq!(expand("%sysevalf(3.5, boolean)"), "1");
        assert_eq!(expand("%sysevalf(0, boolean)"), "0");
    }

    #[test]
    fn sysevalf_conv_ceil_floor_integer() {
        assert_eq!(expand("%sysevalf(7/2, ceil)"), "4");
        assert_eq!(expand("%sysevalf(7/2, floor)"), "3");
        assert_eq!(expand("%sysevalf(7/2, integer)"), "3");
        assert_eq!(expand("%sysevalf(-7/2, integer)"), "-3");
        assert_eq!(expand("%sysevalf(-7/2, floor)"), "-4");
    }

    #[test]
    fn sysevalf_resolves_refs() {
        assert_eq!(expand("%let n=5; %sysevalf(&n / 2)"), "2.5");
    }

    #[test]
    fn sysevalf_power_is_real() {
        assert_eq!(expand("%sysevalf(2 ** 0.5)"), f64::sqrt(2.0).to_string());
    }

    #[test]
    fn sysevalf_syntax_error_no_panic() {
        let out = expand("%sysevalf(2 + + )");
        assert!(out.contains("ERROR"), "got: {out}");
    }

    // --- M19.2 : %include + bibliothèques autocall (SASAUTOS) ---

    use std::io::Write;

    /// Crée un engine déterministe dont la base d'inclusion est `dir`.
    fn engine_in(dir: &std::path::Path) -> MacroEngine {
        let mut e = MacroEngine::new(true);
        e.set_include_base_dir(dir.to_path_buf());
        e
    }

    /// Écrit `content` dans `dir/name` et rend le chemin.
    fn write_file(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn include_simple_relative_path() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "inc.sas", "%let x = 42;");
        let mut e = engine_in(dir.path());
        // Le %include charge inc.sas (pose &x), puis &x se résout.
        let out = e.expand_open_code("%include 'inc.sas'; &x");
        assert_eq!(out.trim(), "42");
    }

    #[test]
    fn include_double_quotes() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "inc.sas", "data a;run;");
        let mut e = engine_in(dir.path());
        let out = e.expand_open_code("%include \"inc.sas\";");
        assert!(out.contains("data a;run;"), "got: {out}");
    }

    #[test]
    fn include_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_file(dir.path(), "abs.sas", "%let y = hi;");
        // Engine sans base : on utilise un chemin absolu.
        let mut e = MacroEngine::new(true);
        let stmt = format!("%include '{}'; &y", p.display());
        let out = e.expand_open_code(&stmt);
        assert_eq!(out.trim(), "hi");
    }

    #[test]
    fn include_defines_macro_then_invoked() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "mac.sas", "%macro greet; hello %mend;");
        let mut e = engine_in(dir.path());
        // Le fichier inclus DÉFINIT %greet ; l'appel suivant l'expanse.
        let out = e.expand_open_code("%include 'mac.sas'; %greet");
        assert_eq!(out.trim(), "hello");
    }

    #[test]
    fn include_nested() {
        let dir = tempfile::tempdir().unwrap();
        // a.sas inclut b.sas ; b.sas pose &z.
        write_file(dir.path(), "b.sas", "%let z = nested;");
        write_file(dir.path(), "a.sas", "%include 'b.sas';");
        let mut e = engine_in(dir.path());
        let out = e.expand_open_code("%include 'a.sas'; &z");
        assert_eq!(out.trim(), "nested");
    }

    #[test]
    fn include_missing_file_emits_note_no_panic() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = engine_in(dir.path());
        let out = e.expand_open_code("%include 'does_not_exist.sas'; after");
        assert!(out.contains("cannot read"), "got: {out}");
        // Le scan se poursuit après le statement.
        assert!(out.contains("after"), "got: {out}");
    }

    #[test]
    fn include_cycle_hits_depth_limit_no_panic() {
        let dir = tempfile::tempdir().unwrap();
        // self.sas s'inclut lui-même : la garde de profondeur arrête le cycle.
        write_file(dir.path(), "self.sas", "%include 'self.sas';");
        let mut e = engine_in(dir.path());
        let out = e.expand_open_code("%include 'self.sas';");
        assert!(out.contains("nesting limit"), "got: {out}");
    }

    #[test]
    fn include_fileref_form_unsupported_note() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = engine_in(dir.path());
        let out = e.expand_open_code("%include myref; tail");
        assert!(out.contains("only quoted file paths"), "got: {out}");
        assert!(out.contains("tail"), "got: {out}");
    }

    #[test]
    fn autocall_basic() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "sayhi.sas", "%macro sayhi; HI %mend;");
        let mut e = MacroEngine::new(true);
        e.set_sasautos_path(vec![dir.path().to_path_buf()]);
        // %sayhi non défini : chargé paresseusement depuis sayhi.sas.
        let out = e.expand_open_code("%sayhi");
        assert_eq!(out.trim(), "HI");
    }

    #[test]
    fn autocall_with_args() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "dbl.sas",
            "%macro dbl(x); &x&x %mend;",
        );
        let mut e = MacroEngine::new(true);
        e.set_sasautos_path(vec![dir.path().to_path_buf()]);
        let out = e.expand_open_code("%dbl(ab)");
        assert_eq!(out.trim(), "abab");
    }

    #[test]
    fn autocall_nested() {
        let dir = tempfile::tempdir().unwrap();
        // outer appelle inner ; les deux sont des fichiers autocall.
        write_file(dir.path(), "inner.sas", "%macro inner; IN %mend;");
        write_file(
            dir.path(),
            "outer.sas",
            "%macro outer; [%inner] %mend;",
        );
        let mut e = MacroEngine::new(true);
        e.set_sasautos_path(vec![dir.path().to_path_buf()]);
        let out = e.expand_open_code("%outer");
        assert_eq!(out.trim(), "[IN]");
    }

    #[test]
    fn autocall_first_dir_wins() {
        let d1 = tempfile::tempdir().unwrap();
        let d2 = tempfile::tempdir().unwrap();
        write_file(d1.path(), "pick.sas", "%macro pick; ONE %mend;");
        write_file(d2.path(), "pick.sas", "%macro pick; TWO %mend;");
        let mut e = MacroEngine::new(true);
        e.set_sasautos_path(vec![d1.path().to_path_buf(), d2.path().to_path_buf()]);
        let out = e.expand_open_code("%pick");
        assert_eq!(out.trim(), "ONE");
    }

    #[test]
    fn autocall_not_found_left_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = MacroEngine::new(true);
        e.set_sasautos_path(vec![dir.path().to_path_buf()]);
        // Macro introuvable : `%nope` laissé verbatim (comportement historique).
        let out = e.expand_open_code("%nope");
        assert_eq!(out, "%nope");
    }

    #[test]
    fn autocall_tried_only_once() {
        // Même sans fichier, la deuxième invocation ne doit pas re-tenter le
        // disque ni paniquer ; le résultat reste verbatim.
        let dir = tempfile::tempdir().unwrap();
        let mut e = MacroEngine::new(true);
        e.set_sasautos_path(vec![dir.path().to_path_buf()]);
        let out = e.expand_open_code("%miss %miss");
        assert_eq!(out, "%miss %miss");
    }

    #[test]
    fn defined_macro_takes_priority_over_autocall() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "m.sas", "%macro m; FROMDISK %mend;");
        let mut e = MacroEngine::new(true);
        e.set_sasautos_path(vec![dir.path().to_path_buf()]);
        // Définition inline : elle prime, autocall n'est pas consulté.
        let out = e.expand_open_code("%macro m; INLINE %mend; %m");
        assert_eq!(out.trim(), "INLINE");
    }

    // --- M19.3 : trace options + %put + %call execute ---

    #[test]
    fn put_simple_text() {
        let mut e = MacroEngine::new(true);
        let _out = e.expand_open_code("%put Hello world;");
        let logs = e.take_pending_log_lines();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0], "Hello world");
    }

    #[test]
    fn put_with_symbol_resolution() {
        let mut e = MacroEngine::new(true);
        let _out = e.expand_open_code("%let name=Alice; %put Hello &name;");
        let logs = e.take_pending_log_lines();
        assert!(logs.iter().any(|l| l.contains("Hello Alice")));
    }

    #[test]
    fn put_empty_line() {
        let mut e = MacroEngine::new(true);
        let _out = e.expand_open_code("%put;");
        let logs = e.take_pending_log_lines();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0], "");
    }

    #[test]
    fn mprint_flag_echoes_macro_output() {
        let mut e = MacroEngine::new(true);
        e.set_mprint(true);
        let _out = e.expand_open_code("%macro m; DATA x; RUN; %mend; %m");
        let logs = e.take_pending_log_lines();
        assert!(logs.iter().any(|l| l.starts_with("MPRINT(M):")));
        assert!(logs.iter().any(|l| l.contains("DATA x")));
    }

    #[test]
    fn mlogic_flag_echoes_macro_entry_exit() {
        let mut e = MacroEngine::new(true);
        e.set_mlogic(true);
        let _out = e.expand_open_code("%macro m(a=1); x=&a; %mend; %m(a=5)");
        let logs = e.take_pending_log_lines();
        assert!(logs.iter().any(|l| l.contains("Beginning execution")));
        assert!(logs.iter().any(|l| l.contains("Parameter A has value 5")));
        assert!(logs.iter().any(|l| l.contains("Ending execution")));
    }

    #[test]
    fn mlogic_flag_echoes_if_condition() {
        let mut e = MacroEngine::new(true);
        e.set_mlogic(true);
        let _out = e.expand_open_code("%macro m; %if 1=1 %then YES; %else NO; %mend; %m");
        let logs = e.take_pending_log_lines();
        assert!(logs.iter().any(|l| l.contains("is TRUE")));
    }

    #[test]
    fn symbolgen_flag_echoes_symbol_resolution() {
        let mut e = MacroEngine::new(true);
        e.set_symbolgen(true);
        // SYMBOLGEN traces when a symbol is USED in the expansion, not just defined
        let _out = e.expand_open_code("%let x=abc; data &x;");
        let logs = e.take_pending_log_lines();
        assert!(logs.iter().any(|l| l.contains("Macro variable X resolves to abc")), "got logs: {:?}", logs);
    }

    #[test]
    fn call_execute_queues_code() {
        let mut e = MacroEngine::new(true);
        let _out = e.expand_open_code("%macro m; %call execute(data step here;); %mend; %m");
        let queue = e.take_pending_call_execute();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0], "data step here;");
    }

    #[test]
    fn call_execute_resolves_symbols() {
        let mut e = MacroEngine::new(true);
        let _out = e.expand_open_code("%let step=SET x; %macro m; %call execute(&step run;); %mend; %m");
        let queue = e.take_pending_call_execute();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0], "SET x run;");
    }

    #[test]
    fn multiple_trace_flags_interact() {
        let mut e = MacroEngine::new(true);
        e.set_mprint(true);
        e.set_mlogic(true);
        e.set_symbolgen(true);
        let _out = e.expand_open_code(
            "%let x=5; %macro m; %if &x > 3 %then YES; %mend; %m"
        );
        let logs = e.take_pending_log_lines();
        // Should have logs from MLOGIC and MPRINT at minimum
        assert!(logs.iter().any(|l| l.contains("MLOGIC")), "got logs: {:?}", logs);
        assert!(logs.iter().any(|l| l.contains("is TRUE")), "got logs: {:?}", logs);
        assert!(logs.iter().any(|l| l.contains("MPRINT")), "got logs: {:?}", logs);
    }
}
