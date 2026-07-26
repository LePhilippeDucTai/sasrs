use super::*;

impl Compiler<'_> {
    /// Compile une assignation `var = expr;` (bras `DsStmt::Assign` de `walk_stmt`).
    pub(super) fn compile_assign(&mut self, var: &str, expr: &Expr) -> Result<()> {
        let upper = var.to_uppercase();
        // `arr = e;` à l'intérieur d'un `DO OVER arr` : assignation à
        // l'élément courant (résolue à l'exécution) — ne crée PAS de
        // variable. Hors DO OVER, un nom d'array nu est illégal.
        if self.arrays.contains_key(&upper) {
            if self.do_over_arrays.contains(&upper) {
                self.assigned.insert(upper);
                self.walk_expr(expr)?;
                return Ok(());
            }
            return Err(SasError::runtime(format!(
                "Illegal reference to the array {var}."
            )));
        }
        // La cible entre au PDV en premier (ordre textuel), avec le
        // type inféré AVANT création des variables de l'expression
        // (les inconnues comptent comme Num, cohérent avec SAS).
        let (ty, length) = self.infer(expr);
        self.add_var(var, ty, length);
        self.assigned.insert(var.to_uppercase());
        self.walk_expr(expr)?;
        Ok(())
    }

    /// Compile une assignation indexée `arr{i} = expr;` (bras `DsStmt::AssignIndexed` de `walk_stmt`).
    pub(super) fn compile_assign_indexed(
        &mut self,
        array: &str,
        indices: &[Expr],
        expr: &Expr,
    ) -> Result<()> {
        let upper = array.to_uppercase();
        let Some(def) = self.arrays.get(&upper) else {
            return Err(SasError::runtime(format!(
                "Undeclared array referenced: {array}."
            )));
        };
        // Tous les éléments sont potentiellement assignés via
        // l'indice : pas de NOTE "uninitialized" pour eux.
        for slot in def.slots.clone() {
            let n = self.pdv.vars()[slot].name.to_uppercase();
            self.assigned.insert(n);
        }
        for index in indices {
            self.walk_expr(index)?;
        }
        self.walk_expr(expr)?;
        Ok(())
    }

    /// Déclare un array (M2/M16.2). Les éléments entrent au PDV ICI (ordre
    /// de première référence). `dims` None (`{*}`) → 1-D, taille déduite de
    /// la liste ; sinon bornes supérieures explicites (le produit = nombre
    /// d'éléments). `vars` vide (et pas de liste spéciale) → éléments
    /// auto-nommés name1..nameN. `char_len` → éléments caractère.
    /// `initial` → valeurs initiales row-major (RETAIN implicite). `temp` →
    /// éléments hors-sortie, retenus. `special` → `_NUMERIC_`/`_CHARACTER_`/
    /// `_ALL_` remplacé par les variables PDV correspondantes. Le registre
    /// `arrays` associe le nom UPPERCASE à la définition (slots + dims).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn compile_array(
        &mut self,
        name: &str,
        dims: Option<&[usize]>,
        char_len: Option<usize>,
        vars: &[String],
        initial: &[crate::ast::Expr],
        temp: bool,
        special: Option<crate::ast::ArraySpecial>,
    ) -> Result<()> {
        use crate::ast::ArraySpecial;
        let upper = name.to_uppercase();
        if self.arrays.contains_key(&upper) {
            return Err(SasError::runtime(format!(
                "An array has already been defined with the name {name}."
            )));
        }

        let (ty, length) = match char_len {
            Some(l) => (VarType::Char, l),
            None => (VarType::Num, 8),
        };

        // Liste effective d'éléments à entrer au PDV (ou slots existants
        // pour les listes spéciales). On collecte directement des slots.
        let slots: Vec<usize> = if let Some(kind) = special {
            // `_NUMERIC_`/`_CHARACTER_`/`_ALL_` : toutes les variables
            // (NON-temporaires) connues au point du statement, du type voulu.
            let want_char = matches!(kind, ArraySpecial::Character)
                || (matches!(kind, ArraySpecial::All) && char_len.is_some());
            let want = if want_char {
                VarType::Char
            } else {
                VarType::Num
            };
            let picked: Vec<usize> = self
                .pdv
                .vars()
                .iter()
                .enumerate()
                .filter(|(_, v)| {
                    !v.temporary
                        && match kind {
                            ArraySpecial::Numeric => v.ty == VarType::Num,
                            ArraySpecial::Character => v.ty == VarType::Char,
                            ArraySpecial::All => v.ty == want,
                        }
                })
                .map(|(i, _)| i)
                .collect();
            if picked.is_empty() {
                return Err(SasError::runtime(format!(
                    "The array {name} has been defined with zero elements."
                )));
            }
            // Une dimension explicite doit correspondre au compte trouvé.
            if let Some(ds) = dims {
                let total: usize = ds.iter().product();
                if total != picked.len() {
                    return Err(SasError::runtime(format!(
                        "The number of variables in the list ({}) does not match \
                         the number of elements ({}) in the array {}.",
                        picked.len(),
                        total,
                        name
                    )));
                }
            }
            picked
        } else {
            // Liste nommée OU éléments auto-générés.
            let total = dims.map(|d| d.iter().product::<usize>());
            let names: Vec<String> = if vars.is_empty() {
                let Some(n) = total else {
                    return Err(SasError::runtime(format!(
                        "The array {name} has been defined with zero elements."
                    )));
                };
                if temp {
                    // Éléments temporaires : noms internes non collisionnables.
                    (1..=n).map(|i| format!("*{name}[{i}]")).collect()
                } else {
                    (1..=n).map(|i| format!("{name}{i}")).collect()
                }
            } else {
                if let Some(n) = total
                    && n != vars.len()
                {
                    return Err(SasError::runtime(format!(
                        "The number of variables in the list ({}) does not match \
                         the number of elements ({}) in the array {}.",
                        vars.len(),
                        n,
                        name
                    )));
                }
                vars.to_vec()
            };
            if temp {
                names
                    .iter()
                    .map(|v| self.add_temp_var(v, ty, length))
                    .collect()
            } else {
                names.iter().map(|v| self.add_var(v, ty, length)).collect()
            }
        };

        // Dimensions résolues : explicites, ou 1-D = nombre d'éléments.
        let dim_vec: Vec<usize> = match dims {
            Some(d) => d.to_vec(),
            None => vec![slots.len()],
        };

        // Valeurs initiales (row-major) : évaluées à la COMPILATION (les
        // littéraux constants suffisent) puis appliquées via `initial_values`
        // avant la 1re itération, comme RETAIN avec init. SAS marque les
        // éléments initialisés comme retenus.
        if !initial.is_empty() {
            if initial.len() > slots.len() {
                return Err(SasError::runtime(format!(
                    "Too many initial values were specified for the array {name}."
                )));
            }
            for (k, expr) in initial.iter().enumerate() {
                let v = const_eval_initial(expr, ty)?;
                self.initial_values.push((slots[k], v));
                self.retained_slots.insert(slots[k]);
                let nm = self.pdv.vars()[slots[k]].name.to_uppercase();
                self.assigned.insert(nm);
            }
        }

        self.arrays.insert(
            upper,
            ArrayDef {
                slots,
                dims: dim_vec,
            },
        );
        Ok(())
    }

    /// Type et longueur des éléments d'un array (premier slot — tous les
    /// éléments d'un array M2 partagent type et longueur déclarés ; un
    /// élément préexistant au PDV garde toutefois les siens).
    pub(super) fn array_elem_type(&self, name: &str) -> (VarType, usize) {
        match self
            .arrays
            .get(&name.to_uppercase())
            .and_then(|def| def.slots.first())
        {
            Some(&slot) => {
                let v = &self.pdv.vars()[slot];
                (v.ty, v.length)
            }
            None => (VarType::Num, 8),
        }
    }
}
