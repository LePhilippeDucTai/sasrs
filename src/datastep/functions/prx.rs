use super::*;
use crate::error::SasError;

// ──────────────────────────────────────────────────────────────────────────────
// PRX — Perl regular expressions (M40.1), moteur `fancy-regex`
// (lookaround + backreferences, comme le PCRE de SAS)
// ──────────────────────────────────────────────────────────────────────────────

/// Table des patterns PRX compilés de l'étape. Les ids sont numériques,
/// 1-based, valables pour la durée de l'étape (comme SAS). PRXPARSE d'un
/// texte-source déjà vu réutilise l'id existant (cache par texte : SAS ne
/// compile un littéral constant qu'une fois ; ici l'usage naïf — sans
/// `if _n_=1 then re=prxparse(...)` — ne recompile donc pas à chaque ligne).
#[derive(Default)]
pub struct PrxState {
    /// id-1 → pattern ; `None` = libéré par CALL PRXFREE.
    entries: Vec<Option<PrxPattern>>,
    /// texte-source → id ; `None` = compilation déjà échouée (l'ERROR n'est
    /// émise qu'une fois par texte de pattern).
    by_source: std::collections::HashMap<String, Option<usize>>,
}

/// Un pattern compilé + les captures de son dernier match réussi (pour
/// PRXPOSN / PRXPAREN / CALL PRXPOSN).
pub struct PrxPattern {
    regex: fancy_regex::Regex,
    /// Chaîne de remplacement d'un pattern `s/…/…/` ; `None` = pattern de
    /// match pur (PRXCHANGE le refuse alors, comme SAS).
    replacement: Option<String>,
    /// Dernier match : par groupe (indice 0 = match entier), `(position
    /// 1-based, longueur)` en CARACTÈRES ; `None` si le groupe n'a pas
    /// participé au match.
    last_match: Option<Vec<Option<(usize, usize)>>>,
}

impl PrxState {
    /// Compile (ou retrouve dans le cache) le texte de pattern SAS `source`.
    /// `Err(Some(msg))` : compilation échouée, message ERROR à émettre ;
    /// `Err(None)` : échec déjà signalé pour ce même texte.
    pub fn parse(&mut self, source: &str) -> std::result::Result<usize, Option<String>> {
        if let Some(cached) = self.by_source.get(source) {
            match cached {
                // Id encore valide (non libéré) → réutilisé.
                Some(id) if self.entries[*id - 1].is_some() => return Ok(*id),
                Some(_) => {} // libéré par PRXFREE : recompiler sous un id neuf
                None => return Err(None),
            }
        }
        match compile_sas_pattern(source) {
            Ok(pat) => {
                self.entries.push(Some(pat));
                let id = self.entries.len();
                self.by_source.insert(source.to_string(), Some(id));
                Ok(id)
            }
            Err(msg) => {
                self.by_source.insert(source.to_string(), None);
                Err(Some(msg))
            }
        }
    }

    /// Valide un id numérique (fourni par l'utilisateur) : entier ≥ 1,
    /// existant et non libéré.
    pub fn check_id(&self, f: f64) -> Option<usize> {
        if !f.is_finite() || f < 1.0 {
            return None;
        }
        let id = f as usize;
        if (id as f64) != f || id > self.entries.len() {
            return None;
        }
        self.entries[id - 1].as_ref().map(|_| id)
    }

    pub fn get_mut(&mut self, id: usize) -> Option<&mut PrxPattern> {
        self.entries.get_mut(id - 1).and_then(Option::as_mut)
    }

    pub fn get(&self, id: usize) -> Option<&PrxPattern> {
        self.entries.get(id - 1).and_then(Option::as_ref)
    }

    /// CALL PRXFREE : invalide l'id (les usages ultérieurs → ERROR) et
    /// détache le texte-source du cache (un PRXPARSE ultérieur recompile).
    pub fn free(&mut self, id: usize) -> bool {
        match self.entries.get_mut(id - 1) {
            Some(slot @ Some(_)) => {
                *slot = None;
                self.by_source.retain(|_, v| *v != Some(id));
                true
            }
            _ => false,
        }
    }
}

/// Décompose un texte de pattern SAS (`/re/flags` ou `s/re/repl/flags`,
/// délimiteur libre non alphanumérique) et compile la regex. Les flags Perl
/// `i`, `m`, `s`, `x` deviennent des flags inline `(?imsx)` ; `o`
/// (compile-once) est accepté et ignoré (le cache par texte le couvre).
/// Limites documentées : pas de délimiteurs appariés (`s{…}{…}`), pas de
/// constructions Perl absentes de fancy-regex (code embarqué `(?{…})`,
/// récursion `(?R)`).
fn compile_sas_pattern(source: &str) -> std::result::Result<PrxPattern, String> {
    let invalid = || {
        format!(
            "The regular expression \"{}\" passed to the PRXPARSE function or routine is invalid.",
            source.trim_end()
        )
    };
    let text = source.trim();
    let mut chars = text.chars();
    let (substitution, delim) = match chars.next() {
        None => return Err(invalid()),
        // `s` + délimiteur non alphanumérique → substitution s/…/…/.
        Some('s') => match chars.next() {
            Some(d) if !d.is_alphanumeric() => (true, d),
            _ => return Err(invalid()),
        },
        Some(d) if !d.is_alphanumeric() => (false, d),
        Some(_) => return Err(invalid()),
    };
    // Sections entre délimiteurs non échappés (un `\` protège le suivant).
    let rest: String = chars.collect();
    let mut sections: Vec<String> = vec![String::new()];
    let mut escaped = false;
    for c in rest.chars() {
        if escaped {
            sections.last_mut().unwrap().push(c);
            escaped = false;
        } else if c == '\\' {
            sections.last_mut().unwrap().push(c);
            escaped = true;
        } else if c == delim {
            sections.push(String::new());
        } else {
            sections.last_mut().unwrap().push(c);
        }
    }
    let expected = if substitution { 3 } else { 2 };
    if sections.len() != expected {
        return Err(invalid());
    }
    let pattern = sections[0].clone();
    let replacement = substitution.then(|| sections[1].clone());
    // Flags Perl après le délimiteur fermant.
    let mut inline = String::new();
    for f in sections[expected - 1].chars() {
        match f {
            'i' | 'm' | 's' | 'x' => {
                if !inline.contains(f) {
                    inline.push(f);
                }
            }
            'o' => {} // compile-once : sans objet (cache par texte)
            _ => return Err(invalid()),
        }
    }
    let full = if inline.is_empty() {
        pattern
    } else {
        format!("(?{inline}){pattern}")
    };
    match fancy_regex::Regex::new(&full) {
        Ok(regex) => Ok(PrxPattern {
            regex,
            replacement,
            last_match: None,
        }),
        Err(_) => Err(invalid()),
    }
}

/// Position 1-based en CARACTÈRES de l'offset OCTET `byte` dans `s`.
fn char_pos_1(s: &str, byte: usize) -> usize {
    s[..byte].chars().count() + 1
}

/// Exécute le pattern sur `source` à partir du caractère `start_char`
/// (0-based) sans dépasser le caractère `stop_char` (1-based inclus ; `None`
/// = fin de chaîne), et mémorise les captures du match dans le pattern.
/// Renvoie `(position 1-based, longueur)` du match entier, ou `None`.
pub(crate) fn find_and_store(
    pat: &mut PrxPattern,
    source: &str,
    start_char: usize,
    stop_char: Option<usize>,
) -> Option<(usize, usize)> {
    let n_chars = source.chars().count();
    if start_char > n_chars {
        return None;
    }
    // Borne haute : tronque la zone de recherche (les positions renvoyées
    // restent valables — le préfixe est inchangé).
    let hay = match stop_char {
        Some(stop) if stop < n_chars => {
            let end_byte = source
                .char_indices()
                .nth(stop)
                .map_or(source.len(), |(b, _)| b);
            &source[..end_byte]
        }
        _ => source,
    };
    let start_byte = hay
        .char_indices()
        .nth(start_char)
        .map_or(hay.len(), |(b, _)| b);
    // Erreur d'exécution du moteur (limite de backtracking) → pas de match.
    let caps = match pat.regex.captures_from_pos(hay, start_byte) {
        Ok(Some(c)) => c,
        _ => return None,
    };
    let groups: Vec<Option<(usize, usize)>> = (0..caps.len())
        .map(|i| {
            caps.get(i)
                .map(|m| (char_pos_1(hay, m.start()), m.as_str().chars().count()))
        })
        .collect();
    let whole = groups[0];
    pat.last_match = Some(groups);
    whole
}

/// Remplacement Perl : `$1`/`\1` (multi-chiffres), `${1}`, `\\` et `\$`
/// littéraux ; groupe absent → vide.
fn expand_replacement(repl: &str, caps: &fancy_regex::Captures<'_, str>) -> String {
    let mut out = String::new();
    let mut it = repl.chars().peekable();
    while let Some(c) = it.next() {
        let group_start = match c {
            '$' | '\\' if it.peek().is_some_and(char::is_ascii_digit) => true,
            '$' if it.peek() == Some(&'{') => {
                // ${n} : consomme jusqu'à `}` ; malformé → littéral.
                let mut clone = it.clone();
                clone.next(); // '{'
                let digits: String = clone.clone().take_while(char::is_ascii_digit).collect();
                if !digits.is_empty() && clone.nth(digits.chars().count()) == Some('}') {
                    it = clone;
                    let n: usize = digits.parse().unwrap_or(0);
                    if let Some(m) = caps.get(n) {
                        out.push_str(m.as_str());
                    }
                    continue;
                }
                out.push(c);
                continue;
            }
            '\\' => {
                // `\\` → `\`, `\$` → `$` ; autre échappement : littéral.
                match it.next() {
                    Some(e @ ('\\' | '$')) => out.push(e),
                    Some(e) => {
                        out.push('\\');
                        out.push(e);
                    }
                    None => out.push('\\'),
                }
                continue;
            }
            _ => {
                out.push(c);
                continue;
            }
        };
        if group_start {
            let mut n = 0usize;
            while let Some(d) = it.peek().and_then(|d| d.to_digit(10)) {
                n = n * 10 + d as usize;
                it.next();
            }
            if let Some(m) = caps.get(n) {
                out.push_str(m.as_str());
            }
        }
    }
    out
}

/// Applique la substitution du pattern sur `source`, `times` fois (`-1` =
/// toutes les occurrences). Renvoie `(résultat, nombre de remplacements)` et
/// mémorise les captures du dernier match. Un match vide avance d'un
/// caractère (convention Perl — pas de boucle infinie).
pub(crate) fn change(pat: &mut PrxPattern, times: i64, source: &str) -> (String, usize) {
    let repl = pat.replacement.clone().unwrap_or_default();
    let mut out = String::new();
    let mut cursor = 0usize; // offset octet dans `source`
    let mut n = 0usize;
    while times < 0 || (n as i64) < times {
        let caps = match pat.regex.captures_from_pos(source, cursor) {
            Ok(Some(c)) => c,
            _ => break,
        };
        let m = caps.get(0).expect("group 0 always present");
        out.push_str(&source[cursor..m.start()]);
        out.push_str(&expand_replacement(&repl, &caps));
        n += 1;
        let groups: Vec<Option<(usize, usize)>> = (0..caps.len())
            .map(|i| {
                caps.get(i)
                    .map(|g| (char_pos_1(source, g.start()), g.as_str().chars().count()))
            })
            .collect();
        cursor = m.end();
        pat.last_match = Some(groups);
        if m.end() == m.start() {
            // Match vide : recopier le caractère courant et avancer.
            match source[cursor..].chars().next() {
                Some(c) => {
                    out.push(c);
                    cursor += c.len_utf8();
                }
                None => break,
            }
        }
    }
    out.push_str(&source[cursor..]);
    (out, n)
}

/// Résout le premier argument d'une fonction PRX : un id numérique (validé
/// dans la table) ou directement un texte de pattern (compilé via le cache
/// — sémantique SAS). Id invalide/libéré → ERROR fatale (l'étape stoppe,
/// comme SAS) ; pattern invalide → ERROR non fatale + `None` ; missing →
/// missing propagé.
fn resolve_pattern(arg: &Value, func: &str, ctx: &mut EvalCtx) -> Option<usize> {
    match arg {
        Value::Char(s) => match ctx.prx.parse(s) {
            Ok(id) => Some(id),
            Err(msg) => {
                if let Some(m) = msg {
                    ctx.runtime_errors.push(m);
                }
                ctx.error_flag = true;
                None
            }
        },
        Value::Num(f) => match ctx.prx.check_id(*f) {
            Some(id) => Some(id),
            None => {
                ctx.fatal = Some(SasError::runtime(format!(
                    "Invalid regular expression id passed to the function {func}."
                )));
                None
            }
        },
        Value::Missing(_) => {
            ctx.missing_generated += 1;
            None
        }
    }
}

/// `PRXPARSE(pattern)` → id numérique du pattern compilé ; pattern invalide
/// → missing + ERROR dans le log (une fois par texte de pattern).
pub(crate) fn fn_prxparse(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let Some(first) = args.first() else {
        return Value::missing();
    };
    if let Value::Missing(_) = first {
        ctx.missing_generated += 1;
        return Value::missing();
    }
    let source = coerce_char(first);
    match ctx.prx.parse(&source) {
        Ok(id) => Value::Num(id as f64),
        Err(msg) => {
            if let Some(m) = msg {
                ctx.runtime_errors.push(m);
            }
            ctx.error_flag = true;
            Value::missing()
        }
    }
}

/// `PRXMATCH(id_ou_pattern, source)` → position 1-based du premier match,
/// 0 sinon. Mémorise les captures (pour PRXPOSN/PRXPAREN).
pub(crate) fn fn_prxmatch(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() != 2 {
        ctx.error_flag = true;
        return Value::missing();
    }
    let Some(id) = resolve_pattern(&args[0], "PRXMATCH", ctx) else {
        return Value::missing();
    };
    let source = coerce_char(&args[1]);
    let pat = ctx.prx.get_mut(id).expect("id validated");
    match find_and_store(pat, &source, 0, None) {
        Some((pos, _)) => Value::Num(pos as f64),
        None => Value::Num(0.0),
    }
}

/// `PRXCHANGE(id_ou_pattern, times, source)` → chaîne substituée (`times` =
/// -1 : toutes les occurrences). Le pattern doit être une substitution
/// `s/…/…/` (sinon ERROR non fatale + résultat blanc).
pub(crate) fn fn_prxchange(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() != 3 {
        ctx.error_flag = true;
        return Value::Char(String::new());
    }
    let Some(id) = resolve_pattern(&args[0], "PRXCHANGE", ctx) else {
        return Value::Char(String::new());
    };
    let Some(times) = coerce_num(&args[1], ctx) else {
        return Value::Char(String::new());
    };
    let source = coerce_char(&args[2]);
    let pat = ctx.prx.get_mut(id).expect("id validated");
    if pat.replacement.is_none() {
        ctx.runtime_errors.push(
            "The regular expression passed to the PRXCHANGE function or routine is not a substitution."
                .to_string(),
        );
        ctx.error_flag = true;
        return Value::Char(String::new());
    }
    let times = if times < 0.0 { -1 } else { times as i64 };
    let (out, _) = change(pat, times, &source);
    Value::Char(out)
}

/// `PRXPOSN(id, n, source)` → texte du groupe capturé `n` (0 = match entier)
/// du DERNIER match réussi de ce pattern, extrait de `source` aux positions
/// mémorisées. Pas de match / groupe non participant → chaîne vide.
pub(crate) fn fn_prxposn(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() != 3 {
        ctx.error_flag = true;
        return Value::Char(String::new());
    }
    let id = match &args[0] {
        Value::Num(f) => match ctx.prx.check_id(*f) {
            Some(id) => id,
            None => {
                ctx.fatal = Some(SasError::runtime(
                    "Invalid regular expression id passed to the function PRXPOSN.".to_string(),
                ));
                return Value::Char(String::new());
            }
        },
        _ => {
            ctx.missing_generated += 1;
            return Value::Char(String::new());
        }
    };
    let Some(n) = coerce_num(&args[1], ctx) else {
        return Value::Char(String::new());
    };
    if n < 0.0 {
        return Value::Char(String::new());
    }
    let source = coerce_char(&args[2]);
    let pat = ctx.prx.get(id).expect("id validated");
    let Some(groups) = &pat.last_match else {
        return Value::Char(String::new());
    };
    match groups.get(n as usize).copied().flatten() {
        Some((pos, len)) => {
            let text: String = source.chars().skip(pos - 1).take(len).collect();
            Value::Char(text)
        }
        None => Value::Char(String::new()),
    }
}

/// `PRXPAREN(id)` → numéro du plus grand groupe ayant participé au dernier
/// match du pattern (0 si aucun groupe) ; missing si aucun match encore.
pub(crate) fn fn_prxparen(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let Some(first) = args.first() else {
        return Value::missing();
    };
    let id = match first {
        Value::Num(f) => match ctx.prx.check_id(*f) {
            Some(id) => id,
            None => {
                ctx.fatal = Some(SasError::runtime(
                    "Invalid regular expression id passed to the function PRXPAREN.".to_string(),
                ));
                return Value::missing();
            }
        },
        _ => {
            ctx.missing_generated += 1;
            return Value::missing();
        }
    };
    let pat = ctx.prx.get(id).expect("id validated");
    match &pat.last_match {
        Some(groups) => {
            let last = groups
                .iter()
                .enumerate()
                .skip(1)
                .filter(|(_, g)| g.is_some())
                .map(|(i, _)| i)
                .max()
                .unwrap_or(0);
            Value::Num(last as f64)
        }
        None => Value::missing(),
    }
}

/// Groupe `n` du dernier match du pattern `id` : `(position, longueur)`, ou
/// `None`. Support de CALL PRXPOSN.
pub(crate) fn last_match_group(pat: &PrxPattern, n: usize) -> Option<(usize, usize)> {
    pat.last_match.as_ref()?.get(n).copied().flatten()
}

/// Le pattern est-il une substitution `s/…/…/` ? Support de CALL PRXCHANGE.
pub(crate) fn is_substitution(pat: &PrxPattern) -> bool {
    pat.replacement.is_some()
}
