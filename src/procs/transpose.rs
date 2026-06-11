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

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::error::Result;
use crate::parser::StatementStream;
use crate::session::Session;

pub struct TransposeAst {
    pub data: Option<DatasetRef>,
    pub out: Option<DatasetRef>,
    pub prefix: Option<String>,
    pub by: Vec<String>,
    pub id: Option<String>,
    pub var: Vec<String>,
}

pub fn parse(ts: &mut StatementStream) -> Result<TransposeAst> {
    todo!()
}

pub fn execute(ast: &TransposeAst, session: &mut Session) -> Result<()> {
    todo!()
}
