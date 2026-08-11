use super::*;

impl Compiler<'_> {
    /// Compile un `RETAIN` (bras `DsStmt::Retain` de `walk_stmt`).
    pub(super) fn compile_retain(&mut self, items: &[(String, Option<Expr>)]) -> Result<()> {
        if items.is_empty() {
            // `retain;` seul : tout le PDV (cf. fin de compile()).
            self.retain_all = true;
            return Ok(());
        }
        for (name, init) in items {
            // RETAIN _ALL_ (M16.6) : retient TOUTES les variables
            // connues du PDV À CE POINT (≠ `retain;` nu qui retient le
            // PDV entier en fin de compilation). Les variables créées
            // APRÈS ce statement ne sont donc PAS retenues. Aucune
            // valeur initiale n'est admise sur `_all_` ; il ne crée
            // jamais de variable nommée `_ALL_`.
            if name.eq_ignore_ascii_case("_all_") {
                if init.is_some() {
                    return Err(SasError::runtime(
                        "An initial value is not allowed with RETAIN _ALL_.",
                    ));
                }
                for slot in 0..self.pdv.vars().len() {
                    self.retained_slots.insert(slot);
                }
                continue;
            }
            // Listes spéciales _NUMERIC_/_CHARACTER_ : retiennent les
            // variables du type voulu connues à ce point (mêmes règles
            // que _ALL_ — créées après = non retenues).
            if name.eq_ignore_ascii_case("_numeric_") || name.eq_ignore_ascii_case("_character_") {
                if init.is_some() {
                    return Err(SasError::runtime(
                        "An initial value is not allowed with a special RETAIN list.",
                    ));
                }
                let want = if name.eq_ignore_ascii_case("_numeric_") {
                    VarType::Num
                } else {
                    VarType::Char
                };
                let slots: Vec<usize> = self
                    .pdv
                    .vars()
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.ty == want)
                    .map(|(i, _)| i)
                    .collect();
                for slot in slots {
                    self.retained_slots.insert(slot);
                }
                continue;
            }
            match init {
                // AVEC init : la variable entre au PDV ICI (ordre de
                // première référence), type/longueur du littéral, et
                // sa valeur initiale part dans `initial_values`. Elle
                // compte comme initialisée (pas de NOTE
                // "uninitialized" — comme SAS).
                Some(expr) => {
                    let (ty, length, value) = retain_literal(expr)?;
                    let slot = self.add_var(name, ty, length);
                    self.retained_slots.insert(slot);
                    self.assigned.insert(name.to_uppercase());
                    self.initial_values.push((slot, value));
                }
                // SANS init : ne crée PAS la variable (le type sera
                // figé par sa prochaine référence) — voir compile().
                None => self.retain_pending.push(name.clone()),
            }
        }
        Ok(())
    }

    /// Compile un sum statement `var + expr;` (bras `DsStmt::Sum` de `walk_stmt`).
    pub(super) fn compile_sum(&mut self, var: &str, expr: &Expr) -> Result<()> {
        // `var + expr;` : var entre au PDV (Num, 8), retenue, valeur
        // initiale 0 — SAUF si un RETAIN avec init a déjà posé une
        // valeur pour ce slot (le RETAIN gagne, comme SAS). La cible
        // entre avant les variables de l'expression (ordre textuel).
        let slot = self.add_var(var, VarType::Num, 8);
        self.retained_slots.insert(slot);
        self.assigned.insert(var.to_uppercase());
        if !self.initial_values.iter().any(|(s, _)| *s == slot) {
            self.initial_values.push((slot, Value::Num(0.0)));
        }
        self.walk_expr(expr)?;
        Ok(())
    }

    /// Compile un `LENGTH` (bras `DsStmt::Length` de `walk_stmt`).
    pub(super) fn compile_length(&mut self, items: &[(String, LengthSpec)]) -> Result<()> {
        for (name, spec) in items {
            // Plages SAS : char 1..=32767, num 3..=8.
            let (lo, hi) = if spec.char { (1, 32767) } else { (3, 8) };
            if spec.len < lo || spec.len > hi {
                return Err(SasError::runtime(format!(
                    "The length {} specified for the variable {} is out of range ({}-{}).",
                    spec.len, name, lo, hi
                )));
            }
            match self.pdv.slot(name) {
                // LENGTH précède la première référence : crée la
                // variable avec cette longueur. Pour une numérique,
                // la longueur (3..=8) est une simple MÉTADONNÉE en
                // M2 — le stockage reste f64 sur 8 octets.
                None => {
                    let ty = if spec.char {
                        VarType::Char
                    } else {
                        VarType::Num
                    };
                    self.add_var(name, ty, spec.len);
                }
                // Déjà au PDV : la longueur est figée. SAS n'émet le
                // WARNING que pour les variables CHAR dont la
                // longueur demandée diffère ; num : silencieux.
                Some(slot) => {
                    let v = &self.pdv.vars()[slot];
                    if v.ty == VarType::Char && spec.char && v.length != spec.len {
                        let name = v.name.clone();
                        self.session.log.warning(&format!(
                            "Length of character variable {name} has already been set."
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Compile un `FORMAT` (bras `DsStmt::Format` de `walk_stmt`).
    pub(super) fn compile_format(&mut self, groups: &[(Vec<String>, String)]) -> Result<()> {
        for (names, token) in groups {
            if crate::formats::FormatSpec::parse(token).is_none() {
                return Err(SasError::runtime(format!(
                    "The format {token} is not valid."
                )));
            }
            for name in names {
                self.formats.insert(name.to_uppercase(), token.clone());
            }
        }
        Ok(())
    }

    /// Compile un `ATTRIB` (bras `DsStmt::Attrib` de `walk_stmt`).
    /// `informat=` n'est PAS traité ici : il est collecté (et validé) par la
    /// pré-passe `collect_informats` (M40.3), avant le walk.
    pub(super) fn compile_attrib(&mut self, items: &[AttribItem]) -> Result<()> {
        for item in items {
            if let Some(token) = &item.format
                && crate::formats::FormatSpec::parse(token).is_none()
            {
                return Err(SasError::runtime(format!(
                    "The format {token} is not valid."
                )));
            }
            for name in &item.vars {
                let upper = name.to_uppercase();
                if let Some(token) = &item.format {
                    self.formats.insert(upper.clone(), token.clone());
                }
                if let Some(label) = &item.label {
                    self.labels.insert(upper.clone(), label.clone());
                }
                // length= : parsé mais non appliqué en M4.
            }
        }
        Ok(())
    }

    /// PRÉ-PASSE M40.3 : collecte (et valide) les informats déclarés par les
    /// statements `INFORMAT` et `ATTRIB informat=` de TOUTE l'étape, AVANT
    /// le walk — l'association est donc indépendante de l'ordre
    /// statement/INPUT (fidèle à SAS : ces statements sont purement
    /// déclaratifs). Une déclaration ultérieure pour la même variable écrase
    /// la précédente (dernier gagne). Un informat qui ne parse pas → même
    /// erreur que l'INPUT formaté avec un informat inconnu.
    pub(super) fn collect_informats(&mut self, stmts: &[DsStmt]) -> Result<()> {
        for stmt in stmts {
            self.collect_informats_stmt(stmt)?;
        }
        Ok(())
    }

    fn collect_informats_stmt(&mut self, stmt: &DsStmt) -> Result<()> {
        let declare =
            |names: &[String], token: &String, informats: &mut HashMap<String, String>| {
                if crate::formats::FormatSpec::parse(token).is_none() {
                    return Err(SasError::runtime(format!(
                        "The informat {token} is not valid."
                    )));
                }
                for name in names {
                    informats.insert(name.to_uppercase(), token.clone());
                }
                Ok(())
            };
        match stmt {
            DsStmt::Informat(groups) => {
                for (names, token) in groups {
                    declare(names, token, &mut self.informats)?;
                }
                Ok(())
            }
            DsStmt::Attrib(items) => {
                for item in items {
                    if let Some(token) = &item.informat {
                        declare(&item.vars, token, &mut self.informats)?;
                    }
                }
                Ok(())
            }
            // Descente dans les statements composés : un INFORMAT/ATTRIB peut
            // vivre dans un IF/DO/SELECT (déclaratif quel que soit le flot).
            DsStmt::If {
                then_branch,
                else_branch,
                ..
            } => {
                self.collect_informats_stmt(then_branch)?;
                if let Some(e) = else_branch {
                    self.collect_informats_stmt(e)?;
                }
                Ok(())
            }
            DsStmt::Block(body)
            | DsStmt::DoLoop { body, .. }
            | DsStmt::DoList { body, .. }
            | DsStmt::DoOver { body, .. } => self.collect_informats(body),
            DsStmt::Select {
                whens, otherwise, ..
            } => {
                for w in whens {
                    self.collect_informats_stmt(&w.body)?;
                }
                if let Some(o) = otherwise {
                    self.collect_informats_stmt(o)?;
                }
                Ok(())
            }
            DsStmt::Labeled { stmt, .. } => self.collect_informats_stmt(stmt),
            _ => Ok(()),
        }
    }

    /// Compile un `CALL <name>(args);` (bras `DsStmt::CallRoutine` de `walk_stmt`).
    pub(super) fn compile_call_routine(&mut self, name: &str, args: &[Expr]) -> Result<()> {
        // CALL SORTN/SORTC (M15.6) acceptent un NOM D'ARRAY entier en
        // argument (`call sortn(arr)`) — ce n'est pas une référence de
        // variable illégale, mais le déballage de tous ses éléments.
        // On ne walke donc PAS un argument qui nomme un array déclaré.
        let is_sort = name.eq_ignore_ascii_case("sortn") || name.eq_ignore_ascii_case("sortc");
        for a in args {
            if is_sort
                && let Expr::Var(n) = a
                && self.arrays.contains_key(&n.to_uppercase())
            {
                continue;
            }
            self.walk_expr(a)?;
        }
        Ok(())
    }

    /// Compile un `INPUT` (bras `DsStmt::Input` de `walk_stmt`).
    pub(super) fn compile_input(&mut self, items: &[crate::ast::InputItem]) -> Result<()> {
        self.seen_input = true;
        for item in items {
            if let crate::ast::InputItem::Var {
                name,
                is_char,
                informat,
                ..
            } = item
            {
                let (ty, length) = input_var_type(*is_char, informat.as_deref())?;
                self.add_var(name, ty, length);
                // Une variable d'INPUT est « assignée » (pas de NOTE
                // uninitialized).
                self.assigned.insert(name.to_uppercase());
            }
        }
        Ok(())
    }
}
