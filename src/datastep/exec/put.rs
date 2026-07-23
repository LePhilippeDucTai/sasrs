//! PUT/FILE : construction de lignes, destinations et replay (méthodes de `Runner`).

use super::*;

impl Runner {
    /// FILE (M14.2) : change la destination courante des PUT. Si une ligne
    /// non maintenue est en construction et que la destination CHANGE, elle
    /// est d'abord relâchée vers l'ancienne destination (la ligne « en cours »
    /// appartient à la destination active au moment de son écriture).
    pub(super) fn exec_file(&mut self, dest: &crate::ast::PutDest) -> Result<Flow> {
        let new_dest = match dest {
            crate::ast::PutDest::Path(p) => PutDestKind::Path(p.clone()),
            crate::ast::PutDest::Log => PutDestKind::Log,
            crate::ast::PutDest::Print => PutDestKind::Print,
        };
        if new_dest != self.put.dest {
            // Relâcher la ligne pendante (non maintenue) vers l'ancienne
            // destination avant de basculer.
            if self.put.started && !self.put.hold && !self.put.hold_double {
                self.put_release_line();
            }
            self.put.dest = new_dest;
        }
        Ok(Flow::Normal)
    }

    /// PUT (M14.2) : rend chaque item dans la ligne de sortie courante puis,
    /// sauf hold `@`/`@@` final, relâche la ligne vers la destination.
    pub(super) fn exec_put(&mut self, items: &[crate::ast::PutItem]) -> Result<Flow> {
        use crate::ast::PutItem;
        // Un nouveau PUT efface le hold simple précédent (la ligne maintenue
        // par `@` est reprise telle quelle ; un nouveau PUT sans `@` final la
        // relâchera). Le hold est recalculé pour CE statement.
        self.put.hold = false;
        self.put.hold_double = false;
        self.put.started = true;

        for item in items {
            match item {
                PutItem::ColumnPointer(n) => {
                    self.put.cursor = n.saturating_sub(1);
                }
                PutItem::SkipColumns(n) => {
                    self.put.cursor += n;
                }
                PutItem::NextLine => {
                    // Saut de ligne DANS le même PUT : relâche la ligne
                    // courante et en commence une nouvelle (même destination).
                    self.put_release_line();
                    self.put.started = true;
                }
                PutItem::HoldLine => self.put.hold = true,
                PutItem::HoldLineDouble => {
                    self.put.hold = true;
                    self.put.hold_double = true;
                }
                PutItem::Literal(s) => {
                    self.put_write_at(s);
                    // Un blanc sépare l'item suivant en mode liste.
                    self.put.cursor += 1;
                }
                PutItem::Var { name, format } => {
                    let text = self.render_put_var(name, format.as_deref())?;
                    self.put_write_at(&text);
                    self.put.cursor += 1;
                }
                PutItem::NamedVar(name) => {
                    let val = self.render_put_var(name, None)?;
                    let text = format!("{}={}", name, val);
                    self.put_write_at(&text);
                    self.put.cursor += 1;
                }
                PutItem::All => {
                    // `var=value` pour chaque variable du PDV, séparés d'un
                    // blanc, dans l'ordre du PDV.
                    let n = self.pdv.vars().len();
                    for slot in 0..n {
                        // Les éléments d'array _TEMPORARY_ ne sont pas listés.
                        if self.pdv.vars()[slot].temporary {
                            continue;
                        }
                        let name = self.pdv.vars()[slot].name.clone();
                        let val = self.render_put_slot(slot, None);
                        let text = format!("{}={}", name, val);
                        self.put_write_at(&text);
                        self.put.cursor += 1;
                    }
                }
            }
        }

        // Fin du PUT : sauf hold, relâcher la ligne.
        if !self.put.hold && !self.put.hold_double {
            self.put_release_line();
        }
        Ok(Flow::Normal)
    }

    /// Écrit `text` dans la ligne de sortie courante à partir de la colonne
    /// `cursor` (0-based), en complétant de blancs si le curseur est au-delà
    /// de la longueur courante, et avance le curseur après le texte écrit.
    fn put_write_at(&mut self, text: &str) {
        let mut chars: Vec<char> = self.put.line.chars().collect();
        let start = self.put.cursor;
        // Compléter de blancs jusqu'à `start`.
        while chars.len() < start {
            chars.push(' ');
        }
        // Écrire (écrasement) à partir de `start`.
        for (i, c) in text.chars().enumerate() {
            let pos = start + i;
            if pos < chars.len() {
                chars[pos] = c;
            } else {
                chars.push(c);
            }
        }
        self.put.cursor = start + text.chars().count();
        self.put.line = chars.into_iter().collect();
    }

    /// Relâche (flush + clear) la ligne de sortie courante vers la
    /// destination active, et réinitialise l'état de ligne.
    pub(super) fn put_release_line(&mut self) {
        let line = std::mem::take(&mut self.put.line);
        // SAS rogne les blancs de fin de la ligne PUT relâchée.
        let line = line.trim_end().to_string();
        let dest = self.put.dest.clone();
        self.put.out.push((dest, line));
        self.put.cursor = 0;
        self.put.started = false;
        self.put.hold = false;
        self.put.hold_double = false;
    }

    /// Flush de fin d'étape : une ligne encore maintenue (`@`/`@@`) ou en
    /// construction est relâchée.
    pub(super) fn put_flush_at_step_end(&mut self) {
        if self.put.started || !self.put.line.is_empty() {
            self.put_release_line();
        }
    }

    /// Rejoue les lignes PUT produites vers leurs destinations (LOG, listing,
    /// fichiers externes). Les fichiers sont regroupés par chemin et écrits
    /// (création/troncature) en une fois.
    pub(super) fn put_replay(&mut self, session: &mut Session) -> Result<()> {
        use std::collections::HashMap;
        // Tampon par fichier (ordre des lignes préservé).
        let mut files: HashMap<String, Vec<String>> = HashMap::new();
        let mut file_order: Vec<String> = Vec::new();
        for (dest, line) in std::mem::take(&mut self.put.out) {
            match dest {
                PutDestKind::Log => session.log.put_line(&line),
                PutDestKind::Print => session.listing.write_line(&line),
                PutDestKind::Path(path) => {
                    files
                        .entry(path.clone())
                        .or_insert_with(|| {
                            file_order.push(path.clone());
                            Vec::new()
                        })
                        .push(line);
                }
            }
        }
        for path in file_order {
            let lines = files.remove(&path).unwrap_or_default();
            let mut content = lines.join("\n");
            // Terminer le fichier par un saut de ligne (convention texte).
            if !content.is_empty() {
                content.push('\n');
            }
            // Chemin relatif résolu sous `base_dir` (cohérent avec LIBNAME et
            // INFILE) ; le message d'erreur garde le chemin source.
            let resolved = session.resolve_path(&path);
            std::fs::write(&resolved, content).map_err(|e| {
                SasError::runtime(format!("Unable to write the FILE '{path}': {e}"))
            })?;
        }
        Ok(())
    }

    /// Rend une variable PUT (par nom) en texte, avec son format explicite
    /// (`format`), ou son format d'affichage, ou le défaut BESTw./$w.
    fn render_put_var(&self, name: &str, format: Option<&str>) -> Result<String> {
        let slot = self.pdv.slot(name).ok_or_else(|| {
            SasError::runtime(format!("Variable {name} is not on the PUT statement."))
        })?;
        Ok(self.render_put_slot(slot, format))
    }

    /// Rend la valeur du slot PDV `slot` en texte pour un PUT. Ordre de
    /// résolution du format : format explicite de l'item > format d'affichage
    /// de la variable > défaut (BEST12. justifié à droite pour un numérique,
    /// valeur brute pour un caractère). Le résultat est rogné de ses blancs
    /// de bord (mode liste SAS : les valeurs formatées sont posées « left
    /// aligned » dans la ligne).
    fn render_put_slot(&self, slot: usize, format: Option<&str>) -> String {
        let value = self.pdv.get(slot).clone();
        // Format explicite, sinon format d'affichage de la variable.
        let fmt_tok = format
            .map(str::to_string)
            .or_else(|| self.pdv.vars()[slot].format.clone());
        if let Some(tok) = fmt_tok {
            if let Some(spec) = crate::formats::FormatSpec::parse(&tok) {
                return self.format_catalog.format(&value, &spec).trim().to_string();
            }
        }
        // Défaut : pas de format.
        match value {
            Value::Missing(kind) => kind.display(),
            Value::Num(f) => format_best(f, 12).trim().to_string(),
            Value::Char(s) => s,
        }
    }
}
