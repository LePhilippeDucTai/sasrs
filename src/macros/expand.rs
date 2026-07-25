//! Dispatch principal de l'expansion macro (`process_impl`) et ses consommateurs.
//!
//! Boucle gauche→droite qui reconnaît chaque déclencheur `%`/`&` et délègue au
//! consommateur dédié. Les helpers de bas niveau (`%macro`/invocation, quoting,
//! scan, etc.) vivent dans les sous-modules frères ; ce module porte le cœur du
//! dispatch et les consommateurs `%let`/`%eval`/`%str`/`%sysevalf`/quoting étendu.

use super::*;

/// Matcher d'un déclencheur `%kw` à la position `i`. Les deux formes usuelles
/// de la table sont `STMT` (`matches_kw` : mot-clé statement borné par une
/// frontière de mot) et `FUNC` (`matches_kw_paren` : fonction macro suivie
/// d'une `(`) ; `%let` et les statements non supportés utilisent un wrapper
/// ad hoc (voir leurs entrées).
type Matcher = fn(&[char], usize, &str) -> bool;

/// Consommateur homogène de la table : rend `Some(next)` si le construct a été
/// consommé (le scan reprend à `next`), `None` pour laisser la main au candidat
/// suivant (puis aux cas irréguliers en dur de `process_impl`).
type Handler = fn(&mut MacroEngine, &[char], usize, &mut String) -> Option<usize>;

const STMT: Matcher = MacroEngine::matches_kw;
const FUNC: Matcher = MacroEngine::matches_kw_paren;

/// Table de dispatch des déclencheurs `%` « réguliers » de `process_impl`,
/// essayée DANS L'ORDRE. L'ordre d'essai est SÉMANTIQUE et reproduit la
/// cascade de `if` historique : les formes `%q*` sont testées AVANT leurs
/// versions nues — grâce à la frontière de mot des matchers il n'y a pas de
/// collision (`%substr` ne matche pas `str`), mais l'ordre garde l'intention
/// explicite. Les handlers à paramètre (mot-clé et/ou booléen) sont adaptés
/// par des fermetures sans capture.
const DISPATCH: &[(&str, Matcher, Handler)] = &[
    // M35.4 — `%goto label;` : pose un saut vers `%label:` (résolu en tête de
    // boucle, éventuellement après remontée hors d'une action `%then`).
    ("goto", STMT, MacroEngine::consume_goto),
    // M35.4 — `%return;` : sortie anticipée du corps de macro courant.
    ("return", STMT, MacroEngine::consume_return),
    // M35.4 — `%abort [cancel|abend|return [n]];` : terminaison propre.
    ("abort", STMT, MacroEngine::consume_abort),
    // M35.4 — constructions hors-périmètre (interactif/OS) : NOTE propre
    // + consommation jusqu'au `;`. Aucune n'est exprimable dans le modèle
    // d'expansion textuel ; on garantit l'absence de crash/boucle. Le mot-clé
    // effectif est reconnu PAR le consommateur (table interne) : matcher
    // « toujours vrai », mot-clé de table inutilisé.
    ("", |_, _, _| true, MacroEngine::consume_unsupported_stmt),
    // `%let` insensible à la casse (matcher dédié : `%let` + blanc).
    ("let", |chars, i, _| MacroEngine::matches_let(chars, i), MacroEngine::consume_let),
    // `%put <texte>;` (M19.3) — écrit son argument (résolu) au log,
    // n'émet RIEN dans le flux de code.
    ("put", STMT, MacroEngine::consume_put),
    // `%call execute(text);` (M19.3) — met en file un fragment de code
    // SAS à exécuter APRÈS le segment courant (sémantique CALL EXECUTE).
    ("call", STMT, MacroEngine::consume_macro_call),
    // `%include 'chemin';` (M19.2) — charge le fichier, l'expanse
    // récursivement et splice le résultat À LA PLACE du statement,
    // AVANT de poursuivre le scan du segment courant.
    ("include", STMT, MacroEngine::consume_include),
    // `%macro name(params); body %mend;` — capture, n'émet rien.
    ("macro", STMT, MacroEngine::consume_macro_def),
    // `%local v1 v2;` / `%global v1 v2;`
    ("local", STMT, |s, c, i, o| s.consume_scope_decl(c, i, true, o)),
    ("global", STMT, |s, c, i, o| s.consume_scope_decl(c, i, false, o)),
    // `%eval(expr)` — évalue et splice le résultat entier.
    ("eval", FUNC, MacroEngine::consume_eval),
    // `%sysfunc(func(args))` / `%qsysfunc(...)` — appelle la fonction
    // DATA-step et splice le résultat formaté en texte.
    ("sysfunc", FUNC, |s, c, i, o| s.consume_sysfunc(c, i, "sysfunc", o)),
    ("qsysfunc", FUNC, |s, c, i, o| s.consume_sysfunc(c, i, "qsysfunc", o)),
    // `%sysevalf(expr [, conv])` — évaluation FLOTTANTE (M19.1).
    ("sysevalf", FUNC, MacroEngine::consume_sysevalf),
    // `%cmpres(text)` / `%qcmpres(text)` — compression des blancs (M19.1).
    ("qcmpres", FUNC, |s, c, i, o| s.consume_cmpres(c, i, "qcmpres", true, o)),
    ("cmpres", FUNC, |s, c, i, o| s.consume_cmpres(c, i, "cmpres", false, o)),
    // `%symexist(name)` / `%sysmexist(name)` / `%sysget(name)` (M19.1).
    ("symexist", FUNC, MacroEngine::consume_symexist),
    ("sysmexist", FUNC, MacroEngine::consume_sysmexist),
    ("sysget", FUNC, MacroEngine::consume_sysget),
    // `%unquote(text)` — ré-active la résolution `&`/`%` masquée (M19.1).
    ("unquote", FUNC, MacroEngine::consume_unquote),
    // `%superq(name)` — valeur d'une variable SANS résolution, masquée.
    ("superq", FUNC, MacroEngine::consume_superq),
    // `%bquote(text)` / `%nrbquote(text)` — résout puis masque.
    ("nrbquote", FUNC, |s, c, i, o| s.consume_bquote(c, i, "nrbquote", true, o)),
    ("bquote", FUNC, |s, c, i, o| s.consume_bquote(c, i, "bquote", false, o)),
    // Fonctions chaîne macro simples et leurs variantes `%q*` (le booléen
    // vaut « résultat masqué », vrai pour les `%q*`).
    ("qupcase", FUNC, |s, c, i, o| s.consume_macro_fn(c, i, "qupcase", true, o)),
    ("upcase", FUNC, |s, c, i, o| s.consume_macro_fn(c, i, "upcase", false, o)),
    ("qlowcase", FUNC, |s, c, i, o| s.consume_macro_fn(c, i, "qlowcase", true, o)),
    ("lowcase", FUNC, |s, c, i, o| s.consume_macro_fn(c, i, "lowcase", false, o)),
    ("qsubstr", FUNC, |s, c, i, o| s.consume_macro_fn(c, i, "qsubstr", true, o)),
    ("substr", FUNC, |s, c, i, o| s.consume_macro_fn(c, i, "substr", false, o)),
    ("qscan", FUNC, |s, c, i, o| s.consume_macro_fn(c, i, "qscan", true, o)),
    ("scan", FUNC, |s, c, i, o| s.consume_macro_fn(c, i, "scan", false, o)),
    ("index", FUNC, |s, c, i, o| s.consume_macro_fn(c, i, "index", false, o)),
    ("length", FUNC, |s, c, i, o| s.consume_macro_fn(c, i, "length", false, o)),
    // `%str(...)` / `%nrstr(...)` — masquage des caractères spéciaux.
    ("str", FUNC, |s, c, i, o| s.consume_quote(c, i, "str", false, o)),
    ("nrstr", FUNC, |s, c, i, o| s.consume_quote(c, i, "nrstr", true, o)),
    // `%if <cond> %then <action>; [%else <action>;]`
    ("if", STMT, MacroEngine::consume_if),
    // `%do ...` (plain group ou itératif `%do i=a %to b`).
    ("do", STMT, MacroEngine::consume_do),
];


impl MacroEngine {
    /// Coeur de l'expansion `%let`/`&var` (une passe gauche→droite). Met à jour
    /// la table de l'engine (état conservé entre appels — donc entre segments).
    /// Les déclencheurs `%` réguliers sont dispatchés via la table `DISPATCH`
    /// ci-dessus ; restent en dur les cas irréguliers : marqueur d'étiquette
    /// `%name:`, invocation `%name` d'une macro utilisateur (avec autocall) et
    /// résolution `&var`.
    pub(super) fn process_impl(&mut self, source: &str) -> String {
        let chars: Vec<char> = source.chars().collect();
        let mut out = String::with_capacity(source.len());
        let mut i = 0;
        'scan: while i < chars.len() {
            // M35.4 — `%return`/`%abort` posés par un statement précédent (ou par
            // une macro invoquée dont l'abort se propage) : on cesse d'expanser
            // le reste de CE corps. `expand_invocation` réinitialise
            // `return_requested` après le corps ; `abort_requested` se propage.
            if self.flow.return_requested || self.flow.abort_requested {
                break;
            }

            // M35.4 — saut `%goto` en attente : on tente de trouver `%label:` dans
            // CE texte. Trouvé → on saute (drapeau réinitialisé) ; introuvable →
            // on laisse remonter au niveau parent (on cesse d'expanser ici). Cela
            // gère le `%goto` posé dans une action `%then`/`%do` imbriquée.
            if let Some(label) = self.flow.goto_requested.clone() {
                if let Some(target) = Self::find_label(&chars, &label) {
                    self.flow.goto_requested = None;
                    i = target;
                    continue;
                } else {
                    break;
                }
            }

            let c = chars[i];

            if c == '%' {
                // M35.4 — marqueur d'étiquette `%name:` (cible de `%goto`). Il
                // n'émet RIEN ; on le saute (jusqu'au `:` inclus). Testé AVANT
                // toute autre forme pour ne pas confondre `%exit:` avec un appel.
                if let Some(next) = Self::skip_label_marker(&chars, i) {
                    i = next;
                    continue;
                }

                // Déclencheurs `%` réguliers : cascade dirigée par la table
                // `DISPATCH` (ordre d'essai SÉMANTIQUE — voir sa doc). Un
                // handler qui rend `None` (syntaxe non conforme) laisse la main
                // au candidat suivant, puis aux cas irréguliers ci-dessous.
                for &(kw, matcher, handler) in DISPATCH {
                    if matcher(&chars, i, kw) {
                        if let Some(next) = handler(self, &chars, i, &mut out) {
                            i = next;
                            continue 'scan;
                        }
                    }
                }

                // Invocation `%name` ou `%name(args)` d'une macro DÉFINIE — ou,
                // à défaut, chargée paresseusement depuis une bibliothèque
                // autocall (`SASAUTOS`, M19.2). `try_autocall` compile `nom.sas`
                // au premier appel (idempotent via `autocall_tried`).
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
                    if self.trace.symbolgen {
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
    /// Consomme un `%let name = value ;` complet à partir de `i` et met la
    /// table à jour. Rend l'index après le `;` (et préserve un `\n` final).
    /// Rend `None` si la syntaxe ne tient pas (on laisse alors le `%` brut).
    pub(super) fn consume_let(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
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

mod sysfunc;
mod quoting;

impl MacroEngine {
    /// Consomme `%eval ( expr )` : résout les `&refs` de `expr`, évalue, et
    /// splice le résultat entier. Rend l'index après la `)`, ou `None` si la
    /// parenthèse n'est pas trouvée (laisse alors le `%` brut).
    pub(super) fn consume_eval(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
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
}


