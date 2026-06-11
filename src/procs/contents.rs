//! PROC CONTENTS (jalon M4).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc contents data=lib.x [varnum] ; run ;` — affiche les métadonnées :
//! en-tête (nom, observations, variables), puis table des variables
//! (# / Variable / Type Num|Char / Len / Format / Label), triée par nom
//! (défaut) ou par position (VARNUM). Lit uniquement les métadonnées
//! (VarMeta + hauteur) — pas besoin de matérialiser les données :
//! prévoir plus tard un `LibraryProvider::read_meta` si les fichiers
//! deviennent gros.
//! `data=lib._all_` : liste les tables de la librairie (via `list()`).

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::error::Result;
use crate::parser::StatementStream;
use crate::session::Session;

pub struct ContentsAst {
    pub data: Option<DatasetRef>,
    pub varnum: bool,
    /// data=lib._all_
    pub all: bool,
}

pub fn parse(ts: &mut StatementStream) -> Result<ContentsAst> {
    todo!()
}

pub fn execute(ast: &ContentsAst, session: &mut Session) -> Result<()> {
    todo!()
}
