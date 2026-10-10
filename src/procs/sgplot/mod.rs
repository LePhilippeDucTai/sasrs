//! PROC SGPLOT — graphique statistique (M29.2).
//!
//! PROC SGPLOT lit un dataset et produit UNE image (PNG ou SVG) via
//! l'infrastructure ODS GRAPHICS (M29.1, module [`crate::graphics::render`]).
//!
//! # Modèle d'exécution selon l'état
//!
//! - `ods_graphics.enabled == false` → NOTE de non-activation, EXIT 0.
//! - `enabled == true` mais build par défaut (sans `--features graphics`) →
//!   NOTE « image deferred », EXIT 0.
//! - `enabled == true` + `--features graphics` → l'image est matérialisée et la
//!   NOTE « Output '...' (WxH) written. » est émise.
//!
//! LOESS et DENSITY sont PARSÉS sans erreur ; le build par défaut les DIFFÈRE
//! (NOTE), le build graphics les rend.
//!
//! # Contrat de support (J02-P6)
//!
//! Aucune demande n'est plus ignorée en silence : une option que le moteur ne
//! rend pas donne un WARNING d'affichage (parsing), un tracé que le moteur
//! n'assemble pas un WARNING de la couche execute, BY et les instructions de
//! tracé non implémentées une ERROR ; `DATA=` et les variables sont validés
//! dans les deux builds (`crate::ods_graphics::contract`).
//!
//! # Invariant build par défaut
//!
//! Tout le code de génération d'image est sous `#[cfg(feature = "graphics")]`.
//! Les champs de l'AST qui ne sont consultés que par ce code sont annotés
//! `#[cfg_attr(not(feature = "graphics"), allow(dead_code))]` pour préserver
//! l'invariant « 0 warning » du build par défaut.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::ods_graphics::contract;
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::expect_eq;
use crate::procs::common::{expect_ident, read_value};
use crate::session::Session;
use crate::token::TokenKind;

mod execute;
mod parse;
mod parse_stmt;
mod parse_util;

pub use execute::execute;
pub use parse::parse;

use parse_stmt::*;
use parse_util::*;

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone)]
pub struct SgplotAst {
    /// `DATA=` ; `None` → `_LAST_`.
    pub data_ref: Option<DatasetRef>,
    /// Statements de tracé (SCATTER, SERIES, …) dans l'ordre du source ; le
    /// moteur en compose une image (tracé principal + superpositions).
    pub plot_stmts: Vec<SgplotStmt>,
    /// Options XAXIS (LABEL= et les bornes de VALUES= sont rendus).
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub xaxis: Option<AxisOpts>,
    /// Options YAXIS.
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub yaxis: Option<AxisOpts>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SgplotStmt {
    Scatter {
        x: String,
        y: String,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        group: Option<String>,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        markerattrs: Option<MarkerAttrs>,
    },
    Series {
        x: String,
        y: String,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        group: Option<String>,
    },
    VBar {
        category: String,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        response: Option<String>,
        stat: BarStat,
    },
    HBar {
        category: String,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        response: Option<String>,
        stat: BarStat,
    },
    Histogram {
        var: String,
        binwidth: Option<f64>,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        scale: HistScale,
    },
    Density {
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        var: String,
        /// `TYPE=KERNEL` → vrai ; défaut (ou `TYPE=NORMAL`) → faux.
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        kernel: bool,
    },
    VBox {
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        category: Option<String>,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        response: String,
    },
    Reg {
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        x: String,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        y: String,
        degree: u32,
    },
    Loess {
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        x: String,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        y: String,
        #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
        smooth: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarStat {
    Freq,
    Sum,
    Mean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistScale {
    Count,
    Percent,
    Proportion,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AxisOpts {
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub label: Option<String>,
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub values_min: Option<f64>,
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub values_max: Option<f64>,
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub type_: Option<AxisType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisType {
    Linear,
    Log,
    Discrete,
}

/// MARKERATTRS=(SYMBOL= COLOR= SIZE=) — parsé, non rendu (WARNING, J02-P6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerAttrs {
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub symbol: Option<String>,
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub color: Option<String>,
    #[cfg_attr(not(feature = "graphics"), allow(dead_code))]
    pub size: Option<String>,
}

// ───────────────────────── Rendu (feature graphics) ─────────────────────────

#[cfg(feature = "graphics")]
pub(crate) mod graphics_impl;

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests;

#[cfg(test)]
mod contract_tests;
