//! Helpers partagés par plusieurs PROCs.
//!
//! Ce module centralise les fonctions utilitaires communes afin d'éviter la
//! duplication de code entre `means`, `freq`, `univariate`, `sort`,
//! `transpose` et `append`. Chaque fonction est extraite verbatim de son
//! premier site d'apparition ; aucune logique n'est modifiée.

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::missing::{num_to_value, value_to_num};
use crate::parser::StatementStream;
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{Value, VarType, format_best};
use std::cmp::Ordering;

mod dist;
mod model;
mod stmt;

pub(crate) use dist::*;
pub(crate) use model::*;
pub(crate) use stmt::*;

mod stats;

mod by;

mod parse;

mod dataset;

mod format;

pub use by::ByCol;

pub use by::by_groups;

pub use by::group_by_keys;

pub use by::resolve_by_cols;

pub use dataset::char_var_meta;

pub use dataset::num_var_meta;

pub use dataset::open_input;

pub use dataset::open_input_display;

pub use dataset::open_resolved;

pub use dataset::resolve_last_dataset;

pub use format::centered;

pub use format::fmt_p;

pub use format::fmt_p_num;

pub use format::value_label;

pub use parse::expect_eq;

pub use parse::parse_dataset_opt;

pub use parse::parse_out_opt;

pub use parse::parse_proc_body;

pub use parse::parse_proc_options;

pub use parse::unknown_option_error;

pub use stats::decode_column;

pub use stats::partition_numeric;

pub use stats::partition_weighted;

pub use stats::sample_std;

pub use stats::t_quantile;

pub use stats::two_sided_p;

// ───────────────────────── shared distributions ─────────────────────────
//
// The distribution machinery (ln_gamma / betai / incomplete gamma / normal
// CDF and probit / log-combinatorics) lives in `crate::stat::dists`; the
// re-exports below keep the historical `procs::common::{...}` paths working.
// The private copies in `corr.rs` and `datastep/functions.rs` are NOT folded
// here on purpose: their algorithms differ (constants, iteration counts) and
// the printed digits in their listings depend on them.

use crate::stat::dists::{gammq, student_t_cdf};

pub use crate::stat::dists::{ln_choose, ln_factorial, phi_inv, probnorm};

#[cfg(test)]
mod tests;
