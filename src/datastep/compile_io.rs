use super::*;

impl Compiler<'_> {
    /// Compile un statement `SET` (bras `DsStmt::Set` de `walk_stmt`).
    pub(super) fn compile_set_stmt(&mut self, specs: &[DatasetSpec], options: &SetOptions) -> Result<()> {
        if self.seen_set {
            return Err(SasError::runtime(
                "Multiple SET statements are not yet implemented.",
            ));
        }
        if self.seen_merge {
            return Err(SasError::runtime(
                "A SET statement is not allowed after a MERGE statement.",
            ));
        }
        self.seen_set = true;
        for spec in specs {
            // `in=` n'est pas valide sur un SET (MERGE seulement).
            if spec.options.in_.is_some() {
                return Err(SasError::runtime(
                    "The IN= data set option is only valid on a MERGE statement.",
                ));
            }
            self.compile_set(spec)?;
        }
        // Options de niveau statement (M16.4). NOBS= crée (ou réutilise)
        // une variable numérique au PDV maintenant (elle est affectée
        // AVANT la boucle ⇒ doit exister) et la marque retenue (sa
        // valeur ne doit pas être remise à missing à chaque itération) ;
        // POINT= référence une variable numérique que l'utilisateur
        // pilote (créée ici si absente, comme une variable assignée).
        // END= ne crée JAMAIS de slot (variable automatique temporaire,
        // servie par EvalCtx, jamais écrite en sortie).
        if let Some(name) = &options.nobs {
            let slot = match self.pdv.slot(name) {
                Some(s) => s,
                None => self.add_var(name, VarType::Num, 8),
            };
            self.retained_slots.insert(slot);
            self.assigned.insert(name.to_uppercase());
        }
        if let Some(name) = &options.point {
            if self.pdv.slot(name).is_none() {
                self.add_var(name, VarType::Num, 8);
            }
            // La variable POINT= est pilotée par l'utilisateur : on la
            // considère "assignée" (pas de NOTE "uninitialized").
            self.assigned.insert(name.to_uppercase());
        }
        self.set_options = options.clone();
        Ok(())
    }

    /// Compile un statement `MERGE` (bras `DsStmt::Merge` de `walk_stmt`).
    pub(super) fn compile_merge(&mut self, specs: &[DatasetSpec]) -> Result<()> {
        if self.seen_set || self.seen_merge {
            return Err(SasError::runtime(
                "A MERGE statement is not allowed after a SET or MERGE statement.",
            ));
        }
        self.seen_merge = true;
        for spec in specs {
            // L'index du dataset dans `input_datasets` AVANT le push.
            let ds_index = self.input_datasets.len();
            if let Some(nm) = &spec.options.in_ {
                // Le nom IN= ne doit PAS entrer en collision avec une
                // variable du PDV (c'est une automatique temporaire).
                self.in_flags.push((nm.to_uppercase(), ds_index));
            }
            self.compile_set(spec)?;
        }
        Ok(())
    }

    /// Compile un statement `UPDATE` (bras `DsStmt::Update` de `walk_stmt`).
    pub(super) fn compile_update(
        &mut self,
        master: &DatasetRef,
        master_where: &Option<Expr>,
        transaction: &DatasetRef,
        key_vars: &[String],
    ) -> Result<()> {
        if self.seen_set || self.seen_merge || self.update.is_some() || self.modify.is_some()
        {
            return Err(SasError::runtime(
                "Only one SET, MERGE, UPDATE, or MODIFY statement is allowed per DATA step.",
            ));
        }
        // Le maître entre au PDV en premier (ordre de référence), puis
        // la transaction (ses variables nouvelles s'ajoutent).
        let master_ds = self.materialize_input(master, &DatasetOptions::default())?;
        let transaction_ds =
            self.materialize_input(transaction, &DatasetOptions::default())?;
        if let Some(w) = master_where {
            self.validate_where_vars(w, &master.display())?;
        }
        self.update = Some(PendingUpdate {
            master: master_ds,
            transaction: transaction_ds,
            master_display: master.display(),
            key_names: key_vars.to_vec(),
            master_where: master_where.clone(),
        });
        Ok(())
    }

    /// Compile un statement `MODIFY` (bras `DsStmt::Modify` de `walk_stmt`).
    pub(super) fn compile_modify(
        &mut self,
        dataset: &DatasetRef,
        key_vars: &[String],
        point: &Option<String>,
        nobs: &Option<String>,
    ) -> Result<()> {
        if self.seen_set || self.seen_merge || self.update.is_some() || self.modify.is_some()
        {
            return Err(SasError::runtime(
                "Only one SET, MERGE, UPDATE, or MODIFY statement is allowed per DATA step.",
            ));
        }
        let (data, out_vars) =
            self.materialize_input_with_meta(dataset, &DatasetOptions::default())?;
        // NOBS= : variable numérique affectée AVANT la boucle (doit
        // exister, retenue). POINT= : pilotée par l'utilisateur.
        if let Some(name) = nobs {
            let slot = match self.pdv.slot(name) {
                Some(s) => s,
                None => self.add_var(name, VarType::Num, 8),
            };
            self.retained_slots.insert(slot);
            self.assigned.insert(name.to_uppercase());
        }
        if let Some(name) = point {
            if self.pdv.slot(name).is_none() {
                self.add_var(name, VarType::Num, 8);
            }
            self.assigned.insert(name.to_uppercase());
        }
        self.modify = Some(PendingModify {
            libref: dataset.libref_or_work(),
            table: dataset.name.clone(),
            display: dataset.display(),
            data,
            out_vars,
            key_names: key_vars.to_vec(),
            point: point.clone(),
            nobs: nobs.clone(),
        });
        Ok(())
    }

    /// Compile UN dataset d'un statement SET : lecture, options de
    /// dataset, entrée des variables au PDV (union en ordre de première
    /// apparition), matérialisation des colonnes.
    /// Matérialise un dataset (toutes ses colonnes) dans le PDV pour UPDATE/
    /// MODIFY (M16.5). Comme `compile_set` mais sans KEEP=/DROP=/RENAME= :
    /// TOUTES les variables entrent au PDV (ordre de première référence), avec
    /// downcast unique par colonne (jamais de get_row). Renvoie l'`InputDataset`
    /// matérialisé (colonnes décodées + slots PDV). `opts` réservé (where=
    /// filtré à l'exécution, non ici).
    pub(super) fn materialize_input(
        &mut self,
        dref: &crate::ast::DatasetRef,
        _opts: &DatasetOptions,
    ) -> Result<InputDataset> {
        Ok(self.materialize_input_with_meta(dref, _opts)?.0)
    }

    /// Comme `materialize_input` mais renvoie aussi les `VarMeta` de CHAQUE
    /// colonne (dans l'ordre `var_slots`), nécessaires à MODIFY pour réécrire
    /// le dataset à l'identique (mêmes types/longueurs/formats/libellés).
    pub(super) fn materialize_input_with_meta(
        &mut self,
        dref: &crate::ast::DatasetRef,
        _opts: &DatasetOptions,
    ) -> Result<(InputDataset, Vec<crate::dataset::VarMeta>)> {
        let libref = dref.libref_or_work();
        let provider = self.session.libs.get(&libref)?;
        if !provider.exists(&dref.name) {
            return Err(SasError::runtime(format!(
                "File {}.DATA does not exist.",
                dref.display()
            )));
        }
        let (ds, notes) = provider.read(&dref.name)?;
        for note in &notes {
            self.session.log.forward(note);
        }
        let mut columns = Vec::with_capacity(ds.vars.len());
        let mut var_slots = Vec::with_capacity(ds.vars.len());
        let mut out_vars = Vec::with_capacity(ds.vars.len());
        for (col, meta) in ds.df.get_columns().iter().zip(&ds.vars) {
            if self
                .pdv
                .slot(&meta.name)
                .is_some_and(|slot| self.pdv.vars()[slot].ty != meta.ty)
            {
                return Err(SasError::runtime(format!(
                    "Variable {} has been defined as both character and numeric.",
                    meta.name
                )));
            }
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
            var_slots.push(slot);
            out_vars.push(meta.clone());
            let s = col.as_materialized_series();
            let values: Vec<Value> = match meta.ty {
                VarType::Num => s.f64()?.iter().map(num_to_value).collect(),
                VarType::Char => s
                    .str()?
                    .iter()
                    .map(|o| Value::Char(o.unwrap_or("").to_string()))
                    .collect(),
            };
            columns.push(values);
        }
        let n_rows = ds.n_obs();
        Ok((
            InputDataset {
                display: dref.display(),
                columns,
                var_slots,
                n_rows,
                where_: None,
                by_cols: Vec::new(),
            },
            out_vars,
        ))
    }

    pub(super) fn compile_set(&mut self, spec: &DatasetSpec) -> Result<()> {
        let r = &spec.dref;
        let opts = &spec.options;
        let libref = r.libref_or_work();
        let provider = self.session.libs.get(&libref)?;
        if !provider.exists(&r.name) {
            return Err(SasError::runtime(format!(
                "File {}.DATA does not exist.",
                r.display()
            )));
        }
        let (ds, notes) = provider.read(&r.name)?;
        for note in &notes {
            self.session.log.forward(note);
        }

        // Validation des options : KEEP=/DROP=/RENAME= référencent les noms
        // D'ORIGINE de l'input (règle SAS : KEEP/DROP s'appliquent AVANT
        // RENAME). Un nom absent de l'input → même erreur que l'existant.
        let input_names: HashSet<String> =
            ds.vars.iter().map(|v| v.name.to_uppercase()).collect();
        for name in opts
            .keep
            .iter()
            .flatten()
            .chain(opts.drop.iter().flatten())
            .chain(opts.rename.iter().map(|(old, _)| old))
        {
            if !input_names.contains(&name.to_uppercase()) {
                return Err(SasError::runtime(format!(
                    "The variable {name} in the DROP, KEEP, or RENAME list has never been referenced."
                )));
            }
        }
        let keep_set: Option<HashSet<String>> = opts
            .keep
            .as_ref()
            .map(|v| v.iter().map(|n| n.to_uppercase()).collect());
        let drop_set: HashSet<String> = opts
            .drop
            .iter()
            .flatten()
            .map(|n| n.to_uppercase())
            .collect();
        let rename: HashMap<String, String> = opts
            .rename
            .iter()
            .map(|(old, new)| (old.to_uppercase(), new.clone()))
            .collect();

        let mut columns = Vec::with_capacity(ds.vars.len());
        let mut var_slots = Vec::with_capacity(ds.vars.len());
        for (col, meta) in ds.df.get_columns().iter().zip(&ds.vars) {
            let upper = meta.name.to_uppercase();
            // KEEP=/DROP= filtrent quelles variables d'input entrent au PDV
            // (une variable renommée mais non gardée est ignorée).
            if keep_set.as_ref().is_some_and(|k| !k.contains(&upper))
                || drop_set.contains(&upper)
            {
                continue;
            }
            // RENAME= : la variable entre au PDV sous le NOUVEAU nom
            // (appliqué APRÈS keep/drop).
            let pdv_name = rename
                .get(&upper)
                .cloned()
                .unwrap_or_else(|| meta.name.clone());
            // Une variable déjà au PDV avec un type INCOMPATIBLE (présente
            // dans un autre dataset du SET, ou référencée avant) → erreur
            // de compilation, comme SAS.
            if let Some(slot) = self.pdv.slot(&pdv_name) {
                if self.pdv.vars()[slot].ty != meta.ty {
                    return Err(SasError::runtime(format!(
                        "Variable {pdv_name} has been defined as both character and numeric."
                    )));
                }
            }
            let slot = self.pdv.add_var(PdvVar {
                name: pdv_name,
                ty: meta.ty,
                length: meta.length,
                retained: false,
                from_input: true,
                format: meta.format.clone(),
                temporary: false,
            });
            // Si la variable existait déjà (référence textuelle antérieure
            // au SET), la marquer issue de l'input malgré tout.
            self.pdv.mark_from_input(slot);
            var_slots.push(slot);

            // Downcast UNE FOIS par colonne — jamais de get_row.
            let s = col.as_materialized_series();
            let values: Vec<Value> = match meta.ty {
                VarType::Num => s.f64()?.iter().map(num_to_value).collect(),
                VarType::Char => s
                    .str()?
                    .iter()
                    .map(|o| Value::Char(o.unwrap_or("").to_string()))
                    .collect(),
            };
            columns.push(values);
        }

        // WHERE= : PAS de filtrage à la compilation — l'Expr est stockée et
        // évaluée par l'exécuteur après chaque chargement de ligne. On
        // walke ses variables pour valider qu'elles existent (elles doivent
        // référencer des variables d'input, déjà au PDV à ce point —
        // post-rename, cf. doc d'InputData).
        if let Some(w) = &opts.where_ {
            self.validate_where_vars(w, &r.display())?;
        }

        // OPTIONS FIRSTOBS=/OBS= : restreindre la fenêtre des observations
        // PHYSIQUES lues, AVANT le filtre WHERE= (ordre SAS). FIRSTOBS=k saute
        // les k-1 premières ; OBS=n borne le numéro de la dernière obs lue.
        let n = ds.n_obs();
        let start = self.session.options.firstobs.saturating_sub(1).min(n);
        let end = self.session.options.obs.map_or(n, |o| o.min(n)).max(start);
        if start != 0 || end != n {
            for c in &mut columns {
                *c = c[start..end].to_vec();
            }
        }

        self.input_datasets.push(InputDataset {
            display: r.display(),
            columns,
            var_slots,
            n_rows: end - start,
            where_: opts.where_.clone(),
            by_cols: Vec::new(),
        });
        Ok(())
    }
}
