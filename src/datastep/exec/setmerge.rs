//! SET/MERGE/POINT : concaténation, interclassement, plan de merge et helpers de clés BY.

use super::*;

mod merge;

impl Runner {
    /// SET sans BY = CONCATÉNATION : le premier dataset en entier, puis le
    /// suivant. Boucle de skip interne : charge des lignes jusqu'à en
    /// trouver une qui passe le WHERE= (faux/missing → ligne suivante SANS
    /// exécuter le reste de l'itération). Les lignes rejetées ne comptent
    /// pas dans `rows_read`. Tous les datasets épuisés → l'étape se
    /// termine IMMÉDIATEMENT (EndStep). Au passage d'un dataset au
    /// suivant, les variables absentes du nouveau dataset GARDENT leur
    /// valeur (RETAIN implicite des variables de SET — règle SAS, pas de
    /// remise à missing).
    pub(super) fn exec_set_concat(&mut self) -> Result<Flow> {
        loop {
            let Some(input) = &self.input else {
                return Err(SasError::runtime("SET statement without input data."));
            };
            let Some(ds) = input.datasets.get(self.set_cursor.cur_ds) else {
                // Fin de TOUS les inputs : fin d'étape immédiate.
                return Ok(Flow::EndStep);
            };
            let row = self.set_cursor.cursors[self.set_cursor.cur_ds];
            if row >= ds.n_rows {
                self.set_cursor.cur_ds += 1;
                continue;
            }
            for (col, slot) in ds.columns.iter().zip(&ds.var_slots) {
                self.pdv.set(*slot, col[row].clone());
            }
            self.set_cursor.cursors[self.set_cursor.cur_ds] += 1;
            let Some(w) = &ds.where_ else {
                self.rows_read[self.set_cursor.cur_ds] += 1;
                self.set_end_flag();
                return Ok(Flow::Normal);
            };
            // Évaluation inline (emprunts disjoints : `input` tient
            // self.input, eval n'utilise que pdv et ctx).
            let v = eval(w, &self.pdv, &mut self.ctx);
            if let Some(err) = self.ctx.fatal.take() {
                return Err(err);
            }
            if self.ctx.error_flag {
                self.pdv.error_ = true;
                self.ctx.error_flag = false;
            }
            if v.truthy() {
                self.rows_read[self.set_cursor.cur_ds] += 1;
                self.set_end_flag();
                return Ok(Flow::Normal);
            }
        }
    }

    /// Met à jour la variable END= (M16.4) après une lecture réussie en mode
    /// concaténation : 1 si AUCUNE observation ne reste à lire (en tenant
    /// compte du WHERE= de chaque dataset), 0 sinon. Sans END= déclaré,
    /// no-op. La détection se fait par un balayage en avant NON destructif
    /// (les curseurs ne sont pas modifiés).
    fn set_end_flag(&mut self) {
        if self.ctx.end_flag.is_none() {
            return;
        }
        let has_more = self.concat_has_more();
        if let Some((_, v)) = &mut self.ctx.end_flag {
            *v = if has_more { 0.0 } else { 1.0 };
        }
    }

    /// Balaye en avant (sans muter les curseurs) pour savoir s'il reste au
    /// moins une observation lisible APRÈS la position courante, en respectant
    /// le WHERE= de chaque dataset. Sert END= en mode concaténation.
    fn concat_has_more(&mut self) -> bool {
        let Some(input) = self.input.take() else {
            return false;
        };
        // Le balayage évalue éventuellement des WHERE= sur des lignes JAMAIS
        // réellement lues : il ne doit donc émettre AUCUNE NOTE/erreur. On
        // mémorise l'état des compteurs de l'évaluateur et on le restaure à la
        // fin (le vrai chargement, lui, comptabilise normalement).
        let saved_ctx = (
            self.ctx.missing_generated,
            self.ctx.division_by_zero,
            self.ctx.note_num_to_char,
            self.ctx.note_char_to_num,
            self.ctx.invalid_data,
            self.ctx.error_flag,
            self.ctx.fatal.take(),
        );
        let mut found = false;
        'outer: for d in self.set_cursor.cur_ds..input.datasets.len() {
            let ds = &input.datasets[d];
            let start = if d == self.set_cursor.cur_ds { self.set_cursor.cursors[d] } else { 0 };
            for row in start..ds.n_rows {
                match &ds.where_ {
                    None => {
                        found = true;
                        break 'outer;
                    }
                    Some(w) => {
                        // Évalue le WHERE= sur une COPIE des valeurs de la ligne
                        // chargées dans le PDV, puis restaure (le balayage ne
                        // doit pas laisser de trace). On sauvegarde/restaure les
                        // slots touchés.
                        let saved: Vec<(usize, Value)> = ds
                            .var_slots
                            .iter()
                            .map(|&s| (s, self.pdv.get(s).clone()))
                            .collect();
                        for (col, slot) in ds.columns.iter().zip(&ds.var_slots) {
                            self.pdv.set(*slot, col[row].clone());
                        }
                        let v = eval(w, &self.pdv, &mut self.ctx);
                        // Restaure les slots (le balayage ne laisse aucune
                        // trace sur le PDV).
                        for (slot, val) in saved {
                            self.pdv.set(slot, val);
                        }
                        if v.truthy() {
                            found = true;
                            break 'outer;
                        }
                    }
                }
            }
        }
        // Restaure intégralement les compteurs de l'évaluateur.
        (
            self.ctx.missing_generated,
            self.ctx.division_by_zero,
            self.ctx.note_num_to_char,
            self.ctx.note_char_to_num,
            self.ctx.invalid_data,
            self.ctx.error_flag,
            self.ctx.fatal,
        ) = saved_ctx;
        self.input = Some(input);
        found
    }

    /// SET ... POINT= (M16.4) : ACCÈS DIRECT. Lit la valeur de la variable
    /// d'index (slot `point_slot`), l'arrondit à l'entier (sémantique SAS),
    /// et charge l'observation correspondante (1-based). Avec plusieurs
    /// datasets en concaténation, l'index est GLOBAL (1..total, parcourant les
    /// datasets dans l'ordre du SET). Index missing / non entier valide /
    /// hors bornes [1, total] → ERROR "Error in variable p." (l'étape
    /// s'arrête). N'avance AUCUN curseur (l'utilisateur pilote l'itération) et
    /// ne compte pas dans les NOTEs "There were N observations read" au sens
    /// d'un balayage séquentiel — mais on incrémente `rows_read` du dataset
    /// servi pour rester cohérent avec le décompte SAS d'obs lues.
    pub(super) fn exec_set_point(&mut self) -> Result<Flow> {
        let Some(input) = self.input.take() else {
            return Err(SasError::runtime("SET statement without input data."));
        };
        let point_slot = input.point_slot.expect("exec_set_point requires POINT=");
        let total: usize = input.datasets.iter().map(|d| d.n_rows).sum();
        let point_name = self.pdv.vars()[point_slot].name.clone();

        // Lecture + coercition de l'index. Une valeur missing ou non
        // convertible → erreur SAS sur la variable d'index.
        let idx_val = self.pdv.get(point_slot).clone();
        let idx = match coerce_num(&idx_val, &mut self.ctx) {
            Some(f) => f.round() as i64,
            None => {
                self.input = Some(input);
                self.pdv.error_ = true;
                return Err(SasError::runtime(format!(
                    "Error in variable {point_name}."
                )));
            }
        };
        if idx < 1 || (idx as usize) > total {
            self.input = Some(input);
            self.pdv.error_ = true;
            return Err(SasError::runtime(format!("Error in variable {point_name}.")));
        }

        // Localiser l'observation globale `idx` (1-based) dans la concaténation.
        let mut remaining = idx as usize - 1; // 0-based offset global
        let mut target: Option<(usize, usize)> = None;
        for (d, ds) in input.datasets.iter().enumerate() {
            if remaining < ds.n_rows {
                target = Some((d, remaining));
                break;
            }
            remaining -= ds.n_rows;
        }
        let (d, row) = target.expect("index validated against total");
        let ds = &input.datasets[d];
        for (col, slot) in ds.columns.iter().zip(&ds.var_slots) {
            self.pdv.set(*slot, col[row].clone());
        }
        self.rows_read[d] += 1;
        // END= avec POINT= : 1 si l'index pointe la DERNIÈRE observation.
        if let Some((_, v)) = &mut self.ctx.end_flag {
            *v = if (idx as usize) == total { 1.0 } else { 0.0 };
        }
        self.input = Some(input);
        Ok(Flow::Normal)
    }

    /// MODIFY+POINT= (M16.5) : au marqueur MODIFY, on CAPTURE la ligne
    /// précédemment chargée (les assignations qui l'ont suivie sont ses
    /// modifications), puis on CHARGE l'obs à l'index POINT= courant (1-based,
    /// arrondi). Index missing / hors bornes → erreur différée (relevée par la
    /// boucle externe). L'état partagé est `self.modify_state`.
    pub(super) fn exec_modify_point(&mut self) -> Result<Flow> {
        // Capture de la ligne précédente.
        let mut state = self.modify_state.take().expect("modify_state present");
        capture_modify_state(&mut state, &self.pdv);
        // Index POINT= courant.
        let idx_val = self.pdv.get(state.point_slot).clone();
        let idx = match coerce_num(&idx_val, &mut self.ctx) {
            Some(f) => f.round() as i64,
            None => {
                state.error = Some(format!("Invalid POINT= value for the data set {}.", state.display));
                self.modify_state = Some(state);
                self.pdv.error_ = true;
                return Ok(Flow::EndStep);
            }
        };
        if idx < 1 || (idx as usize) > state.n_rows {
            state.error = Some(format!("Invalid POINT= value for the data set {}.", state.display));
            self.modify_state = Some(state);
            self.pdv.error_ = true;
            return Ok(Flow::EndStep);
        }
        let row = idx as usize - 1;
        // Charger la ligne `row` depuis le tampon (qui peut déjà porter des
        // modifications d'un tour précédent — fidèle à la réécriture en place).
        for (pos, &slot) in state.var_slots.iter().enumerate() {
            self.pdv.set(slot, state.cols[pos][row].clone());
        }
        state.touched[row] = true;
        state.cur_row = Some(row);
        self.modify_state = Some(state);
        Ok(Flow::Normal)
    }
}

/// Clés BY de la ligne `row` d'un dataset (dans l'ordre du BY).
pub(super) fn keys_at(ds: &InputDataset, row: usize) -> Vec<Value> {
    ds.by_cols.iter().map(|&c| ds.columns[c][row].clone()).collect()
}

/// Comparaison de deux jeux de clés BY : `sas_cmp` clé par clé (les
/// missings SONT ordonnés : `._ < . < .a < nombres`), inversée pour les
/// clés DESCENDING.
fn compare_keys(a: &[Value], b: &[Value], by: &[ByVar]) -> Ordering {
    for (i, bv) in by.iter().enumerate() {
        let mut ord = a[i].sas_cmp(&b[i]);
        if bv.descending {
            ord = ord.reverse();
        }
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

/// Le préfixe de clés `0..=i` diffère-t-il entre deux observations ?
/// (Égalité pure : DESCENDING est sans effet ici.)
pub(super) fn prefix_changed(a: &[Value], b: &[Value], i: usize) -> bool {
    (0..=i).any(|j| a[j].sas_cmp(&b[j]) != Ordering::Equal)
}

/// Choisit la prochaine observation de l'interclassement : parmi les
/// datasets non épuisés (curseur dans `filtered`), celui dont la tête
/// porte la plus petite clé BY ; égalité stricte → le premier dans
/// l'ordre du SET. Renvoie (index du dataset, clés de sa tête).
fn choose_next(
    input: &InputData,
    filtered: &[Vec<usize>],
    cursors: &[usize],
) -> Option<(usize, Vec<Value>)> {
    let mut best: Option<(usize, Vec<Value>)> = None;
    for (d, ds) in input.datasets.iter().enumerate() {
        let Some(&row) = filtered[d].get(cursors[d]) else {
            continue;
        };
        let keys = keys_at(ds, row);
        let better = match &best {
            None => true,
            // Strictement plus petit seulement : à égalité le premier
            // dataset du SET gagne.
            Some((_, bk)) => compare_keys(&keys, bk, &input.by) == Ordering::Less,
        };
        if better {
            best = Some((d, keys));
        }
    }
    best
}


