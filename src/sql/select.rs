use super::*;

// ----------------------------------------------------------------------------
// SELECT → listing
// ----------------------------------------------------------------------------

pub(super) fn exec_select(
    sel: &ast::SelectStmt,
    noprint: bool,
    session: &mut Session,
) -> Result<()> {
    let lf = plan::lower_select(sel, session)?;
    let df = lf.collect()?;
    let (ds, notes) = SasDataset::from_dataframe(df)?;
    for note in notes {
        session.log.forward(&note);
    }

    // NOPRINT (J02-P4) : pas de rendu listing — le dataset reste disponible
    // pour la capture ODS OUTPUT ci-dessous.
    if !noprint {
        render_listing(&ds, session);
    }

    // M42.3 — cette table de listing porte le nom d'objet ODS « SQL_Results »
    // (nom SAS réel : `ods output sql_results=ds;`) : si sa capture est
    // active, la version TYPÉE (déjà utilisée ci-dessus pour le rendu, avant
    // mise en forme) s'accumule. Un run-group `proc sql ; select … ;
    // select … ; quit ;` peut contenir PLUSIEURS SELECT nus : par cohérence
    // avec la convention générale de ce projet (union diagonale des tranches
    // successives au sein d'un même step — voir `OneWayFreqs` en PROC FREQ,
    // doc `session/ods_output.rs`), chaque SELECT ajoute sa tranche au même
    // dataset accumulé plutôt que de ne garder que le premier/dernier. Le
    // comportement SAS réel pour plusieurs SELECT dans un seul
    // `ODS OUTPUT SQL_Results=` n'a pas pu être confirmé avec certitude ; ce
    // choix suit simplement la convention déjà établie dans ce code pour
    // rester cohérent avec le reste du moteur.
    if session.ods_output_active("SQL_Results") {
        session.append_ods_output("SQL_Results", ds)?;
    }

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
                // MQ9.3 — `unwrap_or_default()` rendait un vecteur VIDE si le
                // dtype Polars ne correspondait pas au `VarType` SAS, alors que
                // les lignes sont ensuite indexées par position : colonne
                // tronquée en silence. Un vecteur de blancs de la BONNE
                // longueur est visible et ne désaligne rien.
                .unwrap_or_else(|_| vec![String::new(); n_obs]),
            VarType::Char => series
                .str()
                .map(|ca| ca.iter().map(|o| o.unwrap_or("").to_string()).collect())
                .unwrap_or_else(|_| vec![String::new(); n_obs]),
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

/// Métadonnées SAS d'une colonne SOURCE (celles du sidecar J04-P1) qui
/// doivent survivre à un `CREATE TABLE AS SELECT` quand la colonne est
/// reprise telle quelle.
struct SourceMeta {
    format: Option<String>,
    label: Option<String>,
    length: usize,
}

/// Une table physique du FROM/JOIN, indexée par sa clé de résolution
/// (alias sinon nom de table, en minuscules — cf. l'espace de noms plat du
/// plan SQL) et par nom de colonne UPPERCASE.
struct SourceTable {
    key: String,
    cols: std::collections::HashMap<String, SourceMeta>,
}

/// Charge les métadonnées des tables physiques du FROM/JOIN. Les
/// sous-requêtes, vues SQL et dictionary tables n'ont PAS de métadonnées
/// persistées → ignorées (leurs colonnes sortent sans format/label/longueur,
/// comme une colonne calculée).
fn collect_source_tables(
    query: &ast::SelectStmt,
    session: &mut Session,
) -> Result<Vec<SourceTable>> {
    let visit = |item: &crate::sql::ast::FromItem,
                 session: &mut Session,
                 out: &mut Vec<SourceTable>|
     -> Result<()> {
        if item.subquery.is_some() {
            return Ok(());
        }
        let lib = item.table.libref_or_work();
        let name = item.table.name.to_uppercase();
        if lib == "WORK" && session.views.contains_key(&name) {
            return Ok(());
        }
        if crate::sql::dictionary::dictionary_kind(&lib, &name).is_some() {
            return Ok(());
        }
        // Lecture via le chemin qui APPLIQUE le sidecar (J04-P1) : les
        // VarMeta obtenus portent format/label/longueur persistés. Les notes
        // de lecture (coercition, sidecar invalide) vont au log.
        let Ok(provider) = session.libs.get(&lib) else {
            return Ok(());
        };
        let (ds, notes) = provider.read(&name)?;
        for note in notes {
            session.log.forward(&note);
        }
        let key = item
            .alias
            .clone()
            .unwrap_or_else(|| item.table.name.clone())
            .to_ascii_lowercase();
        let cols = ds
            .vars
            .iter()
            .map(|v| {
                (
                    v.name.to_uppercase(),
                    SourceMeta {
                        format: v.format.clone(),
                        label: v.label.clone(),
                        length: v.length,
                    },
                )
            })
            .collect();
        out.push(SourceTable { key, cols });
        Ok(())
    };
    let mut out = Vec::new();
    for f in &query.from {
        visit(f, session, &mut out)?;
    }
    for j in &query.joins {
        visit(&j.table, session, &mut out)?;
    }
    Ok(out)
}

/// Résout les métadonnées d'une colonne source : réf. qualifiée `t.x` →
/// table de clé `t` ; réf. nue → première table (ordre du FROM) qui la
/// possède, cohérent avec l'espace de noms plat du plan SQL.
fn resolve_meta<'a>(
    tables: &'a [SourceTable],
    qualified: Option<&str>,
    column: &str,
) -> Option<&'a SourceMeta> {
    let col = column.to_uppercase();
    match qualified {
        Some(q) => tables
            .iter()
            .find(|t| t.key == q.to_ascii_lowercase())
            .and_then(|t| t.cols.get(&col)),
        None => tables.iter().find_map(|t| t.cols.get(&col)),
    }
}

/// Applique héritage + attributs `format=`/`label=`/`length=` au VarMeta `v`
/// de la colonne de sortie. `length` sur une colonne caractère TRONQUE les
/// valeurs (sémantique SAS) ; sur du numérique elle n'est que déclarative.
fn apply_item_attrs(
    v: &mut crate::dataset::VarMeta,
    meta: Option<&SourceMeta>,
    attrs: &ast::SqlItemAttrs,
) -> Result<()> {
    if let Some(m) = meta {
        v.format = m.format.clone();
        v.label = m.label.clone();
        if v.ty == VarType::Char {
            v.length = m.length;
        }
    }
    if let Some(f) = &attrs.format {
        if crate::formats::FormatSpec::parse(f).is_none() {
            return Err(SasError::runtime(format!("The format {f} is not valid.")));
        }
        v.format = Some(f.clone());
    }
    if let Some(l) = &attrs.label {
        v.label = Some(l.clone());
    }
    if let Some(n) = attrs.length {
        v.length = n;
    }
    Ok(())
}

/// Tronque la colonne caractère d'index `idx` à `n` caractères (LENGTH= dans
/// le select-list, sémantique SAS).
fn truncate_char_column(ds: &mut SasDataset, idx: usize, n: usize) -> Result<()> {
    let s = ds.df.get_columns()[idx].as_materialized_series().clone();
    let Some(ca) = s.str().ok() else {
        return Ok(());
    };
    let name = s.name().to_string();
    let truncated: polars::prelude::StringChunked = ca
        .iter()
        .map(|o| o.map(|x| x.chars().take(n).collect::<String>()))
        .collect();
    // Le ChunkedArray collecti perd le nom de la série : sans le restaurer,
    // la colonne sortirait anonyme et le sidecar ne correspondrait plus.
    ds.df
        .replace_column(idx, truncated.into_series().with_name(name.clone().into()))?;
    Ok(())
}

/// J04-P4 — métadonnées après transformation SQL : une colonne reprise
/// TELLE QUELLE (`*`, `t.*`, `x`, `t.x`, éventuellement renommée via AS)
/// conserve format, label et longueur de sa source ; une colonne calculée
/// n'hérite de RIEN sauf attributs explicites `FORMAT=`/`LABEL=`/`LENGTH=`
/// du select-list.
fn apply_source_metadata(
    query: &ast::SelectStmt,
    ds: &mut SasDataset,
    session: &mut Session,
) -> Result<()> {
    let tables = collect_source_tables(query, session)?;
    for it in &query.items {
        match &it.expr {
            // `*` : chaque colonne de sortie hérite de sa source (première
            // table du FROM qui la possède).
            ast::SqlExpr::Star => {
                for v in ds.vars.iter_mut() {
                    if let Some(m) = resolve_meta(&tables, None, &v.name) {
                        apply_item_attrs(v, Some(m), &ast::SqlItemAttrs::default())?;
                    }
                }
            }
            // `t.*` : héritage restreint à la table de clé `t`.
            ast::SqlExpr::QualifiedStar(k) => {
                for v in ds.vars.iter_mut() {
                    if let Some(m) = resolve_meta(&tables, Some(k), &v.name) {
                        apply_item_attrs(v, Some(m), &ast::SqlItemAttrs::default())?;
                    }
                }
            }
            // Colonne nue ou qualifiée : héritage + attributs éventuels.
            ast::SqlExpr::Base(Expr::Var(name)) => {
                let meta = resolve_meta(&tables, None, name);
                let out = it.alias.clone().unwrap_or_else(|| name.clone());
                if let Some(v) = ds
                    .vars
                    .iter_mut()
                    .find(|v| v.name.eq_ignore_ascii_case(&out))
                {
                    apply_item_attrs(v, meta, &it.attrs)?;
                }
            }
            ast::SqlExpr::Qualified { table, column } => {
                let meta = resolve_meta(&tables, Some(table), column);
                let out = it.alias.clone().unwrap_or_else(|| column.clone());
                if let Some(v) = ds
                    .vars
                    .iter_mut()
                    .find(|v| v.name.eq_ignore_ascii_case(&out))
                {
                    apply_item_attrs(v, meta, &it.attrs)?;
                }
            }
            // Colonne calculée / agrégat : PAS d'héritage — seuls les
            // attributs explicites s'appliquent (règle SAS).
            _ => {
                let out = plan::output_name(it, query)?;
                if let Some(v) = ds
                    .vars
                    .iter_mut()
                    .find(|v| v.name.eq_ignore_ascii_case(&out))
                {
                    apply_item_attrs(v, None, &it.attrs)?;
                }
            }
        }
    }
    // LENGTH= sur une colonne caractère : tronquer les VALEURS.
    for it in &query.items {
        if let Some(n) = it.attrs.length {
            let out = plan::output_name(it, query).unwrap_or_default();
            if let Some(idx) = ds
                .vars
                .iter()
                .position(|v| v.name.eq_ignore_ascii_case(&out) && v.ty == VarType::Char)
            {
                truncate_char_column(ds, idx, n)?;
            }
        }
    }
    Ok(())
}

pub(super) fn exec_create_table_as(
    table: &DatasetRef,
    query: &ast::SelectStmt,
    session: &mut Session,
) -> Result<()> {
    let lf = plan::lower_select(query, session)?;
    let df = lf.collect()?;
    let (mut ds, notes) = SasDataset::from_dataframe(df)?;
    for note in notes {
        session.log.forward(&note);
    }

    // J04-P4 : format/label/longueur des colonnes reprises telles quelles.
    apply_source_metadata(query, &mut ds, session)?;

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
