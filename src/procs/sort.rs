//! PROC SORT (jalon M3).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc sort data=a [out=b] [nodupkey] [noduprecs] ; by [descending] v1
//! [descending] v2... ; run ;`
//!
//! ## Piège central : la collation SAS des missings
//! Ordre numérique SAS : `._ < . < .A < ... < .Z < nombres`. Les flags
//! nulls_first/last de Polars ne connaissent pas les missings spéciaux
//! (NaN-payload). Solution : pour chaque clé Float64, fabriquer une
//! colonne compagnon de rang i8 (null→1, `._`→0, `.A`..`.Z`→2..27 — la
//! valeur, NaN décodé via `missing::decode_nan` ; non-missing→28) et
//! trier sur (rang, clé) ; le tri Polars est STABLE, l'égalité de rang
//! 28 laisse la clé départager. DESCENDING inverse les DEUX colonnes.
//! - NODUPKEY : déduplication sur les clés BY après tri (garder la
//!   première), NOTE "N observations with duplicate key values were
//!   deleted." ; NODUPRECS compare la ligne entière.
//! - out= absent → remplace le dataset d'entrée.
//! - Conserver les VarMeta du dataset d'entrée à l'identique.

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::error::Result;
use crate::parser::StatementStream;
use crate::session::Session;

pub struct SortAst {
    pub data: Option<DatasetRef>,
    pub out: Option<DatasetRef>,
    pub by: Vec<(String, bool)>, // (var, descending)
    pub nodupkey: bool,
    pub noduprecs: bool,
}

pub fn parse(ts: &mut StatementStream) -> Result<SortAst> {
    todo!()
}

pub fn execute(ast: &SortAst, session: &mut Session) -> Result<()> {
    todo!("cf. piège collation en tête de fichier")
}
