//! PROC UNIVARIATE (jalon M5).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc univariate data=a ; var v... ; [by ... ;] run ;`
//!
//! Sections du rapport par variable (fidèles au listing SAS) :
//! - Moments : N, Mean, Std Deviation, Skewness (définition SAS avec
//!   correction n/(n-1)(n-2)), Kurtosis (excès, formule SAS), Sum,
//!   Variance, Corrected SS, Uncorrected SS, Coeff Variation, Std Error.
//! - Basic Statistical Measures : mean/median/mode ; std/variance/
//!   range/IQR.
//! - Quantiles : 100% Max, 99%, 95%, 90%, 75% Q3, 50% Median, 25% Q1,
//!   10%, 5%, 1%, 0% Min — DÉFINITION 5 de SAS (empirique, moyenne aux
//!   discontinuités) et PAS l'interpolation linéaire par défaut de
//!   Polars : implémenter à la main sur la colonne triée non-missing.
//! - Extreme Observations : 5 plus basses / 5 plus hautes avec n° d'obs.
//! Les missings sont exclus (compter et afficher la section Missing
//! Values si présents).

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::error::Result;
use crate::parser::StatementStream;
use crate::session::Session;

pub struct UnivariateAst {
    pub data: Option<DatasetRef>,
    pub var: Vec<String>,
}

pub fn parse(ts: &mut StatementStream) -> Result<UnivariateAst> {
    todo!()
}

pub fn execute(ast: &UnivariateAst, session: &mut Session) -> Result<()> {
    todo!("quantiles définition 5 à la main, cf. tête de fichier")
}
