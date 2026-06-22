//! Structures de contrôle du processeur macro : `%if`/`%then`/`%else` et les
//! formes `%do` (groupe, itératif `%do i=a %to b`, conditionnel `%do %while`/
//! `%do %until`), plus l'affectation de la variable d'itération.

use super::*;

impl MacroEngine {
    /// Garde anti-boucle-folle pour les `%do` itératifs.
    pub(super) const MAX_LOOP_ITERS: i64 = 1_000_000;
    /// M35.4 — garde anti-boucle pour les sauts `%goto` (un saut arrière permet
    /// de boucler ; on plafonne le nombre total de sauts par expansion de corps).
    pub(super) const MAX_GOTO_JUMPS: i64 = 1_000_000;
}

impl MacroEngine {
    /// Consomme `%if <cond> %then <action> [; %else <action> ;]`.
    ///
    /// `<cond>` court jusqu'au `%then` (insensible casse). `<action>` est soit
    /// un groupe `%do; ... %end;`, soit un fragment de texte jusqu'au `;` de fin
    /// d'action (le `;` est inclus dans le texte émis, comme une instruction
    /// SAS). On émet la branche prise EXPANSÉE et rien pour l'autre. Rend
    /// l'index de reprise, ou `None` si la structure ne tient pas.
    pub(super) fn consume_if(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
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
        // M19.3 — MLOGIC : décision de la condition `%if`.
        if self.mlogic {
            let label = self.current_macro_label();
            self.log_line(format!(
                "MLOGIC({}):  %IF condition {} is {}",
                label,
                cond.trim(),
                if take_then { "TRUE" } else { "FALSE" }
            ));
        }

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

    /// Consomme un `%do` : soit `%do; ... %end;` (groupe), soit
    /// `%do i=a %to b [%by c]; ... %end;` (itératif). Émet le contenu expansé.
    pub(super) fn consume_do(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
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
        // Formes conditionnelles `%do %while(<cond>)` / `%do %until(<cond>)`.
        if Self::matches_kw_paren(chars, j, "while") {
            return self.consume_conditional_do(chars, i, j, "while", true, out);
        }
        if Self::matches_kw_paren(chars, j, "until") {
            return self.consume_conditional_do(chars, i, j, "until", false, out);
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
    pub(super) fn consume_iterative_do(
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

    /// Consomme les formes conditionnelles `%do %while(<cond>); body %end;` et
    /// `%do %until(<cond>); body %end;`.
    ///
    /// `kw_start` pointe sur le `%` de `%while`/`%until`. `is_while` distingue la
    /// sémantique :
    /// - `%while` : test AVANT chaque itération (0 itération si faux d'emblée) ;
    /// - `%until` : test APRÈS chaque itération (≥1 itération, on s'arrête dès
    ///   que la condition devient vraie).
    ///
    /// La condition est ré-évaluée fraîchement à chaque tour (résolution des
    /// `&refs` + `macro_eval`), si bien qu'un `%let` dans le corps influe sur la
    /// terminaison. Réutilise `MAX_LOOP_ITERS` comme garde anti-boucle-folle. Le
    /// rognage des blancs suit la convention de `consume_iterative_do`.
    pub(super) fn consume_conditional_do(
        &mut self,
        chars: &[char],
        _do_start: usize,
        kw_start: usize,
        kw: &str,
        is_while: bool,
        out: &mut String,
    ) -> Option<usize> {
        // La `(` suit le mot-clé (éventuellement après des blancs).
        let mut p = kw_start + 1 + kw.len();
        while matches!(chars.get(p), Some(c) if c.is_whitespace()) {
            p += 1;
        }
        if chars.get(p) != Some(&'(') {
            return None;
        }
        let (cond, after_cond) = Self::read_balanced_parens(chars, p)?;
        // L'en-tête se termine par le `;` suivant la `)` de la condition.
        let semi = Self::find_semicolon(chars, after_cond)?;
        let body_start = semi + 1;
        let (body_end, after) = Self::find_matching_end(chars, body_start)?;
        let body: String = chars[body_start..body_end].iter().collect();
        let body_trimmed = body.trim_start();

        let mut buf = String::new();
        let mut iters: i64 = 0;
        loop {
            // `%while` teste avant le corps ; `%until` après.
            if is_while {
                match self.eval_condition(&cond) {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(e) => {
                        Self::emit_error(&mut buf, &e);
                        break;
                    }
                }
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
            let expanded = self.process_impl(body_trimmed);
            buf.push_str(&expanded);
            if !is_while {
                // `%until` : on s'arrête quand la condition devient vraie.
                match self.eval_condition(&cond) {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(e) => {
                        Self::emit_error(&mut buf, &e);
                        break;
                    }
                }
            }
        }
        out.push_str(buf.trim_end());
        Some(after)
    }

    /// Affecte la variable d'itération dans la portée courante (haut de la pile
    /// de portées si on est dans une macro, sinon la table globale).
    pub(super) fn set_loop_var(&mut self, var: &str, value: i64) {
        let key = var.to_uppercase();
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(key, value.to_string());
        } else {
            self.table.insert(key, value.to_string());
        }
    }
}

// ── M35.4 : %return / %abort / %goto + %label ────────────────────────────────

impl MacroEngine {
    /// Consomme `%return;` : pose le drapeau `return_requested` (la boucle
    /// `process_impl` cessera d'expanser le reste de CE corps en tête du prochain
    /// tour) et avale le `;` terminal. En OPEN CODE (hors d'une macro), `%return`
    /// est sans objet : SAS émet un avertissement ; on émet une NOTE propre et on
    /// NE pose PAS le drapeau (rien à interrompre). Rend l'index après le `;`.
    pub(super) fn consume_return(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let mut j = i + "%return".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        // `;` terminal optionnel (toléré absent en fin de source).
        if chars.get(j) == Some(&';') {
            j += 1;
        }
        if self.macro_stack.is_empty() {
            // Open code : pas de corps à interrompre.
            out.push_str("/* NOTE: %RETURN is not valid in open code; statement ignored */");
        } else {
            self.return_requested = true;
        }
        Some(Self::skip_trailing_newline(chars, j, out))
    }

    /// Consomme `%abort [cancel | abend [n] | return [n]];` : enregistre
    /// l'intention d'abort (drapeau `abort_requested` + variante `abort_kind`),
    /// émet la NOTE SAS-like, et avale jusqu'au `;`. Le drapeau se PROPAGE
    /// (l'appelant l'observe et stoppe à son tour) — il n'est PAS réinitialisé par
    /// `expand_invocation`. On NE fait jamais `process::exit`/`panic`. Rend
    /// l'index après le `;`.
    pub(super) fn consume_abort(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let mut j = i + "%abort".len();
        // Lire les options/arguments jusqu'au `;`.
        let arg_start = j;
        while j < chars.len() && chars[j] != ';' {
            j += 1;
        }
        let args: String = chars[arg_start..j].iter().collect();
        if chars.get(j) == Some(&';') {
            j += 1;
        }
        // Analyser la forme : 1er token = option, 2nd = code retour éventuel.
        let mut toks = args.split_whitespace();
        let opt = toks.next().map(|s| s.to_ascii_uppercase());
        let code: Option<i64> = toks.next().and_then(|s| s.parse::<i64>().ok());
        let kind = match opt.as_deref() {
            None => AbortKind::Plain,
            Some("CANCEL") => AbortKind::Cancel,
            Some("ABEND") => AbortKind::Abend(code),
            Some("RETURN") => AbortKind::Return(code),
            // Option inconnue (ou code retour nu sans mot-clé) : on retombe sur la
            // forme simple en gardant le code retour si c'était un entier.
            Some(other) => match other.parse::<i64>() {
                Ok(n) => AbortKind::Return(Some(n)),
                Err(_) => AbortKind::Plain,
            },
        };
        let detail = match &kind {
            AbortKind::Plain => String::new(),
            AbortKind::Cancel => " CANCEL".to_string(),
            AbortKind::Abend(Some(n)) => format!(" ABEND {n}"),
            AbortKind::Abend(None) => " ABEND".to_string(),
            AbortKind::Return(Some(n)) => format!(" RETURN {n}"),
            AbortKind::Return(None) => " RETURN".to_string(),
        };
        out.push_str(&format!(
            "/* NOTE: %ABORT{detail} encountered; macro expansion stopped */"
        ));
        self.abort_requested = true;
        self.abort_kind = Some(kind);
        Some(Self::skip_trailing_newline(chars, j, out))
    }

    /// Consomme `%goto label;` (ou `%goto label` sans `;`) : POSE une demande de
    /// saut (`goto_requested`). La résolution effective (recherche de `%label:`
    /// et repositionnement du scan) a lieu en tête de la boucle `process_impl`,
    /// éventuellement APRÈS remontée hors d'une action `%then`/`%do` imbriquée
    /// (le `%goto` peut sauter vers une étiquette du corps englobant). `budget`
    /// partagé décrémenté à chaque saut ; épuisé → NOTE d'erreur (anti-boucle).
    /// Open code → NOTE propre. Rend l'index après le `;`.
    pub(super) fn consume_goto(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let mut j = i + "%goto".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        let (label, after_label) = Self::read_name(chars, j)?;
        j = after_label;
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        // `;` terminal optionnel.
        if chars.get(j) == Some(&';') {
            j += 1;
        }
        let after_stmt = Self::skip_trailing_newline(chars, j, out);
        if self.macro_stack.is_empty() {
            out.push_str("/* NOTE: %GOTO is not valid in open code; statement ignored */");
            return Some(after_stmt);
        }
        if self.goto_budget <= 0 {
            Self::emit_error(
                out,
                &MacroError::new(format!(
                    "ERROR: %GOTO jump budget ({}) exhausted (runaway guard)",
                    Self::MAX_GOTO_JUMPS
                )),
            );
            return Some(after_stmt);
        }
        self.goto_budget -= 1;
        // Poser la demande : la boucle `process_impl` la résoudra (ici ou au
        // niveau parent). On cesse d'émettre la suite de CE fragment.
        self.goto_requested = Some(label.to_uppercase());
        Some(after_stmt)
    }

    /// Cherche le marqueur d'étiquette `%<label>:` (insensible casse) dans
    /// `chars`. Rend l'index JUSTE APRÈS le `:` (point de reprise du scan), ou
    /// `None` si absent.
    pub(super) fn find_label(chars: &[char], label: &str) -> Option<usize> {
        let mut k = 0;
        while k < chars.len() {
            if chars[k] == '%' {
                if let Some((name, after)) = Self::read_name(chars, k + 1) {
                    // Un marqueur d'étiquette est `%name` immédiatement suivi de
                    // `:` (blancs optionnels), ce que confirme `skip_label_marker`.
                    let mut m = after;
                    while matches!(chars.get(m), Some(c) if c.is_whitespace()) {
                        m += 1;
                    }
                    if chars.get(m) == Some(&':') && name.eq_ignore_ascii_case(label) {
                        return Some(m + 1);
                    }
                }
            }
            k += 1;
        }
        None
    }

    /// Si `chars[i..]` est un marqueur d'étiquette `%name:` (un nom suivi,
    /// blancs optionnels, d'un `:` qui n'est PAS `:=`), rend l'index après le `:`
    /// (le marqueur n'émet rien). Sinon `None`. On EXCLUT `%name :` suivi d'un
    /// second `:` (pas un cas SAS courant) et `%name(` (invocation).
    pub(super) fn skip_label_marker(chars: &[char], i: usize) -> Option<usize> {
        let (_name, after) = Self::read_name(chars, i + 1)?;
        let mut m = after;
        while matches!(chars.get(m), Some(c) if c.is_whitespace()) {
            m += 1;
        }
        if chars.get(m) == Some(&':') {
            Some(m + 1)
        } else {
            None
        }
    }
}

// ── M35.4 : constructions macro hors-périmètre (NOTE + consommation) ──────────

impl MacroEngine {
    /// Reconnaît et consomme proprement les statements macro NON pris en charge
    /// dans ce build (interactif/OS) : `%syscall`, `%sysexec`, `%window`,
    /// `%display`, `%sysmacdelete`, `%sysmstoreclear`, `%syslput`, `%sysrput`.
    /// Chacun émet une NOTE « not supported in this build » et est consommé
    /// jusqu'à son `;` terminal (en respectant les parenthèses équilibrées du
    /// premier `(...)` éventuel, pour ne pas couper sur un `;` interne). Rend
    /// l'index après le `;`, ou `None` si aucun de ces mots-clés ne matche.
    pub(super) fn consume_unsupported_stmt(
        &mut self,
        chars: &[char],
        i: usize,
        out: &mut String,
    ) -> Option<usize> {
        // (mot-clé, libellé affiché). `%sysexec` et `%syscall` acceptent une
        // forme à parenthèses ou nue ; les autres sont des statements à `;`.
        const KW: &[(&str, &str)] = &[
            ("sysexec", "%SYSEXEC (OS command execution)"),
            ("syscall", "%SYSCALL (CALL routine invocation)"),
            ("window", "%WINDOW (interactive window definition)"),
            ("display", "%DISPLAY (interactive window display)"),
            ("sysmacdelete", "%SYSMACDELETE"),
            ("sysmstoreclear", "%SYSMSTORECLEAR"),
            ("syslput", "%SYSLPUT (remote macro variable)"),
            ("sysrput", "%SYSRPUT (remote macro variable)"),
        ];
        let (matched, label) = KW
            .iter()
            .find(|(kw, _)| Self::matches_kw(chars, i, kw) || Self::matches_kw_paren(chars, i, kw))
            .map(|(kw, label)| (*kw, *label))?;

        let mut j = i + 1 + matched.len();
        // Sauter un premier groupe `(...)` éventuel à parenthèses équilibrées
        // (ex. `%sysexec(rm x;y)` — le `;` interne ne doit pas couper).
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) == Some(&'(') {
            if let Some((_, after)) = Self::read_balanced_parens(chars, j) {
                j = after;
            }
        }
        // Consommer le reste jusqu'au `;` terminal (options/arguments ignorés).
        while j < chars.len() && chars[j] != ';' {
            j += 1;
        }
        if chars.get(j) == Some(&';') {
            j += 1;
        }
        out.push_str(&format!(
            "/* NOTE: {label} is not supported in this build; statement ignored */"
        ));
        Some(Self::skip_trailing_newline(chars, j, out))
    }
}
