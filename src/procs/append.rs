//! PROC APPEND (jalon M7).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc append base=a data=b [force] ; run ;`
//!
//! - base inexistante → créée comme copie de data (NOTE SAS).
//! - Sans FORCE : toute variable de DATA absente de BASE, ou type
//!   différent, ou longueur char supérieure → ERROR "No appending done
//!   because of anomalies...". Avec FORCE : variables en trop ignorées
//!   (WARNING), longueurs tronquées à celle de BASE, variables de BASE
//!   absentes de DATA → missing.
//! - Alignement par NOM (pas par position) ; vstack Polars après
//!   réordonnancement des colonnes de data sur le schéma de base.
//! - NOTEs : "Appending WORK.B to WORK.A." + obs lues / obs ajoutées.

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::error::Result;
use crate::parser::StatementStream;
use crate::session::Session;

pub struct AppendAst {
    pub base: DatasetRef,
    pub data: DatasetRef,
    pub force: bool,
}

pub fn parse(ts: &mut StatementStream) -> Result<AppendAst> {
    todo!()
}

pub fn execute(ast: &AppendAst, session: &mut Session) -> Result<()> {
    todo!()
}
