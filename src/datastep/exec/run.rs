use super::*;

pub fn execute(prog: StepProgram, session: &mut Session) -> Result<StepStats> {
    // Fast-path vectorisé OPTIONNEL (OFF par défaut). Ne s'active que pour les
    // étapes prouvées équivalentes ET une fenêtre d'entrée pleine
    // (FIRSTOBS=1 / OBS=MAX) ; sinon on garde la boucle ligne-à-ligne.
    if session.vectorize
        && session.options.firstobs == 1
        && session.options.obs.is_none()
        && crate::datastep::fastpath::eligible(&prog)
    {
        return crate::datastep::fastpath::run(prog, session);
    }

    // UPDATE/MODIFY (M16.5) ont leur propre boucle d'exécution (sémantique
    // distincte du SET/MERGE) : on les détourne avant la boucle implicite
    // générique.
    if prog.update.is_some() {
        return execute_update(prog, session);
    }
    if prog.modify.is_some() {
        return execute_modify(prog, session);
    }

    let StepProgram {
        pdv,
        stmts,
        input,
        update: _,
        modify: _,
        text_input,
        outputs,
        has_explicit_output,
        uninitialized,
        initial_values,
        arrays,
        labels,
        flow_labels,
        hash_objects,
        hash_iters,
    } = prog;

    for name in &uninitialized {
        session.log.note(&format!("Variable {name} is uninitialized."));
    }

    let builders = outputs
        .iter()
        .map(|o| {
            o.kept_slots
                .iter()
                .map(|s| match pdv.vars()[*s].ty {
                    VarType::Num => ColBuilder::Num(Vec::new()),
                    VarType::Char => ColBuilder::Char(Vec::new()),
                })
                .collect()
        })
        .collect();

    // Une étape avec une source texte (INFILE/INPUT) boucle comme un SET ;
    // sans aucune source (ni SET ni texte) elle ne tourne qu'une fois.
    let single_iteration = input.is_none() && text_input.is_none();
    let n_rows: usize = input
        .as_ref()
        .map_or(0, |i| i.datasets.iter().map(|d| d.n_rows).sum());
    // Garde-fou anti-boucle infinie pour la source texte.
    let n_text_lines = text_input.as_ref().map_or(0, |t| t.lines.len());
    let n_datasets = input.as_ref().map_or(0, |i| i.datasets.len());
    // FIRST./LAST. valent 1 tant qu'aucune observation n'a été servie.
    let by_flags = input
        .as_ref()
        .map_or(Vec::new(), |i| {
            i.by.iter().map(|b| (b.name.clone(), true, true)).collect()
        });
    // IN= : initialisées à 0 (aucun groupe encore servi).
    let in_flags = input.as_ref().map_or(Vec::new(), |i| {
        i.in_flags
            .iter()
            .map(|(name, _)| (name.clone(), false))
            .collect()
    });
    // END= (M16.4) : variable automatique 0/1, initialisée à 0.
    let end_flag = input
        .as_ref()
        .and_then(|i| i.end_var.as_ref().map(|n| (n.clone(), 0.0)));
    // POINT= (M16.4) : si présent, la boucle implicite est REMPLACÉE par un
    // contrôle manuel (pas d'avance de curseur automatique, pas d'output
    // implicite, pas de fin d'étape à l'épuisement). On mémorise le slot.
    let point_slot = input.as_ref().and_then(|i| i.point_slot);
    // NOBS= (M16.4) : slot + total d'observations (somme des datasets).
    let nobs = input.as_ref().and_then(|i| {
        i.nobs_slot
            .map(|slot| (slot, i.datasets.iter().map(|d| d.n_rows).sum::<usize>()))
    });
    let n_outputs = outputs.len();
    // SYMGET (M11.5) : instantané de la table macro pris AU DÉBUT de
    // l'étape. Sous la feature `macros` il porte les `%let`/symput
    // antérieurs ; sous le build par défaut il est vide.
    let macro_symbols = session.macro_engine.symbols_snapshot();

    let mut r = Runner {
        pdv,
        input,
        text_io: TextIo::new(text_input),
        format_catalog: session.format_catalog.clone(),
        set_cursor: SetCursor::new(n_datasets),
        rows_read: vec![0; n_datasets],
        ctx: EvalCtx {
            arrays,
            by_flags,
            in_flags,
            end_flag,
            macro_symbols,
            hashes: hash_objects,
            hash_iters,
            format_catalog: session.format_catalog.clone(),
            yearcutoff: session.options.yearcutoff,
            ..EvalCtx::default()
        },
        outputs,
        builders,
        out_rows: vec![0; n_outputs],
        merge: MergeState::new(),
        labels,
        call_execute_queue: Vec::new(),
        modify_state: None,
        program: std::rc::Rc::new(Vec::new()),
        flow_labels: std::rc::Rc::new(HashMap::new()),
    };

    // Interclassement / match-merge : pré-application des WHERE= par dataset
    // (les lignes rejetées ne comptent pas comme lues), AVANT la boucle. Les
    // NOTEs de conversion d'un WHERE= peuvent donc être émises pour des
    // lignes jamais atteintes (STOP précoce) — divergence mineure assumée.
    if r.input.as_ref().is_some_and(|i| !i.by.is_empty()) {
        r.prefilter()?;
    }
    // MERGE : pré-calcul de la séquence des obs de sortie (groupe par
    // groupe), à partir des lignes retenues par le pré-filtrage. La
    // détection de désordre y est faite (clé de groupe qui régresse →
    // ERROR).
    if r.input.as_ref().is_some_and(|i| i.merge) {
        r.merge.plan = r.build_merge_plan()?;
    }

    // Valeurs initiales (RETAIN avec init, sum statements) : posées AVANT
    // la première itération via `pdv.set` (la troncature char des inits
    // trop longues s'applique donc normalement). Ces slots sont retenus,
    // `reset_non_retained` ne les touchera jamais. Une entrée ultérieure
    // pour le même slot écrase la précédente (le RETAIN gagne sur le 0
    // implicite d'un sum statement).
    for (slot, v) in initial_values {
        r.pdv.set(slot, v);
    }

    // NOBS= (M16.4) : affectée AVANT la boucle (disponible dès la 1re
    // itération, p.ex. `do i = 1 to n;`). Slot retenu ⇒ persiste.
    if let Some((slot, total)) = nobs {
        r.pdv.set(slot, Value::Num(total as f64));
    }

    // POINT= (M16.4) : l'output implicite est SUPPRIMÉ (SAS exige un OUTPUT
    // explicite), et la boucle ne se termine pas sur épuisement d'entrée
    // (c'est l'utilisateur qui pilote l'itération via DO/STOP).
    let suppress_implicit_output = has_explicit_output || point_slot.is_some();

    // M16.6 : programme + étiquettes partagés avec le Runner (LINK exécuté
    // inline, GOTO résolu par `run_step_body`).
    r.program = std::rc::Rc::new(stmts);
    r.flow_labels = std::rc::Rc::new(flow_labels);

    loop {
        r.pdv.n_ += 1;
        r.pdv.error_ = false;
        r.pdv.reset_non_retained();
        // Hold de ligne (M14) : un `@` simple est relâché au DÉBUT de
        // l'itération suivante (le prochain INPUT lira un nouvel
        // enregistrement) ; un `@@` survit.
        if let Some(h) = &r.text_io.held {
            if !h.double {
                r.text_io.held = None;
            }
        }
        // Hold de ligne PUT (M14.2) : un `@` simple relâche la ligne au DÉBUT
        // de l'itération suivante (flush + clear) ; un `@@` la conserve.
        if r.text_io.put.hold && !r.text_io.put.hold_double {
            r.put_release_line();
        }

        let flow = r.run_step_body()?;
        if flow == Flow::EndStep {
            break;
        }
        if flow != Flow::NextIter && !suppress_implicit_output {
            r.push_outputs();
        }
        if single_iteration {
            break;
        }
        // Garde-fou anti-boucle infinie (cf. en-tête).
        if r.pdv.n_ as usize > n_rows + n_text_lines + 10_000 {
            return Err(SasError::runtime(
                "DATA step appears to loop infinitely (no input rows consumed); stopping.",
            ));
        }
    }

    // Test-only (M17.1) : expose l'état final des objets hash à la session
    // pour inspection unitaire (keys/data_vars/options/defined). En production
    // ce bloc n'existe pas.
    #[cfg(test)]
    {
        session.debug_hashes = r.ctx.hashes.clone();
    }

    // Drains post-boucle (symput, hash outputs, call execute, PUT) + NOTEs
    // d'erreurs/conversions — partagés avec les boucles UPDATE/MODIFY.
    drain_runner_side_effects(&mut r, session)?;

    let mut stats = StepStats {
        read: Vec::new(),
        written: Vec::new(),
    };
    if let Some(input) = &r.input {
        // Avec WHERE=, seules les lignes qui PASSENT comptent comme lues
        // (fidèle à la NOTE SAS). Une NOTE par dataset, dans l'ordre du
        // statement SET.
        for (ds, n) in input.datasets.iter().zip(&r.rows_read) {
            session.log.note(&format!(
                "There were {} observations read from the data set {}.",
                n, ds.display
            ));
            stats.read.push((ds.display.clone(), *n));
        }
    }
    // Source texte (M14) : NOTE "N records were read from the infile ..."
    // UNIQUEMENT pour un fichier externe. Pour les données instream
    // DATALINES/CARDS, SAS n'émet aucune NOTE de ce type (elle est réservée
    // aux fichiers physiques).
    if let Some(text) = &r.text_io.src {
        if text.is_file {
            session.log.note(&format!(
                "{} records were read from {}.",
                r.text_io.read, text.display
            ));
            stats.read.push((text.display.clone(), r.text_io.read));
        }
    }

    // Écriture des sorties (ordre du statement DATA ; _LAST_ = la dernière).
    write_runner_outputs(&mut r, session, &mut stats)?;

    Ok(stats)
}

/// Colonne Polars construite depuis des `Value` selon le type SAS déclaré :
/// numérique via `value_to_num` (missings → null), caractère tel quel (toute
/// cellule non-Char devient chaîne vide).
pub(crate) fn column_from_values<'a>(
    name: &str,
    ty: VarType,
    vals: impl Iterator<Item = &'a Value>,
) -> Column {
    match ty {
        VarType::Num => {
            let nums: Vec<Option<f64>> = vals.map(value_to_num).collect();
            Series::new(name.into(), nums).into()
        }
        VarType::Char => {
            let strs: Vec<String> = vals
                .map(|v| match v {
                    Value::Char(s) => s.clone(),
                    _ => String::new(),
                })
                .collect();
            Series::new(name.into(), strs).into()
        }
    }
}

/// Écrit `ds` dans sa bibliothèque, met à jour `_LAST_` et émet la NOTE
/// « The data set X has N observations and M variables. » ; enregistre la
/// ligne dans `stats.written` quand un `StepStats` est fourni.
pub(crate) fn write_dataset_with_note(
    session: &mut Session,
    libref: &str,
    table: &str,
    display: &str,
    ds: &SasDataset,
    n_obs: usize,
    stats: Option<&mut StepStats>,
) -> Result<()> {
    session.libs.get(libref)?.write(table, ds)?;
    session.last_dataset = Some(display.to_string());
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display,
        n_obs,
        ds.vars.len()
    ));
    if let Some(stats) = stats {
        stats.written.push((display.to_string(), n_obs, ds.vars.len()));
    }
    Ok(())
}
