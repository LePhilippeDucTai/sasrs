use super::*;

impl MacroEngine {
    /// Consomme `%return;` : pose le drapeau `return_requested` (la boucle
    /// `process_impl` cessera d'expanser le reste de CE corps en tête du prochain
    /// tour) et avale le `;` terminal. En OPEN CODE (hors d'une macro), `%return`
    /// est sans objet : SAS émet un avertissement ; on émet une NOTE propre et on
    /// NE pose PAS le drapeau (rien à interrompre). Rend l'index après le `;`.
    pub(crate) fn consume_return(
        &mut self,
        chars: &[char],
        i: usize,
        out: &mut String,
    ) -> Option<usize> {
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
            self.flow.return_requested = true;
        }
        Some(Self::skip_trailing_newline(chars, j, out))
    }

    /// Consomme `%abort [cancel | abend [n] | return [n]];` : enregistre
    /// l'intention d'abort (drapeau `abort_requested` + variante `abort_kind`),
    /// émet la NOTE SAS-like, et avale jusqu'au `;`. Le drapeau se PROPAGE
    /// (l'appelant l'observe et stoppe à son tour) — il n'est PAS réinitialisé par
    /// `expand_invocation`. On NE fait jamais `process::exit`/`panic`. Rend
    /// l'index après le `;`.
    pub(crate) fn consume_abort(
        &mut self,
        chars: &[char],
        i: usize,
        out: &mut String,
    ) -> Option<usize> {
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
        self.flow.abort_requested = true;
        self.flow.abort_kind = Some(kind);
        Some(Self::skip_trailing_newline(chars, j, out))
    }

    /// Consomme `%goto label;` (ou `%goto label` sans `;`) : POSE une demande de
    /// saut (`goto_requested`). La résolution effective (recherche de `%label:`
    /// et repositionnement du scan) a lieu en tête de la boucle `process_impl`,
    /// éventuellement APRÈS remontée hors d'une action `%then`/`%do` imbriquée
    /// (le `%goto` peut sauter vers une étiquette du corps englobant). `budget`
    /// partagé décrémenté à chaque saut ; épuisé → NOTE d'erreur (anti-boucle).
    /// Open code → NOTE propre. Rend l'index après le `;`.
    pub(crate) fn consume_goto(
        &mut self,
        chars: &[char],
        i: usize,
        out: &mut String,
    ) -> Option<usize> {
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
        if self.flow.goto_budget <= 0 {
            Self::emit_error(
                out,
                &MacroError::new(format!(
                    "ERROR: %GOTO jump budget ({}) exhausted (runaway guard)",
                    Self::MAX_GOTO_JUMPS
                )),
            );
            return Some(after_stmt);
        }
        self.flow.goto_budget -= 1;
        // Poser la demande : la boucle `process_impl` la résoudra (ici ou au
        // niveau parent). On cesse d'émettre la suite de CE fragment.
        self.flow.goto_requested = Some(label.to_uppercase());
        Some(after_stmt)
    }

    /// Cherche le marqueur d'étiquette `%<label>:` (insensible casse) dans
    /// `chars`. Rend l'index JUSTE APRÈS le `:` (point de reprise du scan), ou
    /// `None` si absent.
    pub(crate) fn find_label(chars: &[char], label: &str) -> Option<usize> {
        let mut k = 0;
        while k < chars.len() {
            if chars[k] == '%'
                && let Some((name, after)) = Self::read_name(chars, k + 1)
            {
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
            k += 1;
        }
        None
    }

    /// Si `chars[i..]` est un marqueur d'étiquette `%name:` (un nom suivi,
    /// blancs optionnels, d'un `:` qui n'est PAS `:=`), rend l'index après le `:`
    /// (le marqueur n'émet rien). Sinon `None`. On EXCLUT `%name :` suivi d'un
    /// second `:` (pas un cas SAS courant) et `%name(` (invocation).
    pub(crate) fn skip_label_marker(chars: &[char], i: usize) -> Option<usize> {
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
