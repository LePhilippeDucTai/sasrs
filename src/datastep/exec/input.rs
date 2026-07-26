//! Lecture INPUT/INFILE texte (méthodes de `Runner`).

use super::*;

impl Runner {
    /// INPUT (M14) : lit un enregistrement de la source texte et applique la
    /// spécification INPUT au PDV. Gère les modes liste/colonne/formaté, les
    /// pointeurs `@n`/`+n`/`/`, et les holds `@`/`@@`.
    ///
    /// Sémantique de fin de source (comme SET) : si aucun enregistrement
    /// n'est disponible quand on doit en lire un nouveau → EndStep.
    /// Résout les items AST d'un statement INPUT en `InputAction` (slots PDV
    /// + informats parsés). Plusieurs INPUT par étape sont ainsi gérés (chacun
    /// avec ses propres items).
    fn resolve_input_items(&self, ast_items: &[crate::ast::InputItem]) -> Result<Vec<InputAction>> {
        use crate::ast::InputItem;
        let mut out = Vec::with_capacity(ast_items.len());
        for item in ast_items {
            let action = match item {
                InputItem::Var {
                    name,
                    is_char,
                    cols,
                    informat,
                    list_modifier,
                } => {
                    let slot = self.pdv.slot(name).ok_or_else(|| {
                        SasError::runtime(format!("Variable {name} is not on the INPUT statement."))
                    })?;
                    let spec = match informat {
                        Some(tok) => {
                            Some(crate::formats::FormatSpec::parse(tok).ok_or_else(|| {
                                SasError::runtime(format!("The informat {tok} is not valid."))
                            })?)
                        }
                        None => None,
                    };
                    let pdv_is_char = self.pdv.vars()[slot].ty == VarType::Char;
                    InputAction::Var {
                        slot,
                        is_char: pdv_is_char || *is_char,
                        cols: *cols,
                        informat: spec,
                        list_modifier: *list_modifier,
                    }
                }
                InputItem::ColumnPointer(n) => InputAction::ColumnPointer(*n),
                InputItem::SkipColumns(n) => InputAction::SkipColumns(*n),
                InputItem::NextLine => InputAction::NextLine,
                InputItem::HoldLine => InputAction::HoldLine,
                InputItem::HoldLineDouble => InputAction::HoldLineDouble,
            };
            out.push(action);
        }
        Ok(out)
    }

    pub(super) fn exec_input(&mut self, ast_items: &[crate::ast::InputItem]) -> Result<Flow> {
        // Récupérer la ligne de travail : soit un hold actif (avec encore des
        // données après le curseur), soit la prochaine ligne de la source. Un
        // hold `@@` dont le reste de ligne n'est que des blancs est épuisé →
        // on lit un nouvel enregistrement (sémantique SAS du « double hold »).
        let held = self.text_io.held.take().filter(|h| {
            let rest: String = h.line.chars().skip(h.cursor).collect();
            !rest.trim().is_empty()
        });
        let (mut line, mut cursor) = match held {
            Some(h) => (h.line, h.cursor),
            None => match self.next_record()? {
                Some(s) => (s, 0usize),
                None => return Ok(Flow::EndStep),
            },
        };

        if self.text_io.src.is_none() {
            return Err(SasError::runtime(
                "INPUT statement without an INFILE source.",
            ));
        }
        // Items résolus en `InputAction` (slots PDV + informats parsés). On les
        // résout depuis l'AST de CE statement INPUT pour gérer plusieurs INPUT
        // par étape (chacun partage la même source mais a ses propres items).
        let items = self.resolve_input_items(ast_items)?;
        let short = self.text_io.src.as_ref().unwrap().options.short;
        let dsd = self.text_io.src.as_ref().unwrap().options.dsd;
        let delim = self.text_io.src.as_ref().unwrap().options.delimiter.clone();

        let mut hold_after = false;
        let mut hold_double = false;

        for action in &items {
            match action {
                InputAction::ColumnPointer(n) => {
                    cursor = n.saturating_sub(1);
                }
                InputAction::SkipColumns(n) => {
                    cursor += n;
                }
                InputAction::NextLine => {
                    // Passe à la ligne d'entrée suivante (curseur réinitialisé).
                    match self.next_record()? {
                        Some(s) => {
                            line = s;
                            cursor = 0;
                        }
                        None => return Ok(Flow::EndStep),
                    }
                }
                InputAction::HoldLine => hold_after = true,
                InputAction::HoldLineDouble => {
                    hold_after = true;
                    hold_double = true;
                }
                InputAction::Var {
                    slot,
                    is_char,
                    cols,
                    informat,
                    list_modifier,
                } => {
                    let outcome = self.read_one_var(
                        &line,
                        &mut cursor,
                        *slot,
                        *is_char,
                        *cols,
                        informat,
                        *list_modifier,
                        &delim,
                        dsd,
                        short,
                    )?;
                    match outcome {
                        ReadOutcome::Ok => {}
                        ReadOutcome::ShortMissover => {
                            // MISSOVER/TRUNCOVER/défaut liste : variables
                            // restantes laissées telles quelles (déjà missing
                            // par le reset). On arrête la lecture des items.
                            break;
                        }
                        ReadOutcome::Stopover => {
                            return Err(SasError::runtime(
                                "INPUT statement exceeded record length (STOPOVER).",
                            ));
                        }
                    }
                }
            }
        }

        // Hold : conserver la ligne pour le prochain INPUT.
        if hold_after {
            self.text_io.held = Some(HeldLine {
                line,
                cursor,
                double: hold_double,
            });
        }
        Ok(Flow::Normal)
    }

    /// Lit le prochain enregistrement brut de la source texte, en respectant
    /// FIRSTOBS=/OBS=. Incrémente `text_io.read`. Renvoie `None` à
    /// l'épuisement.
    fn next_record(&mut self) -> Result<Option<String>> {
        let text = match &self.text_io.src {
            Some(t) => t,
            None => return Ok(None),
        };
        let firstobs = text.options.firstobs;
        let obs = text.options.obs;
        loop {
            // FIRSTOBS= : sauter les lignes avant firstobs (1-based).
            if self.text_io.next_line + 1 < firstobs {
                self.text_io.next_line += 1;
                continue;
            }
            // OBS= : borne supérieure (1-based, inclusive).
            if let Some(o) = obs
                && self.text_io.next_line + 1 > o
            {
                return Ok(None);
            }
            let Some(line) = text.lines.get(self.text_io.next_line) else {
                return Ok(None);
            };
            let line = line.clone();
            self.text_io.next_line += 1;
            self.text_io.read += 1;
            return Ok(Some(line));
        }
    }

    /// Lit UNE variable d'INPUT à partir de `line`, en avançant `cursor`.
    /// Couvre les trois modes (colonne / formaté / liste) et applique la
    /// coercition vers le slot PDV. Renvoie le devenir de la lecture (OK /
    /// ligne trop courte selon MISSOVER/TRUNCOVER/STOPOVER).
    #[allow(clippy::too_many_arguments)]
    fn read_one_var(
        &mut self,
        line: &str,
        cursor: &mut usize,
        slot: usize,
        is_char: bool,
        cols: Option<(usize, usize)>,
        informat: &Option<crate::formats::FormatSpec>,
        list_modifier: bool,
        delim: &Option<String>,
        dsd: bool,
        short: ShortMode,
    ) -> Result<ReadOutcome> {
        let chars: Vec<char> = line.chars().collect();

        // ── Mode COLONNE : champ fixe `a-b` (1-based inclusif). ──────────────
        if let Some((a, b)) = cols {
            let start = a - 1;
            let end = b; // exclusif sur la borne 1-based supérieure
            if start >= chars.len() {
                // Champ entièrement au-delà de la ligne.
                return Ok(self.handle_short(short, slot, is_char));
            }
            let stop = end.min(chars.len());
            let field: String = chars[start..stop].iter().collect();
            *cursor = end;
            self.apply_field(slot, &field, is_char, informat);
            return Ok(ReadOutcome::Ok);
        }

        // ── Modes LISTE et FORMATÉ-COLONNE ───────────────────────────────────
        // Un informat SANS `:`, en mode espace par défaut (ni DSD ni
        // délimiteur explicite), lit une largeur FIXE à partir du curseur
        // (mode formaté colonne). Avec `:`, DSD, ou un délimiteur, il lit un
        // jeton délimité puis applique l'informat (mode liste). En mode liste
        // pur (sans informat), on lit un jeton délimité.
        let delimited_mode = dsd || delim.is_some() || list_modifier;
        let formatted_fixed = informat.is_some() && !delimited_mode;
        if formatted_fixed {
            let w = informat.as_ref().and_then(|s| s.w).map(|w| w as usize);
            // Sans largeur explicite : se comporter comme un jeton délimité.
            if let Some(w) = w {
                if *cursor >= chars.len() {
                    return Ok(self.handle_short(short, slot, is_char));
                }
                let stop = (*cursor + w).min(chars.len());
                // TRUNCOVER/MISSOVER : un champ partiel est lu tel quel.
                let field: String = chars[*cursor..stop].iter().collect();
                *cursor += w;
                self.apply_field(slot, &field, is_char, informat);
                return Ok(ReadOutcome::Ok);
            }
        }

        // ── Mode LISTE : jeton délimité ──────────────────────────────────────
        match self.scan_token(&chars, cursor, delim, dsd) {
            Some(field) => {
                self.apply_field(slot, &field, is_char, informat);
                Ok(ReadOutcome::Ok)
            }
            None => Ok(self.handle_short(short, slot, is_char)),
        }
    }

    /// Comportement « ligne trop courte » selon MISSOVER/TRUNCOVER/STOPOVER.
    /// En mode défaut/MISSOVER/TRUNCOVER, la variable reste à sa valeur de
    /// reset (missing num / chaîne vide) et on signale d'arrêter les items
    /// restants. STOPOVER → erreur.
    fn handle_short(&mut self, short: ShortMode, slot: usize, is_char: bool) -> ReadOutcome {
        if short == ShortMode::Stopover {
            return ReadOutcome::Stopover;
        }
        // La variable manquante reste à missing/blanc (le reset l'a déjà
        // posée ; on force par sûreté).
        let init = if is_char {
            Value::Char(String::new())
        } else {
            Value::missing()
        };
        self.pdv.set(slot, init);
        ReadOutcome::ShortMissover
    }

    /// Découpe le prochain jeton délimité à partir de `cursor`. En mode
    /// DSD : la virgule est le délimiteur par défaut, deux délimiteurs
    /// consécutifs encadrent une valeur manquante (chaîne vide), et les
    /// guillemets protègent les délimiteurs. Renvoie `None` si la fin de
    /// ligne est atteinte avant tout jeton (hors DSD-vide).
    fn scan_token(
        &self,
        chars: &[char],
        cursor: &mut usize,
        delim: &Option<String>,
        dsd: bool,
    ) -> Option<String> {
        // Jeu de délimiteurs.
        let delims: Vec<char> = match delim {
            Some(s) => s.chars().collect(),
            None if dsd => vec![','],
            None => vec![' ', '\t'],
        };
        let is_delim = |c: char| delims.contains(&c);

        if dsd {
            // En DSD, on lit exactement UN champ : il peut être vide (deux
            // délimiteurs consécutifs) ou entre guillemets.
            if *cursor > chars.len() {
                return None;
            }
            if *cursor == chars.len() {
                // Curseur en bout de ligne : plus de champ.
                return None;
            }
            let mut field = String::new();
            // Champ entre guillemets.
            if chars[*cursor] == '"' {
                *cursor += 1;
                while *cursor < chars.len() {
                    let c = chars[*cursor];
                    if c == '"' {
                        // Guillemet doublé = guillemet littéral.
                        if *cursor + 1 < chars.len() && chars[*cursor + 1] == '"' {
                            field.push('"');
                            *cursor += 2;
                            continue;
                        }
                        *cursor += 1;
                        break;
                    }
                    field.push(c);
                    *cursor += 1;
                }
                // Consommer le délimiteur de fin de champ s'il y en a un.
                if *cursor < chars.len() && is_delim(chars[*cursor]) {
                    *cursor += 1;
                }
                return Some(field);
            }
            // Champ nu : jusqu'au prochain délimiteur.
            while *cursor < chars.len() && !is_delim(chars[*cursor]) {
                field.push(chars[*cursor]);
                *cursor += 1;
            }
            // Consommer le délimiteur (sépare du champ suivant).
            if *cursor < chars.len() && is_delim(chars[*cursor]) {
                *cursor += 1;
            }
            return Some(field);
        }

        // Mode liste ordinaire : sauter les délimiteurs de tête, puis lire
        // jusqu'au prochain délimiteur.
        while *cursor < chars.len() && is_delim(chars[*cursor]) {
            *cursor += 1;
        }
        if *cursor >= chars.len() {
            return None;
        }
        let mut field = String::new();
        while *cursor < chars.len() && !is_delim(chars[*cursor]) {
            field.push(chars[*cursor]);
            *cursor += 1;
        }
        Some(field)
    }

    /// Applique un champ texte à un slot PDV : informat si présent, sinon
    /// décodage natif (char → tel quel ; num → parse/missing). La troncature
    /// char est gérée par `pdv.set`.
    fn apply_field(
        &mut self,
        slot: usize,
        field: &str,
        is_char: bool,
        informat: &Option<crate::formats::FormatSpec>,
    ) {
        let value = if let Some(spec) = informat {
            // Informat : on délègue au catalogue (gère le piège des décimales
            // implicites). Le champ est passé tel quel.
            self.format_informat(field, spec)
        } else if is_char {
            // Mode liste/colonne caractère : la valeur est le champ (les
            // blancs de bord sont rognés en mode liste ; en colonne, SAS rogne
            // aussi les blancs de tête/fin).
            Value::Char(field.trim().to_string())
        } else {
            // Numérique : trim + parse ; vide/"." → missing.
            let t = field.trim();
            if t.is_empty() || t == "." {
                Value::missing()
            } else {
                match t.parse::<f64>() {
                    Ok(f) => Value::Num(f),
                    Err(_) => {
                        // Donnée numérique invalide : missing + NOTE + _ERROR_.
                        self.ctx.invalid_data += 1;
                        self.pdv.error_ = true;
                        Value::missing()
                    }
                }
            }
        };
        let target = self.pdv.vars()[slot].ty;
        let coerced = self.coerce_assign(value, target);
        self.pdv.set(slot, coerced);
    }

    /// Applique un informat à un champ via le catalogue (clone de session).
    fn format_informat(&self, field: &str, spec: &crate::formats::FormatSpec) -> Value {
        self.format_catalog.informat(field, spec)
    }

    // ── FILE / PUT (M14.2) ───────────────────────────────────────────────
}
