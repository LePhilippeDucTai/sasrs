//! Registre des PROCs : chaque PROC possède son propre parser (grammaire
//! contextuelle SAS) et son exécuteur.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `parse_proc(name, ts)` est appelé par `parser::next_block()` APRÈS
//! consommation de `proc <name>` ; le sous-parser consomme tout jusqu'à
//! `run;` (ou `quit;` pour les run-group procs : SQL, DATASETS).
//! PROC inconnue → ERROR "Procedure XXX not found." + récupération.
//!
//! `execute_proc` dispatche, entoure d'un `StepTimer`, et écrit la NOTE
//! de fin "NOTE: PROCEDURE XXX used (Total process time): ...".
//!
//! Convention commune : `data=` absent → `session.last_dataset` (_LAST_) ;
//! aucun dataset créé dans la session → ERROR (comme SAS _LAST_ vide).

#![allow(unused_variables, dead_code)]

pub mod append;
pub mod contents;
pub mod datasets;
pub mod format;
pub mod freq;
pub mod means;
pub mod print;
pub mod sort;
pub mod transpose;
pub mod univariate;

use crate::error::Result;
use crate::parser::StatementStream;
use crate::session::Session;

pub enum ProcAst {
    Print(print::PrintAst),
    Sort(sort::SortAst),
    Means(means::MeansAst),
    Freq(freq::FreqAst),
    Transpose(transpose::TransposeAst),
    Univariate(univariate::UnivariateAst),
    Contents(contents::ContentsAst),
    Datasets(datasets::DatasetsAst),
    Append(append::AppendAst),
    Format(format::FormatAst),
    Sql(crate::sql::ast::SqlProgram),
}

pub fn parse_proc(name: &str, ts: &mut StatementStream) -> Result<ProcAst> {
    todo!("match name.to_lowercase() → sous-parser ; inconnu → ERROR")
}

pub fn execute_proc(name: &str, ast: &ProcAst, session: &mut Session) -> Result<()> {
    todo!("dispatch + StepTimer + NOTE 'PROCEDURE XXX used'")
}
