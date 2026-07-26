mod cdf;
mod generic;
mod legacy;
mod pdf;
mod special;

pub(crate) use cdf::*;
pub(crate) use generic::*;
pub(crate) use legacy::*;
pub(crate) use pdf::*;
pub(crate) use special::*;

// ──────────────────────────────────────────────────────────────────────────────
// Numerical helpers for probability distributions (M15.4)
//
// These are intentionally self-contained (no external crates). The accuracy
// target is ~1e-9 absolute, sufficient to match documented SAS/R/scipy results
// to the displayed precision.
// ──────────────────────────────────────────────────────────────────────────────

use super::*;
