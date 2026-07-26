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

mod convert;
mod dml;
mod select;

use convert::*;
use dml::*;
use select::*;

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

use crate::value::{Value, VarType, format_best};

use ast::{SqlProgram, SqlStmt};

use polars::prelude::*;

pub fn execute(program: &SqlProgram, session: &mut Session) -> Result<()> {
    for stmt in &program.stmts {
        match stmt {
            SqlStmt::Select(sel) => exec_select(sel, session)?,
            SqlStmt::CreateTableAs { table, query } => exec_create_table_as(table, query, session)?,
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
            SqlStmt::InsertSelect { table, query } => exec_insert_select(table, query, session)?,
            SqlStmt::DeleteFrom { table, where_ } => exec_delete(table, where_.as_ref(), session)?,
            SqlStmt::Describe(table) => exec_describe(table, session)?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
