//! PROC COMPARE (jalon M21.1, options de production J07-P3).
//!
//! # Syntaxe
//! ```sas
//! proc compare base=lib.x compare=lib.y [out=lib.z]
//!               [novalues] [brief|briefsummary] [noprint] [listall]
//!               [criterion=c] [method=ABSOLUTE|RELATIVE|EXACT|PERCENT]
//!               [maxprint=n] [outbase] [outcomp] [outdif] [outnoequal]
//!               [outpercent];
//!   [by v1 [descending v2]...;] [id k1 k2...;] [var x1 x2...;] [with y1 y2...;]
//! run;
//! ```
//!
//! - BASE= et COMPARE= obligatoires.
//! - Compare structure (variables communes, types, attributs) et valeurs.
//! - BY : comparaison à l'intérieur de chaque groupe BY ; ID :
//!   appariement des observations par clé triée (les observations propres à
//!   chaque table sont signalées) ; sinon appariement par position.
//! - VAR/WITH : paires positionnelles de variables (noms OUT= = VAR).
//! - CRITERION=/METHOD= : jugement des valeurs numériques — ABSOLUTE :
//!   inégales si |y−x| > c ; RELATIVE : |(y−x)/x| > c ; PERCENT :
//!   |100(y−x)/x| > c ; EXACT : égalité stricte. Caractères et missings :
//!   toujours égalité stricte.
//! - NOVALUES : omet la section « Values Comparison ». BRIEF/BRIEFSUMMARY :
//!   rapport condensé. LISTALL : la section valeurs liste toutes les
//!   variables comparées. MAXPRINT=n | (n,p) : plafonne la section « Value
//!   Comparison Results » à n différences par observation et p observations
//!   avec différences (défauts 50/50 ; MAXPRINT=n seul laisse p à 50) ; une
//!   NOTE signale la troncature. NOPRINT : aucun listing.
//! - OUT= : dataset des différences — colonnes BY, ID, VAR, `_TYPE_`
//!   (BASE | COMP | DIF | PERCENT) et `_OBS_` ; OUTBASE/OUTCOMP recopient
//!   chaque observation de BASE/COMPARE ; OUTDIF écrit les différences
//!   (défaut si aucun type demandé) ; OUTPERCENT les écarts en pourcentage ;
//!   OUTNOEQUAL supprime les lignes DIF/PERCENT des paires jugées égales.
//!
//! # &SYSINFO
//!
//! À l'issue de chaque exécution, la variable macro automatique `&SYSINFO`
//! reçoit la somme des bits documentés (doc SAS 9.4, « Macro Return
//! Codes ») : FORMAT 8, LENGTH 16, LABEL 32, BASEOBS 64, COMPOBS 128,
//! BASEBY 256, COMPBY 512, BASEVAR 1024, COMPVAR 2048, VALUE 4096, TYPE
//! 8192, BYVAR 16384, ERROR 32768 — 0 si tout est jugé égal.

use std::cmp::Ordering;
use std::collections::HashMap;

use polars::prelude::*;

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::num_to_value;
use crate::parser::StatementStream;
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{Value, VarType};

mod analyze;
mod output;
mod parse;
mod report;

pub use parse::CmpMethod;
pub use parse::CompareAst;
pub use parse::parse;

use analyze::*;
use output::*;
use report::*;

/// Execute PROC COMPARE.
pub fn execute(ast: &CompareAst, session: &mut Session) -> Result<()> {
    // ── Load BASE dataset ────────────────────────────────────────────────────
    let base_display = ast.base.display();
    let base_ds = read_input(session, &ast.base)?;

    // ── Load COMPARE dataset ─────────────────────────────────────────────────
    let comp_display = ast.compare.display();
    let comp_ds = read_input(session, &ast.compare)?;

    let base_nobs = base_ds.n_obs();
    let base_nvars = base_ds.n_vars();
    let comp_nobs = comp_ds.n_obs();
    let comp_nvars = comp_ds.n_vars();

    // ── Variable analysis + comparison (matching, judgment, &SYSINFO) ───────
    let (only_base, only_comp, common_vars) = analyze_variables(&base_ds, &comp_ds);
    let outcome = compare_datasets(ast, &base_ds, &comp_ds, &common_vars)?;
    let n_matching = outcome.pairs.len();

    // ── Render listing ───────────────────────────────────────────────────────
    if !ast.noprint {
        session.listing.page_header();
        let n_unequal = outcome.matches.iter().filter(|m| m.unequal).count();
        let ctx = ReportCtx {
            base_display: &base_display,
            comp_display: &comp_display,
            base_nobs,
            base_nvars,
            comp_nobs,
            comp_nvars,
            only_base: &only_base,
            only_comp: &only_comp,
            common_vars: &common_vars,
            n_matching,
            n_matched: outcome.matches.len(),
            n_unequal,
            base_only_obs: &outcome.base_only_obs,
            comp_only_obs: &outcome.comp_only_obs,
            var_diffs: &outcome.var_diffs,
            matches: &outcome.matches,
            pairs: &outcome.pairs,
            base_ds: &base_ds,
            comp_ds: &comp_ds,
        };
        if !ast.briefsummary && !ast.brief {
            print_full_report(session, ast, &ctx);
        } else {
            print_brief_report(session, &ctx);
        }
    }

    // ── NOTE log ────────────────────────────────────────────────────────────
    let n_with_diffs = outcome.matches.iter().filter(|m| m.unequal).count();
    if n_with_diffs == 0 && outcome.base_only_obs.is_empty() && outcome.comp_only_obs.is_empty() {
        session
            .log
            .note("No unequal values were found. All values compared are exactly equal.");
    } else {
        session.log.note(&format!(
            "There were {} observations with at least one unequal value.",
            n_with_diffs
        ));
        if !outcome.base_only_obs.is_empty() {
            session.log.note(&format!(
                "Number of observations in {} but not in {}: {}.",
                base_display,
                comp_display,
                outcome.base_only_obs.len()
            ));
        }
        if !outcome.comp_only_obs.is_empty() {
            session.log.note(&format!(
                "Number of observations in {} but not in {}: {}.",
                comp_display,
                base_display,
                outcome.comp_only_obs.len()
            ));
        }
    }

    // ── &SYSINFO (doc SAS 9.4 : remis à chaque exécution) ───────────────────
    session
        .macro_engine
        .set_automatic("SYSINFO", outcome.sysinfo.to_string());

    // ── Write OUT= dataset ───────────────────────────────────────────────────
    if let Some(ref out_ref) = ast.out {
        write_out_dataset(session, ast, out_ref, &outcome, &base_ds, &comp_ds)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
