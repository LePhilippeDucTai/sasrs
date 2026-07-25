//! Shared linear-combination engine for PROC GLM (and future MIXED/GENMOD…).
//!
//! M37.1 — Phase G infrastructure. This module factors the linear-algebra core
//! that PROC GLM's *multiway* path uses for parameter estimates, LS-means and
//! linear-combination tests into a single reusable engine:
//!
//! ```text
//! LinCombEngine { beta, cov, coding, df, mse }
//!   estimate(l, c) -> LinEstimate   // L·β − c with se / t / Pr>|t|
//!   contrast(l, c) -> LinContrast   // single-row F test of L·β − c
//!   lsmeans(effect)  -> Vec<LsMean> // LS-means of a main-effect factor
//! ```
//!
//! ## Byte-identity contract
//!
//! GLM's listing must remain strictly byte-identical. The numeric methods here
//! reproduce the EXACT floating-point operation order of the code they were
//! extracted from (`execute_multiway` / `lsmean_coef_vector` in `glm.rs`):
//!
//! - `cov` is kept as the raw `(XᵀX)⁻¹`; `mse` is carried separately and applied
//!   as `(mse * q).sqrt()`. Pre-folding `mse` into `cov` would change the last
//!   bit (`mse·Σ l·inv·l` ≠ `Σ l·(mse·inv)·l`).
//! - `lsmeans` keeps the `if l[a]==0.0 { continue }` skip and the
//!   `q += l[a]*inv[a][b]*l[b]` accumulation order verbatim.
//! - `estimate` uses the same quadratic form (no skip), matching the
//!   parameter-table SE when called with a unit selector vector (the zero-skip
//!   and the unit vector collapse to `inv[col][col]`).
//!
//! The `coding` describes the reference-cell dummy layout of the fitted design
//! so that LS-means estimable functions can be rebuilt the same way GLM did.

use crate::error::{Result, SasError};
use crate::procs::common::value_label;
use crate::stat::{chisq_cdf, f_cdf, student_t_cdf};
use crate::stat::linalg::invert_matrix;
use crate::value::Value;


mod coding;
mod engine;
mod score;

pub use coding::Coding;
pub use coding::DesignColumn;
pub use coding::Param;
pub use coding::build_design;
pub use coding::class_coding;
pub use coding::class_levels;
pub use engine::LinCombEngine;
pub use engine::lsmean_coef;
pub use score::ScoreTest;
pub use score::score_test;


/// Result of an `estimate` call: L·β − c with inference.
#[derive(Debug, Clone)]
pub struct LinEstimate {
    pub estimate: f64,
    pub se: f64,
    pub t: f64,
    /// Two-sided Pr > |t| (None when t is NaN).
    pub p: Option<f64>,
}

/// Result of a `contrast` call: single-row F test of L·β − c.
#[derive(Debug, Clone)]
pub struct LinContrast {
    pub f: f64,
    /// Pr > F (None when F is NaN).
    pub p: Option<f64>,
    pub df1: f64,
    pub df2: f64,
}

/// One LS-mean row for a main-effect factor level.
#[derive(Debug, Clone)]
pub struct LsMean {
    /// Display label of the level (matches the one-way path scheme).
    pub level_label: String,
    pub estimate: f64,
    pub se: f64,
    pub t: f64,
    pub p: Option<f64>,
}

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests;
