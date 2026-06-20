//! Structures de contrôle du processeur macro : `%if`/`%then`/`%else` et les
//! formes `%do` (groupe, itératif `%do i=a %to b`, conditionnel `%do %while`/
//! `%do %until`), plus l'affectation de la variable d'itération.

use super::*;

impl MacroEngine {
    /// Garde anti-boucle-folle pour les `%do` itératifs.
    pub(super) const MAX_LOOP_ITERS: i64 = 1_000_000;
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
