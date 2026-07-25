use super::*;

impl Runner {
    pub(super) fn exec_stmt(&mut self, stmt: &DsStmt) -> Result<Flow> {
        match stmt {
            DsStmt::Set { .. } => {
                let Some(input) = &self.input else {
                    // Impossible après compile() ; garde-fou.
                    return Err(SasError::runtime("SET statement without input data."));
                };
                if input.point_slot.is_some() {
                    self.exec_set_point()
                } else if input.by.is_empty() {
                    self.exec_set_concat()
                } else {
                    self.exec_set_interleave()
                }
            }
            DsStmt::Merge(_) => self.exec_merge(),
            // UPDATE (M16.5) : marqueur. La ligne maître est chargée par la
            // boucle externe (execute_update) AVANT le corps ; ici no-op.
            DsStmt::Update { .. } => Ok(Flow::Normal),
            // MODIFY (M16.5) : en lecture séquentielle, marqueur no-op (la
            // boucle externe charge/capture). En MODIFY+POINT= (modify_state
            // présent), le marqueur capture la ligne précédente puis charge
            // l'obs à l'index POINT= courant.
            DsStmt::Modify { .. } => {
                if self.modify_state.is_some() {
                    self.exec_modify_point()
                } else {
                    Ok(Flow::Normal)
                }
            }
            DsStmt::Assign { var, expr } => {
                let value = self.eval_checked(expr)?;
                // `arr = e;` sous un `DO OVER arr` : la cible est l'élément
                // courant (slot dans `ctx.do_over`), pas une variable du PDV.
                let slot = if let Some(s) = self.ctx.do_over.get(&var.to_uppercase()) {
                    *s
                } else if let Some(s) = self.pdv.slot(var) {
                    s
                } else {
                    return Err(SasError::runtime(format!(
                        "Variable {var} is not addressable."
                    )));
                };
                let coerced = self.coerce_assign(value, self.pdv.vars()[slot].ty);
                self.pdv.set(slot, coerced);
                Ok(Flow::Normal)
            }
            DsStmt::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let c = self.eval_checked(cond)?;
                if c.truthy() {
                    self.exec_stmt(then_branch)
                } else if let Some(e) = else_branch {
                    self.exec_stmt(e)
                } else {
                    Ok(Flow::Normal)
                }
            }
            DsStmt::SubsettingIf(cond) => {
                let c = self.eval_checked(cond)?;
                if c.truthy() {
                    Ok(Flow::Normal)
                } else {
                    Ok(Flow::NextIter)
                }
            }
            DsStmt::Block(stmts) => {
                for s in stmts {
                    let f = self.exec_stmt(s)?;
                    if f != Flow::Normal {
                        return Ok(f);
                    }
                }
                Ok(Flow::Normal)
            }
            DsStmt::DoLoop {
                index,
                to,
                by,
                while_,
                until,
                body,
            } => self.exec_do_loop(
                index.as_ref(),
                to.as_ref(),
                by.as_ref(),
                while_.as_ref(),
                until.as_ref(),
                body,
            ),
            DsStmt::DoList { index, items, body } => self.exec_do_list(index, items, body),
            DsStmt::DoOver { array, body } => self.exec_do_over(array, body),
            DsStmt::Select {
                selector,
                whens,
                otherwise,
            } => self.exec_select(selector.as_ref(), whens, otherwise.as_deref()),
            DsStmt::Delete => Ok(Flow::NextIter),
            DsStmt::Output(targets) => {
                if targets.is_empty() {
                    // `output;` : toutes les sorties.
                    self.push_outputs();
                } else {
                    // OUTPUT ciblé : uniquement les sorties nommées
                    // (résolues par display "WORK.A" — la compilation a
                    // validé qu'elles existent).
                    for t in targets {
                        let disp = t.display();
                        let Some(o) =
                            self.outputs.iter().position(|s| s.display == disp)
                        else {
                            // Impossible après compile() ; garde-fou.
                            return Err(SasError::runtime(format!(
                                "Output dataset {disp} is not in the DATA statement output list."
                            )));
                        };
                        self.push_one(o);
                    }
                }
                Ok(Flow::Normal)
            }
            DsStmt::Stop => Ok(Flow::EndStep),
            DsStmt::Sum { var, expr } => {
                // Sum statement `var + expr;` — sémantique SUM de SAS : les
                // missings sont IGNORÉS, jamais propagés. Un incrément
                // missing ajoute 0 (sans `missing_generated`), et un
                // accumulateur missing (l'utilisateur a pu assigner `.`)
                // est traité comme 0 : `total=.; total+x;` donne x.
                let value = self.eval_checked(expr)?;
                let incr = self.coerce_sum_operand(value);
                let Some(slot) = self.pdv.slot(var) else {
                    return Err(SasError::runtime(format!(
                        "Variable {var} is not addressable."
                    )));
                };
                let acc = match self.pdv.get(slot) {
                    Value::Num(f) => *f,
                    // Missing (ou char dégénéré) : repart de 0.
                    _ => 0.0,
                };
                self.pdv.set(slot, Value::Num(acc + incr));
                Ok(Flow::Normal)
            }
            DsStmt::AssignIndexed {
                array,
                indices,
                expr,
            } => {
                // Indices évalués avec les MÊMES règles que les rvalues
                // (coercition num + arrondi ; missing/hors bornes → l'étape
                // s'arrête), puis coercition vers le type de l'élément.
                let mut idx_vals = Vec::with_capacity(indices.len());
                for index in indices {
                    idx_vals.push(self.eval_checked(index)?);
                }
                let slot = self.resolve_subscript(array, &idx_vals)?;
                let value = self.eval_checked(expr)?;
                let coerced = self.coerce_assign(value, self.pdv.vars()[slot].ty);
                self.pdv.set(slot, coerced);
                Ok(Flow::Normal)
            }
            DsStmt::CallRoutine { name, args } => self.exec_call_routine(name, args),
            // Étiquette (M16.6) : l'étiquette elle-même est un marqueur
            // (résolue par index dans le pilote de niveau supérieur) ; on
            // exécute simplement le statement étiqueté.
            DsStmt::Labeled { stmt, .. } => self.exec_stmt(stmt),
            // GOTO/LINK/RETURN (M16.6) : remontent comme Flow non-Normal
            // jusqu'au pilote de niveau supérieur (`run_step_body`), qui pilote
            // le compteur de programme et la pile de retour. Traversent les
            // boucles DO englobantes (mêmes règles de propagation que EndStep).
            DsStmt::Goto(label) => Ok(Flow::Goto(label.to_uppercase())),
            // LINK : exécute la sous-routine INLINE (du label au prochain
            // RETURN) puis reprend après le LINK (Flow::Normal). Exécuté ICI —
            // et non remonté — pour qu'un LINK à l'intérieur d'une boucle DO
            // n'abandonne PAS la boucle (la pile d'appels Rust = pile de
            // retour). Un Flow non-Normal de la sous-routine (GOTO non local,
            // DELETE, STOP, fin d'entrée) est propagé tel quel.
            DsStmt::Link(label) => self.exec_link_subroutine(&label.to_uppercase()),
            DsStmt::Return => Ok(Flow::Return),
            // DECLARE HASH (M17.1) : l'objet et ses options sont déjà résolus à
            // la compilation (dans `EvalCtx.hashes`). Le statement DECLARE est un
            // marqueur déclaratif ; aucune action runtime (l'objet existe pour
            // toute l'étape, comme les objets hash SAS au sein d'une étape DATA).
            DsStmt::DeclareHash { .. } => Ok(Flow::Normal),
            // DECLARE HITER (M17.2) : itérateur enregistré à la compilation
            // (dans `EvalCtx.hash_iters`). Marqueur déclaratif, aucune action.
            DsStmt::DeclareHiter { .. } => Ok(Flow::Normal),
            // Appel de méthode d'objet hash en STATEMENT (M17.1/M17.2) : le code
            // retour est ignoré.
            DsStmt::HashMethod(call) => {
                self.exec_hash_method_rc(&call.object, &call.method, &call.args)?;
                Ok(Flow::Normal)
            }
            // INPUT (M14) : lit le PROCHAIN enregistrement de la source texte
            // dans le PDV. Comme SET, l'épuisement de la source termine
            // l'étape IMMÉDIATEMENT (au milieu de l'itération).
            DsStmt::Input(items) => self.exec_input(items),
            // FILE (M14.2) : change la destination courante des PUT.
            DsStmt::File { dest } => self.exec_file(dest),
            // PUT (M14.2) : rend les items dans la ligne de sortie courante.
            DsStmt::Put(items) => self.exec_put(items),
            // Directives de compilation / déclaratives : rien à exécuter.
            DsStmt::Keep(_)
            | DsStmt::Drop(_)
            | DsStmt::Retain(_)
            | DsStmt::Length(_)
            | DsStmt::By(_)
            | DsStmt::Format(_)
            | DsStmt::Label(_)
            | DsStmt::Attrib(_)
            | DsStmt::Infile { .. }
            | DsStmt::Datalines(_)
            | DsStmt::Array { .. } => Ok(Flow::Normal),
        }
    }

    /// Résout un argument qui DOIT être une variable scalaire ou un élément
    /// d'array indexé (`var` ou `arr{i}`) en son slot PDV. Utilisé par les
    /// CALL routines qui écrivent dans leurs arguments (MISSING, CATS, SCAN,
    /// LABEL, VNAME). Une expression qui n'est pas une lvalue → erreur.
    pub(super) fn resolve_lvalue_slot(&mut self, arg: &crate::ast::Expr) -> Result<usize> {
        use crate::ast::Expr;
        match arg {
            Expr::Var(name) => self.pdv.slot(name).ok_or_else(|| {
                SasError::runtime(format!("Variable {name} is not addressable."))
            }),
            Expr::Index { name, indices } => {
                let mut idx_vals = Vec::with_capacity(indices.len());
                for index in indices {
                    idx_vals.push(self.eval_checked(index)?);
                }
                self.resolve_subscript(name, &idx_vals)
            }
            // `arr(i)` / `arr(i,j)` se parse en Call ; si le nom est un array,
            // c'est une référence d'élément.
            Expr::Call { name, args } if !args.is_empty()
                && self.ctx.arrays.contains_key(&name.to_uppercase()) =>
            {
                let mut idx_vals = Vec::with_capacity(args.len());
                for a in args {
                    idx_vals.push(self.eval_checked(a)?);
                }
                self.resolve_subscript(name, &idx_vals)
            }
            _ => Err(SasError::runtime(
                "CALL routine argument must be a variable reference.",
            )),
        }
    }

    /// Résout un sous-script d'array (un ou plusieurs indices) en slot PDV :
    /// coercition numérique (mêmes règles que `eval::coerce_num`), arrondi au
    /// plus proche ; missing, hors bornes ou nombre d'indices invalide →
    /// erreur qui stoppe l'étape. Un index unique sur un array multi-dim est
    /// interprété linéairement (row-major).
    pub(super) fn resolve_subscript(&mut self, array: &str, idx_vals: &[Value]) -> Result<usize> {
        let mut idxs: Vec<i64> = Vec::with_capacity(idx_vals.len());
        for idx_val in idx_vals {
            let idx = coerce_num(idx_val, &mut self.ctx).map(f64::round);
            if self.ctx.error_flag {
                self.pdv.error_ = true;
                self.ctx.error_flag = false;
            }
            match idx {
                Some(i) => idxs.push(i as i64),
                None => return Err(SasError::runtime("Array subscript out of range.")),
            }
        }
        let Some(def) = self.ctx.arrays.get(&array.to_uppercase()) else {
            // Impossible après compile() ; garde-fou.
            return Err(SasError::runtime(format!(
                "Undeclared array referenced: {array}."
            )));
        };
        match def.linear_index(&idxs) {
            Some(lin) => Ok(def.slots[lin]),
            None => Err(SasError::runtime("Array subscript out of range.")),
        }
    }

    /// Valeur courante de l'index pour le test TO. Un index rendu missing
    /// par le corps se classe SOUS tous les nombres (ordre SAS) :
    /// -inf fait sortir avec by<0 et continuer avec by>0.
    pub(super) fn index_value(&self, slot: usize) -> f64 {
        match self.pdv.get(slot) {
            Value::Num(f) => *f,
            Value::Missing(_) => f64::NEG_INFINITY,
            // Impossible : l'index est créé Num par la compilation.
            Value::Char(_) => 0.0,
        }
    }

    /// Coercition numérique d'un opérande de sum statement. Mêmes règles de
    /// conversion char→num que l'évaluateur (note + invalid data + _ERROR_
    /// sur une chaîne invalide), MAIS un résultat missing contribue 0 sans
    /// incrémenter `missing_generated` (le SUM ignore les missings).
    pub(super) fn coerce_sum_operand(&mut self, value: Value) -> f64 {
        match value {
            Value::Num(f) => f,
            Value::Missing(_) => 0.0,
            Value::Char(s) => {
                self.ctx.note_char_to_num = true;
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    0.0
                } else {
                    match trimmed.parse::<f64>() {
                        Ok(f) => f,
                        Err(_) => {
                            self.ctx.invalid_data += 1;
                            self.pdv.error_ = true;
                            0.0
                        }
                    }
                }
            }
        }
    }

    pub(super) fn eval_checked(&mut self, expr: &crate::ast::Expr) -> Result<Value> {
        // Méthode d'objet hash en expression (M17.2) : `rc = h.find();`.
        // Interceptée ICI (et non dans l'évaluateur immuable) car la méthode
        // mute le PDV et les objets hash. Renvoie le code retour numérique.
        if let crate::ast::Expr::HashMethod(call) = expr {
            let rc = self.exec_hash_method_rc(&call.object, &call.method, &call.args)?;
            return Ok(Value::Num(rc as f64));
        }
        let v = eval(expr, &self.pdv, &mut self.ctx);
        if let Some(err) = self.ctx.fatal.take() {
            // Les fatals de l'évaluateur sont déjà des `SasError` typés, sans
            // préfixe "ERROR: " (ajouté par `log.error` à l'affichage).
            return Err(err);
        }
        if self.ctx.error_flag {
            self.pdv.error_ = true;
            self.ctx.error_flag = false;
        }
        Ok(v)
    }

    /// Coercition à l'assignation : expression d'un type vers une variable
    /// de l'autre type (mêmes règles que dans les expressions).
    pub(super) fn coerce_assign(&mut self, value: Value, target: VarType) -> Value {
        match (value, target) {
            (v @ (Value::Num(_) | Value::Missing(_)), VarType::Num) => v,
            (v @ Value::Char(_), VarType::Char) => v,
            (Value::Char(s), VarType::Num) => {
                self.ctx.note_char_to_num = true;
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    self.ctx.missing_generated += 1;
                    Value::missing()
                } else {
                    match trimmed.parse::<f64>() {
                        Ok(f) => Value::Num(f),
                        Err(_) => {
                            self.ctx.invalid_data += 1;
                            self.pdv.error_ = true;
                            Value::missing()
                        }
                    }
                }
            }
            (Value::Num(f), VarType::Char) => {
                self.ctx.note_num_to_char = true;
                Value::Char(format!("{:>12}", format_best(f, 12)))
            }
            (Value::Missing(k), VarType::Char) => {
                self.ctx.note_num_to_char = true;
                Value::Char(format!("{:>12}", k.display()))
            }
        }
    }
}
