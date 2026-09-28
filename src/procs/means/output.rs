use super::*;

/// M22.3 / J07-P2 — écrit la table ODS "Summary" de PROC MEANS comme dataset
/// SAS. La table porte les colonnes BY/CLASS (J07-P2) : une ligne par
/// (groupe BY × combinaison de classes affichée × variable de VAR). Les
/// variables CLASS inactives d'un `_TYPE_` donné prennent la valeur
/// manquante. Sans CLASS/BY, une ligne par variable — byte-identique au
/// comportement d'avant J07-P2.
#[allow(clippy::too_many_arguments)]
pub(super) fn write_ods_summary(
    session: &mut Session,
    ctx: &ExecCtx,
    by_cols: &[crate::procs::common::ByCol],
    by_groups_list: &[(Vec<Value>, Vec<usize>)],
    by_analysis: &[Vec<usize>],
    print_types: &[u64],
    target: &DatasetRef,
) -> Result<()> {
    let ds = ctx.ds;
    let k = ctx.class_cols.len();

    // Une (ligne) par (BY × type × groupe de classes × variable VAR).
    struct Row {
        by_idx: usize,
        class_cells: Vec<Value>,
        var_name: String,
        stats: Vec<Value>,
    }
    let mut rows: Vec<Row> = Vec::new();

    for (by_idx, (_by_key, _)) in by_groups_list.iter().enumerate() {
        let analysis = &by_analysis[by_idx];
        for &ty in print_types {
            let active: Vec<usize> = (0..k).filter(|&i| (ty >> (k - 1 - i)) & 1 == 1).collect();
            let groups = if active.is_empty() {
                vec![(Vec::new(), analysis.clone())]
            } else {
                ctx.ordered_groups(&active, analysis)
            };
            for (key, grp_rows) in &groups {
                let mut class_cells: Vec<Value> = Vec::with_capacity(k);
                let mut ai = 0usize;
                for (i, &col_idx) in ctx.class_cols.iter().enumerate() {
                    if active.contains(&i) {
                        class_cells.push(key[ai].clone());
                        ai += 1;
                    } else {
                        match ds.vars[col_idx].ty {
                            VarType::Num => class_cells.push(Value::missing()),
                            VarType::Char => class_cells.push(Value::Char(String::new())),
                        }
                    }
                }
                for (vi, &vcol) in ctx.var_cols.iter().enumerate() {
                    let stats: Vec<Value> = ctx
                        .report_stats
                        .iter()
                        .map(|s| ctx.stat_over(&ctx.var_values[vi], grp_rows, s))
                        .collect();
                    rows.push(Row {
                        by_idx,
                        class_cells: class_cells.clone(),
                        var_name: ds.vars[vcol].name.clone(),
                        stats,
                    });
                }
            }
        }
    }

    // Colonnes caractère "Variable" : un nom de variable par ligne.
    let var_names: Vec<Option<String>> = rows.iter().map(|r| Some(r.var_name.clone())).collect();
    let name_len = rows
        .iter()
        .map(|r| r.var_name.len())
        .max()
        .unwrap_or(8)
        .max(8);

    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();

    // BY columns first (values from the BY-group key).
    for (bi, bc) in by_cols.iter().enumerate() {
        let meta = &ds.vars[bc.col_idx];
        let series = match meta.ty {
            VarType::Num => {
                let vals: Vec<Option<f64>> = rows
                    .iter()
                    .map(|r| value_to_num(&by_groups_list[r.by_idx].0[bi]))
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
            VarType::Char => {
                let vals: Vec<Option<String>> = rows
                    .iter()
                    .map(|r| match &by_groups_list[r.by_idx].0[bi] {
                        Value::Char(s) if s.is_empty() => None,
                        Value::Char(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
        };
        columns.push(series.into());
        vars.push(meta.clone());
    }

    // CLASS columns.
    for (ci, &col_idx) in ctx.class_cols.iter().enumerate() {
        push_class_column(
            &mut columns,
            &mut vars,
            ds,
            col_idx,
            rows.iter()
                .map(|r| r.class_cells[ci].clone())
                .collect::<Vec<_>>(),
        );
    }

    columns.push(Series::new("Variable".into(), var_names).into());
    vars.push(VarMeta {
        name: "Variable".to_string(),
        ty: VarType::Char,
        length: name_len,
        format: None,
        label: None,
        informat: None,
    });

    // Une colonne numérique par statistique demandée.
    for (si, stat) in ctx.report_stats.iter().enumerate() {
        let colname = ods_summary_stat_colname(stat);
        let vals: Vec<Option<f64>> = rows.iter().map(|r| value_to_num(&r.stats[si])).collect();
        columns.push(Series::new(colname.as_str().into(), vals).into());
        vars.push(num_var_meta(&colname));
    }

    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars };

    let out_libref = target.libref_or_work();
    let out_table = target.name.to_uppercase();
    let display = format!("{out_libref}.{out_table}");
    let n_rows = out_ds.n_obs();
    let n_vars = out_ds.vars.len();

    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(display.clone());

    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_rows, n_vars
    ));

    Ok(())
}

/// Encode one CLASS column of an OUT= / ODS dataset (J07-P2 helper).
fn push_class_column(
    columns: &mut Vec<Column>,
    vars: &mut Vec<VarMeta>,
    ds: &SasDataset,
    col_idx: usize,
    cells: Vec<Value>,
) {
    let meta = &ds.vars[col_idx];
    let series = match meta.ty {
        VarType::Num => {
            let vals: Vec<Option<f64>> = cells.iter().map(value_to_num).collect();
            Series::new(meta.name.as_str().into(), vals)
        }
        VarType::Char => {
            let vals: Vec<Option<String>> = cells
                .iter()
                .map(|c| match c {
                    Value::Char(s) if s.is_empty() => None,
                    Value::Char(s) => Some(s.clone()),
                    _ => None,
                })
                .collect();
            Series::new(meta.name.as_str().into(), vals)
        }
    };
    columns.push(series.into());
    vars.push(meta.clone());
}

/// Nom de colonne du dataset Summary pour une statistique du rapport.
/// (StdDev pour `std`/`stddev` ; libellé capitalisé pour les autres.)
pub(super) fn ods_summary_stat_colname(stat: &str) -> String {
    match stat.to_ascii_lowercase().as_str() {
        "n" => "N".to_string(),
        "nmiss" => "NMiss".to_string(),
        "mean" => "Mean".to_string(),
        "std" | "stddev" => "StdDev".to_string(),
        "min" => "Min".to_string(),
        "max" => "Max".to_string(),
        "sum" => "Sum".to_string(),
        "range" => "Range".to_string(),
        "stderr" => "StdErr".to_string(),
        "cv" => "CV".to_string(),
        "median" => "Median".to_string(),
        "sumwgt" => "SumWgt".to_string(),
        "clm" => "CLM".to_string(),
        "lclm" => "LowerCLMean".to_string(),
        "uclm" => "UpperCLMean".to_string(),
        // Percentile keywords (M33.3): canonical PNN / QRANGE column names.
        p @ ("p1" | "p5" | "p10" | "p20" | "p25" | "p30" | "p40" | "p50" | "p60" | "p70"
        | "p75" | "p80" | "p90" | "p95" | "p99") => p.to_uppercase(),
        "q1" => "P25".to_string(),
        "q3" => "P75".to_string(),
        "qrange" => "QRange".to_string(),
        other => {
            let mut c = other.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        }
    }
}

/// Valeur d'une variable ID pour un groupe (J07-P2) : le plus grand niveau
/// observé (les manquants sont ignorés) ; manquant si le groupe n'en a aucun.
fn id_group_value(col: &[Value], rows: &[usize]) -> Value {
    let mut best: Option<Value> = None;
    for &r in rows {
        let v = &col[r];
        if is_missing_class(v) {
            continue;
        }
        best = match best {
            Some(b) => {
                if v.sas_cmp(&b) == Ordering::Greater {
                    Some(v.clone())
                } else {
                    Some(b)
                }
            }
            None => Some(v.clone()),
        };
    }
    best.unwrap_or_else(Value::missing)
}

/// Effectif d'un groupe sous FREQ (J07-P2) : Σ des fréquences (tronquées,
/// ≥ 1) des observations du groupe — indépendamment des manquants du VAR.
fn group_freq_value(freq_values: &[Value], rows: &[usize]) -> f64 {
    rows.iter()
        .map(|&r| {
            value_to_num(&freq_values[r])
                .map(|f| f.trunc())
                .filter(|f| *f >= 1.0)
                .unwrap_or(0.0)
        })
        .sum()
}

/// Expanded OUTPUT spec: one concrete (statistic, source column, name).
struct ExpandedSpec {
    stat: String,
    outname: String,
    col: Vec<Value>,
}

/// Expand the parsed OUTPUT specs against the VAR list in scope (J07-P2):
/// `mean=` → toutes les variables VAR ; `mean(x y)=` → x et y ; AUTONAME →
/// `<var>_<STAT>` ; sans AUTONAME ni nom, une seule variable d'analyse
/// prend le mot-clé statistique comme nom.
fn expand_specs(
    ds: &SasDataset,
    var_cols: &[usize],
    out: &MeansOutput,
) -> Result<Vec<ExpandedSpec>> {
    let mut expanded: Vec<ExpandedSpec> = Vec::new();
    for sp in &out.specs {
        // Resolve the source variables: explicit list, else every VAR.
        let sources: Vec<usize> = if sp.vars.is_empty() {
            var_cols.to_vec()
        } else {
            sp.vars
                .iter()
                .map(|v| {
                    ds.vars
                        .iter()
                        .position(|m| m.name.eq_ignore_ascii_case(v))
                        .ok_or_else(|| {
                            SasError::runtime(format!("Variable {} not found.", v.to_uppercase()))
                        })
                })
                .collect::<Result<_>>()?
        };
        let names: Vec<String> = if !sp.names.is_empty() {
            if sp.names.len() != sources.len() {
                return Err(SasError::runtime(format!(
                    "The OUTPUT statistic {} applies to {} variables but {} output names were given.",
                    sp.stat.to_uppercase(),
                    sources.len(),
                    sp.names.len()
                )));
            }
            sp.names.clone()
        } else if out.autoname {
            sources
                .iter()
                .map(|&ci| format!("{}_{}", ds.vars[ci].name, sp.stat.to_uppercase()))
                .collect()
        } else if sources.len() == 1 {
            vec![sp.stat.to_uppercase()]
        } else {
            return Err(SasError::runtime(format!(
                "The OUTPUT statistic {} applies to {} variables and needs AUTONAME or explicit output names.",
                sp.stat.to_uppercase(),
                sources.len()
            )));
        };
        for (&ci, name) in sources.iter().zip(&names) {
            expanded.push(ExpandedSpec {
                stat: sp.stat.clone(),
                outname: name.clone(),
                col: decode_column(ds, ci)?,
            });
        }
    }
    Ok(expanded)
}

/// J07-P2 — écrit l'OUT= d'un statement OUTPUT : toutes les combinaisons de
/// sous-ensembles de CLASS (restreintes par WAYS/TYPES/NWAY), niveaux ordonnés
/// per ORDER=, _TYPE_ (numérique ou masque CHARTYPE), _FREQ_ (Σw sous FREQ),
/// colonnes ID, DESCENDTYPES, COMPLETETYPES.
#[allow(clippy::too_many_arguments)]
pub(super) fn write_output(
    session: &mut Session,
    ctx: &ExecCtx,
    out: &MeansOutput,
    id_names: &[String],
    by_cols: &[crate::procs::common::ByCol],
    by_groups_list: &[(Vec<Value>, Vec<usize>)],
    by_analysis: &[Vec<usize>],
    descendtypes: bool,
    completetypes: bool,
    chartype: bool,
    allowed_types: Option<&std::collections::BTreeSet<u64>>,
) -> Result<()> {
    let ds = ctx.ds;
    let k = ctx.class_cols.len();

    // J07-P2 — variables ID : résolues et décodées une fois.
    let id_cols: Vec<(usize, Vec<Value>)> = id_names
        .iter()
        .map(|n| {
            let ci = ds
                .vars
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(n))
                .ok_or_else(|| {
                    SasError::runtime(format!("Variable {} not found.", n.to_uppercase()))
                })?;
            Ok((ci, decode_column(ds, ci)?))
        })
        .collect::<Result<_>>()?;

    // Specs étendues (vars sources résolues contre la liste VAR).
    let specs = expand_specs(ds, ctx.var_cols, out)?;

    // Niveaux observés par variable CLASS (pour COMPLETETYPES) — sur
    // l'ensemble des lignes d'analyse, tous groupes BY.
    let observed_levels: Vec<Vec<Value>> = if completetypes {
        (0..k)
            .map(|i| {
                let mut levels: Vec<Value> = Vec::new();
                for group in by_analysis {
                    for &r in group {
                        let v = &ctx.class_values[i][r];
                        if !levels.iter().any(|l| l.sas_cmp(v) == Ordering::Equal) {
                            levels.push(v.clone());
                        }
                    }
                }
                levels.sort_by(|a, b| a.sas_cmp(b));
                levels
            })
            .collect()
    } else {
        Vec::new()
    };

    // Output rows accumulate as: (BY-group index, type, class cells,
    // sort key, freq, id values, stat values).
    struct OutRow {
        by_idx: usize,
        ty: u64,
        // class cell value per class var (active = group key value;
        // inactive = missing of right type).
        class_cells: Vec<Value>,
        // sort key: only the active classes' values (in class order).
        sort_key: Vec<Value>,
        freq: f64,
        id_values: Vec<Value>,
        stats: Vec<Value>,
    }
    let mut out_rows: Vec<OutRow> = Vec::new();

    // One block of CLASS-subset rows per BY group (one group overall if no BY),
    // restricting the analysis to that BY group's analysis rows (J07-P2 :
    // lignes à CLASS manquant exclues sans MISSING).
    for (by_idx, (_by_key, _)) in by_groups_list.iter().enumerate() {
        let analysis = &by_analysis[by_idx];

        // Enumerate all 2^k CLASS subsets within this BY group.
        for mask in 0u32..(1u32 << k) {
            let active: Vec<usize> = (0..k).filter(|&i| (mask >> i) & 1 == 1).collect();

            // _TYPE_ : LSB corresponds to the LAST class variable.
            let mut ty: u64 = 0;
            for &i in &active {
                ty |= 1u64 << (k - 1 - i);
            }

            // WAYS / TYPES / NWAY restriction (M33.3 / J07-P2).
            if let Some(set) = allowed_types
                && !set.contains(&ty)
            {
                continue;
            }

            // Group this BY group's rows by the active class variables,
            // ordered per ORDER=.
            let mut groups = ctx.ordered_groups(&active, analysis);

            // COMPLETETYPES (J07-P2) : toutes les combinaisons des niveaux
            // observés des CLASS actives, même non observées (freq 0,
            // statistiques manquantes).
            if completetypes && !active.is_empty() {
                let mut combos: Vec<Vec<Value>> = vec![Vec::new()];
                for &i in &active {
                    let levels = &observed_levels[i];
                    let mut next: Vec<Vec<Value>> = Vec::new();
                    for c in &combos {
                        for l in levels {
                            let mut ck = c.clone();
                            ck.push(l.clone());
                            next.push(ck);
                        }
                    }
                    combos = next;
                }
                for key in combos {
                    if !groups.iter().any(|(gk, _)| {
                        gk.len() == key.len()
                            && gk
                                .iter()
                                .zip(&key)
                                .all(|(a, b)| a.sas_cmp(b) == Ordering::Equal)
                    }) {
                        groups.push((key, Vec::new()));
                    }
                }
            }

            for (active_key, grp_rows) in &groups {
                let mut class_cells: Vec<Value> = Vec::with_capacity(k);
                let mut ai = 0usize;
                for (i, &col_idx) in ctx.class_cols.iter().enumerate() {
                    if active.contains(&i) {
                        class_cells.push(active_key[ai].clone());
                        ai += 1;
                    } else {
                        match ds.vars[col_idx].ty {
                            VarType::Num => class_cells.push(Value::missing()),
                            VarType::Char => class_cells.push(Value::Char(String::new())),
                        }
                    }
                }

                // _FREQ_ : effectif simple, ou Σw sous FREQ (J07-P2).
                let freq = match ctx.freq_values {
                    Some(fv) => group_freq_value(fv, grp_rows),
                    None => grp_rows.len() as f64,
                };

                let id_values: Vec<Value> = id_cols
                    .iter()
                    .map(|(_, col)| id_group_value(col, grp_rows))
                    .collect();

                let mut stat_vals: Vec<Value> = Vec::with_capacity(specs.len());
                for sp in &specs {
                    stat_vals.push(ctx.stat_over(&sp.col, grp_rows, &sp.stat));
                }

                out_rows.push(OutRow {
                    by_idx,
                    ty,
                    class_cells,
                    sort_key: active_key.clone(),
                    freq,
                    id_values,
                    stats: stat_vals,
                });
            }
        }
    }

    // Order rows: BY group order (outer, preserved), then _TYPE_ (ascending,
    // or descending under DESCENDTYPES), then the active class-value tuple —
    // per ORDER= when active (J07-P2), else via sas_cmp.
    let rank_of = |class_idx: usize, v: &Value| -> (usize, Value) {
        let pos = ctx.ranks[class_idx]
            .iter()
            .position(|l| l.sas_cmp(v) == Ordering::Equal);
        (pos.unwrap_or(usize::MAX), v.clone())
    };
    out_rows.sort_by(|a, b| {
        match a.by_idx.cmp(&b.by_idx) {
            Ordering::Equal => {}
            other => return other,
        }
        let ta = if descendtypes {
            b.ty.cmp(&a.ty)
        } else {
            a.ty.cmp(&b.ty)
        };
        match ta {
            Ordering::Equal => {}
            other => return other,
        }
        // Même _TYPE_ → mêmes positions actives : comparaison par rangs
        // (ORDER=) ou par valeur (INTERNAL).
        for (pos, x) in a.sort_key.iter().enumerate() {
            let ci = active_pos_for_ty(a.ty, k, pos);
            let (ra, va) = if ctx.order != ClassOrder::Internal && !ctx.ranks.is_empty() {
                rank_of(ci, x)
            } else {
                (0, x.clone())
            };
            let y = &b.sort_key[pos];
            let (rb, vb) = if ctx.order != ClassOrder::Internal && !ctx.ranks.is_empty() {
                rank_of(ci, y)
            } else {
                (0, y.clone())
            };
            match ra.cmp(&rb) {
                Ordering::Equal => {}
                other => return other,
            }
            match va.sas_cmp(&vb) {
                Ordering::Equal => {}
                other => return other,
            }
        }
        Ordering::Equal
    });

    // Build the output DataFrame column-by-column.
    let n_rows = out_rows.len();
    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();

    // BY columns first (copy input VarMeta; values from the BY-group key).
    for (bi, bc) in by_cols.iter().enumerate() {
        let meta = &ds.vars[bc.col_idx];
        let series = match meta.ty {
            VarType::Num => {
                let vals: Vec<Option<f64>> = out_rows
                    .iter()
                    .map(|r| value_to_num(&by_groups_list[r.by_idx].0[bi]))
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
            VarType::Char => {
                let vals: Vec<Option<String>> = out_rows
                    .iter()
                    .map(|r| match &by_groups_list[r.by_idx].0[bi] {
                        Value::Char(s) if s.is_empty() => None,
                        Value::Char(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
        };
        columns.push(series.into());
        vars.push(meta.clone());
    }

    // CLASS columns.
    for (ci, &col_idx) in ctx.class_cols.iter().enumerate() {
        push_class_column(
            &mut columns,
            &mut vars,
            ds,
            col_idx,
            out_rows
                .iter()
                .map(|r| r.class_cells[ci].clone())
                .collect::<Vec<_>>(),
        );
    }

    // ID columns (J07-P2), right after the CLASS columns.
    for (ci, (col_idx, _)) in id_cols.iter().enumerate() {
        let meta = &ds.vars[*col_idx];
        let series = match meta.ty {
            VarType::Num => {
                let vals: Vec<Option<f64>> = out_rows
                    .iter()
                    .map(|r| value_to_num(&r.id_values[ci]))
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
            VarType::Char => {
                let vals: Vec<Option<String>> = out_rows
                    .iter()
                    .map(|r| match &r.id_values[ci] {
                        Value::Char(s) if s.is_empty() => None,
                        Value::Char(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
        };
        columns.push(series.into());
        vars.push(meta.clone());
    }

    // _TYPE_ : numérique, ou masque caractère '101' sous CHARTYPE (J07-P2).
    if chartype && k > 0 {
        let vals: Vec<Option<String>> = out_rows
            .iter()
            .map(|r| {
                let mask: String = (0..k)
                    .map(|i| {
                        if (r.ty >> (k - 1 - i)) & 1 == 1 {
                            '1'
                        } else {
                            '0'
                        }
                    })
                    .collect();
                Some(mask)
            })
            .collect();
        columns.push(Series::new("_TYPE_".into(), vals).into());
        vars.push(crate::procs::common::char_var_meta("_TYPE_", k));
    } else {
        let type_vals: Vec<Option<f64>> = out_rows.iter().map(|r| Some(r.ty as f64)).collect();
        columns.push(Series::new("_TYPE_".into(), type_vals).into());
        vars.push(num_var_meta("_TYPE_"));
    }

    // _FREQ_
    let freq_vals: Vec<Option<f64>> = out_rows.iter().map(|r| Some(r.freq)).collect();
    columns.push(Series::new("_FREQ_".into(), freq_vals).into());
    vars.push(num_var_meta("_FREQ_"));

    // One column per output spec.
    for (si, sp) in specs.iter().enumerate() {
        let vals: Vec<Option<f64>> = out_rows
            .iter()
            .map(|r| value_to_num(&r.stats[si]))
            .collect();
        columns.push(Series::new(sp.outname.as_str().into(), vals).into());
        vars.push(num_var_meta(&sp.outname));
    }

    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars };

    let out_libref = out.out.libref_or_work();
    let out_table = out.out.name.to_uppercase();
    let display = format!("{out_libref}.{out_table}");
    let n_vars = out_ds.vars.len();

    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(display.clone());

    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_rows, n_vars
    ));

    Ok(())
}

/// CLASS variable index of sort-key position `pos` for a `_TYPE_` value
/// (LSB = last class variable).
fn active_pos_for_ty(ty: u64, k: usize, pos: usize) -> usize {
    // Position `pos` (0-based, class order) is active at the same index:
    // reconstruct by enumerating active positions in order.
    let mut seen = 0usize;
    for i in 0..k {
        if (ty >> (k - 1 - i)) & 1 == 1 {
            if seen == pos {
                return i;
            }
            seen += 1;
        }
    }
    pos.saturating_sub(1)
}
