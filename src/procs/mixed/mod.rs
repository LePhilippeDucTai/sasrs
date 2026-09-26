//! PROC MIXED — linear mixed models with REML / ML estimation (M28).
//!
//! Scope implemented:
//! - CLASS statement (categorical variables; used to identify SUBJECT levels
//!   and to build fixed-effects reference coding).
//! - MODEL effects = / [NOINT] [SOLUTION] : general fixed-effects design
//!   (intercept, continuous covariates, CLASS reference coding).
//! - RANDOM INTERCEPT / SUBJECT=<var> TYPE=VC|CS and
//!   REPEATED effect / SUBJECT=<var> TYPE=VC|CS|AR(1)|UN : covariance
//!   structures over V = ZGZ' + R.
//! - METHOD=REML (default) and METHOD=ML (unknown METHOD=/TYPE= values and
//!   DDFM= other than CONTAIN are ERRORs, no silent fallback).
//!
//! Estimation: closed-form (legacy VC single random intercept) or a general
//! (RE)ML optimisation (Nelder-Mead + restarts + coordinate polish) over the
//! V(theta) = ZGZ' + R covariance; the estimates reproduce the balanced
//! single-random-intercept oracle exactly.
//!
//! Parse-accepted but not implemented (NOTE emitted): ESTIMATE, CONTRAST,
//! COVTEST, ASYCOV, NOBOUND, G/GCORR/R/RCORR options; a variance component
//! truncated to 0 emits "NOTE: Estimated G matrix is not positive definite."
//! and a non-converged search is a WARNING, not a silent success.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::decode_column;
use crate::procs::common::un_block;
use crate::procs::common::{fmt2, fmt4};
use crate::session::Session;
use crate::stat::{dot, log_det_spd, matrix_vec_mult};
use crate::stat::{invert_matrix, student_t_cdf};
use crate::token::TokenKind;
use crate::value::Value;

use crate::procs::lincom::build_design;

mod fit;
mod general;
mod general_report;
mod legacy;
mod legacy_report;
mod parse;
mod plan;

pub use parse::parse;

use fit::*;
use general::*;
use general_report::*;
use legacy::*;
use legacy_report::*;
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
