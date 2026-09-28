use super::*;

/// Common variable (by name, case-insensitive), with type match analysis.
pub(super) struct CommonVar {
    pub(super) name: String,
    pub(super) base_idx: usize,
    pub(super) comp_idx: usize,
    pub(super) type_match: bool,
    pub(super) base_type: VarType,
    pub(super) comp_type: VarType,
    /// Attributs divergents (LENGTH/LABEL/FORMAT → bits &SYSINFO 16/32/8).
    pub(super) length_diff: bool,
    pub(super) label_diff: bool,
    pub(super) format_diff: bool,
}

/// Paire comparée : variable BASE (VAR) ↔ variable COMPARE (WITH ou même
/// nom). `out_name` est le nom de la VAR (doc : « the names of the
/// variables in the OUT= data set come from the VAR statement »).
pub(super) struct VarPair {
    pub(super) out_name: String,
    pub(super) base_idx: usize,
    pub(super) comp_idx: usize,
    pub(super) var_type: VarType,
}

/// For each compared pair: count of unequal judgments and max |y−x| (numeric).
pub(super) struct VarDiffSummary {
    pub(super) name: String,
    pub(super) var_type: VarType,
    pub(super) n_diffs: usize,
    pub(super) max_diff: f64, // only for numeric
}

/// Une paire d'observations appariées (indices de lignes dans BASE/COMPARE).
pub(super) struct PairMatch {
    pub(super) base_idx: usize,
    pub(super) comp_idx: usize,
    /// Numéro de séquence de la paire dans son groupe BY (1-based) —
    /// c'est la valeur de _OBS_ pour les lignes DIF/PERCENT.
    pub(super) group_seq: usize,
    /// Vrai si au moins une paire de variables est jugée inégale.
    pub(super) unequal: bool,
}

/// Bits &SYSINFO (doc SAS 9.4, « Macro Return Codes ») — constantes
/// nommées d'après le tableau officiel.
pub(super) mod sysinfo_bits {
    pub const FORMAT: u64 = 8;
    pub const LENGTH: u64 = 16;
    pub const LABEL: u64 = 32;
    pub const BASEOBS: u64 = 64;
    pub const COMPOBS: u64 = 128;
    pub const BASEBY: u64 = 256;
    pub const COMPBY: u64 = 512;
    pub const BASEVAR: u64 = 1024;
    pub const COMPVAR: u64 = 2048;
    pub const VALUE: u64 = 4096;
    pub const TYPE: u64 = 8192;
    pub const BYVAR: u64 = 16384;
}

/// Résultat complet de la comparaison (appariement + jugement + &SYSINFO).
pub(super) struct ComparisonOutcome {
    pub(super) pairs: Vec<VarPair>,
    pub(super) matches: Vec<PairMatch>,
    pub(super) var_diffs: Vec<VarDiffSummary>,
    /// Lignes BASE sans appariement (ID/BY/position) — 1-based pour la log.
    pub(super) base_only_obs: Vec<usize>,
    pub(super) comp_only_obs: Vec<usize>,
    pub(super) sysinfo: u64,
}

/// Read one input dataset (BASE= or COMPARE=), forwarding provider notes.
pub(super) fn read_input(session: &mut Session, dsref: &DatasetRef) -> Result<SasDataset> {
    let libref = dsref.libref_or_work();
    let name = dsref.name.to_uppercase();
    let provider = session.libs.get(&libref)?;
    let (ds, notes) = provider.read(&name)?;
    for note in notes {
        session.log.forward(&note);
    }
    Ok(ds)
}

/// Variable analysis: names only in BASE, only in COMPARE, and the sorted
/// common-variable list (with per-variable type/attribute match).
pub(super) fn analyze_variables(
    base_ds: &SasDataset,
    comp_ds: &SasDataset,
) -> (Vec<String>, Vec<String>, Vec<CommonVar>) {
    // Build maps: name → (index, VarMeta) for each dataset
    let base_var_map: HashMap<String, usize> = base_ds
        .vars
        .iter()
        .enumerate()
        .map(|(i, v)| (v.name.to_uppercase(), i))
        .collect();
    let comp_var_map: HashMap<String, usize> = comp_ds
        .vars
        .iter()
        .enumerate()
        .map(|(i, v)| (v.name.to_uppercase(), i))
        .collect();

    // Variables only in BASE
    let mut only_base: Vec<String> = base_ds
        .vars
        .iter()
        .filter(|v| !comp_var_map.contains_key(&v.name.to_uppercase()))
        .map(|v| v.name.to_uppercase())
        .collect();
    only_base.sort();

    // Variables only in COMPARE
    let mut only_comp: Vec<String> = comp_ds
        .vars
        .iter()
        .filter(|v| !base_var_map.contains_key(&v.name.to_uppercase()))
        .map(|v| v.name.to_uppercase())
        .collect();
    only_comp.sort();

    let mut common_vars: Vec<CommonVar> = base_ds
        .vars
        .iter()
        .enumerate()
        .filter_map(|(bi, bv)| {
            let uname = bv.name.to_uppercase();
            comp_var_map.get(&uname).map(|&ci| {
                let cv = &comp_ds.vars[ci];
                CommonVar {
                    name: uname,
                    base_idx: bi,
                    comp_idx: ci,
                    type_match: bv.ty == cv.ty,
                    base_type: bv.ty,
                    comp_type: cv.ty,
                    length_diff: bv.length != cv.length,
                    label_diff: bv.label != cv.label,
                    format_diff: bv.format != cv.format,
                }
            })
        })
        .collect();
    common_vars.sort_by(|a, b| a.name.cmp(&b.name));

    (only_base, only_comp, common_vars)
}

/// Résolution des variables BY/ID dans chaque dataset : (indices base,
/// indices comp). Une variable absente d'un côté rend l'appariement
/// impossible pour cette variable (bit BYVAR / ERROR d'ID posé par
/// l'appelant).
fn resolve_columns(
    base_ds: &SasDataset,
    comp_ds: &SasDataset,
    names: &[(String, bool)],
) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let find = |ds: &SasDataset, name: &str| {
        ds.vars
            .iter()
            .position(|v| v.name.eq_ignore_ascii_case(name))
    };
    let mut b = Vec::with_capacity(names.len());
    let mut c = Vec::with_capacity(names.len());
    for (name, _) in names {
        b.push(find(base_ds, name));
        c.push(find(comp_ds, name));
    }
    (b, c)
}

/// Clé d'appariement d'une ligne : valeurs des variables BY (ou ID).
fn row_key(ds: &SasDataset, cols: &[Option<usize>], row: usize) -> Vec<Value> {
    cols.iter()
        .map(|&c| match c {
            Some(i) => get_value_at(&ds.df, i, row, ds.vars[i].ty),
            None => Value::missing(),
        })
        .collect()
}

/// Vrai si deux clés sont égales (sémantique sas_cmp : char trim trailing).
fn keys_equal(a: &[Value], b: &[Value]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.sas_cmp(y) == Ordering::Equal)
}

/// Appariement d'un groupe : par ID (clé triée) si des variables ID sont
/// résolues, sinon par position. Rend (paires, lignes base seules, lignes
/// comp seules) — indices 0-based.
fn match_group(
    base_ds: &SasDataset,
    comp_ds: &SasDataset,
    base_rows: &[usize],
    comp_rows: &[usize],
    id_base: &[Option<usize>],
    id_comp: &[Option<usize>],
) -> (Vec<(usize, usize)>, Vec<usize>, Vec<usize>) {
    let has_id = id_base.iter().any(|c| c.is_some()) && id_comp.iter().any(|c| c.is_some());
    if !has_id {
        // Position : les min(n,m) premières lignes se correspondent.
        let n = base_rows.len().min(comp_rows.len());
        let pairs = base_rows[..n]
            .iter()
            .copied()
            .zip(comp_rows[..n].iter().copied())
            .collect();
        return (pairs, base_rows[n..].to_vec(), comp_rows[n..].to_vec());
    }

    // ID : tri stable par clé, puis fusion.
    let mut b: Vec<(Vec<Value>, usize)> = base_rows
        .iter()
        .map(|&r| (row_key(base_ds, id_base, r), r))
        .collect();
    let mut c: Vec<(Vec<Value>, usize)> = comp_rows
        .iter()
        .map(|&r| (row_key(comp_ds, id_comp, r), r))
        .collect();
    b.sort_by(|x, y| cmp_keys(&x.0, &y.0));
    c.sort_by(|x, y| cmp_keys(&x.0, &y.0));

    let mut pairs = Vec::new();
    let mut only_b = Vec::new();
    let mut only_c = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < b.len() && j < c.len() {
        match cmp_keys(&b[i].0, &c[j].0) {
            Ordering::Equal => {
                pairs.push((b[i].1, c[j].1));
                i += 1;
                j += 1;
            }
            Ordering::Less => {
                only_b.push(b[i].1);
                i += 1;
            }
            Ordering::Greater => {
                only_c.push(c[j].1);
                j += 1;
            }
        }
    }
    only_b.extend(b[i..].iter().map(|x| x.1));
    only_c.extend(c[j..].iter().map(|x| x.1));
    (pairs, only_b, only_c)
}

/// Ordre total sur les clés (missing < nombres < chaînes, comme sas_cmp).
fn cmp_keys(a: &[Value], b: &[Value]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        let ord = x.sas_cmp(y);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

/// Jugement d'une paire de valeurs selon METHOD=/CRITERION= (doc SAS 9.4) :
/// ABSOLUTE : inégales si |y−x| > CRITERION ;
/// RELATIVE : inégales si |(y−x)/x| > CRITERION (x=0 → absolu) ;
/// PERCENT  : inégales si |100(y−x)/x| > CRITERION (x=0 → toujours) ;
/// EXACT    : égalité stricte (CRITERION ignoré).
/// Les caractères et les missings relèvent toujours de l'égalité stricte.
pub(super) fn judged_unequal(
    b: &Value,
    c: &Value,
    method: CmpMethod,
    criterion: f64,
    var_type: VarType,
) -> bool {
    if b.sas_cmp(c) == Ordering::Equal {
        return false;
    }
    if var_type != VarType::Num {
        return true;
    }
    let (Value::Num(x), Value::Num(y)) = (b, c) else {
        return true;
    };
    match method {
        CmpMethod::Exact => true,
        CmpMethod::Absolute => (y - x).abs() > criterion,
        CmpMethod::Relative => {
            if *x == 0.0 {
                (y - x).abs() > criterion
            } else {
                ((y - x) / x).abs() > criterion
            }
        }
        CmpMethod::Percent => {
            if *x == 0.0 {
                true
            } else {
                (100.0 * (y - x) / x).abs() > criterion
            }
        }
    }
}

/// Comparison complète : résolution VAR/WITH, appariement BY/ID/position,
/// jugement par paire, bits &SYSINFO.
pub(super) fn compare_datasets(
    ast: &CompareAst,
    base_ds: &SasDataset,
    comp_ds: &SasDataset,
    common_vars: &[CommonVar],
) -> Result<ComparisonOutcome> {
    let mut sysinfo: u64 = 0;

    // ── Paires de variables comparées (VAR/WITH ou common var même type) ──
    // Les variables BY et ID sont exclues : elles servent à l'appariement,
    // pas à la comparaison (doc SAS, « Output Data Set (OUT=) »).
    let aux_names: Vec<String> = ast
        .by
        .iter()
        .map(|(n, _)| n.to_uppercase())
        .chain(ast.id.iter().map(|n| n.to_uppercase()))
        .collect();
    let pairs: Vec<VarPair> = if ast.var.is_empty() {
        common_vars
            .iter()
            .filter(|cv| cv.type_match && !aux_names.contains(&cv.name))
            .map(|cv| VarPair {
                out_name: cv.name.clone(),
                base_idx: cv.base_idx,
                comp_idx: cv.comp_idx,
                var_type: cv.base_type,
            })
            .collect()
    } else {
        let with_names: Vec<String> = if ast.with.is_empty() {
            ast.var.clone()
        } else {
            ast.with.clone()
        };
        let mut out = Vec::with_capacity(ast.var.len());
        for (v, w) in ast.var.iter().zip(&with_names) {
            let bidx = base_ds
                .vars
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(v))
                .ok_or_else(|| {
                    SasError::runtime(format!(
                        "Variable {} in the VAR statement is not in the base data set.",
                        v.to_uppercase()
                    ))
                })?;
            let cidx = comp_ds
                .vars
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(w))
                .ok_or_else(|| {
                    SasError::runtime(format!(
                        "Variable {} in the WITH statement is not in the comparison data set.",
                        w.to_uppercase()
                    ))
                })?;
            if base_ds.vars[bidx].ty != comp_ds.vars[cidx].ty {
                // Types conflictuels : paire non comparée, bit TYPE posé.
                sysinfo |= sysinfo_bits::TYPE;
                continue;
            }
            out.push(VarPair {
                out_name: base_ds.vars[bidx].name.to_uppercase(),
                base_idx: bidx,
                comp_idx: cidx,
                var_type: base_ds.vars[bidx].ty,
            });
        }
        out
    };

    // ── Bits d'attributs / variables (indépendants de l'appariement) ──────
    for cv in common_vars {
        if !cv.type_match {
            sysinfo |= sysinfo_bits::TYPE;
        }
        if cv.format_diff {
            sysinfo |= sysinfo_bits::FORMAT;
        }
        if cv.length_diff {
            sysinfo |= sysinfo_bits::LENGTH;
        }
        if cv.label_diff {
            sysinfo |= sysinfo_bits::LABEL;
        }
    }
    if !pairs.is_empty() || !ast.var.is_empty() {
        // BASEVAR/COMPVAR : variables hors appariement (hors BY/ID).
        let by_names: Vec<String> = ast.by.iter().map(|(n, _)| n.to_uppercase()).collect();
        let id_names: Vec<String> = ast.id.iter().map(|n| n.to_uppercase()).collect();
        let paired: Vec<String> = pairs.iter().map(|p| p.out_name.clone()).collect();
        let is_aux = |n: &str| by_names.iter().any(|b| b == n) || id_names.iter().any(|i| i == n);
        for v in &base_ds.vars {
            let n = v.name.to_uppercase();
            if !paired.contains(&n)
                && !is_aux(&n)
                && !comp_ds.vars.iter().any(|c| c.name.eq_ignore_ascii_case(&n))
            {
                sysinfo |= sysinfo_bits::BASEVAR;
            }
        }
        for v in &comp_ds.vars {
            let n = v.name.to_uppercase();
            if !paired.contains(&n)
                && !is_aux(&n)
                && !base_ds.vars.iter().any(|b| b.name.eq_ignore_ascii_case(&n))
            {
                sysinfo |= sysinfo_bits::COMPVAR;
            }
        }
    }

    // ── Appariement : groupes BY, puis ID/position ────────────────────────
    let (by_base, by_comp) = resolve_columns(base_ds, comp_ds, &ast.by);
    let (id_base, id_comp) = resolve_columns(
        base_ds,
        comp_ds,
        &ast.id
            .iter()
            .map(|n| (n.clone(), false))
            .collect::<Vec<_>>(),
    );
    let id_ok = !ast.id.is_empty()
        && id_base.iter().all(|c| c.is_some())
        && id_comp.iter().all(|c| c.is_some());
    let by_all_ok = ast
        .by
        .iter()
        .enumerate()
        .all(|(i, _)| by_base[i].is_some() && by_comp[i].is_some());
    if ast.id.is_empty() {
        // ID absent : appariement positionnel (avec BY si applicable).
    } else if !id_ok {
        // Variable ID absente d'un dataset : SAS produit une ERROR fatale
        // (bit 32768, comparaison non effectuée).
        return Ok(ComparisonOutcome {
            pairs,
            matches: Vec::new(),
            var_diffs: Vec::new(),
            base_only_obs: Vec::new(),
            comp_only_obs: Vec::new(),
            sysinfo: sysinfo | sysinfo_bits::BYVAR | 32768,
        });
    }

    // Groupes BY : liste ordonnée de lignes par clé.
    let use_by = !ast.by.is_empty() && by_all_ok;
    if !ast.by.is_empty() && !by_all_ok {
        sysinfo |= sysinfo_bits::BYVAR;
    }
    let mut base_groups: Vec<(Vec<Value>, Vec<usize>)> = Vec::new();
    let mut comp_groups: Vec<(Vec<Value>, Vec<usize>)> = Vec::new();
    if use_by {
        for row in 0..base_ds.n_obs() {
            let key = row_key(base_ds, &by_base, row);
            match base_groups.iter_mut().find(|(k, _)| keys_equal(k, &key)) {
                Some((_, rows)) => rows.push(row),
                None => base_groups.push((key, vec![row])),
            }
        }
        for row in 0..comp_ds.n_obs() {
            let key = row_key(comp_ds, &by_comp, row);
            match comp_groups.iter_mut().find(|(k, _)| keys_equal(k, &key)) {
                Some((_, rows)) => rows.push(row),
                None => comp_groups.push((key, vec![row])),
            }
        }
    } else {
        base_groups.push((Vec::new(), (0..base_ds.n_obs()).collect()));
        comp_groups.push((Vec::new(), (0..comp_ds.n_obs()).collect()));
    }

    let mut matches: Vec<PairMatch> = Vec::new();
    let mut base_only_obs: Vec<usize> = Vec::new();
    let mut comp_only_obs: Vec<usize> = Vec::new();
    for (bkey, brows) in &base_groups {
        let in_comp = comp_groups.iter().position(|(k, _)| keys_equal(k, bkey));
        match in_comp {
            Some(ci) => {
                let crows = comp_groups[ci].1.clone();
                let (paired, ob, oc) =
                    match_group(base_ds, comp_ds, brows, &crows, &id_base, &id_comp);
                for (seq_minus_1, (b, c)) in paired.into_iter().enumerate() {
                    matches.push(PairMatch {
                        base_idx: b,
                        comp_idx: c,
                        group_seq: seq_minus_1 + 1,
                        unequal: false,
                    });
                }
                base_only_obs.extend(ob);
                comp_only_obs.extend(oc);
            }
            None => {
                sysinfo |= sysinfo_bits::BASEBY;
                base_only_obs.extend(brows.iter().copied());
            }
        }
    }
    for (ckey, crows) in &comp_groups {
        if !base_groups.iter().any(|(k, _)| keys_equal(k, ckey)) {
            sysinfo |= sysinfo_bits::COMPBY;
            comp_only_obs.extend(crows.iter().copied());
        }
    }
    if !base_only_obs.is_empty() {
        sysinfo |= sysinfo_bits::BASEOBS;
    }
    if !comp_only_obs.is_empty() {
        sysinfo |= sysinfo_bits::COMPOBS;
    }

    // ── Jugement des paires ───────────────────────────────────────────────
    let mut var_diffs: Vec<VarDiffSummary> = pairs
        .iter()
        .map(|p| VarDiffSummary {
            name: p.out_name.clone(),
            var_type: p.var_type,
            n_diffs: 0,
            max_diff: 0.0,
        })
        .collect();

    for m in &mut matches {
        let mut unequal = false;
        for (pi, p) in pairs.iter().enumerate() {
            let bv = get_value_at(&base_ds.df, p.base_idx, m.base_idx, p.var_type);
            let cv = get_value_at(&comp_ds.df, p.comp_idx, m.comp_idx, p.var_type);
            if judged_unequal(&bv, &cv, ast.method, ast.criterion, p.var_type) {
                unequal = true;
                var_diffs[pi].n_diffs += 1;
                if p.var_type == VarType::Num
                    && let (Value::Num(x), Value::Num(y)) = (&bv, &cv)
                {
                    let diff = (y - x).abs();
                    if diff > var_diffs[pi].max_diff {
                        var_diffs[pi].max_diff = diff;
                    }
                }
            }
        }
        m.unequal = unequal;
    }
    if var_diffs.iter().any(|v| v.n_diffs > 0) {
        sysinfo |= sysinfo_bits::VALUE;
    }

    Ok(ComparisonOutcome {
        pairs,
        matches,
        var_diffs,
        base_only_obs,
        comp_only_obs,
        sysinfo,
    })
}

/// Get a SAS Value from a DataFrame column at a given row index.
/// Only handles Num and Char (the SAS type model).
pub(super) fn get_value_at(df: &DataFrame, col_idx: usize, row_idx: usize, ty: VarType) -> Value {
    let col = &df.get_columns()[col_idx];
    match ty {
        VarType::Num => {
            let f64_col = col.as_materialized_series().f64().unwrap();
            num_to_value(f64_col.get(row_idx))
        }
        VarType::Char => {
            let str_col = col.as_materialized_series().str().unwrap();
            match str_col.get(row_idx) {
                None => Value::Char(String::new()),
                Some(s) => Value::Char(s.to_string()),
            }
        }
    }
}

pub(super) fn type_str(ty: VarType) -> &'static str {
    match ty {
        VarType::Num => "Num",
        VarType::Char => "Char",
    }
}
