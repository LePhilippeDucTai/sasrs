use super::*;

/// Ligne OUT= en cours de construction.
struct OutRow {
    row_type: &'static str,
    obs: usize,
    /// Valeurs des colonnes données (BY, ID, paires) — `None` = missing.
    values: Vec<Option<Value>>,
}

/// Valeur DIF d'une paire : numérique → y−x (missing si un des deux est
/// missing) ; caractère → '.' pour les caractères égaux, 'X' pour les
/// inégaux (doc SAS, _TYPE_=DIF).
fn dif_value(b: &Value, c: &Value, ty: VarType) -> Value {
    match ty {
        VarType::Num => match (b, c) {
            (Value::Num(x), Value::Num(y)) => Value::Num(y - x),
            _ => Value::missing(),
        },
        VarType::Char => {
            let x = match b {
                Value::Char(s) => s.trim_end(),
                _ => "",
            };
            let y = match c {
                Value::Char(s) => s.trim_end(),
                _ => "",
            };
            let n = x.len().max(y.len());
            let mut s = String::with_capacity(n);
            for i in 0..n {
                let xb = x.as_bytes().get(i).copied().unwrap_or(b' ');
                let yb = y.as_bytes().get(i).copied().unwrap_or(b' ');
                s.push(if xb == yb { '.' } else { 'X' });
            }
            Value::Char(s)
        }
    }
}

/// Valeur PERCENT d'une paire : 100·(y−x)/x ; missing si x est 0 ou missing
/// (doc SAS : caractères → même valeur que DIF).
fn percent_value(b: &Value, c: &Value, ty: VarType) -> Value {
    match ty {
        VarType::Num => match (b, c) {
            (Value::Num(x), Value::Num(y)) if *x != 0.0 => Value::Num(100.0 * (y - x) / x),
            _ => Value::missing(),
        },
        VarType::Char => dif_value(b, c, ty),
    }
}

/// Écrit le dataset OUT= (doc SAS 9.4, « Output Data Set (OUT=) ») :
///
/// - colonnes : variables BY, variables ID, variables comparées (noms de la
///   VAR), puis `_TYPE_` (BASE | COMP | DIF | PERCENT) et `_OBS_` ;
/// - OUTBASE : une ligne par observation de BASE (`_OBS_` = n° d'obs BASE) ;
/// - OUTCOMP : une ligne par observation de COMPARE (`_OBS_` = n° d'obs) ;
/// - DIF : par défaut si aucun type n'est demandé, ou via OUTDIF ;
///   `_OBS_` = séquence de la paire dans le groupe BY ;
/// - OUTNOEQUAL : supprime les lignes DIF/PERCENT des paires jugées égales ;
/// - ordre : bloc BASE, bloc COMP, bloc DIF, bloc PERCENT.
#[allow(clippy::too_many_arguments)]
pub(super) fn write_out_dataset(
    session: &mut Session,
    ast: &CompareAst,
    out_ref: &DatasetRef,
    outcome: &ComparisonOutcome,
    base_ds: &SasDataset,
    comp_ds: &SasDataset,
) -> Result<()> {
    // Colonnes : (nom, type, colonne base, colonne comp) — BY puis ID puis
    // paires. La longueur retenue est la plus grande des deux (doc SAS).
    struct OutCol {
        name: String,
        ty: VarType,
        base_idx: usize,
        comp_idx: usize,
        length: usize,
    }
    let mut cols: Vec<OutCol> = Vec::new();
    /// Ajoute une colonne BY/ID (présente dans les deux datasets).
    fn push_aux_col(ds_pair: (&SasDataset, &SasDataset), name: &str, cols: &mut Vec<OutCol>) {
        if let Some(b) = ds_pair
            .0
            .vars
            .iter()
            .position(|v| v.name.eq_ignore_ascii_case(name))
            && let Some(c) = ds_pair
                .1
                .vars
                .iter()
                .position(|v| v.name.eq_ignore_ascii_case(name))
        {
            cols.push(OutCol {
                name: ds_pair.0.vars[b].name.to_uppercase(),
                ty: ds_pair.0.vars[b].ty,
                base_idx: b,
                comp_idx: c,
                length: ds_pair.0.vars[b].length.max(ds_pair.1.vars[c].length),
            });
        }
    }
    for (name, _) in &ast.by {
        push_aux_col((base_ds, comp_ds), name, &mut cols);
    }
    for name in &ast.id {
        push_aux_col((base_ds, comp_ds), name, &mut cols);
    }
    // Nombre de colonnes auxiliaires (BY/ID) réellement présentes.
    let n_aux = cols.len();
    for p in &outcome.pairs {
        cols.push(OutCol {
            name: p.out_name.clone(),
            ty: p.var_type,
            base_idx: p.base_idx,
            comp_idx: p.comp_idx,
            length: base_ds.vars[p.base_idx]
                .length
                .max(comp_ds.vars[p.comp_idx].length),
        });
    }

    // Construit une ligne : valeurs d'une source (BASE ou COMP) ou
    // différence/percent d'une paire appariée.
    let base_row = |r: usize| -> Vec<Option<Value>> {
        cols.iter()
            .map(|c| Some(get_value_at(&base_ds.df, c.base_idx, r, c.ty)))
            .collect()
    };
    let comp_row = |r: usize| -> Vec<Option<Value>> {
        cols.iter()
            .map(|c| Some(get_value_at(&comp_ds.df, c.comp_idx, r, c.ty)))
            .collect()
    };
    let pair_row = |m: &PairMatch, percent: bool| -> Vec<Option<Value>> {
        cols.iter()
            .enumerate()
            .map(|(ci, c)| {
                if ci < n_aux {
                    // BY/ID : valeurs de l'observation BASE (doc : « the
                    // values are the values from the original data sets »).
                    Some(get_value_at(&base_ds.df, c.base_idx, m.base_idx, c.ty))
                } else {
                    let bv = get_value_at(&base_ds.df, c.base_idx, m.base_idx, c.ty);
                    let cv = get_value_at(&comp_ds.df, c.comp_idx, m.comp_idx, c.ty);
                    Some(if percent {
                        percent_value(&bv, &cv, c.ty)
                    } else {
                        dif_value(&bv, &cv, c.ty)
                    })
                }
            })
            .collect()
    };

    let mut rows: Vec<OutRow> = Vec::new();
    if ast.outbase {
        for r in 0..base_ds.n_obs() {
            rows.push(OutRow {
                row_type: "BASE",
                obs: r + 1,
                values: base_row(r),
            });
        }
    }
    if ast.outcomp {
        for r in 0..comp_ds.n_obs() {
            rows.push(OutRow {
                row_type: "COMP",
                obs: r + 1,
                values: comp_row(r),
            });
        }
    }
    let write_dif = ast.outdif || (!ast.outbase && !ast.outcomp && !ast.outpercent);
    let keep = |unequal: bool| !ast.outnoequal || unequal;
    if write_dif {
        for m in &outcome.matches {
            if keep(m.unequal) {
                rows.push(OutRow {
                    row_type: "DIF",
                    obs: m.group_seq,
                    values: pair_row(m, false),
                });
            }
        }
    }
    if ast.outpercent {
        for m in &outcome.matches {
            if keep(m.unequal) {
                rows.push(OutRow {
                    row_type: "PERCENT",
                    obs: m.group_seq,
                    values: pair_row(m, true),
                });
            }
        }
    }

    // Assemble le DataFrame : colonnes BY/ID/paires + _TYPE_ + _OBS_.
    let mut columns: Vec<Column> = Vec::with_capacity(cols.len() + 2);
    let mut vars: Vec<VarMeta> = Vec::with_capacity(cols.len() + 2);
    for c in &cols {
        match c.ty {
            VarType::Num => {
                let vals: Float64Chunked = rows
                    .iter()
                    .map(|r| {
                        r.values
                            .get(vars.len())
                            .and_then(|v| v.as_ref())
                            .and_then(crate::missing::value_to_num)
                    })
                    .collect();
                columns.push(Series::new(c.name.as_str().into(), vals).into());
            }
            VarType::Char => {
                let vals: StringChunked =
                    rows.iter()
                        .map(|r| {
                            r.values.get(vars.len()).and_then(|v| v.as_ref()).and_then(
                                |v| match v {
                                    Value::Char(s) => Some(s.as_str()),
                                    _ => None,
                                },
                            )
                        })
                        .collect();
                columns.push(Series::new(c.name.as_str().into(), vals).into());
            }
        }
        vars.push(VarMeta {
            name: c.name.clone(),
            ty: c.ty,
            length: c.length,
            format: None,
            label: None,
        });
    }
    let type_col: StringChunked = rows.iter().map(|r| Some(r.row_type)).collect();
    let obs_col: Float64Chunked = rows.iter().map(|r| Some(r.obs as f64)).collect();
    columns.push(Series::new("_TYPE_".into(), type_col).into());
    columns.push(Series::new("_OBS_".into(), obs_col).into());
    vars.push(VarMeta {
        name: "_TYPE_".to_string(),
        ty: VarType::Char,
        length: 8,
        format: None,
        label: Some("Type of Observation".to_string()),
    });
    vars.push(VarMeta {
        name: "_OBS_".to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: Some("Observation Number".to_string()),
    });

    let df = DataFrame::new(columns)
        .map_err(|e| SasError::runtime(format!("COMPARE OUT= build error: {e}")))?;
    let out_ds = SasDataset { df, vars };
    let out_libref = out_ref.libref_or_work();
    let out_name = out_ref.name.to_uppercase();
    let out_provider = session.libs.get(&out_libref)?;
    out_provider.write(&out_name, &out_ds)?;
    session.log.note(&format!(
        "Output data set: {}.{} ({} observations).",
        out_libref,
        out_name,
        rows.len()
    ));
    session.last_dataset = Some(format!("{}.{}", out_libref, out_name));
    Ok(())
}
