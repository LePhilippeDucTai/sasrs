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

#![allow(unused_variables, dead_code)]

pub mod ast;
pub mod parser;
pub mod plan;

use crate::error::Result;
use crate::session::Session;

pub fn execute(program: &ast::SqlProgram, session: &mut Session) -> Result<()> {
    todo!("exécuter chaque SqlStmt via plan::lower + collect/écriture")
}
