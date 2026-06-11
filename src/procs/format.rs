//! PROC FORMAT (jalon M4).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc format ; value sexfmt 1='Male' 2='Female' other='?' ;
//! value $cityfmt 'PAR'='Paris' ; run ;`
//!
//! - Parser chaque statement VALUE en `formats::userdef::UserFormat`
//!   (plages : valeur, `a-b`, `low-<b`, `a<-high`, listes virgule).
//! - Enregistrer dans `session.format_catalog` (nom upcase, `$` inclus
//!   pour les formats char). NOTE par format : "Format SEXFMT has been
//!   output." — en session seulement, pas de catalogue persistant
//!   (limitation documentée dans README).
//! - INVALUE (informats utilisateur) : M4+, ERROR propre d'ici là.

#![allow(unused_variables, dead_code)]

use crate::error::Result;
use crate::parser::StatementStream;
use crate::session::Session;

pub struct FormatAst {
    /// (nom, définition brute à parser en UserFormat)
    pub values: Vec<(String, crate::formats::userdef::UserFormat)>,
}

pub fn parse(ts: &mut StatementStream) -> Result<FormatAst> {
    todo!()
}

pub fn execute(ast: &FormatAst, session: &mut Session) -> Result<()> {
    todo!()
}
