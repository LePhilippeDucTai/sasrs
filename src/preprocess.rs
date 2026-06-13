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
}
