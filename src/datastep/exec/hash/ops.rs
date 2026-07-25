use super::*;

impl Runner {
    /// Insère une entrée dans un hash, en respectant multidata/duplicate.
    /// `from_replace` = appel issu de `replace` (remplace toujours la 1re
    /// entrée existante sans tenir compte de `duplicate`).
    pub(super) fn hash_insert(
        &mut self,
        upper: &str,
        key_vals: Vec<Value>,
        data_vals: Vec<Value>,
        multidata: bool,
        duplicate: &Option<String>,
        from_replace: bool,
    ) -> Result<i64> {
        let key = crate::datastep::hash_key(&key_vals);
        let obj = self.hash_mut(upper);
        if let Some(existing) = obj.rows.get_mut(&key) {
            if from_replace {
                // replace : écrase la 1re entrée (ou ajoute si multidata vide).
                existing[0] = data_vals;
                return Ok(0);
            }
            if multidata {
                existing.push(data_vals);
                return Ok(0);
            }
            match duplicate.as_deref() {
                Some("replace") | Some("r") => {
                    existing[0] = data_vals;
                    Ok(0)
                }
                Some("error") | Some("e") => {
                    // SAS : code retour ≠0 (et NOTE), l'entrée n'est pas ajoutée.
                    Ok(1)
                }
                // Défaut : la 1re valeur est conservée, l'ajout est ignoré.
                _ => Ok(1),
            }
        } else {
            obj.rows.insert(key.clone(), vec![data_vals]);
            obj.key_values.insert(key.clone(), key_vals);
            obj.insertion_order.push(key);
            Ok(0)
        }
    }

    /// `add()` / `add(key:..., data:...)` (M17.2).
    pub(super) fn hash_add(&mut self, upper: &str, args: &[crate::ast::HashArg]) -> Result<i64> {
        let named = self.hash_named_args(args)?;
        let key_vals = self.hash_key_values(upper, &named)?;
        let data_vals = self.hash_data_values(upper, &named)?;
        let (multidata, duplicate) = {
            let o = self.hash(upper);
            (o.multidata, o.duplicate.clone())
        };
        self.hash_insert(upper, key_vals, data_vals, multidata, &duplicate, false)
    }

    /// `replace()` (M17.2) — remplace les données de la clé courante (ou ajoute).
    pub(super) fn hash_replace(&mut self, upper: &str, args: &[crate::ast::HashArg]) -> Result<i64> {
        let named = self.hash_named_args(args)?;
        let key_vals = self.hash_key_values(upper, &named)?;
        let data_vals = self.hash_data_values(upper, &named)?;
        let key = crate::datastep::hash_key(&key_vals);
        let multidata = self.hash(upper).multidata;
        if self.hash(upper).rows.contains_key(&key) {
            self.hash_insert(upper, key_vals, data_vals, multidata, &None, true)
        } else {
            // Pas d'entrée : replace se comporte comme add.
            self.hash_insert(upper, key_vals, data_vals, multidata, &None, false)
        }
    }

    /// `find()` (copie les données dans le PDV) ou `check()` (ne copie pas).
    /// Code retour 0 si la clé existe, ≠0 sinon. Avec multidata, positionne le
    /// curseur sur la 1re entrée (pour find_next).
    pub(super) fn hash_find(
        &mut self,
        upper: &str,
        args: &[crate::ast::HashArg],
        copy_data: bool,
    ) -> Result<i64> {
        let named = self.hash_named_args(args)?;
        let key_vals = self.hash_key_values(upper, &named)?;
        let key = crate::datastep::hash_key(&key_vals);
        let found = {
            let o = self.hash(upper);
            o.rows.get(&key).map(|entries| entries[0].clone())
        };
        match found {
            Some(data_vals) => {
                // Positionne le curseur multidata sur la 1re entrée.
                self.ctx
                    .hashes
                    .get_mut(upper)
                    .expect("checked")
                    .set_find_cursor(&key, 0);
                if copy_data {
                    self.hash_copy_data(upper, &data_vals)?;
                }
                Ok(0)
            }
            None => Ok(1),
        }
    }

    /// `find_next()` / `find_prev()` (M17.2, multidata) : avance le curseur de
    /// l'entrée multidata courante de `step` (+1 / −1) et copie ses données.
    pub(super) fn hash_find_next(&mut self, upper: &str, step: i64) -> Result<i64> {
        let next = {
            let o = self.hash(upper);
            let Some((key, idx)) = o.find_cursor.clone() else {
                return Ok(1);
            };
            let entries = o.rows.get(&key);
            match entries {
                Some(list) => {
                    let new_idx = idx as i64 + step;
                    if new_idx < 0 || new_idx as usize >= list.len() {
                        None
                    } else {
                        Some((key, new_idx as usize, list[new_idx as usize].clone()))
                    }
                }
                None => None,
            }
        };
        match next {
            Some((key, idx, data_vals)) => {
                self.ctx
                    .hashes
                    .get_mut(upper)
                    .expect("checked")
                    .set_find_cursor(&key, idx);
                self.hash_copy_data(upper, &data_vals)?;
                Ok(0)
            }
            None => Ok(1),
        }
    }

    /// `remove()` (M17.2) — supprime l'entrée de la clé courante.
    pub(super) fn hash_remove(&mut self, upper: &str, args: &[crate::ast::HashArg]) -> Result<i64> {
        let named = self.hash_named_args(args)?;
        let key_vals = self.hash_key_values(upper, &named)?;
        let key = crate::datastep::hash_key(&key_vals);
        let obj = self.hash_mut(upper);
        if obj.rows.remove(&key).is_some() {
            obj.insertion_order.retain(|k| k != &key);
            Ok(0)
        } else {
            Ok(1)
        }
    }

    /// Copie une liste de valeurs de données dans les slots PDV des variables
    /// données du hash (avec coercition de type au besoin).
    pub(super) fn hash_copy_data(&mut self, upper: &str, data_vals: &[Value]) -> Result<()> {
        let data_vars = self.hash(upper).data_vars.clone();
        for (d, val) in data_vars.iter().zip(data_vals) {
            if let Some(slot) = self.pdv.slot(d) {
                let coerced = self.coerce_assign(val.clone(), self.pdv.vars()[slot].ty);
                self.pdv.set(slot, coerced);
            }
        }
        Ok(())
    }

    /// `output(dataset:'lib.tab')` (M17.2) — met en file une sortie du hash
    /// (clés puis données, dans l'ordre de visite). L'écriture effective est
    /// faite après l'étape par `flush_hash_outputs`.
    pub(super) fn hash_output(&mut self, upper: &str, args: &[crate::ast::HashArg]) -> Result<i64> {
        let named = self.hash_named_args(args)?;
        let dsname = named
            .get("dataset")
            .map(|v| match v {
                Value::Char(s) => s.trim().to_string(),
                _ => String::new(),
            })
            .ok_or_else(|| {
                SasError::runtime(format!("{upper}.output requires a dataset: argument."))
            })?;
        let (libref, table) = match dsname.split_once('.') {
            Some((l, t)) => (l.to_uppercase(), t.to_string()),
            None => ("WORK".to_string(), dsname.clone()),
        };
        let display = format!("{libref}.{}", table.to_uppercase());
        // Métadonnées des colonnes : clés puis données, depuis le PDV.
        let obj = self.hash(upper);
        let col_names: Vec<String> = obj.keys.iter().chain(&obj.data_vars).cloned().collect();
        let order = self.hash_visit_order(upper);
        let mut vars = Vec::with_capacity(col_names.len());
        for name in &col_names {
            let slot = self.pdv.slot(name).ok_or_else(|| {
                SasError::runtime(format!("Hash variable {name} is not in the PDV for output."))
            })?;
            let v = &self.pdv.vars()[slot];
            vars.push(VarMeta {
                name: v.name.clone(),
                ty: v.ty,
                length: v.length,
                format: v.format.clone(),
                label: self.labels.get(&v.name.to_uppercase()).cloned(),
            });
        }
        // Lignes : pour chaque clé (ordre de visite), chaque entrée de données.
        let obj = self.hash(upper);
        let n_keys = obj.keys.len();
        let mut rows: Vec<Vec<Value>> = Vec::new();
        for key in &order {
            // Décodage de la clé encodée n'est pas possible ; on reconstitue
            // les valeurs de clé à partir de l'ordre de visite via la 1re
            // entrée — mais la clé encodée perd le type. On stocke donc les
            // valeurs de clé séparément.
            if let Some(entries) = obj.rows.get(key) {
                for data_vals in entries {
                    let mut row = Vec::with_capacity(n_keys + data_vals.len());
                    // Les valeurs de clé sont conservées via `key_values`.
                    if let Some(kv) = obj.key_values.get(key) {
                        row.extend(kv.iter().cloned());
                    } else {
                        row.extend(std::iter::repeat_n(Value::missing(), n_keys));
                    }
                    row.extend(data_vals.iter().cloned());
                    rows.push(row);
                }
            }
        }
        self.ctx.hash_outputs.push(crate::datastep::eval::HashOutput {
            libref,
            table,
            display,
            vars,
            rows,
        });
        Ok(0)
    }

    /// Ordre de visite des clés d'un hash (M17.2) : tri par clés via `sas_cmp`
    /// si `ordered:` (yes/ascending → croissant, descending → décroissant),
    /// sinon ordre d'insertion.
    pub(super) fn hash_visit_order(&self, upper: &str) -> Vec<String> {
        let obj = self.hash(upper);
        let mut order = obj.insertion_order.clone();
        if let Some(ord) = &obj.ordered {
            let descending = ord == "descending" || ord == "d";
            let active = matches!(
                ord.as_str(),
                "yes" | "y" | "a" | "ascending" | "descending" | "d"
            );
            if active {
                order.sort_by(|a, b| {
                    let ka = obj.key_values.get(a);
                    let kb = obj.key_values.get(b);
                    let mut c = match (ka, kb) {
                        (Some(va), Some(vb)) => va
                            .iter()
                            .zip(vb)
                            .map(|(x, y)| x.sas_cmp(y))
                            .find(|o| *o != Ordering::Equal)
                            .unwrap_or(Ordering::Equal),
                        _ => Ordering::Equal,
                    };
                    if descending {
                        c = c.reverse();
                    }
                    c
                });
            }
        }
        order
    }

    /// Méthode d'itérateur de hash (M17.2) : `first`/`next`/`last`/`prev`.
    /// Positionne le curseur de l'itérateur dans l'ordre de visite de l'objet
    /// hash lié, puis copie la clé+les données de l'entrée courante dans le
    /// PDV. Code retour 0 si une entrée existe à la nouvelle position, ≠0 si
    /// l'itérateur sort des bornes (le PDV n'est alors pas modifié).
    pub(super) fn exec_hiter_method(&mut self, iter_upper: &str, method: &str) -> Result<i64> {
        let m = method.to_ascii_lowercase();
        let hash = self
            .ctx
            .hash_iters
            .get(iter_upper)
            .expect("checked")
            .hash
            .clone();
        // Ordre de visite APLATI (clé, index d'entrée) — multidata développé.
        let order = self.hash_visit_order(&hash);
        let mut flat: Vec<(String, usize)> = Vec::new();
        {
            let obj = self.hash(&hash);
            for key in &order {
                if let Some(entries) = obj.rows.get(key) {
                    for i in 0..entries.len() {
                        flat.push((key.clone(), i));
                    }
                }
            }
        }
        if flat.is_empty() {
            self.ctx.hash_iters.get_mut(iter_upper).expect("checked").pos = None;
            return Ok(1);
        }
        let cur = self.ctx.hash_iters.get(iter_upper).expect("checked").pos;
        let new_pos: Option<usize> = match m.as_str() {
            "first" => Some(0),
            "last" => Some(flat.len() - 1),
            "next" => match cur {
                Some(p) if p + 1 < flat.len() => Some(p + 1),
                Some(_) => None,
                None => Some(0),
            },
            "prev" => match cur {
                Some(p) if p > 0 => Some(p - 1),
                Some(_) => None,
                None => Some(flat.len() - 1),
            },
            other => {
                return Err(SasError::runtime(format!(
                    "Hash iterator method {}.{} is not supported.",
                    iter_upper,
                    other.to_uppercase()
                )));
            }
        };
        match new_pos {
            Some(p) => {
                self.ctx.hash_iters.get_mut(iter_upper).expect("checked").pos = Some(p);
                let (key, idx) = flat[p].clone();
                let (key_vals, data_vals) = {
                    let obj = self.hash(&hash);
                    (
                        obj.key_values.get(&key).cloned().unwrap_or_default(),
                        obj.rows.get(&key).map(|e| e[idx].clone()).unwrap_or_default(),
                    )
                };
                // Copie des clés puis des données dans le PDV.
                let keys = self.hash(&hash).keys.clone();
                for (k, v) in keys.iter().zip(&key_vals) {
                    if let Some(slot) = self.pdv.slot(k) {
                        let coerced = self.coerce_assign(v.clone(), self.pdv.vars()[slot].ty);
                        self.pdv.set(slot, coerced);
                    }
                }
                self.hash_copy_data(&hash, &data_vals)?;
                Ok(0)
            }
            None => Ok(1),
        }
    }
}
