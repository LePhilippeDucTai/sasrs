//! Parser de l'étape DATA (sous-ensemble M1 ; M2+ étend).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! Appelé par `parser::next_block()` APRÈS consommation du mot-clé `data`.
//!
//! ## Statement DATA
//! `data ref [ref]* ;` — une ou plusieurs sorties (`DatasetRef`).
//! `data _null_;` → zéro sortie (reconnaître `_NULL_`, insensible casse).
//!
//! ## Statements du corps (boucle jusqu'à `run;` ou frontière implicite)
//! - `set ref ;`                  → `DsStmt::Set` (M1 : un seul dataset,
//!                                  pas d'options ; plusieurs → ERROR
//!                                  "not yet implemented")
//! - `ident = expr ;`             → `DsStmt::Assign`
//! - `if expr then stmt [else stmt]` → `DsStmt::If` ; les branches sont
//!   UN statement (récunsion sur le parseur de statement) ; `do; ...
//!   end;` permet les blocs.
//! - `if expr ;`                  → `DsStmt::SubsettingIf`
//! - `do ; stmts end ;`           → `DsStmt::Block` (M1 : non itératif
//!                                  seulement ; `do i = ...` → ERROR M2)
//! - `output ;`                   → `DsStmt::Output` (M1 : sans cible)
//! - `keep v1 v2... ;` / `drop ... ;`
//! - `stop ;`
//! - mot-clé inconnu (retain, merge, array, length, where, ...) → ERROR
//!   "Statement XXX is not yet implemented", l'étape entière est invalide
//!   (comme une erreur de compilation SAS : "step not executed") mais on
//!   CONTINUE de parser jusqu'à la frontière pour ne pas désynchroniser.
//!
//! Renvoie `DataStepAst { outputs, stmts, span }`. Si erreurs accumulées,
//! renvoyer la première (l'exécuteur loggue et saute le bloc).

#![allow(unused_variables, dead_code)]

use super::StatementStream;
use crate::ast::{DataStepAst, DsStmt};
use crate::error::Result;

pub fn parse_data_step(ts: &mut StatementStream) -> Result<DataStepAst> {
    todo!("cf. plan en tête de fichier")
}

/// Un statement du corps (récursif pour IF/THEN/ELSE et DO/END).
fn parse_statement(ts: &mut StatementStream) -> Result<DsStmt> {
    todo!()
}
