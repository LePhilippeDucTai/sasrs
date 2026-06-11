//! PROC DATASETS (jalon M7) — run-group proc terminée par QUIT;.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc datasets lib=work [nolist] ; delete a b ; change old=new ;
//! [run ;] quit ;`
//!
//! - Run-group : les statements s'exécutent à chaque `run;` ET à
//!   `quit;` — M7 : exécution à quit; suffit, documenter l'écart.
//! - Sans NOLIST : afficher le répertoire de la librairie (table Name /
//!   Member Type DATA / nb obs).
//! - `delete` → `LibraryProvider::delete` (inexistant → WARNING comme
//!   SAS, pas ERROR) ; `change old=new` → rename (ajouter `rename` au
//!   trait LibraryProvider, prévu dans PLAN.md).

#![allow(unused_variables, dead_code)]

use crate::error::Result;
use crate::parser::StatementStream;
use crate::session::Session;

pub struct DatasetsAst {
    pub lib: String,
    pub nolist: bool,
    pub deletes: Vec<String>,
    pub changes: Vec<(String, String)>,
}

pub fn parse(ts: &mut StatementStream) -> Result<DatasetsAst> {
    todo!()
}

pub fn execute(ast: &DatasetsAst, session: &mut Session) -> Result<()> {
    todo!()
}
