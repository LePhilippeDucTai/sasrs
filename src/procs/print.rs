//! PROC PRINT (jalon M1).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! ## Syntaxe M1
//! `proc print data=lib.x [noobs] [label] ; [var v1 v2... ;] run ;`
//!
//! ## Exécution
//! 1. Résoudre le dataset (data= ou _LAST_) ; lire via LibraryProvider ;
//!    forwarder les notes de coercition au log.
//! 2. Colonnes : `var` si présent (ERROR si variable inconnue :
//!    "Variable XXXX not found."), sinon toutes dans l'ordre du dataset.
//! 3. Rendu listing : `page_header()` puis table —
//!    - colonne `Obs` (1..n, alignée droite) sauf NOOBS ;
//!    - numériques : format de la variable si défini (M4 — avant cela
//!      BEST12. trimé via `value::format_best(v, 12)`), missings `.` ou
//!      lettre spéciale ; alignés DROITE ;
//!    - caractères : tels quels, alignés GAUCHE.
//! 4. NOTEs log : "There were N observations read from the data set
//!    WORK.X." (l'appelant procs::execute_proc ajoute la NOTE de timing).

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::error::Result;
use crate::parser::StatementStream;
use crate::session::Session;

pub struct PrintAst {
    pub data: Option<DatasetRef>,
    pub vars: Option<Vec<String>>,
    pub noobs: bool,
}

pub fn parse(ts: &mut StatementStream) -> Result<PrintAst> {
    todo!()
}

pub fn execute(ast: &PrintAst, session: &mut Session) -> Result<()> {
    todo!("cf. plan en tête de fichier")
}
