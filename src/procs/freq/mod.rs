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
//!
//! ## Statistiques avancées (M21.2)
//! Options après `/` sur le statement TABLES :
//! - **CHISQ** : en deux voies → Pearson + Likelihood Ratio (M10) ; en UNE
//!   voie → test d'ajustement à l'équiprobabilité ("Chi-Square Test for Equal
//!   Proportions", DF = k-1).
//! - **FISHER** / **EXACT** : test exact de Fisher pour les tables 2×2
//!   (probabilités hypergéométriques exactes : F, left/right one-sided, P,
//!   two-sided). r×c (> 2×2) → note de non-support (différé, sans panic).
//! - **MEASURES** / **RELRISK** : odds ratio + risques relatifs cohorte
//!   (Col1/Col2) avec IC 95 % Wald sur l'échelle log, pour les tables 2×2.
//!   Cellules nulles → estimation manquante ("."), jamais de division par 0.
//! - **AGREE** : kappa simple de Cohen pour une table CARRÉE (Po, Pe, ASE,
//!   IC 95 %). Table non carrée → note propre.
//! - **TREND** : test de tendance de Cochran-Armitage pour une table 2×c ou
//!   r×2 (scores 1..k, statistique Z, p uni/bilatérale via `probnorm`).
//! Les blocs ne s'impriment que si leur option est demandée : la sortie par
//! défaut (et le CHISQ deux voies) restent byte-identiques.
//!
//! ## Choix de rendu (documenté pour l'orchestrateur)
//! - Titre centré "The FREQ Procedure" via `page_header()` puis une ligne
//!   centrée.
//! - Une voie : table à 5 colonnes (`<Var>`, Frequency, Percent,
//!   Cumulative Frequency, Cumulative Percent). Sans MISSING et avec des
//!   missings présents, une ligne "Frequency Missing = N" suit la table.
//! - Crosstab v1*v2 : une table dont la colonne de tête liste les valeurs
//!   de `v1` (plus une ligne "Total"), et qui porte une colonne par valeur
//!   de `v2` (plus "Total"). Chaque cellule (croisement) est rendue sur
//!   QUATRE lignes empilées dans la même colonne : Frequency, Percent
//!   (du total général), Row Pct (du total de la ligne), Col Pct (du total
//!   de la colonne). Les cellules de marge "Total" ne portent que
//!   Frequency et Percent (les deux dernières lignes restent vides). On
//!   construit ces lignes empilées à la main puis on les passe à
//!   `write_table` (une "ligne logique" = 4 lignes physiques).

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, chisq_sf, decode_column, ln_choose, probnorm};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{format_best, Value, VarType};
use polars::prelude::*;
use std::cmp::Ordering;


mod parse;
mod tally;
mod oneway;
mod twoway;
mod stats;
mod output;

pub use parse::parse;

use tally::*;
use oneway::*;
use twoway::*;
use stats::*;
use output::*;

pub struct FreqAst {
    pub data: Option<DatasetRef>,
    pub tables: Vec<TableRequest>,
    /// WEIGHT statement variable (cell frequencies become the sum of weights).
    pub weight: Option<String>,
    /// BY statement variables (one independent analysis per BY group).
    pub by: Vec<(String, bool)>,
}

pub struct TableRequest {
    /// 1 nom = une voie ; 2 noms = crosstab v1*v2.
    pub vars: Vec<String>,
    pub missing: bool,
    pub out: Option<DatasetRef>,
    /// Display-suppression options (parsed AND honored).
    pub nofreq: bool,
    pub nopercent: bool,
    pub norow: bool,
    pub nocol: bool,
    pub nocum: bool,
    /// CHISQ statistics request (one-way goodness-of-fit OR two-way).
    pub chisq: bool,
    /// Fisher exact test (two-way 2x2).
    pub fisher: bool,
    /// AGREE (Cohen's simple kappa, square two-way table).
    pub agree: bool,
    /// MEASURES / RELRISK (odds ratio + relative risks, 2x2).
    pub measures: bool,
    /// TREND (Cochran-Armitage trend test, 2xc or rx2).
    pub trend: bool,
    /// LIST layout (one row per non-empty cell instead of the grid).
    pub list: bool,
}

/// Execute PROC FREQ. Called by `procs::execute_proc`.
pub fn execute(ast: &FreqAst, session: &mut Session) -> Result<()> {
    let (ds, in_libref, in_table) = common::open_input(&ast.data, session)?;

    let n_obs = ds.n_obs();

    // --- WEIGHT statement: decode the weight column once (or None). ---
    let weight_values: Option<Vec<Value>> = match &ast.weight {
        Some(wname) => {
            let widx = find_var(&ds, wname)?;
            Some(decode_column(&ds, widx)?)
        }
        None => None,
    };

    // --- BY processing: resolve, verify sortedness, partition into groups. ---
    // No BY → a single group spanning all rows (output byte-identical).
    let by_cols = common::resolve_by_cols(&ds, &ast.by)?;
    let by_values: Vec<Vec<Value>> = by_cols
        .iter()
        .map(|c| decode_column(&ds, c.col_idx))
        .collect::<Result<_>>()?;
    let by_names: Vec<String> = by_cols.iter().map(|c| c.name.clone()).collect();
    let by_groups_list: Vec<(Vec<Value>, Vec<usize>)> = if by_cols.is_empty() {
        vec![(Vec::new(), (0..n_obs).collect())]
    } else {
        let descending: Vec<bool> = by_cols.iter().map(|c| c.descending).collect();
        let in_display = format!("{in_libref}.{in_table}");
        common::by_groups(&by_values, &descending, n_obs, &by_names, &in_display)?
    };

    session.listing.page_header();
    // Centered procedure title line.
    let title = "The FREQ Procedure";
    let ls = session.listing.ls();
    let pad = ls.saturating_sub(title.len()) / 2;
    session
        .listing
        .write_line(&format!("{}{}", " ".repeat(pad), title));
    session.listing.blank();

    for (by_key, grp_rows) in &by_groups_list {
        if !by_names.is_empty() {
            emit_by_heading(session, &by_names, by_key);
        }
        for req in &ast.tables {
            match req.vars.len() {
                1 => one_way(session, &ds, req, grp_rows, weight_values.as_deref())?,
                2 => two_way(session, &ds, req, grp_rows, weight_values.as_deref())?,
                _ => n_way(session, &ds, req, grp_rows, weight_values.as_deref())?,
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
