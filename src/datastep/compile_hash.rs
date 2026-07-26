use super::*;

impl Compiler<'_> {
    /// Compile un `DECLARE HASH` (bras `DsStmt::DeclareHash` de `walk_stmt`).
    pub(super) fn compile_hash_decl(
        &mut self,
        name: &str,
        options: &[(String, String)],
    ) -> Result<()> {
        let mut obj = HashObject::default();
        for (key, value) in options {
            match key.as_str() {
                "ordered" => obj.ordered = Some(value.trim().to_ascii_lowercase()),
                "duplicate" => obj.duplicate = Some(value.trim().to_ascii_lowercase()),
                "multidata" => {
                    obj.multidata = matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "yes" | "y" | "1"
                    );
                }
                "dataset" | "data" => obj.dataset = Some(value.clone()),
                "hashexp" | "suminc" | "initialgrouptype" => {
                    // Options de réglage/perf : acceptées et ignorées.
                }
                // Option inconnue → erreur claire.
                other => {
                    return Err(SasError::runtime(format!(
                        "Hash object option {} is not supported.",
                        other.to_uppercase()
                    )));
                }
            }
        }
        // dataset: (M17.2) — pré-lit les colonnes à la compilation
        // (`&mut Session` disponible) et entre chaque colonne au PDV
        // (SAS exige que les variables clé/données existent au PDV ;
        // les charger ici les crée comme un SET implicite).
        if let Some(dsname) = obj.dataset.clone() {
            let (cols, nrows) = self.preload_hash_dataset(&dsname)?;
            obj.dataset_cols = Some(cols);
            obj.dataset_nrows = nrows;
        }
        self.hash_objects.insert(name.to_uppercase(), obj);
        Ok(())
    }

    /// Compile un `DECLARE HITER` (bras `DsStmt::DeclareHiter` de `walk_stmt`).
    pub(super) fn compile_hiter_decl(&mut self, name: &str, hash_name: &str) -> Result<()> {
        let hupper = hash_name.to_uppercase();
        if !self.hash_objects.contains_key(&hupper) {
            return Err(SasError::runtime(format!(
                "Hash object {hupper} bound to iterator {} has not been declared.",
                name.to_uppercase()
            )));
        }
        self.hash_iters.insert(
            name.to_uppercase(),
            HashIter {
                hash: hupper,
                pos: None,
            },
        );
        Ok(())
    }

    /// Validation compile-time d'un appel de méthode hash (forme statement OU
    /// expression). Partagée par `DsStmt::HashMethod` et `Expr::HashMethod`.
    pub(super) fn validate_hash_method(
        &mut self,
        object: &str,
        method: &str,
        args: &[crate::ast::HashArg],
    ) -> Result<()> {
        use crate::ast::HashArg;
        let upper = object.to_uppercase();
        // Itérateur de hash (DECLARE HITER) : first/next/last/prev sans arg.
        if self.hash_iters.contains_key(&upper) {
            for a in args {
                match a {
                    HashArg::Positional(e) | HashArg::Named(_, e) => self.walk_expr(e)?,
                }
            }
            return Ok(());
        }
        if !self.hash_objects.contains_key(&upper) {
            return Err(SasError::runtime(format!(
                "Hash object {upper} has not been declared."
            )));
        }
        let m = method.to_ascii_lowercase();
        if m == "definekey" || m == "definedata" {
            for a in args {
                let HashArg::Positional(Expr::Str(varname)) = a else {
                    return Err(SasError::runtime(format!(
                        "Argument of {upper}.{method} must be a quoted variable name."
                    )));
                };
                // La variable clé/donnée doit être au PDV. Si elle n'y est pas
                // encore (déclarée avant son 1er usage textuel), on la crée —
                // fidèle à SAS, qui définit ces variables dans le PDV.
                if self.pdv.slot(varname).is_none() {
                    self.add_var(varname, VarType::Num, 8);
                }
            }
        } else {
            for a in args {
                match a {
                    HashArg::Positional(e) | HashArg::Named(_, e) => self.walk_expr(e)?,
                }
            }
        }
        Ok(())
    }

    /// Pré-lit le dataset `lib.table` d'une option `dataset:` (M17.2) :
    /// décode chaque colonne en `Value` et entre la colonne au PDV (slot créé
    /// comme un SET). Renvoie `(colonnes UPPERCASE → valeurs, n_rows)`.
    pub(super) fn preload_hash_dataset(
        &mut self,
        dsname: &str,
    ) -> Result<(HashMap<String, Vec<Value>>, usize)> {
        let (libref, table) = match dsname.split_once('.') {
            Some((l, t)) => (l.to_uppercase(), t.to_string()),
            None => ("WORK".to_string(), dsname.to_string()),
        };
        let provider = self.session.libs.get(&libref)?;
        if !provider.exists(&table) {
            return Err(SasError::runtime(format!(
                "File {libref}.{} does not exist.",
                table.to_uppercase()
            )));
        }
        let (ds, notes) = provider.read(&table)?;
        for note in &notes {
            self.session.log.forward(note);
        }
        let mut cols: HashMap<String, Vec<Value>> = HashMap::new();
        for (col, meta) in ds.df.get_columns().iter().zip(&ds.vars) {
            // Entrée au PDV (crée le slot si absent ; type cohérent vérifié).
            if let Some(slot) = self.pdv.slot(&meta.name) {
                if self.pdv.vars()[slot].ty != meta.ty {
                    return Err(SasError::runtime(format!(
                        "Variable {} has been defined as both character and numeric.",
                        meta.name
                    )));
                }
            } else {
                let slot = self.pdv.add_var(PdvVar {
                    name: meta.name.clone(),
                    ty: meta.ty,
                    length: meta.length,
                    retained: false,
                    from_input: true,
                    format: meta.format.clone(),
                    temporary: false,
                });
                self.pdv.mark_from_input(slot);
            }
            let s = col.as_materialized_series();
            let values: Vec<Value> = match meta.ty {
                VarType::Num => s.f64()?.iter().map(num_to_value).collect(),
                VarType::Char => s
                    .str()?
                    .iter()
                    .map(|o| Value::Char(o.unwrap_or("").to_string()))
                    .collect(),
            };
            cols.insert(meta.name.to_uppercase(), values);
        }
        Ok((cols, ds.n_obs()))
    }
}
