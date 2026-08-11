use super::*;

pub struct StepProgram {
    pub pdv: Pdv,
    pub stmts: Vec<crate::ast::DsStmt>,
    /// Entrée du PREMIER statement SET (site 0), ou du MERGE. `None` = pas
    /// de SET/MERGE dans l'étape.
    pub input: Option<InputData>,
    /// Sites SET supplémentaires (M40.2) : un `InputData` par statement SET
    /// au-delà du premier (index = site − 1, cf. `DsStmt::Set::site`).
    /// Chaque site a son propre curseur séquentiel à l'exécution ; jamais de
    /// BY / MERGE / POINT= (refusés à la compilation avec plusieurs SET).
    pub extra_inputs: Vec<InputData>,
    /// Entrée UPDATE (M16.5), exclusive de `input`/`text_input`/`modify`.
    pub update: Option<UpdateData>,
    /// Entrée MODIFY (M16.5), exclusive de `input`/`text_input`/`update`.
    pub modify: Option<ModifyData>,
    /// Source d'entrée TEXTE (M14 : INFILE/INPUT/DATALINES), parallèle à
    /// `input` (SET). Une étape ne peut avoir QUE l'un des deux.
    pub text_input: Option<TextInput>,
    pub outputs: Vec<OutputSpec>,
    pub has_explicit_output: bool,
    /// Noms (casse de première référence, ordre PDV) des variables jamais
    /// assignées ni lues d'un input : l'exécuteur émet la NOTE
    /// "Variable x is uninitialized." à la première itération.
    pub uninitialized: Vec<String>,
    /// Valeurs initiales (slot, valeur) issues de RETAIN avec init et des
    /// sum statements (0) : l'exécuteur les applique via `pdv.set` AVANT la
    /// première itération (la troncature char s'applique donc normalement).
    /// Appliquées dans l'ordre — une entrée ultérieure pour le même slot
    /// gagne (cas `n + 1; retain n 100;` : le RETAIN l'emporte).
    pub initial_values: Vec<(usize, Value)>,
    /// Arrays : nom UPPERCASE → définition (slots + dimensions). Passé tel
    /// quel à l'EvalCtx par l'exécuteur.
    pub arrays: HashMap<String, ArrayDef>,
    /// Libellés déclarés (LABEL/ATTRIB) : nom UPPERCASE → libellé.
    /// Appliqués aux `VarMeta` de sortie par l'exécuteur.
    pub labels: HashMap<String, String>,
    /// Étiquettes de contrôle (M16.6) : nom UPPERCASE → index dans `stmts`
    /// (niveau supérieur de l'étape). Cibles des GOTO/LINK, résolues à la
    /// compilation. L'exécuteur pilote un compteur de programme sur `stmts`.
    pub flow_labels: HashMap<String, usize>,
    /// Objets hash déclarés (M17.1) : nom UPPERCASE → objet initial (options
    /// résolues, sans clés/données ni lignes). L'exécuteur les copie dans
    /// `EvalCtx.hashes` au début de l'étape ; defineKey/defineData/defineDone
    /// les remplissent à l'exécution.
    pub hash_objects: HashMap<String, HashObject>,
    /// Itérateurs de hash déclarés (M17.2) : nom UPPERCASE → itérateur
    /// (objet lié + position). L'exécuteur les copie dans `EvalCtx.hash_iters`.
    pub hash_iters: HashMap<String, HashIter>,
}

pub fn compile(ast: &DataStepAst, session: &mut Session) -> Result<StepProgram> {
    let mut c = Compiler {
        pdv: Pdv::new(),
        session,
        input_datasets: Vec::new(),
        seen_set: false,
        set_options: crate::ast::SetOptions::default(),
        seen_merge: false,
        in_flags: Vec::new(),
        by: None,
        first_last_refs: Vec::new(),
        keeps: Vec::new(),
        drops: Vec::new(),
        output_displays: ast.outputs.iter().map(|s| s.display()).collect(),
        assigned: HashSet::new(),
        has_explicit_output: false,
        retain_all: false,
        retain_pending: Vec::new(),
        retained_slots: HashSet::new(),
        initial_values: Vec::new(),
        arrays: HashMap::new(),
        labels: HashMap::new(),
        formats: HashMap::new(),
        informats: HashMap::new(),
        where_stmt: None,
        infile: None,
        datalines: None,
        seen_input: false,
        do_over_arrays: HashSet::new(),
        update: None,
        modify: None,
        labels_defined: HashSet::new(),
        goto_link_refs: Vec::new(),
        hash_objects: HashMap::new(),
        hash_iters: HashMap::new(),
        extra_set_sites: Vec::new(),
    };
    // M40.2 — numérote chaque statement SET (pré-ordre textuel) : chaque SET
    // est un SITE DE LECTURE indépendant. Le n° stampé relie la compilation
    // (InputData du site) à l'exécution (curseur du site) ; on walke ensuite
    // l'arbre STAMPÉ, qui est aussi celui livré au Runner.
    let mut stmts = ast.stmts.clone();
    stamp_set_sites(&mut stmts, &mut 0);
    // M40.3 — pré-passe déclarative : collecte les informats des statements
    // INFORMAT/ATTRIB informat= de toute l'étape (l'ordre statement/INPUT
    // n'importe pas, comme en SAS), puis les injecte dans les items INPUT en
    // mode liste sans informat explicite. Le walk (et l'exécution) voient
    // ensuite des items INPUT ordinaires — type/longueur et lecture passent
    // par le chemin existant de l'informat explicite.
    c.collect_informats(&stmts)?;
    if !c.informats.is_empty() {
        apply_declared_informats(&mut stmts, &c.informats);
    }
    for stmt in &stmts {
        c.walk_stmt(stmt)?;
    }
    // FORMAT/ATTRIB format= : appliqués au PDV maintenant que TOUTES les
    // variables y sont entrées (l'ordre déclaration/référence n'importe
    // plus). Variable inconnue → ignorée (SIMPLIFICATION M4 documentée).
    let formats = std::mem::take(&mut c.formats);
    for (name, token) in &formats {
        if let Some(slot) = c.pdv.slot(name) {
            c.pdv.set_format(slot, token.clone());
        }
    }

    // RETAIN sans valeur initiale — SIMPLIFICATION M2 ASSUMÉE : en vrai
    // SAS, `retain x;` ne fige PAS le type — la variable le prend à sa
    // prochaine référence. Pour approcher ça sans bouleverser la passe
    // unique, le statement n'a fait que mémoriser le nom ; ICI (fin de
    // compilation) on applique le flag `retained` à la variable, qui doit
    // alors exister (créée par une autre référence). Sinon on la crée Num
    // + uninitialized — elle arrive donc en FIN d'ordre PDV (divergence
    // mineure assumée par rapport à l'ordre de première référence SAS).
    let pending = std::mem::take(&mut c.retain_pending);
    for name in &pending {
        let slot = match c.pdv.slot(name) {
            Some(slot) => slot,
            None => c.add_var(name, VarType::Num, 8),
        };
        c.retained_slots.insert(slot);
    }
    // `retain;` seul — SIMPLIFICATION M2 : retient TOUT le PDV (en vrai
    // SAS, seulement ce qui est connu au point du statement).
    if c.retain_all {
        c.retained_slots.extend(0..c.pdv.vars().len());
    }
    // Le PDV ne permet pas de modifier une variable existante (première
    // référence fige tout, et `pdv.rs` n'expose pas de mutateur) : on le
    // reconstruit à l'identique en appliquant les flags `retained`. Les
    // slots sont préservés (même ordre d'insertion) et aucune valeur n'a
    // encore été posée à la compilation.
    if !c.retained_slots.is_empty() {
        c.pdv = rebuild_with_retained(&c.pdv, &c.retained_slots);
    }

    let (input, extra_inputs) = c.build_input()?;
    let update = c.build_update()?;
    let modify = c.build_modify()?;
    let text_input = c.build_text_input()?;
    // Une étape ne peut pas mélanger plusieurs sources d'entrée concurrentes.
    let n_sources = [
        input.is_some(),
        update.is_some(),
        modify.is_some(),
        text_input.is_some(),
    ]
    .iter()
    .filter(|b| **b)
    .count();
    if n_sources > 1 {
        return Err(SasError::runtime(
            "Mixing SET/UPDATE/MODIFY with INFILE/INPUT in the same step is not yet implemented.",
        ));
    }
    // MODIFY interdit l'OUTPUT explicite (les valeurs sont écrites en place).
    if modify.is_some() && c.has_explicit_output {
        return Err(SasError::runtime(
            "The OUTPUT statement is not allowed with the MODIFY statement.",
        ));
    }
    // Étiquettes de contrôle (M16.6) : index des `DsStmt::Labeled` AU NIVEAU
    // SUPÉRIEUR de l'étape. GOTO/LINK ne ciblent QUE des étiquettes de premier
    // niveau (sauter DANS un bloc DO est indéfini en SAS et non supporté). Une
    // étiquette définie uniquement dans un bloc imbriqué n'est donc pas une
    // cible valide.
    let mut flow_labels: HashMap<String, usize> = HashMap::new();
    for (i, stmt) in stmts.iter().enumerate() {
        if let DsStmt::Labeled { name, .. } = stmt {
            flow_labels.insert(name.to_uppercase(), i);
        }
    }
    // Validation des références GOTO/LINK : la cible doit être une étiquette de
    // premier niveau. Inconnue (ou seulement imbriquée) → erreur de compilation.
    for label in &c.goto_link_refs {
        if !flow_labels.contains_key(label) {
            if c.labels_defined.contains(label) {
                return Err(SasError::runtime(format!(
                    "The label {label} is nested inside a block and cannot be a GOTO/LINK target."
                )));
            }
            return Err(SasError::runtime(format!(
                "The statement label {label} is not defined in the DATA step."
            )));
        }
    }

    let outputs = c.resolve_outputs(&ast.outputs)?;
    let uninitialized = c
        .pdv
        .vars()
        .iter()
        .filter(|v| !v.from_input && !v.temporary && !c.assigned.contains(&v.name.to_uppercase()))
        .map(|v| v.name.clone())
        .collect();
    Ok(StepProgram {
        pdv: c.pdv,
        stmts,
        input,
        extra_inputs,
        update,
        modify,
        text_input,
        outputs,
        has_explicit_output: c.has_explicit_output,
        uninitialized,
        initial_values: c.initial_values,
        arrays: c.arrays,
        labels: c.labels,
        flow_labels,
        hash_objects: c.hash_objects,
        hash_iters: c.hash_iters,
    })
}

/// M40.2 — pose le n° de SITE sur chaque `DsStmt::Set` de l'étape, en
/// pré-ordre textuel (le 1ᵉʳ SET rencontré = site 0). Ne descend que dans
/// les variantes qui portent des statements enfants (un SET peut vivre dans
/// un IF/DO/SELECT — `if _n_ = 1 then set totals;` est un idiome SAS).
fn stamp_set_sites(stmts: &mut [DsStmt], next: &mut usize) {
    for stmt in stmts {
        stamp_set_site(stmt, next);
    }
}

fn stamp_set_site(stmt: &mut DsStmt, next: &mut usize) {
    match stmt {
        DsStmt::Set { site, .. } => {
            *site = *next;
            *next += 1;
        }
        DsStmt::If {
            then_branch,
            else_branch,
            ..
        } => {
            stamp_set_site(then_branch, next);
            if let Some(e) = else_branch {
                stamp_set_site(e, next);
            }
        }
        DsStmt::Block(body)
        | DsStmt::DoLoop { body, .. }
        | DsStmt::DoList { body, .. }
        | DsStmt::DoOver { body, .. } => stamp_set_sites(body, next),
        DsStmt::Select {
            whens, otherwise, ..
        } => {
            for w in whens {
                stamp_set_site(&mut w.body, next);
            }
            if let Some(o) = otherwise {
                stamp_set_site(o, next);
            }
        }
        DsStmt::Labeled { stmt, .. } => stamp_set_site(stmt, next),
        _ => {}
    }
}

/// M40.3 — injecte les informats déclarés (INFORMAT / ATTRIB informat=) dans
/// les items INPUT en MODE LISTE sans informat explicite : l'item devient une
/// lecture « modified list input » (jeton délimité puis conversion par
/// l'informat, exactement comme `:informat.` — la largeur ne borne pas le
/// scan). L'informat explicite d'un item INPUT l'emporte toujours sur le
/// statement ; le mode colonne (`a-b`) est laissé tel quel (lecture standard,
/// divergence documentée). Comme les statements déclaratifs, la descente
/// couvre les INPUT nichés dans IF/DO/SELECT.
fn apply_declared_informats(
    stmts: &mut [DsStmt],
    informats: &std::collections::HashMap<String, String>,
) {
    for stmt in stmts {
        apply_declared_informats_stmt(stmt, informats);
    }
}

fn apply_declared_informats_stmt(
    stmt: &mut DsStmt,
    informats: &std::collections::HashMap<String, String>,
) {
    match stmt {
        DsStmt::Input(items) => {
            for item in items {
                if let crate::ast::InputItem::Var {
                    name,
                    cols: None,
                    informat: informat @ None,
                    list_modifier,
                    ..
                } = item
                    && let Some(token) = informats.get(&name.to_uppercase())
                {
                    *informat = Some(token.clone());
                    *list_modifier = true;
                }
            }
        }
        DsStmt::If {
            then_branch,
            else_branch,
            ..
        } => {
            apply_declared_informats_stmt(then_branch, informats);
            if let Some(e) = else_branch {
                apply_declared_informats_stmt(e, informats);
            }
        }
        DsStmt::Block(body)
        | DsStmt::DoLoop { body, .. }
        | DsStmt::DoList { body, .. }
        | DsStmt::DoOver { body, .. } => apply_declared_informats(body, informats),
        DsStmt::Select {
            whens, otherwise, ..
        } => {
            for w in whens {
                apply_declared_informats_stmt(&mut w.body, informats);
            }
            if let Some(o) = otherwise {
                apply_declared_informats_stmt(o, informats);
            }
        }
        DsStmt::Labeled { stmt, .. } => apply_declared_informats_stmt(stmt, informats),
        _ => {}
    }
}
