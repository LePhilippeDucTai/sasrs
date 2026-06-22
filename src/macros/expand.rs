//! Dispatch principal de l'expansion macro (`process_impl`) et ses consommateurs.
//!
//! Boucle gauche→droite qui reconnaît chaque déclencheur `%`/`&` et délègue au
//! consommateur dédié. Les helpers de bas niveau (`%macro`/invocation, quoting,
//! scan, etc.) vivent dans les sous-modules frères ; ce module porte le cœur du
//! dispatch et les consommateurs `%let`/`%eval`/`%str`/`%sysevalf`/quoting étendu.

use super::*;

impl MacroEngine {
    /// Coeur de l'expansion `%let`/`&var` (une passe gauche→droite). Met à jour
    /// la table de l'engine (état conservé entre appels — donc entre segments).
    pub(super) fn process_impl(&mut self, source: &str) -> String {
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

    // ── M11.6 : %sysfunc ─────────────────────────────────────────────────────

    /// Parse `func(a, b, ...)` et évalue la fonction. Le contenu a déjà ses
    /// `&refs` résolus. Rend le texte du résultat ou une `MacroError`.
    pub(super) fn eval_sysfunc(&self, content: &str) -> Result<String, MacroError> {
        let content = content.trim();
        let chars: Vec<char> = content.chars().collect();
        // Nom de fonction.
        let (name, after) = Self::read_name(&chars, 0)
            .ok_or_else(|| MacroError::new("ERROR: %SYSFUNC requires a function call"))?;
        let mut j = after;
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        // Arguments : `(...)` optionnel (fonctions sans argument : TODAY()). On
        // garde l'index APRÈS la parenthèse fermante de la fonction pour repérer
        // un éventuel `, <format>` de niveau supérieur (M35.1).
        let (arg_strings, after_call) = if chars.get(j) == Some(&'(') {
            let (args_inner, after_call) = Self::read_balanced_parens(&chars, j)
                .ok_or_else(|| MacroError::new("ERROR: unbalanced parentheses in %SYSFUNC"))?;
            (Self::split_top_level_commas(&args_inner), after_call)
        } else {
            (Vec::new(), j)
        };
        // Argument de format optionnel : `%sysfunc(func(args), format)`. Le
        // contenu restant après l'appel doit être `, <format>` (un seul token au
        // niveau supérieur). On ne consomme PAS de virgule à l'intérieur des
        // parenthèses de la fonction (elles sont équilibrées par read_balanced).
        let mut k = after_call;
        while matches!(chars.get(k), Some(c) if c.is_whitespace()) {
            k += 1;
        }
        let format: Option<String> = if chars.get(k) == Some(&',') {
            let rest: String = chars[k + 1..].iter().collect();
            let f = rest.trim().to_string();
            if f.is_empty() {
                None
            } else {
                Some(f)
            }
        } else {
            None
        };
        let upper = name.to_uppercase();
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
        // EvalCtx minimal jetable : `Default` suffit. Les fonctions qui auraient
        // besoin du PDV/dataset n'y ont pas accès (comme %SYSFUNC sous SAS) et
        // rendent une valeur missing — comportement acceptable.
        let mut ctx = crate::datastep::eval::EvalCtx::default();
        // Délégation à la bibliothèque COMPLÈTE de fonctions DATA-step (plus de
        // liste blanche, M35.1). Fonction inconnue → None → note d'erreur propre.
        let result = crate::datastep::functions::call(&upper, &args, &mut ctx).ok_or_else(|| {
            MacroError::new(format!(
                "ERROR: Function {} not found or not supported by %SYSFUNC",
                name
            ))
        })?;
        match format {
            // Format présent : on l'applique via le MÊME chemin que PUT (module
            // formats), puis on rogne les blancs (SAS gauche-aligne / retire les
            // blancs de tête du résultat formaté de %sysfunc).
            Some(fmt) => {
                let formatted = crate::datastep::functions::call(
                    "PUT",
                    &[result, crate::value::Value::Char(fmt)],
                    &mut ctx,
                )
                .map(|v| Self::value_to_text(&v))
                .unwrap_or_default();
                Ok(formatted.trim().to_string())
            }
            None => Ok(Self::value_to_text(&result)),
        }
    }

    // ── M11.6 : %str / %nrstr (quoting par sentinelles) ─────────────────────
    //
    // Les primitives de masquage (constantes `MASK_BASE`/`STR_MASKED`/
    // `NRSTR_EXTRA` et helpers `mask_char`/`unmask`/`mask_special`) vivent dans
    // le sous-module `quoting`.

    /// Consomme `%str ( ... )` (si `!nrstr`) ou `%nrstr ( ... )`. Masque les
    /// caractères spéciaux du contenu (pour `%str`, `&`/`%` restent ACTIFS et
    /// sont donc résolus ; pour `%nrstr` ils sont AUSSI masqués → inertes). Pour
    /// `%str`, on ré-expanse le contenu masqué afin de résoudre les `&x`/`%m`
    /// éventuels ; pour `%nrstr`, on émet le contenu masqué tel quel. Rend
    /// l'index après la `)`, ou `None` si la parenthèse n'est pas trouvée.
    pub(super) fn consume_quote(
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
        // `%nrstr` masque AUSSI `&`/`%` (contenu inerte, émis tel quel) ; `%str`
        // ne masque que la ponctuation et ré-expanse pour résoudre les `&x`/`%m`
        // résiduels (déclencheurs restés actifs).
        let mask = if nrstr { MaskSet::All } else { MaskSet::Punct };
        out.push_str(&Self::apply_quoting(self, &inner, mask, !nrstr));
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
    pub(super) fn consume_unquote(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
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
    pub(super) fn consume_sysevalf(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
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

    // ── M12.2 : quoting étendu (%superq, %bquote, %nrbquote) ─────────────────

    /// Consomme `%superq ( name )`. Prend un NOM de variable (pas `&name`),
    /// lit sa valeur SANS résoudre aucun `&`/`%` qu'elle contient, et masque
    /// TOUT (y compris `&`/`%`) afin que le résultat soit littéral et inerte en
    /// aval — l'outil idéal pour des valeurs contenant des `&`/`%` parasites.
    /// L'argument peut lui-même être un `&ref` désignant le nom (SAS résout
    /// l'argument en un nom). Variable indéfinie → chaîne vide (SAS émet un
    /// WARNING ; on se contente d'émettre vide). Rend l'index après la `)`.
    pub(super) fn consume_superq(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
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
            Some(v) => out.push_str(&Self::apply_quoting(self, &v, MaskSet::All, false)),
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
    pub(super) fn consume_bquote(
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
        let mask = if nr { MaskSet::All } else { MaskSet::Punct };
        out.push_str(&Self::apply_quoting(self, &expanded, mask, false));
        Some(after)
    }
}
