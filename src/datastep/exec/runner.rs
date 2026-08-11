use super::*;

/// Écrit les sorties de hash en attente (M17.2) accumulées par
/// `h.output(dataset:)` vers leurs providers de bibliothèque. Appelée APRÈS la
/// boucle implicite (`&mut Session` disponible). Chaque sortie devient un
/// dataset (clés puis données, dans l'ordre de l'objet hash).
pub(super) fn flush_hash_outputs(r: &mut Runner, session: &mut Session) -> Result<()> {
    for out in std::mem::take(&mut r.ctx.hash_outputs) {
        let columns: Vec<Column> = out
            .vars
            .iter()
            .enumerate()
            .map(|(c, meta)| {
                column_from_values(&meta.name, meta.ty, out.rows.iter().map(|row| &row[c]))
            })
            .collect();
        let df = DataFrame::new(columns)?;
        let ds = SasDataset {
            df,
            vars: out.vars.clone(),
        };
        write_dataset_with_note(
            session,
            &out.libref,
            &out.table,
            &out.display,
            &ds,
            out.rows.len(),
            None,
        )?;
    }
    Ok(())
}

/// Matériel de construction d'un Runner UPDATE/MODIFY (M16.5), entièrement
/// issu de la décomposition du `StepProgram` par l'appelant.
pub(super) struct RunnerConfig {
    pub(super) pdv: Pdv,
    pub(super) outputs: Vec<OutputSpec>,
    pub(super) arrays: HashMap<String, crate::datastep::ArrayDef>,
    pub(super) labels: HashMap<String, String>,
}

/// Construit un Runner « squelette » pour les boucles UPDATE/MODIFY (M16.5),
/// avec le PDV, les arrays, les builders de sortie et l'instantané macro déjà
/// posés (catalogue de formats, symboles macro et YEARCUTOFF pris sur la
/// session). Les champs spécifiques au SET/MERGE/texte sont vides.
pub(super) fn build_um_runner(cfg: RunnerConfig, by: &[ByVar], session: &Session) -> Runner {
    let RunnerConfig {
        pdv,
        outputs,
        arrays,
        labels,
    } = cfg;
    let builders: Vec<Vec<ColBuilder>> = outputs
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
    let n_outputs = outputs.len();
    let by_flags = by.iter().map(|b| (b.name.clone(), true, true)).collect();
    Runner {
        pdv,
        input: None,
        text_io: TextIo::new(None),
        format_catalog: std::rc::Rc::clone(&session.format_catalog),
        set_cursor: SetCursor::new(0),
        rows_read: vec![0; 1],
        ctx: EvalCtx {
            arrays,
            by_flags,
            macro_symbols: session.macro_engine.symbols_snapshot(),
            format_catalog: std::rc::Rc::clone(&session.format_catalog),
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
    }
}

/// Charge la ligne `row` du dataset matérialisé `ds` dans le PDV (tous ses
/// slots). Downcast déjà fait à la compilation (colonnes décodées).
pub(super) fn load_row(pdv: &mut Pdv, ds: &InputDataset, row: usize) {
    for (col, slot) in ds.columns.iter().zip(&ds.var_slots) {
        pdv.set(*slot, col[row].clone());
    }
}

/// Clé d'appariement canonique d'une liste de `Value` (UPDATE/MODIFY KEY=).
/// Encode la sémantique d'égalité SAS : `. == .`, char insensible aux blancs
/// finaux. Sert de clé de `HashMap`.
pub(super) fn key_string(values: &[Value]) -> String {
    let mut s = String::new();
    for v in values {
        match v {
            Value::Num(n) => {
                s.push('N');
                s.push_str(&format!("{:?}", n));
            }
            Value::Missing(k) => {
                s.push('M');
                s.push_str(&k.display());
            }
            Value::Char(c) => {
                s.push('C');
                s.push_str(c.trim_end());
            }
        }
        s.push('\u{1}');
    }
    s
}

/// Émet les NOTEs d'erreurs/conversions accumulées par l'évaluateur + draine
/// CALL SYMPUT / hash outputs / CALL EXECUTE / PUT. Partagé par les trois
/// boucles d'exécution (principale, UPDATE, MODIFY) ; les ordres relatifs sont
/// sémantiques (le rejeu PUT précède les NOTEs de fin d'étape, cf. SAS).
pub(super) fn drain_runner_side_effects(r: &mut Runner, session: &mut Session) -> Result<()> {
    // CALL SYMPUT (M11.5) : drain des écritures différées vers la table
    // macro APRÈS le RUN de l'étape (règle de visibilité SAS — le symbole
    // n'est pas visible dans la même étape). Sous le build par défaut,
    // `set_symbol_global` est un no-op (l'engine identité n'a pas de table) :
    // `call symput` parse et s'exécute mais n'a aucun effet macro.
    for (name, value) in std::mem::take(&mut r.ctx.symput_writes) {
        session.macro_engine.set_symbol_global(&name, value);
    }
    // Hash output (M17.2) : drain des `h.output(dataset:)` accumulés vers les
    // providers de bibliothèque (où `&mut Session` est disponible). No-op sur
    // les chemins UPDATE/MODIFY (leur `EvalCtx` n'a pas d'objets hash, donc
    // `hash_outputs` y est toujours vide).
    flush_hash_outputs(r, session)?;
    // CALL EXECUTE (M15.6) : drain de la file de code généré pendant l'étape
    // vers la session. L'exécuteur le rejoue APRÈS le RUN de l'étape (fidèle à
    // SAS : les pas mis en file par CALL EXECUTE s'exécutent une fois l'étape
    // courante terminée). On préserve l'ordre d'accumulation.
    session
        .call_execute_queue
        .extend(std::mem::take(&mut r.call_execute_queue));
    // PUT (M14.2) : flush de la ligne maintenue en fin d'étape, puis rejeu
    // des lignes produites vers leurs destinations. Le rejeu a lieu AVANT les
    // NOTEs de fin d'étape (la sortie PUT « pendant » l'étape précède la NOTE
    // « N records were read »/« data set has N obs » dans le log SAS).
    r.put_flush_at_step_end();
    r.put_replay(session)?;
    // ERREURs non fatales collectées par l'évaluateur (M40.1 : pattern PRX
    // invalide) — rejouées AVANT les NOTEs de conversion.
    for msg in std::mem::take(&mut r.ctx.runtime_errors) {
        session.log.error(&msg);
    }
    // NOTEs d'erreurs/conversions collectées par l'évaluateur.
    if r.ctx.note_num_to_char {
        session
            .log
            .note("Numeric values have been converted to character values.");
    }
    if r.ctx.note_char_to_num {
        session
            .log
            .note("Character values have been converted to numeric values.");
    }
    if r.ctx.division_by_zero > 0 {
        session.log.note("Division by zero detected.");
    }
    if r.ctx.invalid_data > 0 {
        session.log.note("Invalid numeric data.");
    }
    if r.ctx.missing_generated > 0 {
        session.log.note(
            "Missing values were generated as a result of performing an operation on missing values.",
        );
    }
    Ok(())
}

/// Écrit les sorties DATA (ordre du statement DATA ; _LAST_ = la dernière) à
/// partir des builders du Runner. Partagé par les trois boucles d'exécution
/// (principale, UPDATE, MODIFY). Consomme `outputs`/`builders` (mem::take).
pub(super) fn write_runner_outputs(
    r: &mut Runner,
    session: &mut Session,
    stats: &mut StepStats,
) -> Result<()> {
    let outputs = std::mem::take(&mut r.outputs);
    let builders = std::mem::take(&mut r.builders);
    for ((spec, bset), n_out) in outputs.iter().zip(builders).zip(&r.out_rows) {
        let mut columns: Vec<Column> = Vec::with_capacity(spec.kept_slots.len());
        let mut vars: Vec<VarMeta> = Vec::with_capacity(spec.kept_slots.len());
        for ((slot, b), out_name) in spec.kept_slots.iter().zip(bset).zip(&spec.out_names) {
            let v = &r.pdv.vars()[*slot];
            // RENAME= de sortie : la colonne écrite porte `out_name` (le
            // slot PDV garde son nom).
            let series = match b {
                ColBuilder::Num(vals) => Series::new(out_name.as_str().into(), vals),
                ColBuilder::Char(vals) => Series::new(out_name.as_str().into(), vals),
            };
            columns.push(series.into());
            let label = r.labels.get(&v.name.to_uppercase()).cloned();
            vars.push(VarMeta {
                name: out_name.clone(),
                ty: v.ty,
                length: v.length,
                format: v.format.clone(),
                label,
            });
        }
        let df = DataFrame::new(columns)?;
        let ds = SasDataset { df, vars };
        write_dataset_with_note(
            session,
            &spec.libref,
            &spec.table,
            &spec.display,
            &ds,
            *n_out,
            Some(stats),
        )?;
    }
    Ok(())
}

/// Convertit une `Value` en la chaîne stockée par CALL SYMPUT (M11.5).
///
/// - Char : la valeur telle quelle (SAS ne rogne PAS la valeur d'un
///   symput ; on garde la chaîne du PDV avec ses blancs internes/finaux).
/// - Num : formaté en BEST12. puis CADRÉ À GAUCHE (les blancs de tête de
///   BEST12. sont supprimés). `call symput('x', 42)` donne `&x` = "42".
/// - Missing : le point/lettre du missing (cadrage à gauche d'un BEST12.).
pub(super) fn symput_string(value: Value) -> String {
    match value {
        Value::Char(s) => s,
        Value::Num(f) => format_best(f, 12),
        Value::Missing(k) => k.display(),
    }
}
