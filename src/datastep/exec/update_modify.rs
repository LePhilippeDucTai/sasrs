use super::*;

/// Élément du plan d'exécution d'un UPDATE (J03-P6) : soit une obs maître
/// (retenue après WHERE=), soit une obs de transaction SANS maître — SAS
/// l'AJOUTE comme nouvelle observation du dataset de sortie (Language
/// Reference by Example, ch. 21 : « If an observation in the transaction
/// data set does not have a corresponding observation in the master data
/// set, then SAS adds an observation to the master output data set. »).
enum UpdatePlanItem {
    Master(usize),
    NewTrans(usize),
}

/// Ordre total sur une clé multi-variables (pour interclasser les
/// transactions sans maître parmi les obs maître — SAS exige les deux
/// datasets triés par clé ; en cas de types hétérogènes ou de clés de types
/// différents, on considère les clés égales et la transaction suit le
/// maître, ce qui reste correct pour l'ordre d'émission).
fn key_less(a: &[Value], b: &[Value]) -> bool {
    for (x, y) in a.iter().zip(b.iter()) {
        let ord = match (x, y) {
            (Value::Num(u), Value::Num(v)) => u.partial_cmp(v),
            (Value::Missing(_), Value::Num(_)) => Some(std::cmp::Ordering::Less),
            (Value::Num(_), Value::Missing(_)) => Some(std::cmp::Ordering::Greater),
            (Value::Char(u), Value::Char(v)) => Some(u.cmp(v)),
            _ => None,
        };
        match ord {
            Some(std::cmp::Ordering::Less) => return true,
            Some(std::cmp::Ordering::Greater) => return false,
            _ => continue,
        }
    }
    false
}

/// Exécute une étape DATA pilotée par un UPDATE (M16.5, J03-P6).
///
/// Le maître est lu séquentiellement (pilote l'itération). Pour chaque obs
/// maître (qui passe le WHERE= du maître), on superpose TOUTES les obs de la
/// transaction de même clé, DANS L'ORDRE ; par défaut (MISSINGCHECK) seules
/// les valeurs NON MANQUANTES (hors clés) écrasent le PDV — avec
/// UPDATEMODE=NOMISSINGCHECK, les valeurs manquantes écrasent aussi. Les obs
/// de transaction SANS maître correspondant sont AJOUTÉES comme nouvelles
/// observations (interclassées par clé, comme SAS qui exige des entrées
/// triées ; toute transaction non triée restante est ajoutée en fin). Le
/// corps de l'étape s'exécute pour CHAQUE obs produite, puis l'obs est
/// sortie (output implicite, sauf OUTPUT explicite).
///
/// Sources : SAS Language Reference by Example, ch. 21 « Examples: Update
/// Data » (Output 21.30–21.32) —
/// https://go.documentation.sas.com/api/collections/pgmsascdc/9.4_3.5/docsets/lepg/content/lepg.pdf
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
        session
            .log
            .note(&format!("Variable {name} is uninitialized."));
    }

    let trans = &upd.transaction;
    let trans_key_pos: Vec<usize> = upd
        .key_slots
        .iter()
        .map(|&slot| trans.var_slots.iter().position(|&s| s == slot).unwrap())
        .collect();
    // J03-P6 : TOUTES les obs de transaction d'une même clé, dans l'ordre
    // (SAS applique successivement les doublons de clé — la dernière
    // valeur non manquante gagne, cf. Output 21.30 : Dewberry → Dill).
    let mut trans_index: HashMap<String, Vec<usize>> = HashMap::new();
    for row in 0..trans.n_rows {
        let key_vals: Vec<Value> = trans_key_pos
            .iter()
            .map(|&pos| trans.columns[pos][row].clone())
            .collect();
        trans_index
            .entry(key_string(&key_vals))
            .or_default()
            .push(row);
    }
    let overlay_pos: Vec<(usize, usize)> = upd
        .overlay_slots
        .iter()
        .map(|&slot| {
            (
                slot,
                trans.var_slots.iter().position(|&s| s == slot).unwrap(),
            )
        })
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

    // Séquence des obs maître RETENUES (après WHERE=). FIRST./LAST. sont
    // calculés sur les transitions de clé BY DANS la séquence émise (master
    // + nouvelles obs de transaction).
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

    // Transactions sans maître : leur clé n'apparaît dans AUCUNE obs maître
    // retenue. Elles sont émises comme nouvelles observations,
    // interclassées par clé avec les obs maître (curseur sur la liste en
    // ordre de transaction ; les éventuelles non consommées — transaction
    // non triée — suivent en fin de plan, ordre de transaction).
    // NB : la clé est celle du KEY= (slots `key_slots`), PAS celle des
    // colonnes BY (qui peut être vide sans statement BY).
    let master_key_pos: Vec<usize> = upd
        .key_slots
        .iter()
        .map(|&slot| master.var_slots.iter().position(|&s| s == slot).unwrap())
        .collect();
    let master_key_vals: Vec<Vec<Value>> = kept_rows
        .iter()
        .map(|&row| {
            master_key_pos
                .iter()
                .map(|&pos| master.columns[pos][row].clone())
                .collect()
        })
        .collect();
    let master_key_set: std::collections::HashSet<String> =
        master_key_vals.iter().map(|k| key_string(k)).collect();
    let unmatched: Vec<(Vec<Value>, usize)> = (0..trans.n_rows)
        .filter(|&row| {
            let key_vals: Vec<Value> = trans_key_pos
                .iter()
                .map(|&pos| trans.columns[pos][row].clone())
                .collect();
            !master_key_set.contains(&key_string(&key_vals))
        })
        .map(|row| {
            let key_vals: Vec<Value> = trans_key_pos
                .iter()
                .map(|&pos| trans.columns[pos][row].clone())
                .collect();
            (key_vals, row)
        })
        .collect();

    let mut plan: Vec<UpdatePlanItem> = Vec::with_capacity(kept_rows.len() + unmatched.len());
    let mut u = 0usize;
    for (i, &m_row) in kept_rows.iter().enumerate() {
        while u < unmatched.len() && key_less(&unmatched[u].0, &master_key_vals[i]) {
            plan.push(UpdatePlanItem::NewTrans(unmatched[u].1));
            u += 1;
        }
        plan.push(UpdatePlanItem::Master(m_row));
    }
    while u < unmatched.len() {
        plan.push(UpdatePlanItem::NewTrans(unmatched[u].1));
        u += 1;
    }
    // Clés BY de CHAQUE élément du plan (pour FIRST./LAST. sur la séquence
    // réellement émise).
    let plan_keys: Vec<Vec<Value>> = plan
        .iter()
        .map(|item| match item {
            UpdatePlanItem::Master(m_row) => keys_at(master, *m_row),
            UpdatePlanItem::NewTrans(t_row) => trans_key_pos
                .iter()
                .map(|&pos| trans.columns[pos][*t_row].clone())
                .collect(),
        })
        .collect();

    // Slots issus UNIQUEMENT de la transaction (absents du maître). Comme ils
    // sont `from_input`, `reset_non_retained` ne les blanchit pas ; il faut les
    // remettre à MISSING au début de CHAQUE obs pour qu'une obs sans
    // transaction correspondante ne « traîne » pas la valeur d'une précédente.
    // Symétriquement, les slots issus uniquement du MAÎTRE doivent être
    // blanchis pour une NOUVELLE obs de transaction (la variable animal est
    // manquante pour la nouvelle obs b/g de l'exemple LEPG 21.31).
    let trans_only_slots: Vec<usize> = upd
        .overlay_slots
        .iter()
        .copied()
        .filter(|s| !master.var_slots.contains(s))
        .collect();
    let master_only_slots: Vec<usize> = master
        .var_slots
        .iter()
        .copied()
        .filter(|s| !trans.var_slots.contains(s))
        .collect();

    // Remise à missing d'un slot du PDV selon son type (utilisé pour les
    // variables présentes dans un seul des deux datasets).
    fn blank_slot(pdv: &mut Pdv, slot: usize) {
        let init = match pdv.vars()[slot].ty {
            VarType::Num => Value::missing(),
            VarType::Char => Value::Char(String::new()),
        };
        pdv.set(slot, init);
    }

    for (seq, item) in plan.iter().enumerate() {
        r.pdv.n_ += 1;
        r.pdv.error_ = false;
        r.pdv.reset_non_retained();
        match item {
            UpdatePlanItem::Master(m_row) => {
                for &slot in &trans_only_slots {
                    blank_slot(&mut r.pdv, slot);
                }
                load_row(&mut r.pdv, master, *m_row);
                master_read += 1;
            }
            UpdatePlanItem::NewTrans(t_row) => {
                for &slot in &master_only_slots {
                    blank_slot(&mut r.pdv, slot);
                }
                // Charger la transaction : clés PUIS variables overlay.
                for (&k, &pos) in upd.key_slots.iter().zip(&trans_key_pos) {
                    r.pdv.set(k, trans.columns[pos][*t_row].clone());
                }
                for &(slot, pos) in &overlay_pos {
                    r.pdv.set(slot, trans.columns[pos][*t_row].clone());
                }
            }
        }
        // FIRST./LAST. par variable BY (préfixe de clés vs voisins émis).
        if !upd.by.is_empty() {
            let cur = &plan_keys[seq];
            for (i, flags) in r.ctx.by_flags.iter_mut().enumerate() {
                let first = match seq.checked_sub(1) {
                    None => true,
                    Some(p) => prefix_changed(cur, &plan_keys[p], i),
                };
                let last = match plan_keys.get(seq + 1) {
                    None => true,
                    Some(next) => prefix_changed(cur, next, i),
                };
                flags.1 = first;
                flags.2 = last;
            }
        }
        if let UpdatePlanItem::Master(_) = item {
            // Superposer TOUTES les transactions de la clé, dans l'ordre.
            let key_vals: Vec<Value> = upd
                .key_slots
                .iter()
                .map(|&slot| r.pdv.get(slot).clone())
                .collect();
            if let Some(rows) = trans_index.get(&key_string(&key_vals)) {
                for &t_row in rows {
                    for &(slot, pos) in &overlay_pos {
                        let tv = &trans.columns[pos][t_row];
                        // Défaut SAS (MISSINGCHECK) : une valeur manquante
                        // de transaction n'a AUCUN effet ; NOMISSINGCHECK :
                        // elle écrase aussi le maître (LEPG 21.32).
                        if upd.nomissingcheck || !tv.is_missing() {
                            r.pdv.set(slot, tv.clone());
                        }
                    }
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
        session
            .log
            .note(&format!("Variable {name} is uninitialized."));
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
