//! PROC MIXED — linear mixed models with REML / ML estimation (M28).
//!
//! Scope implemented:
//! - CLASS statement (categorical variables; used to identify SUBJECT levels).
//! - MODEL response = <fixed> / [solution] [ddfm=...] [noint] : here the only
//!   fully-supported fixed-effects structure is intercept-only (`model y = `),
//!   matching the verified oracle. Additional fixed CLASS effects are accepted
//!   but only the intercept design (X = ones) is exercised by the oracle.
//! - RANDOM intercept / SUBJECT=<var> TYPE=VC|CS : random intercept per
//!   subject. VC and CS are identical for a single random intercept (balanced
//!   or not), so both map to the same variance-components model:
//!   V = σ²_u · Z Z' + σ²_e · I
//! - METHOD=REML (default) and METHOD=ML.
//!
//! Estimation: for a single random intercept the REML/ML estimates have a
//! closed form for *balanced* designs (equal #obs per subject) via the method
//! of moments; this is exact and is what SAS reports. For unbalanced designs we
//! fall back to a 1-D profile search on λ = σ²_u/σ²_e. β̂ and SE(β̂) are then
//! formed from the general V-based formulas, which reproduce the balanced oracle
//! exactly.
//!
//! Parse-accepted but not implemented (NOTE emitted): TYPE=AR(1)/UN (proper
//! error), REPEATED, ESTIMATE, CONTRAST, COVTEST, ASYCOV, NOBOUND, G/GCORR/
//! R/RCORR options, DDFM= (we always print/use Contain).

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::decode_column;
use crate::session::Session;
use crate::stat::{invert_matrix, student_t_cdf};
use crate::token::TokenKind;
use crate::value::Value;

use crate::procs::lincom::build_design;

mod fit;
mod general;
mod general_report;
mod legacy;
mod legacy_report;
mod linalg;
mod parse;
mod plan;

pub use parse::parse;

use fit::*;
use general::*;
use general_report::*;
use legacy::*;
use legacy_report::*;
use linalg::*;
use plan::*;

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Reml,
    Ml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CovType {
    Vc,
    Cs,
    Ar1,
    Un,
}

#[derive(Debug, Clone)]
pub struct RandomSpec {
    /// The random effect terms (e.g. ["intercept"]). Only `intercept` is
    /// implemented; other terms produce an error.
    pub effects: Vec<String>,
    pub subject: Option<String>,
    pub cov_type: CovType,
}

#[derive(Debug, Clone)]
pub struct RepeatedSpec {
    pub subject: Option<String>,
    pub cov_type: CovType,
}

#[derive(Debug, Clone)]
pub struct ModelSpec {
    pub response: String,
    pub fixed: Vec<String>,
    pub solution: bool,
    pub noint: bool,
    pub ddfm: Option<String>,
    pub nofit: bool,
}

#[derive(Debug, Clone)]
pub struct LsmeansSpec {
    pub effect: String,
    pub diff: bool,
    pub pdiff: bool,
    pub cl: bool,
    pub alpha: f64,
}

#[derive(Debug, Clone)]
pub struct MixedAst {
    pub data: Option<DatasetRef>,
    pub method: Method,
    pub covtest: bool,
    pub nobound: bool,
    pub asycov: bool,
    pub class_vars: Vec<String>,
    pub model: Option<ModelSpec>,
    pub random: Option<RandomSpec>,
    pub repeated: Option<RepeatedSpec>,
    pub lsmeans: Vec<LsmeansSpec>,
    /// Labels of ESTIMATE statements seen (for NOTE emission).
    pub estimate_labels: Vec<String>,
    /// Labels of CONTRAST statements seen (for NOTE emission).
    pub contrast_labels: Vec<String>,
}

use crate::procs::common::fmt_p_num as fmt_p;

use crate::procs::common::centered;

use crate::procs::common::value_label;

pub fn execute(ast: &MixedAst, session: &mut Session) -> Result<()> {
    if is_legacy_case(ast) {
        execute_legacy(ast, session)
    } else {
        execute_general(ast, session)
    }
}

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests;
