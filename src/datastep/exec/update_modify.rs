use super::*;

/// Exécute une étape DATA pilotée par un UPDATE (M16.5).
///
/// Le maître est lu séquentiellement (pilote l'itération). Pour chaque obs
/// maître (qui passe le WHERE= du maître), on cherche la PREMIÈRE obs de la
/// transaction de même clé ; si trouvée, on superpose ses variables NON
/// MANQUANTES (hors clés) au PDV. Le corps de l'étape s'exécute puis l'obs est
/// sortie (output implicite, sauf OUTPUT explicite). Les obs de transaction
/// sans maître correspondant sont IGNORÉES en v1 (divergence documentée vs SAS,
/// qui les insère). Plusieurs transactions pour une même clé : seule la
/// PREMIÈRE est appliquée.
pub(super) fn execute_update(prog: StepProgram, session: &mut Session) -> Result<StepStats> {
    let StepProgram {
        pdv,
        stmts,
        update,
        outputs,
        has_explicit_output,
        uninitialized,
        initial_values,
        arrays,
        labels,
        flow_labels,
        ..
    } = prog;
    let upd = update.expect("execute_update requires UpdateData");

    for name in &uninitialized {
        session.log.note(&format!("Variable {name} is uninitialized."));
    }

    let trans = &upd.transaction;
    let trans_key_pos: Vec<usize> = upd
        .key_slots
        .iter()
        .map(|&slot| trans.var_slots.iter().position(|&s| s == slot).unwrap())
        .collect();
    let mut trans_index: HashMap<String, usize> = HashMap::new();
    for row in 0..trans.n_rows {
        let key_vals: Vec<Value> = trans_key_pos
            .iter()
            .map(|&pos| trans.columns[pos][row].clone())
            .collect();
        trans_index.entry(key_string(&key_vals)).or_insert(row);
    }
    let overlay_pos: Vec<(usize, usize)> = upd
        .overlay_slots
        .iter()
        .map(|&slot| (slot, trans.var_slots.iter().position(|&s| s == slot).unwrap()))
        .collect();

    let mut r = build_um_runner(
        RunnerConfig {
            pdv,
            outputs,
            arrays,
            labels,
        },
        &upd.by,
        session,
    );

    for (slot, v) in initial_values {
        r.pdv.set(slot, v);
    }

    // M16.6 : programme + étiquettes partagés (LINK/GOTO dans un UPDATE).
    r.program = std::rc::Rc::new(stmts);
    r.flow_labels = std::rc::Rc::new(flow_labels);

    let master = &upd.master;
    let mut master_read = 0usize;
    let suppress_implicit_output = has_explicit_output;

    // Slots issus UNIQUEMENT de la transaction (absents du maître). Comme ils
    // sont `from_input`, `reset_non_retained` ne les blanchit pas ; il faut les
    // remettre à MISSING au début de CHAQUE obs maître pour qu'une obs sans
    // transaction correspondante ne « traîne » pas la valeur d'une précédente.
    let trans_only_slots: Vec<usize> = upd
        .overlay_slots
        .iter()
        .copied()
        .filter(|s| !master.var_slots.contains(s))
        .collect();

    // Séquence des obs maître RETENUES (après WHERE=). FIRST./LAST. sont
    // calculés sur les transitions de clé BY DANS cette séquence.
    let mut kept_rows: Vec<usize> = Vec::with_capacity(master.n_rows);
    for m_row in 0..master.n_rows {
        if let Some(w) = &upd.master_where {
            // Charger seulement les variables maître pour évaluer le WHERE=.
            load_row(&mut r.pdv, master, m_row);
            let v = eval(w, &r.pdv, &mut r.ctx);
            if let Some(err) = r.ctx.fatal.take() {
                return Err(err);
            }
            if !v.truthy() {
                continue;
            }
        }
        kept_rows.push(m_row);
    }
    // Clés BY de chaque obs retenue (vide si pas de BY).
    let by_keys: Vec<Vec<Value>> = kept_rows
        .iter()
        .map(|&row| keys_at(master, row))
        .collect();

    for (seq, &m_row) in kept_rows.iter().enumerate() {
        r.pdv.n_ += 1;
        r.pdv.error_ = false;
        r.pdv.reset_non_retained();
        for &slot in &trans_only_slots {
            let init = match r.pdv.vars()[slot].ty {
                VarType::Num => Value::missing(),
                VarType::Char => Value::Char(String::new()),
            };
            r.pdv.set(slot, init);
        }
        load_row(&mut r.pdv, master, m_row);
        // FIRST./LAST. par variable BY (préfixe de clés vs voisins retenus).
        if !upd.by.is_empty() {
            let cur = &by_keys[seq];
            for (i, flags) in r.ctx.by_flags.iter_mut().enumerate() {
                let first = match seq.checked_sub(1) {
                    None => true,
                    Some(p) => prefix_changed(cur, &by_keys[p], i),
                };
                let last = match by_keys.get(seq + 1) {
                    None => true,
                    Some(next) => prefix_changed(cur, next, i),
                };
                flags.1 = first;
                flags.2 = last;
            }
        }
        master_read += 1;
        let key_vals: Vec<Value> = upd
            .key_slots
            .iter()
            .map(|&slot| r.pdv.get(slot).clone())
            .collect();
        if let Some(&t_row) = trans_index.get(&key_string(&key_vals)) {
            for &(slot, pos) in &overlay_pos {
                let tv = &trans.columns[pos][t_row];
                if !tv.is_missing() {
                    r.pdv.set(slot, tv.clone());
                }
            }
        }
        let flow = r.run_step_body()?;
        if flow == Flow::EndStep {
            break;
        }
        if flow != Flow::NextIter && !suppress_implicit_output {
            r.push_outputs();
        }
    }

    drain_runner_side_effects(&mut r, session)?;

    let mut stats = StepStats {
        read: Vec::new(),
        written: Vec::new(),
    };
    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        master_read, master.display
    ));
    stats.read.push((master.display.clone(), master_read));
    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        trans.n_rows, trans.display
    ));
    stats.read.push((trans.display.clone(), trans.n_rows));

    write_runner_outputs(&mut r, session, &mut stats)?;
    Ok(stats)
}

/// Exécute une étape DATA pilotée par un MODIFY (M16.5) : modification EN
/// PLACE. Le dataset est lu (séquentiellement, ou via POINT= en accès direct),
/// le corps modifie ses variables, et le dataset est RÉÉCRIT à l'identique
/// (mêmes colonnes/ordre) avec les valeurs modifiées. Pas d'output implicite ;
/// OUTPUT interdit (vérifié à la compilation).
pub(super) fn execute_modify(prog: StepProgram, session: &mut Session) -> Result<StepStats> {
    let StepProgram {
        pdv,
        stmts,
        modify,
        outputs,
        uninitialized,
        initial_values,
        arrays,
        labels,
        flow_labels,
        ..
    } = prog;
    let m = modify.expect("execute_modify requires ModifyData");

    for name in &uninitialized {
        session.log.note(&format!("Variable {name} is uninitialized."));
    }

    let mut r = build_um_runner(
        RunnerConfig {
            pdv,
            outputs,
            arrays,
            labels,
        },
        &[],
        session,
    );

    for (slot, v) in initial_values {
        r.pdv.set(slot, v);
    }
    // M16.6 : programme + étiquettes partagés (LINK/GOTO dans un MODIFY).
    r.program = std::rc::Rc::new(stmts);
    r.flow_labels = std::rc::Rc::new(flow_labels);
    let n_rows = m.data.n_rows;
    if let Some(slot) = m.nobs_slot {
        r.pdv.set(slot, Value::Num(n_rows as f64));
    }

    let mut buffer: Vec<Vec<Value>> = m.data.columns.clone();
    let mut rows_processed = 0usize;

    if let Some(point_slot) = m.point_slot {
        // ACCÈS DIRECT par POINT= : boucle implicite supprimée. Le corps
        // (typiquement `do i = 1 to nobs; p = i; modify ds; ...; end;`) pilote
        // l'itération ; chaque marqueur MODIFY charge l'obs à l'index POINT=
        // courant et capture la ligne PRÉCÉDEMMENT chargée (les assignations
        // entre deux marqueurs modifient l'obs courante). La dernière ligne est
        // capturée en fin d'étape. L'état partagé vit sur le Runner pour que le
        // bras `DsStmt::Modify` standard l'utilise.
        r.modify_state = Some(ModifyState {
            point_slot,
            cols: m.data.columns.clone(),
            var_slots: m.data.var_slots.clone(),
            cur_row: None,
            display: m.display.clone(),
            n_rows,
            error: None,
            touched: vec![false; n_rows],
        });
        r.pdv.n_ += 1;
        r.pdv.error_ = false;
        let _flow = r.run_step_body()?;
        if let Some(msg) = r.modify_state.as_mut().and_then(|st| st.error.take()) {
            return Err(SasError::runtime(msg));
        }
        if let Some(mut state) = r.modify_state.take() {
            capture_modify_state(&mut state, &r.pdv);
            buffer[..m.data.var_slots.len()]
                .clone_from_slice(&state.cols[..m.data.var_slots.len()]);
            rows_processed = state.touched.iter().filter(|t| **t).count();
        }
    } else {
        // `row` indexe à la fois le chargement et la capture du tampon : la
        // boucle range est intentionnelle.
        #[allow(clippy::needless_range_loop)]
        for row in 0..n_rows {
            r.pdv.n_ += 1;
            r.pdv.error_ = false;
            r.pdv.reset_non_retained();
            load_row(&mut r.pdv, &m.data, row);
            rows_processed += 1;
            let flow = r.run_step_body()?;
            for (pos, &slot) in m.data.var_slots.iter().enumerate() {
                buffer[pos][row] = r.pdv.get(slot).clone();
            }
            if flow == Flow::EndStep {
                break;
            }
        }
    }

    drain_runner_side_effects(&mut r, session)?;

    let columns: Vec<Column> = m
        .out_vars
        .iter()
        .enumerate()
        .map(|(pos, meta)| column_from_values(&meta.name, meta.ty, buffer[pos].iter()))
        .collect();
    let df = DataFrame::new(columns)?;
    let ds = SasDataset {
        df,
        vars: m.out_vars.clone(),
    };
    session.libs.get(&m.libref)?.write(&m.table, &ds)?;
    session.last_dataset = Some(m.display.clone());

    let mut stats = StepStats {
        read: Vec::new(),
        written: Vec::new(),
    };
    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        rows_processed, m.display
    ));
    stats.read.push((m.display.clone(), rows_processed));
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        m.display,
        n_rows,
        m.out_vars.len()
    ));
    stats
        .written
        .push((m.display.clone(), n_rows, m.out_vars.len()));

    // Les sorties DATA (le dataset nommé par `data X;`) coïncident avec la
    // table MODIFY réécrite en place : on les IGNORE (pas d'output implicite, et
    // l'écriture vide des builders écraserait la réécriture). OUTPUT explicite
    // est déjà interdit à la compilation ; un OUT= vers un autre dataset n'est
    // pas supporté en v1.
    let _ = &r.outputs;
    Ok(stats)
}

/// Capture les valeurs courantes du PDV dans le tampon `cols` à la ligne MODIFY
/// chargée (`cur_row`), puis remet le marqueur à `None`. No-op si aucune ligne
/// n'est chargée.
pub(super) fn capture_modify_state(state: &mut ModifyState, pdv: &Pdv) {
    if let Some(row) = state.cur_row.take() {
        for (pos, &slot) in state.var_slots.iter().enumerate() {
            state.cols[pos][row] = pdv.get(slot).clone();
        }
    }
}
