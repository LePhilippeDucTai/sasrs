use super::*;

impl Compiler<'_> {
    /// Crée les variables simplement référencées (Num par défaut), en ordre
    /// textuel gauche→droite. Les noms d'array ne créent JAMAIS de variable
    /// au PDV : `Expr::Index` ne walke que son indice (le nom doit être un
    /// array déclaré), `dim(arr)` ne crée pas `arr`, et un nom d'array nu
    /// (`Expr::Var`) est une référence illégale.
    pub(super) fn walk_expr(&mut self, expr: &Expr) -> Result<()> {
        match expr {
            Expr::Num(_) | Expr::Str(_) | Expr::Missing(_) => Ok(()),
            Expr::Var(name) => {
                let upper = name.to_uppercase();
                // Variables automatiques (_N_, _ERROR_) : servies par
                // l'évaluateur depuis les champs dédiés du PDV — elles ne
                // doivent JAMAIS créer de slot (sinon elles deviendraient
                // des colonnes de sortie + NOTE "uninitialized" parasite).
                if upper == "_N_" || upper == "_ERROR_" {
                    return Ok(());
                }
                // FIRST.x / LAST.x : variables automatiques de groupe BY,
                // servies par l'évaluateur depuis les flags du Runner —
                // jamais de slot PDV (donc jamais écrites en sortie). On
                // mémorise la référence pour la valider contre le BY en
                // fin de compilation.
                if upper.starts_with("FIRST.") || upper.starts_with("LAST.") {
                    self.first_last_refs.push(upper);
                    return Ok(());
                }
                // Variable IN= d'un MERGE : automatique temporaire 0/1,
                // servie par EvalCtx — jamais de slot PDV (donc jamais
                // écrite en sortie).
                if self.in_flags.iter().any(|(n, _)| *n == upper) {
                    return Ok(());
                }
                // Variable END= du SET (M16.4) : automatique temporaire 0/1,
                // servie par EvalCtx — jamais de slot PDV (donc jamais écrite
                // en sortie). On la reconnaît au nom déclaré sur le SET.
                if self
                    .set_options
                    .end
                    .as_ref()
                    .is_some_and(|e| e.eq_ignore_ascii_case(name))
                {
                    return Ok(());
                }
                if self.arrays.contains_key(&upper) {
                    // Référence nue à un array : autorisée si un `DO OVER` est
                    // actif (élément courant, résolu à l'exécution) ; ne crée
                    // pas de variable. Sinon illégale.
                    if self.do_over_arrays.contains(&upper) {
                        return Ok(());
                    }
                    return Err(SasError::runtime(format!(
                        "Illegal reference to the array {name}."
                    )));
                }
                self.add_var(name, VarType::Num, 8);
                Ok(())
            }
            Expr::Unary { expr, .. } => self.walk_expr(expr),
            Expr::Binary { left, right, .. } => {
                self.walk_expr(left)?;
                self.walk_expr(right)
            }
            Expr::In { expr, list } => {
                self.walk_expr(expr)?;
                for e in list {
                    self.walk_expr(e)?;
                }
                Ok(())
            }
            Expr::Index { name, indices } => {
                if !self.arrays.contains_key(&name.to_uppercase()) {
                    return Err(SasError::runtime(format!(
                        "Undeclared array referenced: {name}."
                    )));
                }
                for index in indices {
                    self.walk_expr(index)?;
                }
                Ok(())
            }
            Expr::Call { name, args } => {
                // `dim(arr)`/`hbound(arr[, n])`/`lbound(arr[, n])` : le 1er
                // argument nomme un array — il ne crée PAS de variable. Les
                // autres arguments (dimension) sont walkés normalement.
                let is_dim_fn = name.eq_ignore_ascii_case("dim")
                    || name.eq_ignore_ascii_case("hbound")
                    || name.eq_ignore_ascii_case("lbound");
                if is_dim_fn
                    && !args.is_empty()
                    && let Expr::Var(n) | Expr::Index { name: n, .. } = &args[0]
                    && self.arrays.contains_key(&n.to_uppercase())
                {
                    // `dim(a{i})` : l'indice du 1er argument reste walké.
                    if let Expr::Index { indices, .. } = &args[0] {
                        for index in indices {
                            self.walk_expr(index)?;
                        }
                    }
                    for a in &args[1..] {
                        self.walk_expr(a)?;
                    }
                    return Ok(());
                }
                for a in args {
                    self.walk_expr(a)?;
                }
                Ok(())
            }
            // Méthode d'objet hash en expression (M17.2) : même validation que
            // la forme statement.
            Expr::HashMethod(call) => {
                self.validate_hash_method(&call.object, &call.method, &call.args)
            }
        }
    }

    /// `add_var` PDV : la première référence fige tout (le PDV ignore les
    /// ajouts suivants du même nom).
    pub(super) fn add_var(&mut self, name: &str, ty: VarType, length: usize) -> usize {
        self.pdv.add_var(PdvVar {
            name: name.to_string(),
            ty,
            length,
            retained: false,
            from_input: false,
            format: None,
            temporary: false,
        })
    }

    /// Slot d'un élément d'array `_TEMPORARY_` : hors-PDV-de-sortie, retenu
    /// implicitement. Les noms internes (`*name[i]`) ne peuvent collisionner
    /// avec une variable utilisateur (`*` interdit en SAS).
    pub(super) fn add_temp_var(&mut self, name: &str, ty: VarType, length: usize) -> usize {
        self.pdv.add_var(PdvVar {
            name: name.to_string(),
            ty,
            length,
            retained: true,
            from_input: false,
            format: None,
            temporary: true,
        })
    }

    /// Toute variable d'un WHERE= de SET doit déjà être au PDV (= une
    /// variable de l'input, après keep/drop/rename) — message proche du
    /// SAS "Variable x is not on file WORK.A.". On ne walke PAS via
    /// `walk_expr` : cela créerait des variables Num parasites au PDV.
    pub(super) fn validate_where_vars(&self, expr: &Expr, file: &str) -> Result<()> {
        match expr {
            Expr::Num(_) | Expr::Str(_) | Expr::Missing(_) => Ok(()),
            Expr::Var(name) => {
                let upper = name.to_uppercase();
                if upper == "_N_" || upper == "_ERROR_" || self.pdv.slot(name).is_some() {
                    Ok(())
                } else {
                    Err(SasError::runtime(format!(
                        "Variable {name} is not on file {file}."
                    )))
                }
            }
            Expr::Unary { expr, .. } => self.validate_where_vars(expr, file),
            Expr::Binary { left, right, .. } => {
                self.validate_where_vars(left, file)?;
                self.validate_where_vars(right, file)
            }
            Expr::In { expr, list } => {
                self.validate_where_vars(expr, file)?;
                for e in list {
                    self.validate_where_vars(e, file)?;
                }
                Ok(())
            }
            Expr::Index { indices, .. } => {
                for index in indices {
                    self.validate_where_vars(index, file)?;
                }
                Ok(())
            }
            Expr::Call { args, .. } => {
                for a in args {
                    self.validate_where_vars(a, file)?;
                }
                Ok(())
            }
            // Une méthode hash dans un WHERE= de SET n'a pas de sens : rejet.
            Expr::HashMethod(_) => Err(SasError::runtime(format!(
                "Hash method calls are not allowed in a WHERE= clause on file {file}."
            ))),
        }
    }

    /// Type et longueur inférés d'une expression (compile-time, comme SAS).
    pub(super) fn infer(&self, expr: &Expr) -> (VarType, usize) {
        match expr {
            Expr::Num(_) | Expr::Missing(_) => (VarType::Num, 8),
            Expr::Str(s) => (VarType::Char, s.chars().count().max(1)),
            Expr::Var(name) => match self.pdv.slot(name) {
                Some(slot) => {
                    let v = &self.pdv.vars()[slot];
                    (v.ty, v.length)
                }
                // Inconnue au moment de l'inférence : numérique.
                None => (VarType::Num, 8),
            },
            Expr::Unary { .. } => (VarType::Num, 8),
            Expr::Binary {
                op: BinaryOp::Concat,
                left,
                right,
            } => (VarType::Char, self.char_len(left) + self.char_len(right)),
            Expr::Binary { .. } | Expr::In { .. } => (VarType::Num, 8),
            // `arr{i}` : type/longueur des éléments de l'array.
            Expr::Index { name, .. } => self.array_elem_type(name),
            Expr::Call { name, args } => {
                // Forme parenthèses `arr(i)`/`arr(i,j)` : l'array masque la
                // fonction.
                if !args.is_empty() && self.arrays.contains_key(&name.to_uppercase()) {
                    return self.array_elem_type(name);
                }
                let lower = name.to_ascii_lowercase();
                match lower.as_str() {
                    "upcase" | "lowcase" | "trim" | "strip" | "left" | "right" => {
                        let len = args.first().map_or(200, |a| self.char_len(a));
                        (VarType::Char, len)
                    }
                    "substr" => {
                        let len = args.first().map_or(200, |a| self.char_len(a));
                        (VarType::Char, len)
                    }
                    _ if lower.starts_with("cat") => (VarType::Char, 200),
                    "put" => (VarType::Char, put_width(args)),
                    _ => (VarType::Num, 8),
                }
            }
            // Méthode hash en expression (M17.2) : renvoie un code retour
            // numérique (8 octets).
            Expr::HashMethod(_) => (VarType::Num, 8),
        }
    }

    /// Longueur d'un opérande en contexte caractère : un opérande numérique
    /// contribue 12 (conversion implicite BEST12., comme SAS).
    pub(super) fn char_len(&self, expr: &Expr) -> usize {
        match self.infer(expr) {
            (VarType::Char, l) => l,
            (VarType::Num, _) => 12,
        }
    }
}
