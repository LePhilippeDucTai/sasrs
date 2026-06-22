//! Fonctions macro (M32.7) : `%sysfunc`/`%qsysfunc`, `%cmpres`/`%qcmpres`,
//! `%symexist`/`%sysmexist`/`%sysget`, et les fonctions chaîne macro
//! (`%upcase`/`%lowcase`/`%substr`/`%scan`/`%index`/`%length` + variantes `%q*`).
//!
//! Extrait VERBATIM de `crate::macros` (technique impl-block-in-submodule) :
//! les méthodes restent des `impl MacroEngine`, les appels via `self.`/`Self::`
//! résolvent inchangés.

use super::*;

impl MacroEngine {
    /// Consomme `%sysfunc ( func(args) )` (ou `%qsysfunc`). Résout les `&refs`,
    /// parse `func(arg1, arg2, ...)`, appelle `functions::call` avec les args
    /// typés (numérique si l'argument parse en nombre, sinon `Char`), puis
    /// splice le résultat formaté en texte. Fonction inconnue → note d'erreur
    /// propre (pas de panic). Rend l'index après la `)`, ou `None` si la
    /// parenthèse externe n'est pas trouvée.
    pub(super) fn consume_sysfunc(
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

    /// Formate une `Value` en texte pour l'insertion macro : `Char` tel quel
    /// (trim de droite), `Num` via le format BEST (entier sans décimales),
    /// missing → chaîne vide.
    pub(super) fn value_to_text(v: &crate::value::Value) -> String {
        match v {
            crate::value::Value::Char(s) => s.trim_end().to_string(),
            crate::value::Value::Num(n) => crate::value::format_best(*n, 12),
            crate::value::Value::Missing(_) => String::new(),
        }
    }

    /// Consomme `%cmpres ( text )` / `%qcmpres ( text )`. Résout les `&refs`,
    /// puis COMPRESSE les blancs : rogne les blancs de bord et réduit toute
    /// suite de blancs interne à UN seul espace (fidèle à SAS CMPRES). La
    /// variante `q` masque le résultat (ponctuation + déclencheurs) comme les
    /// autres `%q*`. Rend l'index après la `)`.
    pub(super) fn consume_cmpres(
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
            out.push_str(&Self::apply_quoting(self, &compressed, MaskSet::All, false));
        } else {
            out.push_str(&compressed);
        }
        Some(after)
    }

    /// Rogne les blancs de bord et réduit chaque suite de blancs interne à un
    /// unique espace. Helper de `%cmpres`/`%qcmpres`.
    pub(super) fn compress_blanks(s: &str) -> String {
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
    pub(super) fn read_name_arg(&mut self, chars: &[char], kw: &str, i: usize) -> Option<(String, usize)> {
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
    pub(super) fn consume_symexist(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let (name, after) = self.read_name_arg(chars, "symexist", i)?;
        let exists = self.lookup(&name).is_some();
        out.push_str(if exists { "1" } else { "0" });
        Some(after)
    }

    /// Consomme `%sysmexist ( name )`. Rend `1` si la macro (définie via
    /// `%macro`) existe, `0` sinon. Rend l'index après la `)`.
    pub(super) fn consume_sysmexist(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let (name, after) = self.read_name_arg(chars, "sysmexist", i)?;
        let exists = self.macros.contains_key(&name.to_uppercase());
        out.push_str(if exists { "1" } else { "0" });
        Some(after)
    }

    /// Consomme `%sysget ( name )`. Rend la valeur de la variable d'environnement
    /// nommée. Une variable inexistante rend la CHAÎNE VIDE (SAS émet un WARNING ;
    /// on se contente de produire vide pour rester déterministe). Cf. la note de
    /// déterminisme dans l'en-tête du module. Rend l'index après la `)`.
    pub(super) fn consume_sysget(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let (name, after) = self.read_name_arg(chars, "sysget", i)?;
        if let Ok(v) = std::env::var(&name) {
            out.push_str(&v);
        }
        Some(after)
    }

    /// Consomme une fonction chaîne macro `%kw ( args )` parmi
    /// `upcase`/`lowcase`/`substr`/`scan`/`index`/`length` et leurs variantes
    /// `q*` (`masked == true` → résultat masqué par sentinelles, comme
    /// `%bquote`). Les arguments sont d'abord résolus (`&refs`) puis découpés
    /// sur les `,` de niveau supérieur. Positions 1-basées (convention SAS).
    /// Le résultat texte n'est PAS masqué pour les variantes nues. Rend l'index
    /// après la `)`, ou `None` si la parenthèse manque / arité invalide.
    pub(super) fn consume_macro_fn(
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
            out.push_str(&Self::apply_quoting(self, &result, MaskSet::All, false));
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
    pub(super) fn eval_macro_fn(name: &str, args: &[String]) -> Option<String> {
        // M32.7 PART 2 — la table `STRING_FNS` remplace le `match name` ouvert.
        // Le découpage logique (nom q-strippé) et le masquage `%q*` restent dans
        // `consume_macro_fn` (le `masked` y est décidé par la boucle de dispatch),
        // donc chaque `eval` ci-dessous reproduit EXACTEMENT l'ancien bras.
        let f = lookup(name)?;
        (f.eval)(args)
    }
}

/// Une fonction chaîne macro de la table. `eval` reproduit le bras `match`
/// d'origine (même trim, même indexation 1-basée, mêmes bornes/cas vides).
///
/// NB masquage `%q*` : il N'EST PAS porté ici. La décision « masquer ou non »
/// est faite PAR APPEL dans la boucle de dispatch (`qupcase` → `masked=true`,
/// `upcase` → `false`) et appliquée par le caller `consume_macro_fn` via
/// `apply_quoting`. La table est indexée par le nom LOGIQUE (q-strippé) ; un
/// hypothétique champ `masked` par entrée serait soit inutilisé, soit
/// incohérent (une même entrée sert les deux variantes). On l'omet donc pour
/// rester byte-identical ET sans warning de champ mort.
pub(super) struct MacroFn {
    pub(super) name: &'static str,
    pub(super) eval: fn(&[String]) -> Option<String>,
}

/// Table des fonctions chaîne macro, par nom LOGIQUE (préfixe `q` déjà retiré).
/// L'ordre n'est pas sémantique (lookup linéaire par nom exact) ; il suit
/// l'ordre des anciens bras `match` pour la lisibilité.
static STRING_FNS: &[MacroFn] = &[
    MacroFn { name: "upcase", eval: fn_upcase },
    MacroFn { name: "lowcase", eval: fn_lowcase },
    MacroFn { name: "substr", eval: fn_substr },
    MacroFn { name: "scan", eval: fn_scan },
    MacroFn { name: "index", eval: fn_index },
    MacroFn { name: "length", eval: fn_length },
];

/// Recherche une fonction chaîne macro par son nom logique (déjà q-strippé,
/// minuscules). `None` si le nom n'est pas une fonction connue.
pub(super) fn lookup(name_lower: &str) -> Option<&'static MacroFn> {
    STRING_FNS.iter().find(|f| f.name == name_lower)
}

fn fn_upcase(args: &[String]) -> Option<String> {
    let t = args.first().map(String::as_str).unwrap_or("");
    Some(t.to_uppercase())
}

fn fn_lowcase(args: &[String]) -> Option<String> {
    let t = args.first().map(String::as_str).unwrap_or("");
    Some(t.to_lowercase())
}

fn fn_substr(args: &[String]) -> Option<String> {
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

fn fn_scan(args: &[String]) -> Option<String> {
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

fn fn_index(args: &[String]) -> Option<String> {
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

fn fn_length(args: &[String]) -> Option<String> {
    let t = args.first().map(String::as_str).unwrap_or("");
    Some(t.chars().count().to_string())
}
