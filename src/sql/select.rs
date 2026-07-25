use super::*;

// ----------------------------------------------------------------------------
// SELECT → listing
// ----------------------------------------------------------------------------

pub(super) fn exec_select(sel: &ast::SelectStmt, session: &mut Session) -> Result<()> {
    let lf = plan::lower_select(sel, session)?;
    let df = lf.collect()?;
    let (ds, notes) = SasDataset::from_dataframe(df)?;
    for note in notes {
        session.log.forward(&note);
    }
    render_listing(&ds, session);
    Ok(())
}

/// Rend un dataset au listing dans le style PROC PRINT, MAIS sans la colonne
/// `Obs` (le SELECT de PROC SQL n'en produit pas). Numériques alignés à
/// droite (BEST12., missings via `MissingKind::display`), caractères à gauche.
pub(super) fn render_listing(ds: &SasDataset, session: &mut Session) {
    let n_obs = ds.n_obs();
    let mut headers: Vec<String> = Vec::with_capacity(ds.vars.len());
    let mut aligns: Vec<Align> = Vec::with_capacity(ds.vars.len());
    for v in &ds.vars {
        headers.push(v.name.clone());
        aligns.push(match v.ty {
            VarType::Num => Align::Right,
            VarType::Char => Align::Left,
        });
    }

    // Décode chaque colonne UNE seule fois (jamais par cellule).
    let mut col_cells: Vec<Vec<String>> = Vec::with_capacity(ds.vars.len());
    for (i, v) in ds.vars.iter().enumerate() {
        let series = ds.df.get_columns()[i].as_materialized_series();
        let cells: Vec<String> = match v.ty {
            VarType::Num => series
                .f64()
                .map(|ca| {
                    ca.iter()
                        .map(|o| match num_to_value(o) {
                            Value::Missing(kind) => kind.display(),
                            Value::Num(f) => format_best(f, 12),
                            Value::Char(_) => unreachable!(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            VarType::Char => series
                .str()
                .map(|ca| ca.iter().map(|o| o.unwrap_or("").to_string()).collect())
                .unwrap_or_default(),
        };
        col_cells.push(cells);
    }

    let mut rows: Vec<Vec<String>> = Vec::with_capacity(n_obs);
    for row_i in 0..n_obs {
        let mut row: Vec<String> = Vec::with_capacity(headers.len());
        for cells in &col_cells {
            row.push(cells[row_i].clone());
        }
        rows.push(row);
    }

    session.listing.page_header();
    session.listing.write_table(&headers, &aligns, &rows);
}

// ----------------------------------------------------------------------------
// CREATE TABLE AS SELECT
// ----------------------------------------------------------------------------

pub(super) fn exec_create_table_as(
    table: &DatasetRef,
    query: &ast::SelectStmt,
    session: &mut Session,
) -> Result<()> {
    let lf = plan::lower_select(query, session)?;
    let df = lf.collect()?;
    let (ds, notes) = SasDataset::from_dataframe(df)?;
    for note in notes {
        session.log.forward(&note);
    }

    let libref = table.libref_or_work();
    let name = table.name.to_uppercase();
    let display = table.display();
    let n = ds.n_obs();
    let m = ds.n_vars();

    let provider = session.libs.get(&libref)?;
    provider.write(&name, &ds)?;

    session.last_dataset = Some(display.clone());
    session.log.note(&format!(
        "Table {} created, with {} rows and {} columns.",
        display, n, m
    ));
    Ok(())
}

// ----------------------------------------------------------------------------
// DROP TABLE
// ----------------------------------------------------------------------------

pub(super) fn exec_drop(refs: &[DatasetRef], session: &mut Session) -> Result<()> {
    for r in refs {
        let libref = r.libref_or_work();
        let name = r.name.to_uppercase();
        let display = r.display();
        // DROP TABLE et DROP VIEW partagent la logique : si la cible est une
        // vue stockée (espace WORK), on la supprime de la session.
        if libref == "WORK" && session.views.contains_key(&name) {
            session.views.remove(&name);
            session
                .log
                .note(&format!("Table {} has been dropped.", display));
            continue;
        }
        let provider = session.libs.get(&libref)?;
        if provider.exists(&name) {
            provider.delete(&name)?;
            session
                .log
                .note(&format!("Table {} has been dropped.", display));
        } else {
            session
                .log
                .error(&format!("Table {} does not exist.", display));
        }
    }
    Ok(())
}

// ----------------------------------------------------------------------------
// CREATE VIEW / DROP VIEW (M20.4)
// ----------------------------------------------------------------------------

/// `CREATE VIEW <name> AS <select>` : valide le nom (≤ 32 caractères, espace
/// WORK uniquement) puis stocke la requête en mémoire dans `Session.views`
/// (clé UPPERCASE). Une redéclaration écrase la vue précédente. La requête
/// n'est PAS exécutée ici (sémantique paresseuse SAS : une vue n'est résolue
/// qu'à l'utilisation).
pub(super) fn exec_create_view(
    name: &DatasetRef,
    query: &ast::SelectStmt,
    session: &mut Session,
) -> Result<()> {
    let libref = name.libref_or_work();
    if libref != "WORK" {
        return Err(SasError::runtime(format!(
            "PROC SQL views are only supported in the WORK library, not {}.",
            libref
        )));
    }
    let key = name.name.to_uppercase();
    if key.len() > 32 {
        return Err(SasError::runtime(format!(
            "The view name {} exceeds the 32-character limit.",
            key
        )));
    }
    let display = name.display();
    let existed = session.views.contains_key(&key);
    session.views.insert(key, query.clone());
    if existed {
        session
            .log
            .note(&format!("View {} has been redefined.", display));
    } else {
        session
            .log
            .note(&format!("SQL view {} has been defined.", display));
    }
    Ok(())
}

/// `DROP VIEW <ref> [, ...]` : supprime des vues de `Session.views`. Une vue
/// absente → ERROR au log (symétrique de DROP TABLE).
pub(super) fn exec_drop_view(refs: &[DatasetRef], session: &mut Session) -> Result<()> {
    for r in refs {
        let libref = r.libref_or_work();
        let name = r.name.to_uppercase();
        let display = r.display();
        if libref == "WORK" && session.views.remove(&name).is_some() {
            session
                .log
                .note(&format!("View {} has been dropped.", display));
        } else {
            session
                .log
                .error(&format!("View {} does not exist.", display));
        }
    }
    Ok(())
}
