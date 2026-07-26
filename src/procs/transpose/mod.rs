//! PROC TRANSPOSE (jalon M7).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc transpose data=a out=b [prefix=P] [name=_name_] ; [by v...;]
//! [id v;] [var v...;] run ;`
//!
//! NE PAS utiliser le pivot Polars : les règles de nommage SAS sont
//! spécifiques — implémenter par itération de groupes BY :
//! - VAR absent : toutes les numériques hors BY/ID.
//! - Sortie : une ligne par variable VAR (par groupe BY) ; `_NAME_` =
//!   nom de la variable source ; colonnes = `COL1..COLn` (n = max
//!   d'observations par groupe) ou, si ID, les valeurs (formatées) de
//!   la variable ID — valeurs dupliquées dans un groupe → ERROR comme
//!   SAS ("The ID value ... occurs twice in the same BY group"),
//!   noms invalides normalisés règle SAS (préfixe _ si chiffre...).
//! - Transposer du char et du num ensemble → toutes les COL deviennent
//!   char (longueur max), num convertis via BEST12. trimé.
//!
//! # Décisions d'implémentation (documentées pour l'orchestrateur)
//!
//! ## Nommage des colonnes transposées
//! - SANS `id` : `COL1..COLn` où `n` = MAX du nombre d'observations sur
//!   tous les groupes BY (les groupes plus courts sont complétés par des
//!   missings, comme SAS). Avec `prefix=P` : `P1..Pn`.
//! - AVEC `id` : une colonne par valeur DISTINCTE de la variable ID, dans
//!   l'ordre de PREMIÈRE APPARITION dans les données (choix documenté ;
//!   SAS utilise l'ordre de première apparition). Les valeurs sont
//!   formatées : char telle quelle (trimée), num via `format_best(v,12)`
//!   trimé. Les noms invalides sont normalisés (cf. `normalize_name`).
//!   Une valeur d'ID dupliquée DANS UN GROUPE BY → ERROR exacte SAS.
//!
//! ## Mixage char / numérique des variables VAR
//! - Si TOUTES les variables VAR transposées sont numériques → colonnes
//!   transposées NUMÉRIQUES (f64) ; missing préservé.
//! - Si AU MOINS UNE variable VAR est caractère (mixage) → TOUTES les
//!   colonnes transposées deviennent CARACTÈRE : les valeurs numériques
//!   sont converties via `format_best(v,12).trim()`, un missing numérique
//!   devient une chaîne vide (blanc), un missing char reste vide.
//!
//! ## `out=` absent
//! - Pour M7 on EXIGE `out=` : son absence renvoie une ERROR propre
//!   (SAS produirait sinon `WORK._DATAn_`, hors périmètre M7).

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, decode_column};
use crate::procs::common::{char_var_meta, num_var_meta};
use crate::session::Session;
use crate::value::{Value, VarType, format_best};
use polars::prelude::*;
use std::cmp::Ordering;

mod naming;

pub(crate) use naming::*;

pub struct TransposeAst {
    pub data: Option<DatasetRef>,
    pub out: Option<DatasetRef>,
    pub prefix: Option<String>,
    pub by: Vec<String>,
    pub id: Option<String>,
    pub var: Vec<String>,
    /// Name of the `_NAME_` column (from `name=`); defaults to `_NAME_`.
    pub name: Option<String>,
}

/// Parse `proc transpose [data=a] [out=b] [prefix=P] [name=N] ; [by v...;]
/// [id v;] [var v...;] run;`. Called AFTER "proc transpose" has been
/// consumed. Consumes through `run;` / `quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<TransposeAst> {
    let mut data: Option<DatasetRef> = None;
    let mut out: Option<DatasetRef> = None;
    let mut prefix: Option<String> = None;
    let mut name: Option<String> = None;

    // --- PROC TRANSPOSE statement options, until `;` (combinateur M31) ---
    common::parse_proc_options(ts, "TRANSPOSE", |ts, kw| {
        Ok(match kw {
            "data" => {
                data = Some(common::parse_dataset_opt(ts, "DATA")?);
                true
            }
            "out" => {
                out = Some(common::parse_dataset_opt(ts, "OUT")?);
                true
            }
            "prefix" => {
                common::expect_eq(ts, "PREFIX")?;
                prefix = Some(expect_ident(ts, "PREFIX")?);
                true
            }
            "name" => {
                common::expect_eq(ts, "NAME")?;
                name = Some(expect_ident(ts, "NAME")?);
                true
            }
            _ => false,
        })
    })?;

    // --- sub-statements until run;/quit; (combinateur M31) ---
    let mut by: Vec<String> = Vec::new();
    let mut id: Option<String> = None;
    let mut var: Vec<String> = Vec::new();

    common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "by" => {
                ts.next();
                by = ts.parse_name_list()?;
                ts.expect_semi()?;
                true
            }
            "id" => {
                ts.next();
                // Single ID variable for M7: take the first name, skip the rest.
                let names = ts.parse_name_list()?;
                id = names.into_iter().next();
                ts.expect_semi()?;
                true
            }
            "var" => {
                ts.next();
                var = ts.parse_name_list()?;
                ts.expect_semi()?;
                true
            }
            _ => false,
        })
    })?;

    Ok(TransposeAst {
        data,
        out,
        prefix,
        by,
        id,
        var,
        name,
    })
}

/// Execute PROC TRANSPOSE. Called by `procs::execute_proc` (timing wrapper).
pub fn execute(ast: &TransposeAst, session: &mut Session) -> Result<()> {
    let (ds, _, _) = common::open_input(&ast.data, session)?;

    let n_obs = ds.n_obs();

    // Resolve BY columns.
    let mut by_cols: Vec<usize> = Vec::with_capacity(ast.by.len());
    for vname in &ast.by {
        by_cols.push(resolve_var(&ds, vname)?);
    }

    // Resolve ID column (if any).
    let id_col: Option<usize> = match &ast.id {
        Some(vname) => Some(resolve_var(&ds, vname)?),
        None => None,
    };

    // Determine VAR list: explicit `var`, else all NUMERIC variables not in
    // BY and not the ID variable.
    let var_cols: Vec<usize> = if !ast.var.is_empty() {
        let mut v = Vec::with_capacity(ast.var.len());
        for vname in &ast.var {
            v.push(resolve_var(&ds, vname)?);
        }
        v
    } else {
        (0..ds.vars.len())
            .filter(|&i| {
                ds.vars[i].ty == VarType::Num && !by_cols.contains(&i) && Some(i) != id_col
            })
            .collect()
    };

    if var_cols.is_empty() {
        return Err(SasError::runtime(
            "No variables to transpose (the VAR list is empty).",
        ));
    }

    // Decode BY, ID, and VAR columns once each.
    let by_values: Vec<Vec<Value>> = by_cols
        .iter()
        .map(|&ci| decode_column(&ds, ci))
        .collect::<Result<_>>()?;
    let id_values: Option<Vec<Value>> = match id_col {
        Some(ci) => Some(decode_column(&ds, ci)?),
        None => None,
    };
    let var_values: Vec<Vec<Value>> = var_cols
        .iter()
        .map(|&ci| decode_column(&ds, ci))
        .collect::<Result<_>>()?;

    // Mixing rule: transposed columns are char iff ANY VAR is character.
    let any_char = var_cols.iter().any(|&ci| ds.vars[ci].ty == VarType::Char);

    // Group rows by the BY tuple (first-appearance order).
    let groups = group_by_tuple(&by_values, n_obs);

    // Determine the transposed columns layout.
    // WITHOUT ID: COL1..COLn (or P1..Pn), n = max group size.
    // WITH ID: one column per distinct ID value (first appearance order).
    let prefix = ast.prefix.as_deref().unwrap_or("COL");

    // Column names + per-(group,var) value access.
    // We materialise, for every output row, the list of transposed cell
    // Values in the layout column order, then build series.
    let name_col = ast.name.as_deref().unwrap_or("_NAME_");

    // Each output row carries: BY group key (for the leading BY cells),
    // the source var name (for _NAME_), and the transposed cells.
    struct OutRow {
        by_key: Vec<Value>,
        source_name: String,
        cells: Vec<Value>,
    }
    let mut out_rows: Vec<OutRow> = Vec::new();

    // Transposed column names, computed below per layout.
    let trans_names: Vec<String>;

    if let Some(idv) = &id_values {
        // Distinct ID values in first-appearance order across all data.
        let mut distinct: Vec<Value> = Vec::new();
        for v in idv.iter() {
            if !distinct.iter().any(|d| d.sas_cmp(v) == Ordering::Equal) {
                distinct.push(v.clone());
            }
        }
        trans_names = distinct.iter().map(id_value_to_name).collect();

        for (key, grp_rows) in &groups {
            // Map each distinct ID value -> the row (within this group) whose
            // ID matches it. Duplicate ID within a group -> ERROR.
            let mut row_for_id: Vec<Option<usize>> = vec![None; distinct.len()];
            for &r in grp_rows {
                let di = distinct
                    .iter()
                    .position(|d| d.sas_cmp(&idv[r]) == Ordering::Equal)
                    .expect("ID value must be in the distinct set");
                if row_for_id[di].is_some() {
                    let disp = id_value_display(&idv[r]);
                    return Err(SasError::runtime(format!(
                        "The ID value \"{}\" occurs twice in the same BY group.",
                        disp
                    )));
                }
                row_for_id[di] = Some(r);
            }
            for (vi, &vci) in var_cols.iter().enumerate() {
                let mut cells: Vec<Value> = Vec::with_capacity(distinct.len());
                for &maybe_row in &row_for_id {
                    let v = match maybe_row {
                        Some(r) => var_values[vi][r].clone(),
                        None => Value::missing(),
                    };
                    cells.push(v);
                }
                out_rows.push(OutRow {
                    by_key: key.clone(),
                    source_name: ds.vars[vci].name.clone(),
                    cells,
                });
            }
        }
    } else {
        // COL1..COLn where n = max group size.
        let n_cols = groups.iter().map(|(_, r)| r.len()).max().unwrap_or(0);
        trans_names = (1..=n_cols).map(|j| format!("{prefix}{j}")).collect();

        for (key, grp_rows) in &groups {
            for (vi, &vci) in var_cols.iter().enumerate() {
                let mut cells: Vec<Value> = Vec::with_capacity(n_cols);
                for j in 0..n_cols {
                    let v = match grp_rows.get(j) {
                        Some(&r) => var_values[vi][r].clone(),
                        None => Value::missing(),
                    };
                    cells.push(v);
                }
                out_rows.push(OutRow {
                    by_key: key.clone(),
                    source_name: ds.vars[vci].name.clone(),
                    cells,
                });
            }
        }
    }

    // --- Build the output DataFrame column by column ---
    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();

    // Leading BY columns (copy input VarMeta).
    for (bi, &col_idx) in by_cols.iter().enumerate() {
        let meta = &ds.vars[col_idx];
        let series = match meta.ty {
            VarType::Num => {
                let vals: Vec<Option<f64>> = out_rows
                    .iter()
                    .map(|r| value_to_num(&r.by_key[bi]))
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
            VarType::Char => {
                let vals: Vec<Option<String>> =
                    out_rows.iter().map(|r| char_cell(&r.by_key[bi])).collect();
                Series::new(meta.name.as_str().into(), vals)
            }
        };
        columns.push(series.into());
        vars.push(meta.clone());
    }

    // _NAME_ column (char). Length = max source-name length.
    let name_vals: Vec<Option<String>> = out_rows
        .iter()
        .map(|r| Some(r.source_name.clone()))
        .collect();
    let name_len = out_rows
        .iter()
        .map(|r| r.source_name.len())
        .max()
        .unwrap_or(8)
        .max(1);
    columns.push(Series::new(name_col.into(), name_vals).into());
    vars.push(char_var_meta(name_col, name_len));

    // Transposed columns.
    if any_char {
        // CHAR columns. Length = max char-cell length across all cells.
        let mut char_len = 1usize;
        for (ci, name) in trans_names.iter().enumerate() {
            let vals: Vec<Option<String>> = out_rows
                .iter()
                .map(|r| value_to_char(&r.cells[ci]))
                .collect();
            for s in vals.iter().flatten() {
                char_len = char_len.max(s.len());
            }
            columns.push(Series::new(name.as_str().into(), vals).into());
        }
        for nm in &trans_names {
            vars.push(char_var_meta(nm, char_len));
        }
    } else {
        // NUMERIC columns.
        for (ci, name) in trans_names.iter().enumerate() {
            let vals: Vec<Option<f64>> = out_rows
                .iter()
                .map(|r| value_to_num(&r.cells[ci]))
                .collect();
            columns.push(Series::new(name.as_str().into(), vals).into());
            vars.push(num_var_meta(name));
        }
    }

    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars };

    // out= is required for M7.
    let out_ref = ast
        .out
        .clone()
        .ok_or_else(|| SasError::runtime("The OUT= option is required for PROC TRANSPOSE."))?;
    let out_libref = out_ref.libref_or_work();
    let out_table = out_ref.name.to_uppercase();
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

#[cfg(test)]
mod tests;
