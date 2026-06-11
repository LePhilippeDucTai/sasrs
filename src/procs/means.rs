//! PROC MEANS / SUMMARY (jalon M5).
//!
//! # Plan du fichier — voir PLAN.md  (difficulté : ÉLEVÉE — _TYPE_)
//!
//! `proc means data=a [noprint] [stats...] ; class c1 c2 ; var v1 v2 ;
//! output out=b [stat(var)=name...] ; run ;`  — SUMMARY = MEANS noprint
//! par défaut.
//!
//! ## Sémantique à répliquer
//! - Stats défaut du rapport : N, Mean, Std Dev, Minimum, Maximum.
//!   Stats demandables : n nmiss mean std min max sum range stderr cv.
//!   SAS EXCLUT les missings : appliquer `missing::nullify_specials`
//!   puis utiliser les agrégations Polars (qui ignorent null).
//! - CLASS sans OUTPUT : rapport par combinaison de classes.
//! - OUTPUT OUT= avec CLASS : produit TOUTES les combinaisons de
//!   sous-ensembles de classes — `_TYPE_` = masque binaire (bit le plus
//!   à droite = dernière variable CLASS), `_FREQ_` = effectif. Ordre :
//!   _TYPE_ croissant puis valeurs de classes. Lignes des classes non
//!   actives → missing.
//! - VAR absent : toutes les numériques hors CLASS/BY.
//! - Rapport listing : table par variable x stat, en-tête style SAS
//!   ("The MEANS Procedure", "Analysis Variable : x ...").

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::error::Result;
use crate::parser::StatementStream;
use crate::session::Session;

pub struct MeansAst {
    pub data: Option<DatasetRef>,
    pub summary: bool,
    pub noprint: bool,
    pub stats: Vec<String>,
    pub class: Vec<String>,
    pub var: Vec<String>,
    pub output: Option<MeansOutput>,
}

pub struct MeansOutput {
    pub out: DatasetRef,
    /// (stat, var source, nom de sortie)
    pub specs: Vec<(String, String, String)>,
}

pub fn parse(ts: &mut StatementStream) -> Result<MeansAst> {
    todo!()
}

pub fn execute(ast: &MeansAst, session: &mut Session) -> Result<()> {
    todo!("cf. _TYPE_/_FREQ_ en tête de fichier")
}
