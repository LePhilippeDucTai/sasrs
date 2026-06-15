//! PROC SQL (jalon M6) : dialecte SQL de SAS compilé vers Polars lazy.
//!
//! # Plan du sous-système — voir PLAN.md
//!
//! Décision actée : parser DÉDIÉ du dialecte SAS (CALCULATED, remerge
//! automatique, `lib.table`, options de dataset) — PAS le SQLContext de
//! Polars (dialecte ANSI, sémantique missing différente).
//!
//! Run-group proc : `proc sql ; stmt ; stmt ; quit ;` — chaque statement
//! s'exécute IMMÉDIATEMENT (pas d'attente de quit), le parser est donc
//! appelé statement par statement par l'exécuteur de la proc.
//!
//! # Exécution (M6 box 3)
//!
//! `execute` itère `program.stmts` DANS L'ORDRE et exécute chacun
//! immédiatement (sémantique run-group de SAS). Chaque type de statement :
//!   - SELECT nu → abaissé via `plan::lower_select`, collecté, coercé au
//!     modèle SAS (f64/char) puis rendu au listing dans le style PROC PRINT
//!     (mais SANS colonne `Obs` — le SELECT de PROC SQL n'en a pas) ;
//!   - CREATE TABLE AS → abaissé, collecté, coercé, écrit dans la
//!     bibliothèque ; `_LAST_` mis à jour ; NOTE de création ;
//!   - DROP TABLE → suppression (ou ERROR si absente) ;
//!   - INSERT VALUES / INSERT SELECT → lignes ajoutées à la table existante ;
//!   - DELETE FROM → filtre lazy via `plan::translate_predicate` /
//!     `plan::normalize_specials` (chemin LAZY) puis réécriture ;
//!   - DESCRIBE → définition `create table` écrite au LOG.
//!
//! Coercition : les frames résultat SQL portent des types natifs Polars
//! (u32 pour `count`, i64, bool, etc.). On les ramène TOUJOURS au modèle SAS
//! strict (`SasDataset::from_dataframe`) avant écriture/rendu.

#![allow(unused_variables, dead_code)]

pub mod ast;
pub mod dictionary;
pub mod parser;
pub mod plan;

use crate::ast::{DatasetRef, Expr, UnaryOp};
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::{num_to_value, value_to_num};
use crate::session::Session;
use crate::value::{format_best, Value, VarType};
use ast::{SqlProgram, SqlStmt};
use polars::prelude::*;

pub fn execute(program: &SqlProgram, session: &mut Session) -> Result<()> {
    for stmt in &program.stmts {
        match stmt {
            SqlStmt::Select(sel) => exec_select(sel, session)?,
            SqlStmt::CreateTableAs { table, query } => {
                exec_create_table_as(table, query, session)?
            }
            SqlStmt::CreateView { name, query } => exec_create_view(name, query, session)?,
            SqlStmt::DropTable(refs) => exec_drop(refs, session)?,
            SqlStmt::DropView(refs) => exec_drop_view(refs, session)?,
            SqlStmt::Update {
                table,
                assignments,
                where_,
            } => exec_update(table, assignments, where_.as_ref(), session)?,
            SqlStmt::InsertValues {
                table,
                columns,
                rows,
            } => exec_insert_values(table, columns, rows, session)?,
            SqlStmt::InsertSelect { table, query } => {
                exec_insert_select(table, query, session)?
            }
            SqlStmt::DeleteFrom { table, where_ } => {
                exec_delete(table, where_.as_ref(), session)?
            }
            SqlStmt::Describe(table) => exec_describe(table, session)?,
        }
    }
    Ok(())
}

// ----------------------------------------------------------------------------
// SELECT → listing
// ----------------------------------------------------------------------------

fn exec_select(sel: &ast::SelectStmt, session: &mut Session) -> Result<()> {
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
fn render_listing(ds: &SasDataset, session: &mut Session) {
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

fn exec_create_table_as(
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

fn exec_drop(refs: &[DatasetRef], session: &mut Session) -> Result<()> {
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
fn exec_create_view(
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
fn exec_drop_view(refs: &[DatasetRef], session: &mut Session) -> Result<()> {
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

// ----------------------------------------------------------------------------
// INSERT
// ----------------------------------------------------------------------------

/// Évaluateur de littéraux pour `INSERT ... VALUES`. Les expressions doivent
/// être constantes : `Num`, `Str`, `Missing` ou un moins unaire sur un `Num`.
/// Tout le reste → ERROR propre.
fn expr_to_value(e: &Expr) -> Result<Value> {
    match e {
        Expr::Num(n) => Ok(Value::Num(*n)),
        Expr::Str(s) => Ok(Value::Char(s.clone())),
        Expr::Missing(k) => Ok(Value::Missing(*k)),
        Expr::Unary {
            op: UnaryOp::Minus,
            expr,
        } => match expr.as_ref() {
            Expr::Num(n) => Ok(Value::Num(-n)),
            _ => Err(SasError::runtime(
                "Only constant expressions are supported in INSERT ... VALUES.",
            )),
        },
        Expr::Unary {
            op: UnaryOp::Plus,
            expr,
        } => match expr.as_ref() {
            Expr::Num(n) => Ok(Value::Num(*n)),
            _ => Err(SasError::runtime(
                "Only constant expressions are supported in INSERT ... VALUES.",
            )),
        },
        _ => Err(SasError::runtime(
            "Only constant expressions are supported in INSERT ... VALUES.",
        )),
    }
}

/// Décode chaque colonne d'un dataset en Vec<Value> (downcast par colonne).
fn decode_columns(ds: &SasDataset) -> Result<Vec<Vec<Value>>> {
    let mut cols: Vec<Vec<Value>> = Vec::with_capacity(ds.vars.len());
    for (i, v) in ds.vars.iter().enumerate() {
        let series = ds.df.get_columns()[i].as_materialized_series();
        let values: Vec<Value> = match v.ty {
            VarType::Num => series.f64()?.iter().map(num_to_value).collect(),
            VarType::Char => series
                .str()?
                .iter()
                .map(|o| Value::Char(o.unwrap_or("").to_string()))
                .collect(),
        };
        cols.push(values);
    }
    Ok(cols)
}

/// Coerce une Value à la cible (char/num) selon le VarMeta. Pour une cible
/// char, tronque à la longueur de stockage ; pour une cible num, garde le
/// nombre/missing (un littéral char vers une num → missing).
fn coerce_to_target(v: Value, meta: &VarMeta) -> Value {
    match meta.ty {
        VarType::Char => {
            let s = match v {
                Value::Char(s) => s,
                Value::Num(_) | Value::Missing(_) => String::new(),
            };
            let truncated: String = s.chars().take(meta.length.max(1)).collect();
            Value::Char(truncated)
        }
        VarType::Num => match v {
            Value::Num(_) | Value::Missing(_) => v,
            Value::Char(_) => Value::missing(),
        },
    }
}

/// Coercition d'assignation UPDATE (M20.4), proche de la sémantique SAS du
/// signe `=` : char→num parse une chaîne numérique (sinon missing), num→char
/// formate en BEST12. puis tronque à la longueur déclarée, missing reste
/// missing. Distincte de `coerce_to_target` (INSERT, conversion stricte) pour
/// ne pas altérer les snapshots existants.
fn coerce_update_target(v: Value, meta: &VarMeta) -> Value {
    match meta.ty {
        VarType::Char => {
            let s = match v {
                Value::Char(s) => s,
                Value::Num(f) => format_best(f, 12),
                Value::Missing(_) => String::new(),
            };
            let truncated: String = s.chars().take(meta.length.max(1)).collect();
            Value::Char(truncated)
        }
        VarType::Num => match v {
            Value::Num(_) | Value::Missing(_) => v,
            Value::Char(s) => {
                let t = s.trim();
                if t.is_empty() {
                    Value::missing()
                } else {
                    match t.parse::<f64>() {
                        Ok(f) => Value::Num(f),
                        Err(_) => Value::missing(),
                    }
                }
            }
        },
    }
}

/// Reconstruit un DataFrame depuis des colonnes de Value alignées sur les
/// VarMeta (num → Float64, char → String).
fn build_dataframe(vars: &[VarMeta], cols: &[Vec<Value>]) -> Result<DataFrame> {
    let mut series: Vec<Column> = Vec::with_capacity(vars.len());
    for (i, v) in vars.iter().enumerate() {
        let col = &cols[i];
        let s = match v.ty {
            VarType::Num => {
                let ca: Float64Chunked = col
                    .iter()
                    .map(value_to_num)
                    .collect::<Float64Chunked>()
                    .with_name(v.name.as_str().into());
                ca.into_series()
            }
            VarType::Char => {
                let ca: StringChunked = col
                    .iter()
                    .map(|val| match val {
                        Value::Char(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect::<StringChunked>()
                    .with_name(v.name.as_str().into());
                ca.into_series()
            }
        };
        series.push(s.into());
    }
    Ok(DataFrame::new(series)?)
}

fn exec_insert_values(
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

fn exec_insert_select(
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

// ----------------------------------------------------------------------------
// UPDATE ... SET ... [WHERE] (M20.4)
// ----------------------------------------------------------------------------

/// Décode une série Polars arbitraire (Float64 / String, ou tout type coercible)
/// en Vec<Value>. Les colonnes calculées d'un UPDATE peuvent ressortir en
/// Float64 (arithmétique) ou String (concaténation/littéral) ; on ramène
/// chaque cellule à une `Value` SAS canonique.
fn decode_series(series: &Series) -> Vec<Value> {
    match series.dtype() {
        DataType::Float64 => series
            .f64()
            .map(|ca| ca.iter().map(num_to_value).collect())
            .unwrap_or_default(),
        DataType::String => series
            .str()
            .map(|ca| {
                ca.iter()
                    .map(|o| match o {
                        Some(s) => Value::Char(s.to_string()),
                        None => Value::Char(String::new()),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => {
            // Type natif (entiers, booléens...) : on caste en Float64 puis décode.
            match series.cast(&DataType::Float64) {
                Ok(f) => f
                    .f64()
                    .map(|ca| ca.iter().map(num_to_value).collect())
                    .unwrap_or_default(),
                Err(_) => vec![Value::missing(); series.len()],
            }
        }
    }
}

/// `UPDATE <table> SET col=expr [, ...] [WHERE cond]`. On charge la table,
/// on filtre (lazy, sémantique missing standard) pour connaître les lignes
/// cibles, on évalue chaque expression d'assignation dans le contexte de la
/// frame normalisée, puis on réécrit les colonnes ciblées des lignes
/// sélectionnées en coerçant au type SAS de la colonne (char tronqué à sa
/// longueur déclarée, num← littéral char → missing). NOTE "N rows were
/// updated".
fn exec_update(
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
fn exec_delete(
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
    session.log.note(&format!(
        "{} rows were deleted from {}.",
        deleted, display
    ));
    Ok(())
}

// ----------------------------------------------------------------------------
// DESCRIBE TABLE
// ----------------------------------------------------------------------------

fn exec_describe(table: &DatasetRef, session: &mut Session) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::parser::StatementStream;
    use crate::session::Session;
    use crate::source::SourceFile;
    use crate::sql::parser::parse_sql_program;
    use crate::value::VarType;
    use polars::df;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn num(name: &str) -> VarMeta {
        VarMeta {
            name: name.into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        }
    }
    fn chr(name: &str, len: usize) -> VarMeta {
        VarMeta {
            name: name.into(),
            ty: VarType::Char,
            length: len,
            format: None,
            label: None,
        }
    }

    fn write_table(session: &mut Session, name: &str, df: DataFrame, vars: Vec<VarMeta>) {
        let ds = SasDataset { df, vars };
        session.libs.get("WORK").unwrap().write(name, &ds).unwrap();
    }

    fn write_people(session: &mut Session) {
        let df = df![
            "name" => ["Al", "Bo", "Cy", "Di"],
            "sex"  => ["M", "M", "F", "F"],
            "age"  => [10.0_f64, 14.0, 13.0, 11.0],
            "height" => [50.0_f64, 60.0, 55.0, 52.0],
        ]
        .unwrap();
        write_table(
            session,
            "T",
            df,
            vec![chr("name", 8), chr("sex", 1), num("age"), num("height")],
        );
    }

    /// Parse and execute a PROC SQL body (the statements between `proc sql;`
    /// and `quit;`).
    fn run_sql(src: &str, session: &mut Session) {
        let file = SourceFile::new(src);
        let mut ts = StatementStream::new(&file).unwrap();
        let prog = parse_sql_program(&mut ts).unwrap();
        execute(&prog, session).unwrap();
    }

    fn read_work(session: &mut Session, name: &str) -> SasDataset {
        session.libs.get("WORK").unwrap().read(name).unwrap().0
    }

    #[test]
    fn create_table_as_select_writes_and_notes() {
        let mut s = make_session();
        write_people(&mut s);
        run_sql(
            "create table summary as select sex, count(*) as n from t group by sex;",
            &mut s,
        );
        assert!(s.libs.get("WORK").unwrap().exists("SUMMARY"));
        let ds = read_work(&mut s, "SUMMARY");
        assert_eq!(ds.n_obs(), 2);
        assert_eq!(ds.n_vars(), 2);
        // count(*) came back as u32 → must be coerced to f64 num.
        assert!(ds
            .vars
            .iter()
            .all(|v| matches!(v.ty, VarType::Num | VarType::Char)));
        let n_col = ds.df.column("n").unwrap();
        assert_eq!(n_col.dtype(), &DataType::Float64);
        let log = s.log.into_string();
        assert!(
            log.contains("Table WORK.SUMMARY created, with 2 rows and 2 columns."),
            "log: {log}"
        );
        assert_eq!(s.last_dataset.as_deref(), Some("WORK.SUMMARY"));
    }

    #[test]
    fn bare_select_renders_to_listing() {
        let mut s = make_session();
        write_people(&mut s);
        run_sql("select name, age from t where age > 12;", &mut s);
        let listing = s.listing.into_string();
        assert!(listing.contains("Bo"), "listing: {listing}");
        assert!(listing.contains("Cy"), "listing: {listing}");
        assert!(listing.contains("14"), "listing: {listing}");
        // No Obs column in SQL SELECT output.
        assert!(!listing.contains("Obs"), "listing: {listing}");
        // Bare SELECT must not set _LAST_.
        assert!(s.last_dataset.is_none());
    }

    #[test]
    fn insert_values_grows_row_count() {
        let mut s = make_session();
        write_people(&mut s);
        run_sql(
            "insert into t (name, sex, age, height) values ('Ed', 'M', 9, 48);",
            &mut s,
        );
        let ds = read_work(&mut s, "T");
        assert_eq!(ds.n_obs(), 5);
        let names: Vec<String> = ds
            .df
            .column("name")
            .unwrap()
            .str()
            .unwrap()
            .iter()
            .map(|o| o.unwrap_or("").to_string())
            .collect();
        assert!(names.contains(&"Ed".to_string()));
        let ages: Vec<f64> = ds
            .df
            .column("age")
            .unwrap()
            .f64()
            .unwrap()
            .into_no_null_iter()
            .collect();
        assert!(ages.contains(&9.0));
        let log = s.log.into_string();
        assert!(
            log.contains("1 rows were inserted into WORK.T."),
            "log: {log}"
        );
    }

    #[test]
    fn insert_values_positional() {
        let mut s = make_session();
        let df = df!["x" => [1.0_f64, 2.0]].unwrap();
        write_table(&mut s, "T", df, vec![num("x")]);
        run_sql("insert into t values (3) values (4);", &mut s);
        let ds = read_work(&mut s, "T");
        assert_eq!(ds.n_obs(), 4);
    }

    #[test]
    fn delete_with_where_removes_rows() {
        let mut s = make_session();
        write_people(&mut s);
        run_sql("delete from t where age > 12;", &mut s);
        let ds = read_work(&mut s, "T");
        assert_eq!(ds.n_obs(), 2);
        let ages: Vec<f64> = ds
            .df
            .column("age")
            .unwrap()
            .f64()
            .unwrap()
            .into_no_null_iter()
            .collect();
        assert!(ages.iter().all(|a| *a <= 12.0));
        let log = s.log.into_string();
        assert!(
            log.contains("2 rows were deleted from WORK.T."),
            "log: {log}"
        );
    }

    #[test]
    fn delete_all_rows() {
        let mut s = make_session();
        write_people(&mut s);
        run_sql("delete from t;", &mut s);
        let ds = read_work(&mut s, "T");
        assert_eq!(ds.n_obs(), 0);
        assert_eq!(ds.n_vars(), 4);
    }

    #[test]
    fn drop_table_removes_it() {
        let mut s = make_session();
        write_people(&mut s);
        assert!(s.libs.get("WORK").unwrap().exists("T"));
        run_sql("drop table t;", &mut s);
        assert!(!s.libs.get("WORK").unwrap().exists("T"));
        let log = s.log.into_string();
        assert!(
            log.contains("Table WORK.T has been dropped."),
            "log: {log}"
        );
    }

    #[test]
    fn drop_missing_table_errors_in_log() {
        let mut s = make_session();
        run_sql("drop table nope;", &mut s);
        let log = s.log.into_string();
        assert!(
            log.contains("Table WORK.NOPE does not exist."),
            "log: {log}"
        );
    }

    #[test]
    fn describe_writes_table_definition_to_log() {
        let mut s = make_session();
        write_people(&mut s);
        run_sql("describe table t;", &mut s);
        let log = s.log.into_string();
        assert!(log.contains("WORK.T"), "log: {log}");
        assert!(log.contains("create table"), "log: {log}");
        // char column should show its declared length.
        assert!(log.contains("char("), "log: {log}");
    }

    #[test]
    fn insert_select_appends() {
        let mut s = make_session();
        let a = df!["x" => [1.0_f64, 2.0]].unwrap();
        let b = df!["x" => [10.0_f64, 20.0, 30.0]].unwrap();
        write_table(&mut s, "A", a, vec![num("x")]);
        write_table(&mut s, "B", b, vec![num("x")]);
        run_sql("insert into a select x from b;", &mut s);
        let ds = read_work(&mut s, "A");
        assert_eq!(ds.n_obs(), 5);
    }

    // ── M20.4 : CREATE VIEW ────────────────────────────────────────────────

    #[test]
    fn create_view_basic() {
        let mut s = make_session();
        write_people(&mut s);
        run_sql(
            "create view v as select name, age from t; select * from v;",
            &mut s,
        );
        // The view is purely in memory (no parquet written).
        assert!(!s.libs.get("WORK").unwrap().exists("V"));
        assert!(s.views.contains_key("V"));
        let listing = s.listing.into_string();
        assert!(listing.contains("Al"), "listing: {listing}");
        assert!(listing.contains("14"), "listing: {listing}");
        let log = s.log.into_string();
        assert!(
            log.contains("SQL view WORK.V has been defined."),
            "log: {log}"
        );
    }

    #[test]
    fn drop_view() {
        let mut s = make_session();
        write_people(&mut s);
        run_sql("create view v as select name from t;", &mut s);
        assert!(s.views.contains_key("V"));
        run_sql("drop view v;", &mut s);
        assert!(!s.views.contains_key("V"));
        let log = s.log.into_string();
        assert!(
            log.contains("View WORK.V has been dropped."),
            "log: {log}"
        );
    }

    #[test]
    fn drop_view_via_drop_table() {
        // DROP TABLE on a view name removes the view (shared logic).
        let mut s = make_session();
        write_people(&mut s);
        run_sql("create view v as select name from t;", &mut s);
        run_sql("drop table v;", &mut s);
        assert!(!s.views.contains_key("V"));
    }

    #[test]
    fn create_view_overwrites() {
        let mut s = make_session();
        write_people(&mut s);
        run_sql("create view v as select name from t;", &mut s);
        run_sql("create view v as select age from t;", &mut s);
        // Redeclared: the second definition wins (age, not name).
        let q = s.views.get("V").unwrap();
        assert_eq!(
            q.items[0].expr,
            crate::sql::ast::SqlExpr::Base(Expr::Var("age".to_string()))
        );
        let log = s.log.into_string();
        assert!(log.contains("redefined"), "log: {log}");
    }

    #[test]
    fn view_from_qualified_table() {
        // The view's own FROM uses an explicit `work.t` qualifier.
        let mut s = make_session();
        write_people(&mut s);
        run_sql(
            "create view v as select name, age from work.t where age >= 13; \
             select name from v;",
            &mut s,
        );
        let listing = s.listing.into_string();
        assert!(listing.contains("Bo"), "listing: {listing}");
        assert!(listing.contains("Cy"), "listing: {listing}");
        assert!(!listing.contains("Al"), "listing: {listing}");
    }

    #[test]
    fn view_reference_missing() {
        // The view body references a nonexistent table → error when used.
        let mut s = make_session();
        run_sql("create view v as select x from nonexistent;", &mut s);
        // The CREATE itself is lazy (no error yet).
        let file = SourceFile::new("select * from v;");
        let mut ts = StatementStream::new(&file).unwrap();
        let prog = parse_sql_program(&mut ts).unwrap();
        let err = execute(&prog, &mut s).unwrap_err();
        assert!(!err.to_string().is_empty(), "expected an error");
    }

    #[test]
    fn view_select_from_missing() {
        // Selecting from a view that was never defined → clean error
        // (treated as a missing table).
        let mut s = make_session();
        let file = SourceFile::new("select * from ghost;");
        let mut ts = StatementStream::new(&file).unwrap();
        let prog = parse_sql_program(&mut ts).unwrap();
        let err = execute(&prog, &mut s).unwrap_err();
        assert!(!err.to_string().is_empty(), "expected an error");
    }

    #[test]
    fn view_referencing_view() {
        // Nested views: v2 selects from v1.
        let mut s = make_session();
        write_people(&mut s);
        run_sql(
            "create view v1 as select name, age from t where age >= 12; \
             create view v2 as select name from v1; \
             select name from v2;",
            &mut s,
        );
        let listing = s.listing.into_string();
        assert!(listing.contains("Bo"), "listing: {listing}");
        assert!(listing.contains("Cy"), "listing: {listing}");
        assert!(!listing.contains("Al"), "listing: {listing}");
    }

    // ── M20.4 : UPDATE ... SET ──────────────────────────────────────────────

    fn ages(ds: &SasDataset) -> Vec<f64> {
        ds.df
            .column("age")
            .unwrap()
            .f64()
            .unwrap()
            .into_no_null_iter()
            .collect()
    }

    #[test]
    fn update_basic() {
        let mut s = make_session();
        write_people(&mut s);
        run_sql("update t set age = 0;", &mut s);
        let ds = read_work(&mut s, "T");
        assert!(ages(&ds).iter().all(|a| *a == 0.0));
        let log = s.log.into_string();
        assert!(
            log.contains("4 rows were updated in WORK.T."),
            "log: {log}"
        );
    }

    #[test]
    fn update_multiple_columns() {
        let mut s = make_session();
        write_people(&mut s);
        run_sql("update t set age = 99, height = 100;", &mut s);
        let ds = read_work(&mut s, "T");
        assert!(ages(&ds).iter().all(|a| *a == 99.0));
        let heights: Vec<f64> = ds
            .df
            .column("height")
            .unwrap()
            .f64()
            .unwrap()
            .into_no_null_iter()
            .collect();
        assert!(heights.iter().all(|h| *h == 100.0));
    }

    #[test]
    fn update_with_where() {
        let mut s = make_session();
        write_people(&mut s);
        run_sql("update t set age = 100 where age > 12;", &mut s);
        let ds = read_work(&mut s, "T");
        // Only Bo(14) and Cy(13) updated → two rows are 100, others unchanged.
        let a = ages(&ds);
        assert_eq!(a.iter().filter(|x| **x == 100.0).count(), 2);
        assert!(a.contains(&10.0)); // Al unchanged
        assert!(a.contains(&11.0)); // Di unchanged
        let log = s.log.into_string();
        assert!(
            log.contains("2 rows were updated in WORK.T."),
            "log: {log}"
        );
    }

    #[test]
    fn update_self_reference() {
        // UPDATE based on another column / the same column.
        let mut s = make_session();
        write_people(&mut s);
        run_sql("update t set age = age + 1;", &mut s);
        let ds = read_work(&mut s, "T");
        let a = ages(&ds);
        assert!(a.contains(&11.0)); // 10+1
        assert!(a.contains(&15.0)); // 14+1
    }

    #[test]
    fn update_unknown_column() {
        let mut s = make_session();
        write_people(&mut s);
        let file = SourceFile::new("update t set nope = 1;");
        let mut ts = StatementStream::new(&file).unwrap();
        let prog = parse_sql_program(&mut ts).unwrap();
        let err = execute(&prog, &mut s).unwrap_err();
        assert!(
            err.to_string().contains("NOPE") || err.to_string().contains("could not be found"),
            "got: {err}"
        );
    }

    #[test]
    fn update_nonexistent_table() {
        let mut s = make_session();
        let file = SourceFile::new("update ghost set x = 1;");
        let mut ts = StatementStream::new(&file).unwrap();
        let prog = parse_sql_program(&mut ts).unwrap();
        let err = execute(&prog, &mut s).unwrap_err();
        assert!(
            err.to_string().contains("does not exist"),
            "got: {err}"
        );
    }

    #[test]
    fn update_type_coercion() {
        let mut s = make_session();
        write_people(&mut s);
        // num ← char numeric string → parsed to number; char ← num → BEST12.
        run_sql("update t set age = '21', sex = 5;", &mut s);
        let ds = read_work(&mut s, "T");
        // age stays numeric 21 (char "21" coerced via numeric assignment path).
        let a = ages(&ds);
        assert!(a.iter().all(|x| *x == 21.0), "ages: {a:?}");
        // sex is char(1): num 5 → "5" then truncated to its length.
        let sexes: Vec<String> = ds
            .df
            .column("sex")
            .unwrap()
            .str()
            .unwrap()
            .iter()
            .map(|o| o.unwrap_or("").to_string())
            .collect();
        assert!(sexes.iter().all(|x| x == "5"), "sexes: {sexes:?}");
    }

    #[test]
    fn update_all_rows() {
        // Explicit expectation: UPDATE without WHERE touches every row.
        let mut s = make_session();
        write_people(&mut s);
        run_sql("update t set height = 1;", &mut s);
        let ds = read_work(&mut s, "T");
        let heights: Vec<f64> = ds
            .df
            .column("height")
            .unwrap()
            .f64()
            .unwrap()
            .into_no_null_iter()
            .collect();
        assert_eq!(heights.len(), 4);
        assert!(heights.iter().all(|h| *h == 1.0));
        let log = s.log.into_string();
        assert!(
            log.contains("4 rows were updated in WORK.T."),
            "log: {log}"
        );
    }

    #[test]
    fn update_missing_values() {
        // Assigning a missing keeps it missing (no spurious conversion).
        let mut s = make_session();
        write_people(&mut s);
        run_sql("update t set age = . where age = 10;", &mut s);
        let ds = read_work(&mut s, "T");
        let col = ds.df.column("age").unwrap().f64().unwrap();
        let n_missing = col.iter().filter(|o| o.is_none()).count();
        assert_eq!(n_missing, 1, "exactly one age should now be missing");
    }

    // ── M20.4 : subqueries in INSERT ────────────────────────────────────────

    #[test]
    fn insert_select_with_scalar_subquery() {
        let mut s = make_session();
        write_people(&mut s);
        // target table: two num columns.
        let tgt = df!["n" => [0.0_f64], "a" => [0.0_f64]].unwrap();
        write_table(&mut s, "DEST", tgt, vec![num("n"), num("a")]);
        run_sql(
            "insert into dest select (select count(*) from t) as n, age as a from t where age = 10;",
            &mut s,
        );
        let ds = read_work(&mut s, "DEST");
        // started with 1 row, inserted 1 (Al, age 10) → 2 rows.
        assert_eq!(ds.n_obs(), 2);
        let ns: Vec<f64> = ds
            .df
            .column("n")
            .unwrap()
            .f64()
            .unwrap()
            .into_no_null_iter()
            .collect();
        // the scalar subquery COUNT(*) over t = 4.
        assert!(ns.contains(&4.0), "ns: {ns:?}");
    }

    #[test]
    fn insert_select_with_in_subquery() {
        let mut s = make_session();
        write_people(&mut s);
        let dest = df!["name" => ["zz"], "sex" => ["M"], "age" => [0.0_f64], "height" => [0.0_f64]]
            .unwrap();
        write_table(
            &mut s,
            "DEST",
            dest,
            vec![chr("name", 8), chr("sex", 1), num("age"), num("height")],
        );
        run_sql(
            "insert into dest select * from t where age in (select age from t where sex = 'F');",
            &mut s,
        );
        let ds = read_work(&mut s, "DEST");
        // F rows of t: Cy(13), Di(11) → two inserted, plus the seed row = 3.
        assert_eq!(ds.n_obs(), 3);
    }

    #[test]
    fn insert_select_with_exists() {
        let mut s = make_session();
        write_people(&mut s);
        let dest = df!["name" => ["zz"], "sex" => ["M"], "age" => [0.0_f64], "height" => [0.0_f64]]
            .unwrap();
        write_table(
            &mut s,
            "DEST",
            dest,
            vec![chr("name", 8), chr("sex", 1), num("age"), num("height")],
        );
        // EXISTS true → all rows of t pass; non-correlated.
        run_sql(
            "insert into dest select * from t where exists (select * from t where sex = 'F');",
            &mut s,
        );
        let ds = read_work(&mut s, "DEST");
        // all 4 rows of t inserted + seed = 5.
        assert_eq!(ds.n_obs(), 5);
    }

    #[test]
    fn insert_select_with_union_subquery() {
        let mut s = make_session();
        let a = df!["x" => [1.0_f64, 2.0]].unwrap();
        let b = df!["x" => [2.0_f64, 3.0]].unwrap();
        write_table(&mut s, "A", a, vec![num("x")]);
        write_table(&mut s, "B", b, vec![num("x")]);
        let dest = df!["x" => [0.0_f64]].unwrap();
        write_table(&mut s, "DEST", dest, vec![num("x")]);
        run_sql(
            "insert into dest select * from (select x from a union select x from b) as u;",
            &mut s,
        );
        let ds = read_work(&mut s, "DEST");
        // union distinct of {1,2} and {2,3} = {1,2,3} → 3 rows + seed = 4.
        assert_eq!(ds.n_obs(), 4);
    }

    #[test]
    fn multi_statement_program() {
        let mut s = make_session();
        write_people(&mut s);
        // create, then select (listing), then drop — all in one program.
        run_sql(
            "create table big as select * from t where age >= 12; \
             select name from big; \
             drop table big;",
            &mut s,
        );
        // big was dropped at the end.
        assert!(!s.libs.get("WORK").unwrap().exists("BIG"));
        let listing = s.listing.into_string();
        // selected names of those with age >= 12 (Bo, Cy).
        assert!(listing.contains("Bo"), "listing: {listing}");
        assert!(listing.contains("Cy"), "listing: {listing}");
        let log = s.log.into_string();
        assert!(
            log.contains("Table WORK.BIG created, with 2 rows and"),
            "log: {log}"
        );
        assert!(
            log.contains("Table WORK.BIG has been dropped."),
            "log: {log}"
        );
    }
}
