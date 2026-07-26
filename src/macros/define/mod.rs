//! Définition (`%macro`/`%mend`) et invocation (`%name(...)`) de macros, plus
//! les déclarations de portée (`%local`/`%global`) et `%call execute`.

use super::*;

mod types;

pub use types::MacroDef;
pub use types::MacroParam;

impl MacroEngine {
    /// Profondeur maximale d'invocation de macro (garde anti-récursion).
    pub(super) const MAX_MACRO_DEPTH: usize = 100;
}

mod invoke;

impl MacroEngine {
    /// Consomme une définition `%macro name[(params)] ; <body> %mend [name];`.
    /// Enregistre la définition et n'émet RIEN. Rend l'index après le `;` du
    /// `%mend` (un `\n` final juste après est préservé pour la numérotation),
    /// ou `None` si la syntaxe ne tient pas (le `%` est alors laissé brut).
    pub(super) fn consume_macro_def(
        &mut self,
        chars: &[char],
        i: usize,
        out: &mut String,
    ) -> Option<usize> {
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
        self.macros
            .insert(name.to_uppercase(), MacroDef { name, params, body });
        Some(j)
    }

    /// Parse une liste de paramètres `(p1, p2, kw=def, ...)` à partir de `(`.
    /// Rend `(params, index après `)`)`. `None` si pas de `)` fermant.
    pub(super) fn parse_param_list(
        chars: &[char],
        lparen: usize,
    ) -> Option<(Vec<MacroParam>, usize)> {
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
    pub(super) fn consume_scope_decl(
        &mut self,
        chars: &[char],
        i: usize,
        is_local: bool,
        out: &mut String,
    ) -> Option<usize> {
        let kwlen = if is_local {
            "%local".len()
        } else {
            "%global".len()
        };
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
}
