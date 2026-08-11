use super::*;

impl Compiler<'_> {
    /// Assemble l'`InputData` final : résolution des clés BY en slots PDV,
    /// localisation de chaque clé dans CHAQUE dataset (`by_cols`),
    /// validation des références FIRST./LAST. contre les variables BY.
    /// Renvoie `(site 0 ou MERGE, sites SET supplémentaires)` (M40.2).
    pub(super) fn build_input(&mut self) -> Result<(Option<InputData>, Vec<InputData>)> {
        // UPDATE/MODIFY gèrent leur propre BY (résolu dans build_update/
        // build_modify) : ne pas consommer `by`/`first_last_refs` ici.
        if self.update.is_some() || self.modify.is_some() {
            // M40.3 — le WHERE statement avec UPDATE/MODIFY (SAS l'applique
            // aux deux datasets de l'UPDATE) n'est pas implémenté : refus
            // honnête plutôt qu'un filtrage partiel (seul le WHERE= du
            // maître d'UPDATE est supporté, cf. README).
            if self.where_stmt.is_some() {
                return Err(SasError::runtime(
                    "The WHERE statement is not supported with UPDATE or MODIFY in this build.",
                ));
            }
            return Ok((None, Vec::new()));
        }
        let mut datasets = std::mem::take(&mut self.input_datasets);
        let mut extra_sites = std::mem::take(&mut self.extra_set_sites);
        let by_items = self.by.take();
        if datasets.is_empty() {
            // WHERE statement sans SET/MERGE (étape INPUT/DATALINES ou sans
            // entrée) : ERROR SAS.
            if self.where_stmt.is_some() {
                return Err(SasError::runtime(
                    "No input data sets available for WHERE statement.",
                ));
            }
            // BY ou FIRST./LAST. sans SET : message SAS.
            if by_items.is_some() || !self.first_last_refs.is_empty() {
                return Err(SasError::runtime(
                    "No SET, MERGE, UPDATE, or MODIFY statement.",
                ));
            }
            return Ok((None, Vec::new()));
        }
        // M40.3 — WHERE statement standalone : même effet qu'un `WHERE=()`
        // posé sur CHAQUE dataset d'entrée (tous les sites SET + MERGE). Un
        // dataset qui porte déjà son option WHERE= la GARDE (règle SAS :
        // l'option remplace le statement pour ce dataset — pas de cumul).
        // Chaque variable du WHERE doit exister dans CHAQUE dataset filtré
        // (« Variable x is not on file WORK.A. »).
        if let Some(w) = self.where_stmt.take() {
            // POINT= remplace la boucle implicite (accès direct) : le filtre
            // pré-chargement n'y a pas de sens — refus, comme SAS.
            if self.set_options.point.is_some() {
                return Err(SasError::runtime(
                    "The WHERE statement cannot be used with the POINT= option.",
                ));
            }
            for ds in datasets
                .iter_mut()
                .chain(extra_sites.iter_mut().flat_map(|(v, _)| v.iter_mut()))
            {
                if ds.where_.is_none() {
                    self.validate_where_stmt_vars(&w, ds)?;
                    ds.where_ = Some(w.clone());
                }
            }
        }
        // M40.2 — restrictions avec PLUSIEURS statements SET : le BY (match
        // par site) et POINT= (accès direct) ne sont pas supportés — refus
        // honnête plutôt qu'un résultat faux (cf. README).
        if !extra_sites.is_empty() {
            if by_items.is_some() {
                return Err(SasError::runtime(
                    "The BY statement is not supported with multiple SET statements.",
                ));
            }
            if self.set_options.point.is_some()
                || extra_sites.iter().any(|(_, o)| o.point.is_some())
            {
                return Err(SasError::runtime(
                    "POINT= is not supported with multiple SET statements.",
                ));
            }
        }
        let mut by: Vec<ByVar> = Vec::new();
        if let Some(items) = by_items {
            for (name, descending) in items {
                let Some(slot) = self.pdv.slot(&name) else {
                    return Err(SasError::runtime(format!(
                        "BY variable {name} is not on input data set {}.",
                        datasets[0].display
                    )));
                };
                by.push(ByVar {
                    name: name.to_uppercase(),
                    slot,
                    descending,
                });
            }
            // Chaque variable BY doit exister dans CHAQUE dataset du SET
            // (après keep=/drop=/rename=).
            for ds in &mut datasets {
                for bv in &by {
                    let Some(pos) = ds.var_slots.iter().position(|&s| s == bv.slot) else {
                        return Err(SasError::runtime(format!(
                            "BY variable {} is not on input data set {}.",
                            bv.name, ds.display
                        )));
                    };
                    ds.by_cols.push(pos);
                }
            }
        }
        // FIRST.x / LAST.x : x doit être une variable BY.
        for full in &self.first_last_refs {
            let suffix = full
                .split_once('.')
                .map(|(_, s)| s)
                .unwrap_or(full.as_str());
            if !by.iter().any(|b| b.name == suffix) {
                return Err(SasError::runtime(format!(
                    "Variable {full} is not defined: {suffix} is not a BY variable."
                )));
            }
        }
        // MERGE exige un BY (sinon match-merge non défini : SAS le tolère en
        // « one-to-one merge » positionnel, hors périmètre M3 → erreur).
        if self.seen_merge && by.is_empty() {
            return Err(SasError::runtime(
                "A MERGE statement requires a BY statement.",
            ));
        }
        let in_flags = std::mem::take(&mut self.in_flags);

        // Options de niveau statement du SET (M16.4).
        let opts = std::mem::take(&mut self.set_options);
        let end_var = opts.end.as_ref().map(|n| n.to_uppercase());
        let nobs_slot = match &opts.nobs {
            Some(n) => Some(self.pdv.slot(n).ok_or_else(|| {
                SasError::runtime(format!("NOBS= variable {n} is not addressable."))
            })?),
            None => None,
        };
        let point_slot = match &opts.point {
            Some(n) => Some(self.pdv.slot(n).ok_or_else(|| {
                SasError::runtime(format!("POINT= variable {n} is not addressable."))
            })?),
            None => None,
        };
        // POINT= remplace la boucle implicite : il est incompatible avec un
        // interclassement BY (l'accès direct n'a pas de sémantique BY) et avec
        // un MERGE. Les datasets multiples en concaténation sont tolérés (index
        // global 1..total), mais SAS le déconseille (documenté).
        if point_slot.is_some() {
            if !by.is_empty() {
                return Err(SasError::runtime(
                    "POINT= cannot be used with a BY statement.",
                ));
            }
            if self.seen_merge {
                return Err(SasError::runtime(
                    "POINT= cannot be used with a MERGE statement.",
                ));
            }
        }

        // M40.2 — sites SET supplémentaires : concaténation séquentielle
        // pure (jamais de BY/MERGE/IN=/POINT=), avec END=/NOBS= PAR SITE.
        let mut extra_inputs = Vec::with_capacity(extra_sites.len());
        for (site_datasets, opts) in extra_sites {
            let nobs_slot = match &opts.nobs {
                Some(n) => Some(self.pdv.slot(n).ok_or_else(|| {
                    SasError::runtime(format!("NOBS= variable {n} is not addressable."))
                })?),
                None => None,
            };
            extra_inputs.push(InputData {
                datasets: site_datasets,
                by: Vec::new(),
                merge: false,
                in_flags: Vec::new(),
                end_var: opts.end.as_ref().map(|n| n.to_uppercase()),
                nobs_slot,
                point_slot: None,
            });
        }

        Ok((
            Some(InputData {
                datasets,
                by,
                merge: self.seen_merge,
                in_flags,
                end_var,
                nobs_slot,
                point_slot,
            }),
            extra_inputs,
        ))
    }

    /// Assemble l'`UpdateData` final (M16.5) : résolution des clés en slots
    /// PDV, calcul des slots overlay (variables transaction hors clés),
    /// résolution du BY optionnel (FIRST./LAST.). Renvoie `None` si pas
    /// d'UPDATE dans l'étape.
    pub(super) fn build_update(&mut self) -> Result<Option<UpdateData>> {
        let Some(pending) = self.update.take() else {
            return Ok(None);
        };
        let by_items = self.by.take();
        // Clés : doivent exister dans le PDV (donc dans le maître OU la
        // transaction). On résout par nom ; une clé absente du maître ET de la
        // transaction → erreur.
        let mut key_slots = Vec::with_capacity(pending.key_names.len());
        for name in &pending.key_names {
            let Some(slot) = self.pdv.slot(name) else {
                return Err(SasError::runtime(format!(
                    "KEY variable {name} is not on the UPDATE data sets."
                )));
            };
            // La clé doit appartenir à la transaction (sert la recherche) ET
            // au maître (l'obs maître la porte).
            if !pending.transaction.var_slots.contains(&slot) {
                return Err(SasError::runtime(format!(
                    "KEY variable {name} is not on the transaction data set {}.",
                    pending.transaction.display
                )));
            }
            if !pending.master.var_slots.contains(&slot) {
                return Err(SasError::runtime(format!(
                    "KEY variable {name} is not on the master data set {}.",
                    pending.master_display
                )));
            }
            key_slots.push(slot);
        }
        // Slots overlay : toutes les variables de la transaction SAUF les clés.
        let overlay_slots: Vec<usize> = pending
            .transaction
            .var_slots
            .iter()
            .copied()
            .filter(|s| !key_slots.contains(s))
            .collect();

        // BY optionnel : chaque clé BY doit exister au PDV ; on remplit
        // `by_cols` du maître (pilote l'itération / FIRST./LAST.).
        let mut by: Vec<ByVar> = Vec::new();
        let mut master = pending.master;
        if let Some(items) = by_items {
            for (name, descending) in items {
                let Some(slot) = self.pdv.slot(&name) else {
                    return Err(SasError::runtime(format!(
                        "BY variable {name} is not on the master data set {}.",
                        master.display
                    )));
                };
                let Some(pos) = master.var_slots.iter().position(|&s| s == slot) else {
                    return Err(SasError::runtime(format!(
                        "BY variable {name} is not on the master data set {}.",
                        master.display
                    )));
                };
                master.by_cols.push(pos);
                by.push(ByVar {
                    name: name.to_uppercase(),
                    slot,
                    descending,
                });
            }
        }
        // FIRST.x / LAST.x : x doit être une variable BY.
        for full in &self.first_last_refs {
            let suffix = full
                .split_once('.')
                .map(|(_, s)| s)
                .unwrap_or(full.as_str());
            if !by.iter().any(|b| b.name == suffix) {
                return Err(SasError::runtime(format!(
                    "Variable {full} is not defined: {suffix} is not a BY variable."
                )));
            }
        }
        Ok(Some(UpdateData {
            master,
            transaction: pending.transaction,
            key_slots,
            overlay_slots,
            master_where: pending.master_where,
            by,
        }))
    }

    /// Assemble le `ModifyData` final (M16.5) : résolution des clés et des
    /// slots POINT=/NOBS=. Renvoie `None` si pas de MODIFY dans l'étape.
    pub(super) fn build_modify(&mut self) -> Result<Option<ModifyData>> {
        let Some(pending) = self.modify.take() else {
            return Ok(None);
        };
        let mut key_slots = Vec::with_capacity(pending.key_names.len());
        for name in &pending.key_names {
            let Some(slot) = self.pdv.slot(name) else {
                return Err(SasError::runtime(format!(
                    "KEY variable {name} is not on the MODIFY data set {}.",
                    pending.display
                )));
            };
            if !pending.data.var_slots.contains(&slot) {
                return Err(SasError::runtime(format!(
                    "KEY variable {name} is not on the MODIFY data set {}.",
                    pending.display
                )));
            }
            key_slots.push(slot);
        }
        let point_slot = match &pending.point {
            Some(n) => Some(self.pdv.slot(n).ok_or_else(|| {
                SasError::runtime(format!("POINT= variable {n} is not addressable."))
            })?),
            None => None,
        };
        let nobs_slot = match &pending.nobs {
            Some(n) => Some(self.pdv.slot(n).ok_or_else(|| {
                SasError::runtime(format!("NOBS= variable {n} is not addressable."))
            })?),
            None => None,
        };
        Ok(Some(ModifyData {
            libref: pending.libref,
            table: pending.table,
            display: pending.display,
            data: pending.data,
            key_slots,
            point_slot,
            nobs_slot,
            out_vars: pending.out_vars,
        }))
    }

    /// Assemble la source d'entrée TEXTE (M14) à partir de l'INFILE, de
    /// l'INPUT et du bloc DATALINES rencontrés. Renvoie `None` si l'étape
    /// n'a ni INFILE ni INPUT ni DATALINES (= pas de lecture texte).
    pub(super) fn build_text_input(&mut self) -> Result<Option<TextInput>> {
        let infile = self.infile.take();
        let datalines = self.datalines.take();

        // Pas de lecture texte du tout.
        if infile.is_none() && !self.seen_input && datalines.is_none() {
            return Ok(None);
        }

        // Source : INFILE explicite, sinon DATALINES inline implicite. Un
        // chemin relatif résout sous `base_dir` (cohérent avec LIBNAME) ; la
        // NOTE affiche le chemin SOURCE tel quel (entre guillemets, fidèle à
        // SAS, et stable pour les snapshots — pas de tempdir absolu).
        let (lines, display, is_file) = match &infile {
            Some((crate::ast::InfileSource::Path(path), _)) => {
                let resolved = self.session.resolve_path(path);
                let content = std::fs::read_to_string(&resolved).map_err(|e| {
                    SasError::runtime(format!("Unable to read INFILE '{path}': {e}"))
                })?;
                // Lignes sans le `\n` ; un `\r` final est retiré.
                let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
                (lines, format!("the infile '{path}'"), true)
            }
            Some((crate::ast::InfileSource::Datalines, _)) | None => {
                let lines = datalines.clone().ok_or_else(|| {
                    SasError::runtime(
                        "INPUT/INFILE DATALINES used but no DATALINES block is present.",
                    )
                })?;
                (lines, "the infile DATALINES".to_string(), false)
            }
        };

        // Options d'exécution.
        let opts = infile.as_ref().map(|(_, o)| o);
        let dsd = opts.is_some_and(|o| o.dsd);
        let delimiter = opts.and_then(|o| o.delimiter.clone());
        let short = match opts {
            Some(o) if o.stopover => ShortMode::Stopover,
            Some(o) if o.truncover => ShortMode::Truncover,
            Some(o) if o.missover => ShortMode::Missover,
            _ => ShortMode::Default,
        };
        let firstobs = opts.and_then(|o| o.firstobs).unwrap_or(1).max(1);
        let obs = opts.and_then(|o| o.obs);

        let options = TextOptions {
            delimiter,
            dsd,
            firstobs,
            obs,
            short,
        };

        Ok(Some(TextInput {
            display,
            lines,
            options,
            is_file,
        }))
    }

    pub(super) fn resolve_outputs(&mut self, specs: &[DatasetSpec]) -> Result<Vec<OutputSpec>> {
        // Toute variable de KEEP/DROP (statements) doit exister au PDV.
        for name in self.keeps.iter().chain(self.drops.iter()) {
            if self.pdv.slot(name).is_none() {
                return Err(SasError::runtime(format!(
                    "The variable {} in the DROP, KEEP, or RENAME list has never been referenced.",
                    name
                )));
            }
        }
        let stmt_keep: Option<HashSet<String>> = if self.keeps.is_empty() {
            None
        } else {
            Some(self.keeps.iter().map(|n| n.to_uppercase()).collect())
        };
        let stmt_drop: HashSet<String> = self.drops.iter().map(|n| n.to_uppercase()).collect();

        // KEEP ∩ DROP (statements) : DROP gagne, avec WARNING.
        if let Some(ref ks) = stmt_keep {
            for d in &stmt_drop {
                if ks.contains(d) {
                    self.session.log.warning(&format!(
                        "Variable {d} is in both the KEEP and DROP lists; it will be dropped."
                    ));
                }
            }
        }

        let mut outputs = Vec::with_capacity(specs.len());
        for spec in specs {
            let opts = &spec.options;
            // WHERE= n'est pas valide sur une sortie (règle SAS).
            if opts.where_.is_some() {
                return Err(SasError::runtime(
                    "WHERE= is not a valid data set option for output data sets.",
                ));
            }
            // IN= n'est valide qu'en INPUT de MERGE (règle SAS).
            if opts.in_.is_some() {
                return Err(SasError::runtime(
                    "IN= is not a valid data set option for output data sets.",
                ));
            }
            // Les variables des options KEEP=/DROP=/RENAME= doivent exister
            // au PDV (KEEP/DROP avant RENAME : tout référence les noms PDV).
            for name in opts
                .keep
                .iter()
                .flatten()
                .chain(opts.drop.iter().flatten())
                .chain(opts.rename.iter().map(|(old, _)| old))
            {
                if self.pdv.slot(name).is_none() {
                    return Err(SasError::runtime(format!(
                        "The variable {name} in the DROP, KEEP, or RENAME list has never been referenced."
                    )));
                }
            }
            let opt_keep: Option<HashSet<String>> = opts
                .keep
                .as_ref()
                .map(|v| v.iter().map(|n| n.to_uppercase()).collect());
            let opt_drop: HashSet<String> = opts
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

            // Combinaison statements + options : INTERSECTION des keeps
            // (un slot doit passer tous les KEEP présents), union des
            // drops (DROP gagne, sans WARNING supplémentaire pour les
            // options — simplification documentée).
            let mut kept_slots = Vec::new();
            let mut out_names = Vec::new();
            for (i, v) in self.pdv.vars().iter().enumerate() {
                // Les éléments d'array _TEMPORARY_ ne sont JAMAIS écrits.
                if v.temporary {
                    continue;
                }
                let u = v.name.to_uppercase();
                let kept = stmt_keep.as_ref().is_none_or(|k| k.contains(&u))
                    && opt_keep.as_ref().is_none_or(|k| k.contains(&u))
                    && !stmt_drop.contains(&u)
                    && !opt_drop.contains(&u);
                if kept {
                    kept_slots.push(i);
                    // RENAME= : la colonne ÉCRITE porte le nouveau nom (le
                    // slot PDV, lui, garde son nom).
                    out_names.push(rename.get(&u).cloned().unwrap_or_else(|| v.name.clone()));
                }
            }
            outputs.push(OutputSpec {
                libref: spec.libref_or_work(),
                table: spec.dref.name.clone(),
                display: spec.display(),
                kept_slots,
                out_names,
            });
        }
        Ok(outputs)
    }
}
