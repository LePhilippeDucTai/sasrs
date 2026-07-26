use super::*;

pub(super) fn exec_insert_values(
    table: &DatasetRef,
    columns: &[String],
    rows: &[Vec<Expr>],
    session: &mut Session,
) -> Result<()> {
    let libref = table.libref_or_work();
    let name = table.name.to_uppercase();
    let display = table.display();

    let provider = session.libs.get(&libref)?;
    if !provider.exists(&name) {
        return Err(SasError::runtime(format!(
            "Table {} does not exist.",
            display
        )));
    }
    let (ds, notes) = provider.read(&name)?;
    for note in notes {
        session.log.forward(&note);
    }

    // Décode l'existant en colonnes de Value, on appendra dedans.
    let mut cols = decode_columns(&ds)?;

    // Indices des colonnes ciblées (par nom si fournis, sinon positionnel).
    let target_idx: Vec<usize> = if columns.is_empty() {
        (0..ds.vars.len()).collect()
    } else {
        let mut idxs = Vec::with_capacity(columns.len());
        for c in columns {
            let idx = ds
                .vars
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(c))
                .ok_or_else(|| {
                    SasError::runtime(format!("Variable {} not found.", c.to_uppercase()))
                })?;
            idxs.push(idx);
        }
        idxs
    };

    let inserted = rows.len();
    for row in rows {
        if row.len() != target_idx.len() {
            return Err(SasError::runtime(format!(
                "The number of values ({}) does not match the number of columns ({}) for INSERT into {}.",
                row.len(),
                target_idx.len(),
                display
            )));
        }
        // Valeur par défaut pour les colonnes non ciblées : missing/blank.
        let mut new_vals: Vec<Value> = ds
            .vars
            .iter()
            .map(|m| match m.ty {
                VarType::Num => Value::missing(),
                VarType::Char => Value::Char(String::new()),
            })
            .collect();
        for (slot, expr) in target_idx.iter().zip(row) {
            let v = expr_to_value(expr)?;
            new_vals[*slot] = coerce_to_target(v, &ds.vars[*slot]);
        }
        for (i, v) in new_vals.into_iter().enumerate() {
            cols[i].push(v);
        }
    }

    let df = build_dataframe(&ds.vars, &cols)?;
    let new_ds = SasDataset {
        df,
        vars: ds.vars.clone(),
    };
    let provider = session.libs.get(&libref)?;
    provider.write(&name, &new_ds)?;
    session.last_dataset = Some(display.clone());
    session.log.note(&format!(
        "{} rows were inserted into {}.",
        inserted, display
    ));
    Ok(())
}

pub(super) fn exec_insert_select(
    table: &DatasetRef,
    query: &ast::SelectStmt,
    session: &mut Session,
) -> Result<()> {
    let libref = table.libref_or_work();
    let name = table.name.to_uppercase();
    let display = table.display();

    let provider = session.libs.get(&libref)?;
    if !provider.exists(&name) {
        return Err(SasError::runtime(format!(
            "Table {} does not exist.",
            display
        )));
    }
    let (ds, notes) = provider.read(&name)?;
    for note in notes {
        session.log.forward(&note);
    }

    // Frame source du SELECT, coercé au modèle SAS.
    let lf = plan::lower_select(query, session)?;
    let src_df = lf.collect()?;
    let (src_ds, src_notes) = SasDataset::from_dataframe(src_df)?;
    for note in src_notes {
        session.log.forward(&note);
    }

    if src_ds.n_vars() != ds.n_vars() {
        return Err(SasError::runtime(format!(
            "The SELECT produces {} columns but {} has {} columns.",
            src_ds.n_vars(),
            display,
            ds.n_vars()
        )));
    }

    let mut cols = decode_columns(&ds)?;
    let src_cols = decode_columns(&src_ds)?;
    let inserted = src_ds.n_obs();

    // Alignement positionnel, coercé au type de la colonne cible.
    for (i, target) in ds.vars.iter().enumerate() {
        for v in &src_cols[i] {
            cols[i].push(coerce_to_target(v.clone(), target));
        }
    }

    let df = build_dataframe(&ds.vars, &cols)?;
    let new_ds = SasDataset {
        df,
        vars: ds.vars.clone(),
    };
    let provider = session.libs.get(&libref)?;
    provider.write(&name, &new_ds)?;
    session.last_dataset = Some(display.clone());
    session.log.note(&format!(
        "{} rows were inserted into {}.",
        inserted, display
    ));
    Ok(())
}

/// `UPDATE <table> SET col=expr [, ...] [WHERE cond]`. On charge la table,
/// on filtre (lazy, sémantique missing standard) pour connaître les lignes
/// cibles, on évalue chaque expression d'assignation dans le contexte de la
/// frame normalisée, puis on réécrit les colonnes ciblées des lignes
/// sélectionnées en coerçant au type SAS de la colonne (char tronqué à sa
/// longueur déclarée, num← littéral char → missing). NOTE "N rows were
/// updated".
pub(super) fn exec_update(
    table: &DatasetRef,
    assignments: &[(String, ast::SqlExpr)],
    where_: Option<&ast::SqlExpr>,
    session: &mut Session,
) -> Result<()> {
    let libref = table.libref_or_work();
    let name = table.name.to_uppercase();
    let display = table.display();

    let provider = session.libs.get(&libref)?;
    if !provider.exists(&name) {
        return Err(SasError::runtime(format!(
            "Table {} does not exist.",
            display
        )));
    }
    let (ds, notes) = provider.read(&name)?;
    for note in notes {
        session.log.forward(&note);
    }

    // Indices des colonnes ciblées : chaque colonne du SET doit exister.
    let mut target_idx: Vec<usize> = Vec::with_capacity(assignments.len());
    for (col, _) in assignments {
        let idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(col))
            .ok_or_else(|| {
                SasError::runtime(format!(
                    "Column {} could not be found in the table {}.",
                    col.to_uppercase(),
                    display
                ))
            })?;
        target_idx.push(idx);
    }

    // Colonnes existantes décodées (on les modifiera en place).
    let mut cols = decode_columns(&ds)?;
    let n_rows = ds.n_obs();

    // Masque WHERE : true = ligne à mettre à jour. Évalué via le chemin lazy
    // standard (normalize_specials + translate_predicate) ; sans WHERE → tout.
    let mask: Vec<bool> = match where_ {
        None => vec![true; n_rows],
        Some(pred) => {
            let provider = session.libs.get(&libref)?;
            let lf = plan::normalize_specials(provider.scan(&name)?)?;
            let p = plan::translate_predicate(pred)?;
            // `with_column` (et non `select`) pour diffuser un prédicat éventuel
            // sur la hauteur de la frame.
            let masked = lf.with_column(p.alias("__upd_mask__")).collect()?;
            let s = masked.column("__upd_mask__")?.as_materialized_series();
            match s.bool() {
                Ok(ca) => ca.iter().map(|o| o.unwrap_or(false)).collect(),
                Err(_) => vec![false; n_rows],
            }
        }
    };

    // Évalue chaque expression d'assignation sur la frame normalisée complète,
    // puis applique aux lignes du masque (coerçue au type de la cible).
    let provider = session.libs.get(&libref)?;
    let base_lf = plan::normalize_specials(provider.scan(&name)?)?;
    for ((_, value), &slot) in assignments.iter().zip(target_idx.iter()) {
        let expr = plan::translate_expr(value)?;
        // `with_column` diffuse les littéraux scalaires (`set x = 0`) sur toutes
        // les lignes ; `select` ne le ferait pas (colonne d'une seule ligne).
        let evaluated = base_lf
            .clone()
            .with_column(expr.alias("__upd_val__"))
            .collect()?;
        let s = evaluated.column("__upd_val__")?.as_materialized_series();
        let new_vals = decode_series(s);
        let target_meta = &ds.vars[slot];
        for (row, keep) in mask.iter().enumerate() {
            if *keep {
                cols[slot][row] = coerce_update_target(new_vals[row].clone(), target_meta);
            }
        }
    }

    let updated = mask.iter().filter(|b| **b).count();
    let df = build_dataframe(&ds.vars, &cols)?;
    let new_ds = SasDataset {
        df,
        vars: ds.vars.clone(),
    };
    let provider = session.libs.get(&libref)?;
    provider.write(&name, &new_ds)?;
    session.last_dataset = Some(display.clone());
    session
        .log
        .note(&format!("{} rows were updated in {}.", updated, display));
    Ok(())
}

// ----------------------------------------------------------------------------
// DELETE FROM ... [WHERE]
// ----------------------------------------------------------------------------

/// Chemin LAZY : on scanne la table, on normalise les missings spéciaux
/// (NaN-payload → null) comme `lower_select`, puis on garde les lignes qui ne
/// satisfont PAS le prédicat (`filter(NOT pred)`). Les helpers
/// `plan::translate_predicate` / `plan::normalize_specials` sont exposés en
/// `pub(crate)` exactement pour ce besoin.
pub(super) fn exec_delete(
    table: &DatasetRef,
    where_: Option<&ast::SqlExpr>,
    session: &mut Session,
) -> Result<()> {
    let libref = table.libref_or_work();
    let name = table.name.to_uppercase();
    let display = table.display();

    let provider = session.libs.get(&libref)?;
    if !provider.exists(&name) {
        return Err(SasError::runtime(format!(
            "Table {} does not exist.",
            display
        )));
    }

    // Nombre de lignes initial (pour la NOTE).
    let before = provider.scan(&name)?.collect()?.height();

    let kept_df = match where_ {
        None => {
            // Suppression totale : on garde le schéma, 0 ligne.
            provider.scan(&name)?.limit(0).collect()?
        }
        Some(pred) => {
            let lf = provider.scan(&name)?;
            let lf = plan::normalize_specials(lf)?;
            let p = plan::translate_predicate(pred)?;
            lf.filter(p.not()).collect()?
        }
    };

    let deleted = before - kept_df.height();
    let (ds, notes) = SasDataset::from_dataframe(kept_df)?;
    for note in notes {
        session.log.forward(&note);
    }
    let provider = session.libs.get(&libref)?;
    provider.write(&name, &ds)?;
    session.last_dataset = Some(display.clone());
    session
        .log
        .note(&format!("{} rows were deleted from {}.", deleted, display));
    Ok(())
}

// ----------------------------------------------------------------------------
// DESCRIBE TABLE
// ----------------------------------------------------------------------------

pub(super) fn exec_describe(table: &DatasetRef, session: &mut Session) -> Result<()> {
    let libref = table.libref_or_work();
    let name = table.name.to_uppercase();
    let display = table.display();

    let provider = session.libs.get(&libref)?;
    if !provider.exists(&name) {
        return Err(SasError::runtime(format!(
            "Table {} does not exist.",
            display
        )));
    }
    let (ds, notes) = provider.read(&name)?;
    for note in notes {
        session.log.forward(&note);
    }

    session
        .log
        .note(&format!("SQL table {} was created like:", display));
    session.log.note(&format!("create table {} (", display));
    let n = ds.vars.len();
    for (i, v) in ds.vars.iter().enumerate() {
        let comma = if i + 1 < n { "," } else { "" };
        let ty = match v.ty {
            VarType::Num => "num".to_string(),
            VarType::Char => format!("char({})", v.length),
        };
        let mut extra = String::new();
        if let Some(f) = &v.format {
            extra.push_str(&format!(" format={}", f));
        }
        if let Some(l) = &v.label {
            extra.push_str(&format!(" label='{}'", l));
        }
        session
            .log
            .note(&format!("  {} {}{}{}", v.name, ty, extra, comma));
    }
    session.log.note(");");
    Ok(())
}
