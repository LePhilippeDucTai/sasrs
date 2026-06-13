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
#[cfg(feature = "macros")]
#[derive(Default)]
pub struct MacroEngine {
    table: std::collections::HashMap<String, String>,
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
                    match self.table.get(&name.to_uppercase()) {
                        Some(v) => out.push_str(v),
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
                    match self.table.get(&name.to_uppercase()) {
                        Some(v) => out.push_str(&self.resolve_value(v)),
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
        self.table
            .insert(name.to_uppercase(), resolved.trim().to_string());
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
}
