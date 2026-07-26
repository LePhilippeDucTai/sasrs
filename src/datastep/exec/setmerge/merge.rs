use super::*;

impl Runner {
    /// SET avec BY = INTERCLASSEMENT : parmi les datasets non épuisés,
    /// servir celui dont la prochaine observation (après pré-filtrage
    /// WHERE=) porte la PLUS PETITE clé BY (`sas_cmp`, DESCENDING par clé
    /// respecté) ; égalité → ordre du statement SET. Met à jour les flags
    /// FIRST./LAST. (comparaison des préfixes de clés avec l'observation
    /// précédente / suivante) et détecte le désordre (clé servie < clé
    /// précédente → ERROR, l'étape s'arrête).
    pub(crate) fn exec_set_interleave(&mut self) -> Result<Flow> {
        let Some(input) = &self.input else {
            return Err(SasError::runtime("SET statement without input data."));
        };
        let Some((d, cur_keys)) =
            choose_next(input, &self.set_cursor.filtered, &self.set_cursor.cursors)
        else {
            // Tous les datasets épuisés : fin d'étape immédiate.
            return Ok(Flow::EndStep);
        };
        let ds = &input.datasets[d];
        // Détection de désordre : l'interclassement choisit toujours la
        // plus petite clé disponible ; si elle régresse, c'est qu'un input
        // n'est pas trié selon le BY.
        if let Some(prev) = &self.set_cursor.prev_keys {
            if compare_keys(&cur_keys, prev, &input.by) == Ordering::Less {
                return Err(SasError::runtime(format!(
                    "BY variables are not properly sorted on data set {}.",
                    ds.display
                )));
            }
        }
        let row = self.set_cursor.filtered[d][self.set_cursor.cursors[d]];
        // Les variables absentes du dataset servi GARDENT leur valeur
        // précédente (RETAIN implicite des variables de SET — règle SAS,
        // pas de remise à missing).
        for (col, slot) in ds.columns.iter().zip(&ds.var_slots) {
            self.pdv.set(*slot, col[row].clone());
        }
        self.set_cursor.cursors[d] += 1;
        self.rows_read[d] += 1;
        // FIRST.var_i : première obs, ou clé j ≤ i différente de l'obs
        // précédente. LAST.var_i : dernière obs, ou clé j ≤ i différente
        // de l'obs SUIVANTE (la tête du prochain choix d'interclassement).
        let next_keys =
            choose_next(input, &self.set_cursor.filtered, &self.set_cursor.cursors).map(|(_, k)| k);
        for (i, flags) in self.ctx.by_flags.iter_mut().enumerate() {
            flags.1 = match &self.set_cursor.prev_keys {
                None => true,
                Some(prev) => prefix_changed(&cur_keys, prev, i),
            };
            flags.2 = match &next_keys {
                None => true,
                Some(next) => prefix_changed(&cur_keys, next, i),
            };
        }
        self.set_cursor.prev_keys = Some(cur_keys);
        // END= (M16.4) : 1 si plus aucune observation à interclasser.
        if let Some((_, v)) = &mut self.ctx.end_flag {
            *v = if next_keys.is_none() { 1.0 } else { 0.0 };
        }
        Ok(Flow::Normal)
    }

    /// MERGE (M3) : sert la prochaine observation pré-calculée du plan. À
    /// l'épuisement du plan → EndStep (fin d'étape immédiate, comme SET).
    /// Applique les MISSING des datasets absents (1re obs du groupe), les
    /// chargements (gauche→droite, le dernier contributeur écrase les
    /// variables partagées), puis met à jour les flags FIRST./LAST. et IN=.
    pub(crate) fn exec_merge(&mut self) -> Result<Flow> {
        let Some(input) = &self.input else {
            return Err(SasError::runtime("MERGE statement without input data."));
        };
        let Some(obs) = self.merge.plan.get(self.merge.cursor) else {
            return Ok(Flow::EndStep);
        };
        // Emprunts disjoints : on copie les petites données nécessaires.
        let blank_slots = obs.blank_slots.clone();
        let loads = obs.loads.clone();
        let in_active = obs.in_active.clone();
        let first = obs.first.clone();
        let last = obs.last.clone();

        // (1) Variables PROPRES des datasets absents → MISSING (persistées
        // ensuite tout le groupe, car from_input).
        for &slot in &blank_slots {
            let init = match self.pdv.vars()[slot].ty {
                VarType::Num => Value::missing(),
                VarType::Char => Value::Char(String::new()),
            };
            self.pdv.set(slot, init);
        }
        // (2) Chargements gauche→droite (les datasets non chargés PERSISTENT
        // leurs valeurs — c'est la « persistance du côté court »).
        for &(d, row) in &loads {
            let ds = &input.datasets[d];
            for (col, slot) in ds.columns.iter().zip(&ds.var_slots) {
                self.pdv.set(*slot, col[row].clone());
            }
        }
        // (3) Flags FIRST./LAST. (sur la clé de groupe).
        for (i, flags) in self.ctx.by_flags.iter_mut().enumerate() {
            flags.1 = first[i];
            flags.2 = last[i];
        }
        // (4) Flags IN= : 1 pour les datasets ayant participé au groupe.
        let input = self.input.as_ref().unwrap();
        for (name, ds_idx) in &input.in_flags {
            if let Some((_, flag)) = self.ctx.in_flags.iter_mut().find(|(n, _)| n == name) {
                *flag = in_active[*ds_idx];
            }
        }
        self.merge.cursor += 1;
        Ok(Flow::Normal)
    }

    /// Pré-calcule la séquence des observations de sortie d'un MERGE, groupe
    /// par groupe (cf. en-tête de fichier). Pour chaque clé de l'UNION triée
    /// des clés présentes dans au moins un dataset, le groupe produit
    /// `max_i(n_i)` observations. Détecte le désordre (clés non triées dans
    /// un dataset) → ERROR.
    pub(crate) fn build_merge_plan(&mut self) -> Result<Vec<MergeObs>> {
        let input = self.input.as_ref().unwrap();
        let n_ds = input.datasets.len();
        let n_by = input.by.len();

        // Groupes consécutifs par dataset : Vec<(clé, début, longueur)> sur
        // les lignes RETENUES (`filtered`). Détection de désordre intra-ds.
        let mut ds_groups: Vec<Vec<(Vec<Value>, usize, usize)>> = Vec::with_capacity(n_ds);
        for (d, ds) in input.datasets.iter().enumerate() {
            let rows = &self.set_cursor.filtered[d];
            let mut groups: Vec<(Vec<Value>, usize, usize)> = Vec::new();
            let mut prev_key: Option<Vec<Value>> = None;
            for (pos, &row) in rows.iter().enumerate() {
                let key = keys_at(ds, row);
                match groups.last_mut() {
                    Some((k, _, len)) if compare_keys(&key, k, &input.by) == Ordering::Equal => {
                        *len += 1;
                    }
                    _ => {
                        // Nouvelle clé : doit être STRICTEMENT supérieure à la
                        // précédente (sinon dataset non trié).
                        if let Some(prev) = &prev_key {
                            if compare_keys(&key, prev, &input.by) == Ordering::Less {
                                return Err(SasError::runtime(format!(
                                    "BY variables are not properly sorted on data set {}.",
                                    ds.display
                                )));
                            }
                        }
                        prev_key = Some(key.clone());
                        groups.push((key, pos, 1));
                    }
                }
            }
            ds_groups.push(groups);
        }

        // Curseurs de groupe par dataset.
        let mut g_cursors = vec![0usize; n_ds];
        let mut plan: Vec<MergeObs> = Vec::new();
        let mut prev_group_key: Option<Vec<Value>> = None;

        loop {
            // Plus petite clé de groupe parmi les datasets non épuisés.
            let mut best: Option<Vec<Value>> = None;
            for d in 0..n_ds {
                if let Some((key, _, _)) = ds_groups[d].get(g_cursors[d]) {
                    let better = match &best {
                        None => true,
                        Some(b) => compare_keys(key, b, &input.by) == Ordering::Less,
                    };
                    if better {
                        best = Some(key.clone());
                    }
                }
            }
            let Some(group_key) = best else { break };

            // Par dataset : participe-t-il à ce groupe ? Si oui, (début,
            // longueur) de ses lignes dans `filtered`.
            let mut participate: Vec<Option<(usize, usize)>> = vec![None; n_ds];
            let mut n = vec![0usize; n_ds];
            for d in 0..n_ds {
                if let Some((key, start, len)) = ds_groups[d].get(g_cursors[d]) {
                    if compare_keys(key, &group_key, &input.by) == Ordering::Equal {
                        participate[d] = Some((*start, *len));
                        n[d] = *len;
                        g_cursors[d] += 1;
                    }
                }
            }
            let in_active: Vec<bool> = n.iter().map(|&c| c > 0).collect();
            let max = n.iter().copied().max().unwrap_or(0);

            // Slots PROPRES des datasets absents (n_i == 0) à blanchir au
            // début du groupe : un slot d'un dataset absent n'est blanchi que
            // s'il n'appartient à AUCUN dataset participant (sinon le
            // participant l'écrit).
            let mut blank_slots: Vec<usize> = Vec::new();
            for d in 0..n_ds {
                if n[d] > 0 {
                    continue;
                }
                for &slot in &input.datasets[d].var_slots {
                    let owned_by_participant =
                        (0..n_ds).any(|p| n[p] > 0 && input.datasets[p].var_slots.contains(&slot));
                    if !owned_by_participant && !blank_slots.contains(&slot) {
                        blank_slots.push(slot);
                    }
                }
            }

            // FIRST./LAST. du groupe vs groupes voisins (préfixe de clés).
            let first_flags: Vec<bool> = (0..n_by)
                .map(|i| match &prev_group_key {
                    None => true,
                    Some(prev) => prefix_changed(&group_key, prev, i),
                })
                .collect();

            // La clé du groupe SUIVANT (pour LAST.) : plus petite clé restante
            // après consommation de ce groupe.
            let mut next_group_key: Option<Vec<Value>> = None;
            for d in 0..n_ds {
                if let Some((key, _, _)) = ds_groups[d].get(g_cursors[d]) {
                    let better = match &next_group_key {
                        None => true,
                        Some(b) => compare_keys(key, b, &input.by) == Ordering::Less,
                    };
                    if better {
                        next_group_key = Some(key.clone());
                    }
                }
            }
            let last_flags: Vec<bool> = (0..n_by)
                .map(|i| match &next_group_key {
                    None => true,
                    Some(next) => prefix_changed(&group_key, next, i),
                })
                .collect();

            // `max` observations de sortie pour ce groupe. FIRST.x n'est vrai
            // qu'à la PREMIÈRE obs du groupe (j==0), LAST.x qu'à la DERNIÈRE
            // (j==max-1) — combiné au changement de préfixe vs groupe voisin.
            for j in 0..max {
                let mut loads: Vec<(usize, usize)> = Vec::new();
                for d in 0..n_ds {
                    if let Some((start, len)) = participate[d] {
                        if j < len {
                            // j-ème ligne du groupe dans `filtered`.
                            let row = self.set_cursor.filtered[d][start + j];
                            loads.push((d, row));
                        }
                        // j >= len : PERSISTANCE (pas de chargement).
                    }
                }
                let first: Vec<bool> = first_flags.iter().map(|&f| f && j == 0).collect();
                let last: Vec<bool> = last_flags.iter().map(|&l| l && j + 1 == max).collect();
                plan.push(MergeObs {
                    // Blanchiment seulement à la 1re obs du groupe.
                    blank_slots: if j == 0 {
                        blank_slots.clone()
                    } else {
                        Vec::new()
                    },
                    loads,
                    in_active: in_active.clone(),
                    first,
                    last,
                });
            }
            // Compte des lignes lues (toutes les obs participantes du groupe).
            for d in 0..n_ds {
                self.rows_read[d] += n[d];
            }
            prev_group_key = Some(group_key);
        }
        Ok(plan)
    }

    /// Pré-applique les WHERE= des datasets d'un SET avec BY : remplit
    /// `filtered` (indices des lignes retenues) en évaluant chaque ligne
    /// sur le PDV, puis remet les slots d'input à leur état initial
    /// (missing / chaîne vide). Un `_ERROR_` levé pendant ce pré-filtrage
    /// n'est pas reporté aux itérations (divergence mineure documentée) ;
    /// les compteurs de NOTEs (conversions, invalid data) sont conservés.
    pub(crate) fn prefilter(&mut self) -> Result<()> {
        let Some(input) = &self.input else {
            return Ok(());
        };
        for (d, ds) in input.datasets.iter().enumerate() {
            let Some(w) = &ds.where_ else {
                self.set_cursor.filtered[d] = (0..ds.n_rows).collect();
                continue;
            };
            let mut keep = Vec::new();
            for row in 0..ds.n_rows {
                for (col, slot) in ds.columns.iter().zip(&ds.var_slots) {
                    self.pdv.set(*slot, col[row].clone());
                }
                let v = eval(w, &self.pdv, &mut self.ctx);
                if let Some(err) = self.ctx.fatal.take() {
                    return Err(err);
                }
                self.ctx.error_flag = false;
                if v.truthy() {
                    keep.push(row);
                }
            }
            self.set_cursor.filtered[d] = keep;
        }
        // Restaurer l'état initial des slots d'input touchés par le
        // pré-filtrage.
        for ds in &input.datasets {
            if ds.where_.is_none() {
                continue;
            }
            for &slot in &ds.var_slots {
                let init = match self.pdv.vars()[slot].ty {
                    VarType::Num => Value::missing(),
                    VarType::Char => Value::Char(String::new()),
                };
                self.pdv.set(slot, init);
            }
        }
        Ok(())
    }
}
