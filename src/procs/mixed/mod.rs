//! PROC MIXED — linear mixed models with REML / ML estimation (M28).
//!
//! Scope implemented:
//! - CLASS statement (categorical variables; used to identify SUBJECT levels
//!   and to build fixed-effects reference coding).
//! - MODEL effects = / [NOINT] [SOLUTION] [DDFM=CONTAIN] : general
//!   fixed-effects design (intercept, continuous covariates, CLASS reference
//!   coding; NOINT with a CLASS effect is an ERROR until J06-P2).
//! - one RANDOM INTERCEPT / SUBJECT=<var> TYPE=VC|CS, or one
//!   REPEATED [effect] / SUBJECT=<var> TYPE=AR(1)|UN : covariance structures
//!   over V = ZGZ' + R (R indexed by order of appearance within a subject; a
//!   given repeated effect must agree with that order).
//! - METHOD=REML (default) and METHOD=ML; NOBOUND on the closed-form path.
//!
//! Estimation: closed-form (legacy VC single random intercept) or a general
//! (RE)ML optimisation (Nelder-Mead + restarts + coordinate polish) over the
//! V(theta) = ZGZ' + R covariance; the estimates reproduce the balanced
//! single-random-intercept oracle exactly.
//!
//! Contract (CONTRIBUTING §5, docs/support-contract.md): every other option
//! or statement that can change a result is an ERROR at parse time
//! (COVTEST/ASYCOV, ESTIMATE/CONTRAST/LSMEANS, PROC/MODEL/RANDOM/REPEATED
//! options…); display-only options (CL, G, GCORR, V, R, RCORR, RANDOM
//! SOLUTION…) are WARNINGs. A variance component truncated to 0 emits
//! "NOTE: Estimated G matrix is not positive definite." and a non-converged
//! search is a WARNING, not a silent success.

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
    /// The repeated effect variable (`REPEATED time / …`), if given.
    pub effect: Option<String>,
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
}

#[derive(Debug, Clone)]
pub struct MixedAst {
    pub data: Option<DatasetRef>,
    pub method: Method,
    pub nobound: bool,
    pub class_vars: Vec<String>,
    pub model: Option<ModelSpec>,
    pub random: Option<RandomSpec>,
    pub repeated: Option<RepeatedSpec>,
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
mod contract_tests;
#[cfg(test)]
mod tests;
