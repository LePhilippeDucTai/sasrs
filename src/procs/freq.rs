//! PROC FREQ (jalon M5).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc freq data=a ; tables v1 v2 v1*v2 [/ missing nopercent norow
//! nocol nofreq out=b] ; run ;`
//!
//! - Table 1 voie : valeurs triées (ordre sas_cmp), colonnes Frequency,
//!   Percent, Cumulative Frequency, Cumulative Percent. Par défaut les
//!   missings sont EXCLUS du tableau et comptés sous une ligne
//!   "Frequency Missing = N" ; option MISSING les réintègre (et ils
//!   entrent alors dans les pourcentages).
//! - Table 2 voies (v1*v2) : crosstab avec Frequency / Percent / Row Pct
//!   / Col Pct par cellule + marges. Implémenter par group_by Polars sur
//!   les paires puis mise en forme manuelle (le rendu SAS en blocs de 4
//!   lignes par cellule).
//! - out= (1 voie) : colonnes <var>, COUNT, PERCENT.

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::error::Result;
use crate::parser::StatementStream;
use crate::session::Session;

pub struct FreqAst {
    pub data: Option<DatasetRef>,
    pub tables: Vec<TableRequest>,
}

pub struct TableRequest {
    /// 1 nom = une voie ; 2 noms = crosstab v1*v2.
    pub vars: Vec<String>,
    pub missing: bool,
    pub out: Option<DatasetRef>,
}

pub fn parse(ts: &mut StatementStream) -> Result<FreqAst> {
    todo!()
}

pub fn execute(ast: &FreqAst, session: &mut Session) -> Result<()> {
    todo!()
}
