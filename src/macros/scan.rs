//! Scanners de bas niveau (associés à `MacroEngine`) opérant sur `&[char]`.
//!
//! Fonctions pures sans état : lecture de noms, parenthèses équilibrées,
//! détection de mots-clés `%kw`, recherche de `;`/`%end`, découpage d'arguments.

use super::*;

impl MacroEngine {
    /// Lit un nom SAS (lettre/`_` puis alnum/`_`) à partir de `chars[i]`.
    /// Rend `(nom, index après le nom)`, ou `None` si pas un nom valide.
    pub(super) fn read_name(chars: &[char], i: usize) -> Option<(String, usize)> {
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

    /// Vrai si `chars[i..]` commence par `%let` (insensible casse) suivi d'un
    /// blanc (pour ne pas matcher `%letx`).
    pub(super) fn matches_let(chars: &[char], i: usize) -> bool {
        let kw = ['%', 'l', 'e', 't'];
        for (k, &kc) in kw.iter().enumerate() {
            match chars.get(i + k) {
                Some(c) if c.to_ascii_lowercase() == kc => {}
                _ => return false,
            }
        }
        matches!(chars.get(i + 4), Some(c) if c.is_whitespace())
    }

    /// Préserve un éventuel `\n` immédiatement après l'index `j` (poussé dans
    /// `out`) afin de conserver la numérotation des lignes, comme le font les
    /// autres `consume_*`. Rend l'index après ce `\n` (ou `j` inchangé).
    pub(super) fn skip_trailing_newline(chars: &[char], mut j: usize, out: &mut String) -> usize {
        while matches!(chars.get(j), Some(c) if *c == ' ' || *c == '\t') {
            j += 1;
        }
        if chars.get(j) == Some(&'\n') {
            out.push('\n');
            j += 1;
        }
        j
    }

    /// Vrai si `chars[i..]` commence par `%<kw>` (insensible casse) suivi d'un
    /// non-identifiant (blanc, `(`, `;` ...). Évite de matcher `%macrox`.
    pub(super) fn matches_kw(chars: &[char], i: usize, kw: &str) -> bool {
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

    /// Vrai si `chars[i..]` commence par `%<kw>` (insensible casse) suivi
    /// éventuellement de blancs puis d'une `(` — pour les fonctions macro comme
    /// `%eval(...)`. Évite de matcher un identifiant plus long (`%evalx`).
    pub(super) fn matches_kw_paren(chars: &[char], i: usize, kw: &str) -> bool {
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

    /// Lit le contenu entre `(` (à l'index `lparen`) et sa `)` équilibrée.
    /// Rend `(contenu_sans_parenthèses, index_après_la_parenthèse_fermante)`.
    pub(super) fn read_balanced_parens(chars: &[char], lparen: usize) -> Option<(String, usize)> {
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

    /// Découpe une chaîne d'arguments sur les `,` de niveau supérieur (les
    /// parenthèses imbriquées sont équilibrées). Chaîne vide → aucun argument.
    pub(super) fn split_top_level_commas(s: &str) -> Vec<String> {
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

    /// Scanne une "action" de `%if`/`%then`/`%else` à partir de `start` :
    /// soit un groupe `%do; ... %end;` (le texte retourné est le corps interne
    /// du `%do`, sans le `%do;`/`%end;`), soit un fragment de texte jusqu'au
    /// `;` terminal inclus. Rend `(texte_action, index_après)`.
    pub(super) fn scan_action(chars: &[char], start: usize) -> Option<(String, usize)> {
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
    pub(super) fn scan_do_block(chars: &[char], start: usize) -> Option<(String, usize)> {
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

    /// Trouve le mot-clé `%<kw>` (insensible casse) à partir de `from`, au
    /// niveau de `%do` 0 (ne descend pas dans un `%do ... %end` imbriqué). Rend
    /// l'index du `%` du mot-clé. Utilisé pour `%then`.
    pub(super) fn find_kw(chars: &[char], from: usize, kw: &str) -> Option<usize> {
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
    pub(super) fn find_kw_before_semicolon(chars: &[char], from: usize, kw: &str) -> Option<usize> {
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
    pub(super) fn find_semicolon(chars: &[char], from: usize) -> Option<usize> {
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
    pub(super) fn find_matching_end(chars: &[char], body_start: usize) -> Option<(usize, usize)> {
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
