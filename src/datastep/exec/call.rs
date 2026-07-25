use super::*;

impl Runner {
    /// Exécute une CALL routine. Routines supportées (M11.5 + M15.6) :
    /// STREAMINIT, SYMPUT, SYMPUTX, MISSING, EXECUTE, SORTN, SORTC, CATS,
    /// SCAN, LABEL, VNAME. Toute autre → erreur « not yet implemented ».
    ///
    /// Les routines qui ÉCRIVENT dans un argument (MISSING, SORTN/SORTC, CATS,
    /// SCAN, LABEL, VNAME) résolvent cet argument en lvalue (variable ou
    /// élément d'array) via `resolve_lvalue_slot`. SYMPUT/SYMPUTX diffèrent
    /// l'écriture macro à la fin de l'étape (règle de visibilité SAS) ;
    /// EXECUTE met du code en file pour exécution post-étape.
    pub(super) fn exec_call_routine(&mut self, name: &str, args: &[crate::ast::Expr]) -> Result<Flow> {
        // CALL STREAMINIT(seed) — initialise the RNG stream. Accepts an
        // optional single argument (integer seed); no argument → no-op.
        if name.eq_ignore_ascii_case("streaminit") {
            if let Some(seed_expr) = args.first() {
                let seed_val = self.eval_checked(seed_expr)?;
                if let Value::Num(f) = seed_val {
                    self.ctx.rng_state = crate::datastep::functions::streaminit_seed(f as i64);
                    self.ctx.rng_spare = None; // invalidate cached Box-Muller spare
                }
                // missing seed value → no-op (as per spec)
            }
            return Ok(Flow::Normal);
        }

        let upper = name.to_uppercase();
        match upper.as_str() {
            "SYMPUT" => self.call_symput(args, false),
            "SYMPUTX" => self.call_symput(args, true),
            "MISSING" => self.call_missing(args),
            "EXECUTE" => self.call_execute(args),
            "SORTN" => self.call_sort(args, false),
            "SORTC" => self.call_sort(args, true),
            "CATS" => self.call_cats(args),
            "SCAN" => self.call_scan(args),
            "LABEL" => self.call_label(args),
            "VNAME" => self.call_vname(args),
            _ => Err(SasError::runtime(format!(
                "CALL routine {upper} is not yet implemented."
            ))),
        }
    }

    /// CALL SYMPUT(name, value) / CALL SYMPUTX(name, value) — écrit un
    /// symbole macro. SYMPUTX rogne EN PLUS les blancs de tête ET de fin de
    /// la valeur (et un nombre est formaté sans blancs) ; SYMPUT garde la
    /// valeur char telle quelle. Les deux trim­ent le nom.
    pub(super) fn call_symput(&mut self, args: &[crate::ast::Expr], x: bool) -> Result<Flow> {
        if args.len() != 2 {
            return Err(SasError::runtime(if x {
                "CALL SYMPUTX requires exactly two arguments (name, value)."
            } else {
                "CALL SYMPUT requires exactly two arguments (name, value)."
            }));
        }
        let name_val = self.eval_checked(&args[0])?;
        let value_val = self.eval_checked(&args[1])?;
        let sym_name = symput_string(name_val);
        let sym_value = symput_string(value_val);
        // SYMPUTX rogne les deux bords de la valeur ; SYMPUT la garde telle
        // quelle (mais BEST12. d'un nombre est déjà cadré à gauche).
        let sym_value = if x {
            sym_value.trim().to_string()
        } else {
            sym_value
        };
        self.ctx
            .symput_writes
            .push((sym_name.trim().to_string(), sym_value));
        Ok(Flow::Normal)
    }

    /// CALL MISSING(var, var, ...) — met chaque variable argument à missing
    /// (`.` pour numérique, `""` pour caractère). Chaque argument doit être
    /// une lvalue (variable scalaire ou élément d'array).
    pub(super) fn call_missing(&mut self, args: &[crate::ast::Expr]) -> Result<Flow> {
        for arg in args {
            let slot = self.resolve_lvalue_slot(arg)?;
            let init = match self.pdv.vars()[slot].ty {
                VarType::Num => Value::missing(),
                VarType::Char => Value::Char(String::new()),
            };
            self.pdv.set(slot, init);
        }
        Ok(Flow::Normal)
    }

    /// CALL EXECUTE(arg) — met le texte résolu de `arg` en file pour
    /// exécution APRÈS l'étape DATA courante. `arg` est évalué comme une
    /// expression caractère ; sa valeur est concaténée (avec un espace de
    /// séparation) au code mis en file. La file est rejouée par l'exécuteur
    /// une fois l'étape terminée.
    ///
    /// Limites documentées : la résolution macro (`%nrstr`, exécution macro à
    /// l'évaluation vs à l'exécution) n'est PAS distinguée — le texte est
    /// rejoué tel quel comme un programme SAS ordinaire (qui passe par le
    /// processeur macro à son tour). Les références `&`/`%` du texte mis en
    /// file sont donc résolues au MOMENT du rejeu, pas de l'appel.
    pub(super) fn call_execute(&mut self, args: &[crate::ast::Expr]) -> Result<Flow> {
        if args.len() != 1 {
            return Err(SasError::runtime(
                "CALL EXECUTE requires exactly one argument.",
            ));
        }
        let v = self.eval_checked(&args[0])?;
        let code = match v {
            Value::Char(s) => s,
            Value::Num(f) => format_best(f, 12).trim().to_string(),
            Value::Missing(_) => String::new(),
        };
        self.call_execute_queue.push(code);
        Ok(Flow::Normal)
    }

    /// CALL SORTN(arr, ...) / CALL SORTC(arr, ...) — trie EN PLACE, par ordre
    /// croissant (`sas_cmp`), les valeurs des variables/éléments passés en
    /// arguments. La forme habituelle est un nom d'array (`call sortn(of a[*])`
    /// — ici on accepte chaque élément ou un array entier), mais SAS accepte
    /// aussi une liste de variables. On collecte donc tous les slots cibles
    /// (un argument array entier dépliant ses slots), on récupère les valeurs,
    /// on les trie, puis on les ré-assigne dans l'ordre des slots.
    pub(super) fn call_sort(&mut self, args: &[crate::ast::Expr], char_sort: bool) -> Result<Flow> {
        use crate::ast::Expr;
        // Collecte des slots cibles, dans l'ordre des arguments. Un argument
        // qui nomme un array entier (`call sortn(arr)`) déplie tous ses slots.
        let mut slots: Vec<usize> = Vec::new();
        for arg in args {
            match arg {
                Expr::Var(name) if self.ctx.arrays.contains_key(&name.to_uppercase()) => {
                    let elems = self.ctx.arrays[&name.to_uppercase()].slots.clone();
                    slots.extend(elems);
                }
                _ => slots.push(self.resolve_lvalue_slot(arg)?),
            }
        }
        if slots.is_empty() {
            return Ok(Flow::Normal);
        }
        // Cohérence de type : SORTN attend du numérique, SORTC du caractère.
        // On ne bloque pas (SAS est permissif) mais on lit les valeurs telles
        // quelles ; `sas_cmp` ordonne num et char dans leur domaine.
        let _ = char_sort;
        let mut values: Vec<Value> = slots.iter().map(|&s| self.pdv.get(s).clone()).collect();
        values.sort_by(|a, b| a.sas_cmp(b));
        for (&slot, v) in slots.iter().zip(values) {
            let coerced = self.coerce_assign(v, self.pdv.vars()[slot].ty);
            self.pdv.set(slot, coerced);
        }
        Ok(Flow::Normal)
    }

    /// CALL CATS(result, item, ...) — concatène `item...` (chacun rogné des
    /// blancs de bord, comme la fonction CATS) dans la variable caractère
    /// `result`. Le résultat est tronqué à la longueur de `result` (sémantique
    /// PDV normale via `set`). Le premier argument est l'lvalue de sortie.
    pub(super) fn call_cats(&mut self, args: &[crate::ast::Expr]) -> Result<Flow> {
        if args.is_empty() {
            return Err(SasError::runtime(
                "CALL CATS requires at least one argument (the result variable).",
            ));
        }
        let result_slot = self.resolve_lvalue_slot(&args[0])?;
        let mut out = String::new();
        for arg in &args[1..] {
            let v = self.eval_checked(arg)?;
            let s = match v {
                Value::Char(s) => s,
                Value::Num(f) => format_best(f, 12).trim().to_string(),
                Value::Missing(k) => k.display(),
            };
            out.push_str(s.trim());
        }
        let coerced = self.coerce_assign(Value::Char(out), self.pdv.vars()[result_slot].ty);
        self.pdv.set(result_slot, coerced);
        Ok(Flow::Normal)
    }

    /// CALL SCAN(string, n, result[, delims]) — extrait le n-ième mot de
    /// `string` (n<0 = depuis la fin) dans la variable caractère `result`.
    /// Réutilise la sémantique de la fonction SCAN. Le 3e argument est
    /// l'lvalue de sortie.
    pub(super) fn call_scan(&mut self, args: &[crate::ast::Expr]) -> Result<Flow> {
        if args.len() < 3 {
            return Err(SasError::runtime(
                "CALL SCAN requires at least three arguments (string, n, result).",
            ));
        }
        // Le mot est calculé par la fonction SCAN (string, n[, delims]).
        let mut fn_args = vec![self.eval_checked(&args[0])?, self.eval_checked(&args[1])?];
        if let Some(delim_arg) = args.get(3) {
            fn_args.push(self.eval_checked(delim_arg)?);
        }
        let result_slot = self.resolve_lvalue_slot(&args[2])?;
        let word = crate::datastep::functions::call("SCAN", &fn_args, &mut self.ctx)
            .unwrap_or(Value::Char(String::new()));
        if let Some(err) = self.ctx.fatal.take() {
            return Err(err);
        }
        let coerced = self.coerce_assign(word, self.pdv.vars()[result_slot].ty);
        self.pdv.set(result_slot, coerced);
        Ok(Flow::Normal)
    }

    /// CALL LABEL(var, result) — pose dans la variable caractère `result` le
    /// libellé de `var`. Si `var` n'a pas de libellé, SAS renvoie le NOM de la
    /// variable (comportement reproduit ici).
    pub(super) fn call_label(&mut self, args: &[crate::ast::Expr]) -> Result<Flow> {
        if args.len() != 2 {
            return Err(SasError::runtime(
                "CALL LABEL requires exactly two arguments (variable, result).",
            ));
        }
        let var_slot = self.resolve_lvalue_slot(&args[0])?;
        let result_slot = self.resolve_lvalue_slot(&args[1])?;
        let var_name = self.pdv.vars()[var_slot].name.clone();
        let label = self
            .labels
            .get(&var_name.to_uppercase())
            .cloned()
            .unwrap_or(var_name);
        let coerced = self.coerce_assign(Value::Char(label), self.pdv.vars()[result_slot].ty);
        self.pdv.set(result_slot, coerced);
        Ok(Flow::Normal)
    }

    /// CALL VNAME(var, result) — pose dans la variable caractère `result` le
    /// NOM de `var` (tel que stocké au PDV, casse de première référence).
    pub(super) fn call_vname(&mut self, args: &[crate::ast::Expr]) -> Result<Flow> {
        if args.len() != 2 {
            return Err(SasError::runtime(
                "CALL VNAME requires exactly two arguments (variable, result).",
            ));
        }
        let var_slot = self.resolve_lvalue_slot(&args[0])?;
        let result_slot = self.resolve_lvalue_slot(&args[1])?;
        let var_name = self.pdv.vars()[var_slot].name.clone();
        let coerced =
            self.coerce_assign(Value::Char(var_name), self.pdv.vars()[result_slot].ty);
        self.pdv.set(result_slot, coerced);
        Ok(Flow::Normal)
    }
}
