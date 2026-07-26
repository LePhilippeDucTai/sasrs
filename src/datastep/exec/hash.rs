//! Objets hash et itérateurs hiter (méthodes de `Runner`).

use super::*;

mod ops;

impl Runner {
    /// Appel de méthode d'objet hash (M17.1/M17.2). Renvoie le CODE RETOUR
    /// (0 = succès, ≠0 = échec) — utilisé par la forme expression
    /// (`rc = h.find();`) et ignoré par la forme statement (`h.find();`).
    /// Les méthodes `find`/replace copient les données dans le PDV, `clear`/
    /// `remove`/`add` mutent l'objet, `output` met une sortie en file.
    pub(super) fn exec_hash_method_rc(
        &mut self,
        object: &str,
        method: &str,
        args: &[crate::ast::HashArg],
    ) -> Result<i64> {
        let upper = object.to_uppercase();
        // Itérateur de hash (DECLARE HITER) : route vers les méthodes
        // first/next/last/prev avant le dispatch des méthodes d'objet hash.
        if self.ctx.hash_iters.contains_key(&upper) {
            return self.exec_hiter_method(&upper, method);
        }
        if !self.ctx.hashes.contains_key(&upper) {
            return Err(SasError::runtime(format!(
                "Hash object {upper} has not been declared."
            )));
        }
        let m = method.to_ascii_lowercase();
        match m.as_str() {
            "definekey" | "definedata" => {
                let mut names = Vec::with_capacity(args.len());
                for a in args {
                    let crate::ast::HashArg::Positional(crate::ast::Expr::Str(varname)) = a else {
                        return Err(SasError::runtime(format!(
                            "Argument of {upper}.{method} must be a quoted variable name."
                        )));
                    };
                    names.push(varname.to_uppercase());
                }
                let obj = self.hash_mut(&upper);
                if m == "definekey" {
                    obj.keys = names;
                } else {
                    obj.data_vars = names;
                }
                Ok(0)
            }
            "definedone" => self.hash_define_done(&upper),
            "add" => self.hash_add(&upper, args),
            "replace" => self.hash_replace(&upper, args),
            "find" => self.hash_find(&upper, args, true),
            "check" => self.hash_find(&upper, args, false),
            "remove" => self.hash_remove(&upper, args),
            "clear" => {
                let obj = self.hash_mut(&upper);
                obj.rows.clear();
                obj.insertion_order.clear();
                Ok(0)
            }
            "find_next" => self.hash_find_next(&upper, 1),
            "find_prev" => self.hash_find_next(&upper, -1),
            "num_items" | "item_size" => {
                let obj = self.hash(&upper);
                let n: usize = obj.rows.values().map(|v| v.len()).sum();
                Ok(n as i64)
            }
            "output" => self.hash_output(&upper, args),
            other => Err(SasError::runtime(format!(
                "Hash method {}.{} is not yet implemented.",
                upper,
                other.to_uppercase()
            ))),
        }
    }

    /// `defineDone()` (M17.2) : finalise l'objet (idempotent) et, si une option
    /// `dataset:` était posée, charge ses lignes pré-lues dans le hash.
    fn hash_define_done(&mut self, upper: &str) -> Result<i64> {
        let obj = self.hash(upper);
        if obj.defined {
            return Ok(0);
        }
        let load = obj.dataset_cols.clone();
        let nrows = obj.dataset_nrows;
        let keys = obj.keys.clone();
        let data_vars = obj.data_vars.clone();
        let multidata = obj.multidata;
        let duplicate = obj.duplicate.clone();
        if let Some(cols) = load {
            for row in 0..nrows {
                let key_vals: Vec<Value> = keys
                    .iter()
                    .map(|k| cols.get(k).map_or(Value::missing(), |c| c[row].clone()))
                    .collect();
                let data_vals: Vec<Value> = data_vars
                    .iter()
                    .map(|d| cols.get(d).map_or(Value::missing(), |c| c[row].clone()))
                    .collect();
                self.hash_insert(upper, key_vals, data_vals, multidata, &duplicate, false)?;
            }
        }
        let obj = self.hash_mut(upper);
        obj.defined = true;
        Ok(0)
    }

    /// Récupère les valeurs des variables clé d'un hash depuis le PDV (ou
    /// depuis l'argument nommé `key:` s'il est fourni).
    fn hash_key_values(
        &mut self,
        upper: &str,
        named: &std::collections::HashMap<String, Value>,
    ) -> Result<Vec<Value>> {
        let keys = self.hash(upper).keys.clone();
        let mut vals = Vec::with_capacity(keys.len());
        for k in &keys {
            // `key:` nommé n'est supporté que pour une clé unique (cas courant).
            if let (1, Some(v)) = (keys.len(), named.get("key")) {
                vals.push(v.clone());
                continue;
            }
            let slot = self.pdv.slot(k).ok_or_else(|| {
                SasError::runtime(format!("Hash key variable {k} is not addressable."))
            })?;
            vals.push(self.pdv.get(slot).clone());
        }
        Ok(vals)
    }

    /// Récupère les valeurs des variables données d'un hash depuis le PDV (ou
    /// depuis l'argument nommé `data:` pour une donnée unique).
    fn hash_data_values(
        &mut self,
        upper: &str,
        named: &std::collections::HashMap<String, Value>,
    ) -> Result<Vec<Value>> {
        let data_vars = self.hash(upper).data_vars.clone();
        let mut vals = Vec::with_capacity(data_vars.len());
        for d in &data_vars {
            if let (1, Some(v)) = (data_vars.len(), named.get("data")) {
                vals.push(v.clone());
                continue;
            }
            let slot = self.pdv.slot(d).ok_or_else(|| {
                SasError::runtime(format!("Hash data variable {d} is not addressable."))
            })?;
            vals.push(self.pdv.get(slot).clone());
        }
        Ok(vals)
    }

    /// Évalue les arguments nommés (`key:`, `data:`, `dataset:`) d'un appel.
    fn hash_named_args(
        &mut self,
        args: &[crate::ast::HashArg],
    ) -> Result<std::collections::HashMap<String, Value>> {
        use crate::ast::HashArg;
        let mut map = std::collections::HashMap::new();
        for a in args {
            if let HashArg::Named(name, expr) = a {
                let v = self.eval_checked(expr)?;
                map.insert(name.clone(), v);
            }
        }
        Ok(map)
    }
}
