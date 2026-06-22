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

    // --- M35.1 : %sysfunc délègue à la bibliothèque COMPLÈTE (plus de whitelist) ---

    #[test]
    fn sysfunc_non_whitelisted_reverse() {
        // REVERSE n'était PAS dans l'ancienne liste blanche.
        assert_eq!(run("%sysfunc(reverse(abc))"), "cba");
    }

    #[test]
    fn sysfunc_non_whitelisted_repeat() {
        // REPEAT(x, 2) → 2 copies (implémentation s.repeat(n) de la lib).
        assert_eq!(run("%sysfunc(repeat(x,2))"), "xx");
    }

    #[test]
    fn sysfunc_non_whitelisted_propcase() {
        assert_eq!(run("%sysfunc(propcase(hello world))"), "Hello World");
    }

    #[test]
    fn sysfunc_non_whitelisted_math_sqrt() {
        assert_eq!(run("%sysfunc(sqrt(16))"), "4");
    }

    // --- M35.1 : argument de format optionnel ---

    #[test]
    fn sysfunc_format_dollar() {
        // sum(1000, 234.5) = 1234.5 reformaté en dollar10.2 ; les blancs de tête
        // sont retirés pour l'insertion macro.
        assert_eq!(run("%sysfunc(sum(1000,234.5), dollar10.2)"), "$1,234.50");
    }

    #[test]
    fn sysfunc_format_round_width() {
        // round(3.7) = 4, formaté en 8. → "4" (blancs de tête retirés).
        assert_eq!(run("%sysfunc(round(3.7), 8.)"), "4");
    }

    #[test]
    fn sysfunc_format_date9() {
        assert_eq!(run("%sysfunc(mdy(1,1,2020), date9.)"), "01JAN2020");
    }

    #[test]
    fn sysfunc_no_format_unchanged() {
        // Sans format : comportement identique à avant (texte brut).
        assert_eq!(run("%sysfunc(mdy(1,1,2020))"), "21915");
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
