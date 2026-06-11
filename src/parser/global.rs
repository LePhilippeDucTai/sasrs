//! Statements globaux : LIBNAME, OPTIONS, TITLEn.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! Appelé par `parser::next_block()` ; le mot-clé de tête est encore dans
//! le stream (peek) ou déjà identifié par l'appelant — convention :
//! l'appelant N'A PAS consommé le mot-clé, `parse_global` le consomme.
//!
//! ## LIBNAME
//! - `libname ref 'chemin' ;`  → `GlobalStmt::Libname` (chemin = littéral
//!   chaîne ; relatif → résolu contre `Session::base_dir` à l'exécution).
//! - `libname ref clear ;`     → `GlobalStmt::LibnameClear`.
//!
//! ## TITLE
//! - `title 'texte' ;` / `titleN 'texte' ;` (N=1..9, suffixe dans
//!   l'ident) ; sans texte → efface. M1 : seul TITLE1 est rendu par le
//!   listing.
//!
//! ## OPTIONS
//! - `options name[=valeur]... ;` → liste brute. L'exécution (executor)
//!   applique `ls=` (40..=256) et ignore le reste avec WARNING
//!   "Option XXX is not yet supported".

#![allow(unused_variables, dead_code)]

use super::StatementStream;
use crate::ast::GlobalStmt;
use crate::error::Result;

pub fn parse_global(ts: &mut StatementStream) -> Result<GlobalStmt> {
    todo!("cf. plan en tête de fichier")
}
