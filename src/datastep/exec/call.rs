use super::*;

impl Runner {
    /// Exécute une CALL routine. Routines supportées (M11.5 + M15.6 + M40.1) :
    /// STREAMINIT, SYMPUT, SYMPUTX, MISSING, EXECUTE, SORTN, SORTC, CATS,
    /// SCAN, LABEL, VNAME, PRXCHANGE, PRXSUBSTR, PRXNEXT, PRXPOSN, PRXFREE,
    /// PRXDEBUG. Toute autre → erreur « not yet implemented ».
    ///
    /// Les routines qui ÉCRIVENT dans un argument (MISSING, SORTN/SORTC, CATS,
    /// SCAN, LABEL, VNAME) résolvent cet argument en lvalue (variable ou
    /// élément d'array) via `resolve_lvalue_slot`. SYMPUT/SYMPUTX diffèrent
    /// l'écriture macro à la fin de l'étape (règle de visibilité SAS) ;
    /// EXECUTE met du code en file pour exécution post-étape.
    pub(super) fn exec_call_routine(
        &mut self,
        name: &str,
        args: &[crate::ast::Expr],
    ) -> Result<Flow> {
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
            "PRXCHANGE" => self.call_prxchange(args),
            "PRXSUBSTR" => self.call_prxsubstr(args),
            "PRXNEXT" => self.call_prxnext(args),
            "PRXPOSN" => self.call_prxposn(args),
            "PRXFREE" => self.call_prxfree(args),
            "PRXDEBUG" => self.call_prxdebug(args),
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
        let coerced = self.coerce_assign(Value::Char(var_name), self.pdv.vars()[result_slot].ty);
        self.pdv.set(result_slot, coerced);
        Ok(Flow::Normal)
    }

    // ── Routines PRX (M40.1) ─────────────────────────────────────────────

    /// Évalue un argument `regex-id` de routine PRX et le valide dans la
    /// table des patterns. Id manquant, non numérique, inconnu ou libéré →
    /// ERROR qui stoppe l'étape (comme SAS).
    fn prx_id_arg(&mut self, arg: &crate::ast::Expr, routine: &str) -> Result<usize> {
        let v = self.eval_checked(arg)?;
        let f = coerce_num(&v, &mut self.ctx).ok_or_else(|| {
            SasError::runtime(format!(
                "Invalid regular expression id passed to the CALL routine {routine}."
            ))
        })?;
        self.ctx.prx.check_id(f).ok_or_else(|| {
            SasError::runtime(format!(
                "Invalid regular expression id passed to the CALL routine {routine}."
            ))
        })
    }

    /// Évalue un argument en chaîne (conversion num→char BEST12. standard).
    fn prx_char_arg(&mut self, arg: &crate::ast::Expr) -> Result<String> {
        let v = self.eval_checked(arg)?;
        match self.coerce_assign(v, VarType::Char) {
            Value::Char(s) => Ok(s),
            _ => Ok(String::new()),
        }
    }

    /// Écrit une valeur numérique dans une lvalue (coercition standard).
    fn prx_write_num(&mut self, arg: &crate::ast::Expr, value: f64) -> Result<()> {
        let slot = self.resolve_lvalue_slot(arg)?;
        let coerced = self.coerce_assign(Value::Num(value), self.pdv.vars()[slot].ty);
        self.pdv.set(slot, coerced);
        Ok(())
    }

    /// CALL PRXCHANGE(id, times, source <, result <, res-length <,
    /// trunc-value <, n-changes>>>) — applique la substitution `s/…/…/` du
    /// pattern `id`, `times` fois (-1 = toutes). Sans `result`, `source` est
    /// modifié EN PLACE (il doit alors être une lvalue). `res-length` reçoit
    /// la longueur du résultat stocké (sans blancs finaux), `trunc-value`
    /// 1/0 selon que le résultat a été tronqué à la longueur de la variable,
    /// `n-changes` le nombre de remplacements.
    pub(super) fn call_prxchange(&mut self, args: &[crate::ast::Expr]) -> Result<Flow> {
        if !(3..=7).contains(&args.len()) {
            return Err(SasError::runtime(
                "CALL PRXCHANGE requires three to seven arguments (id, times, source, result, ...).",
            ));
        }
        let id = self.prx_id_arg(&args[0], "PRXCHANGE")?;
        let times_v = self.eval_checked(&args[1])?;
        let times = match coerce_num(&times_v, &mut self.ctx) {
            Some(f) if f < 0.0 => -1i64,
            Some(f) => f as i64,
            None => {
                return Err(SasError::runtime(
                    "CALL PRXCHANGE requires a numeric times argument.",
                ));
            }
        };
        let source = self.prx_char_arg(&args[2])?;
        let pat = self.ctx.prx.get_mut(id).expect("id validated");
        if !crate::datastep::functions::prx::is_substitution(pat) {
            return Err(SasError::runtime(
                "The regular expression passed to CALL PRXCHANGE is not a substitution.",
            ));
        }
        let (out, n_changes) = crate::datastep::functions::prx::change(pat, times, &source);
        // Résultat : 4e argument, ou `source` en place s'il est absent.
        let result_slot = self.resolve_lvalue_slot(args.get(3).unwrap_or(&args[2]))?;
        let out_len = out.trim_end().chars().count();
        let truncated = self.pdv.vars()[result_slot].ty == VarType::Char
            && out_len > self.pdv.vars()[result_slot].length;
        let coerced = self.coerce_assign(Value::Char(out), self.pdv.vars()[result_slot].ty);
        self.pdv.set(result_slot, coerced);
        if let Some(len_arg) = args.get(4) {
            // Longueur STOCKÉE (donc après troncature), sans blancs finaux.
            let stored_len = match self.pdv.get(result_slot) {
                Value::Char(s) => s.chars().count(),
                _ => out_len,
            };
            self.prx_write_num(len_arg, stored_len as f64)?;
        }
        if let Some(trunc_arg) = args.get(5) {
            self.prx_write_num(trunc_arg, if truncated { 1.0 } else { 0.0 })?;
        }
        if let Some(n_arg) = args.get(6) {
            self.prx_write_num(n_arg, n_changes as f64)?;
        }
        Ok(Flow::Normal)
    }

    /// CALL PRXSUBSTR(id, source, position <, length>) — position (1-based)
    /// et longueur du premier match dans `source` ; 0/0 si pas de match.
    /// Mémorise les captures (pour PRXPOSN / CALL PRXPOSN).
    pub(super) fn call_prxsubstr(&mut self, args: &[crate::ast::Expr]) -> Result<Flow> {
        if !(3..=4).contains(&args.len()) {
            return Err(SasError::runtime(
                "CALL PRXSUBSTR requires three or four arguments (id, source, position, length).",
            ));
        }
        let id = self.prx_id_arg(&args[0], "PRXSUBSTR")?;
        let source = self.prx_char_arg(&args[1])?;
        let pat = self.ctx.prx.get_mut(id).expect("id validated");
        let found = crate::datastep::functions::prx::find_and_store(pat, &source, 0, None);
        let (pos, len) = found.unwrap_or((0, 0));
        self.prx_write_num(&args[2], pos as f64)?;
        if let Some(len_arg) = args.get(3) {
            self.prx_write_num(len_arg, len as f64)?;
        }
        Ok(Flow::Normal)
    }

    /// CALL PRXNEXT(id, start, stop, source, position, length) — cherche le
    /// prochain match à partir de la position `start` (1-based), sans
    /// dépasser `stop` (missing ou < 1 = fin de chaîne). Après un match,
    /// `start` est avancé à `position + MAX(length, 1)` pour l'appel
    /// suivant ; sinon position/length valent 0 et `start` est inchangé.
    pub(super) fn call_prxnext(&mut self, args: &[crate::ast::Expr]) -> Result<Flow> {
        if args.len() != 6 {
            return Err(SasError::runtime(
                "CALL PRXNEXT requires exactly six arguments (id, start, stop, source, position, length).",
            ));
        }
        let id = self.prx_id_arg(&args[0], "PRXNEXT")?;
        let start_v = self.eval_checked(&args[1])?;
        let start = match coerce_num(&start_v, &mut self.ctx) {
            Some(f) if f >= 1.0 => f as usize,
            _ => 1,
        };
        let stop_v = self.eval_checked(&args[2])?;
        let stop = coerce_num(&stop_v, &mut self.ctx)
            .filter(|f| *f >= 1.0)
            .map(|f| f as usize);
        let source = self.prx_char_arg(&args[3])?;
        let pat = self.ctx.prx.get_mut(id).expect("id validated");
        let found = crate::datastep::functions::prx::find_and_store(pat, &source, start - 1, stop);
        let (pos, len) = found.unwrap_or((0, 0));
        self.prx_write_num(&args[4], pos as f64)?;
        self.prx_write_num(&args[5], len as f64)?;
        if pos > 0 {
            self.prx_write_num(&args[1], (pos + len.max(1)) as f64)?;
        }
        Ok(Flow::Normal)
    }

    /// CALL PRXPOSN(id, n, position <, length>) — position/longueur du
    /// groupe capturé `n` (0 = match entier) du DERNIER match du pattern
    /// (PRXMATCH, CALL PRXSUBSTR/PRXNEXT/PRXCHANGE) ; 0/0 si pas de match
    /// ou groupe non participant.
    pub(super) fn call_prxposn(&mut self, args: &[crate::ast::Expr]) -> Result<Flow> {
        if !(3..=4).contains(&args.len()) {
            return Err(SasError::runtime(
                "CALL PRXPOSN requires three or four arguments (id, n, position, length).",
            ));
        }
        let id = self.prx_id_arg(&args[0], "PRXPOSN")?;
        let n_v = self.eval_checked(&args[1])?;
        let n = match coerce_num(&n_v, &mut self.ctx) {
            Some(f) if f >= 0.0 => f as usize,
            _ => {
                return Err(SasError::runtime(
                    "CALL PRXPOSN requires a numeric capture-buffer number.",
                ));
            }
        };
        let pat = self.ctx.prx.get(id).expect("id validated");
        let (pos, len) =
            crate::datastep::functions::prx::last_match_group(pat, n).unwrap_or((0, 0));
        self.prx_write_num(&args[2], pos as f64)?;
        if let Some(len_arg) = args.get(3) {
            self.prx_write_num(len_arg, len as f64)?;
        }
        Ok(Flow::Normal)
    }

    /// CALL PRXFREE(id) — libère le pattern : l'id devient invalide (tout
    /// usage ultérieur → ERROR) et son texte-source quitte le cache.
    pub(super) fn call_prxfree(&mut self, args: &[crate::ast::Expr]) -> Result<Flow> {
        if args.len() != 1 {
            return Err(SasError::runtime(
                "CALL PRXFREE requires exactly one argument (the regular expression id).",
            ));
        }
        let id = self.prx_id_arg(&args[0], "PRXFREE")?;
        self.ctx.prx.free(id);
        Ok(Flow::Normal)
    }

    /// CALL PRXDEBUG(on-off) — trace de debug interactive de SAS : acceptée
    /// comme no-op silencieux (l'argument est évalué pour ses effets).
    pub(super) fn call_prxdebug(&mut self, args: &[crate::ast::Expr]) -> Result<Flow> {
        for arg in args {
            self.eval_checked(arg)?;
        }
        Ok(Flow::Normal)
    }
}
