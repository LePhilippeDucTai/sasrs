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

/// No-op stage used until the macro facility is implemented.
pub struct IdentityMacroStage;

impl TextStage for IdentityMacroStage {
    fn process(&mut self, source: &str) -> String {
        source.to_string()
    }
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
/// déclencheur macro résolu. Sous le build PAR DÉFAUT (sans `--features
/// macros`), l'engine n'a aucune table et `expand_open_code` renvoie l'entrée
/// inchangée — comportement identique à l'ancien `IdentityMacroStage`.
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
#[cfg(feature = "macros")]
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
}

/// Définition d'une macro capturée par `%macro name(params); <body> %mend;`.
///
/// `body` est le texte VERBATIM entre le `;` qui clôt la liste de paramètres et
/// le `%mend` correspondant. Il n'est PAS expansé à la définition ; il l'est à
/// chaque invocation, dans la portée locale créée pour cet appel.
#[cfg(feature = "macros")]
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
#[cfg(feature = "macros")]
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
#[cfg(feature = "macros")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MacroError {
    /// Message lisible (proche du libellé SAS quand pertinent).
    pub message: String,
}

#[cfg(feature = "macros")]
impl MacroError {
    fn new(msg: impl Into<String>) -> Self {
        MacroError { message: msg.into() }
    }
}

#[cfg(feature = "macros")]
impl std::fmt::Display for MacroError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// Variante PAR DÉFAUT (sans feature `macros`) : engine vide, identité pure.
/// Aucune table, aucune logique — garantit l'octet-identité du build par
/// défaut (équivalent strict de l'ancien `IdentityMacroStage`).
#[cfg(not(feature = "macros"))]
#[derive(Default)]
pub struct MacroEngine;

impl MacroEngine {
    /// Construit l'engine de session. (Le paramètre `deterministic` est réservé
    /// pour les variables automatiques figées des unités M11 ultérieures —
    /// `&SYSDATE9`/`&SYSTIME`/`&SYSVER` ; inutilisé pour `%let`/`&var`.)
    pub fn new(_deterministic: bool) -> Self {
        Self::default()
    }

    /// Expanse un segment de "open code" (texte SAS hors corps de `%macro`).
    ///
    /// Sous `--features macros` : applique le `%let`/`&var` du spike. Pour un
    /// segment SANS déclencheur (`%`/`&`) le résultat est l'entrée inchangée.
    /// Sous le build par défaut : identité stricte.
    #[cfg(feature = "macros")]
    pub fn expand_open_code(&mut self, raw: &str) -> String {
        // Fast-path identité : sans déclencheur macro, rien à expanser. Garantit
        // l'invariant byte-identical pour le source sans tokens macro.
        if !raw.contains('%') && !raw.contains('&') {
            return raw.to_string();
        }
        self.process(raw)
    }

    /// Build par défaut : identité stricte (équivalent `IdentityMacroStage`).
    #[cfg(not(feature = "macros"))]
    pub fn expand_open_code(&mut self, raw: &str) -> String {
        raw.to_string()
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
#[cfg(feature = "macros")]
pub type MacroStage = MacroEngine;

#[cfg(feature = "macros")]
impl TextStage for MacroEngine {
    fn process(&mut self, source: &str) -> String {
        self.process_impl(source)
    }
}

#[cfg(feature = "macros")]
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

#[cfg(feature = "macros")]
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

            // Invocation `%name` ou `%name(args)` d'une macro DÉFINIE.
            if c == '%' {
                if let Some((name, after)) = Self::read_name(&chars, i + 1) {
                    if self.macros.contains_key(&name.to_uppercase()) {
                        let next = self.expand_invocation(&chars, i + 1, &name, after, &mut out);
                        i = next;
                        continue;
                    }
                }
            }

            if c == '&' {
                // `&&` -> un seul `&`.
                if chars.get(i + 1) == Some(&'&') {
                    out.push('&');
                    i += 2;
                    continue;
                }
                if let Some((name, after)) = Self::read_name(&chars, i + 1) {
                    let mut next = after;
                    if chars.get(next) == Some(&'.') {
                        next += 1;
                    }
                    match self.lookup(&name) {
                        Some(v) => out.push_str(&self.resolve_value(&v)),
                        None => {
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

#[cfg(feature = "macros")]
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
        // Valeur = tout jusqu'au prochain `;`.
        let val_start = j;
        while j < chars.len() && chars[j] != ';' {
            j += 1;
        }
        if chars.get(j) != Some(&';') {
            return None; // pas de `;` terminal : abandon, on n'avale rien.
        }
        let raw_value: String = chars[val_start..j].iter().collect();
        let resolved = self.resolve_value(&raw_value);
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

#[cfg(feature = "macros")]
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
        self.scopes.push(scope);
        self.depth += 1;
        let expanded = self.process_impl(&def.body);
        self.depth -= 1;
        self.scopes.pop();
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

#[cfg(feature = "macros")]
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
        // Vérifier `%do %while`/`%until` -> déféré.
        if Self::matches_kw(chars, j, "while") || Self::matches_kw(chars, j, "until") {
            let (_text, after) = Self::scan_do_block(chars, i)?;
            Self::emit_error(
                out,
                &MacroError::new("ERROR: %DO %WHILE/%UNTIL is not supported by this interpreter"),
            );
            return Some(after);
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
#[cfg(feature = "macros")]
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

#[cfg(feature = "macros")]
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
                _ if c.is_ascii_digit() => {
                    let start = i;
                    while matches!(chars.get(i), Some(d) if d.is_ascii_digit()) {
                        i += 1;
                    }
                    // Un opérande alphanumérique mixte (ex. `3a`) est un mot.
                    if matches!(chars.get(i), Some(d) if d.is_ascii_alphabetic() || *d == '_') {
                        let wstart = start;
                        while matches!(chars.get(i), Some(d) if d.is_ascii_alphanumeric() || *d == '_') {
                            i += 1;
                        }
                        let w: String = chars[wstart..i].iter().collect();
                        toks.push(EvalTok::Word(w));
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
#[cfg(feature = "macros")]
struct EvalParser<'a> {
    toks: &'a [EvalTok],
    pos: usize,
}

#[cfg(feature = "macros")]
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

#[cfg(all(test, feature = "macros"))]
mod macro_tests {
    use super::*;

    fn run(input: &str) -> String {
        MacroStage::default().process(input)
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

    #[test]
    fn do_while_until_deferred() {
        let out = run("%do %while(1); x %end;");
        assert!(out.contains("not supported"), "got: {out}");
    }
}
