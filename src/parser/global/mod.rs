//! Statements globaux : LIBNAME, OPTIONS, TITLEn.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! Appelé par `parser::next_block()` ; le mot-clé de tête est encore dans
//! le stream (peek) ou déjà identifié par l'appelant — convention :
//! l'appelant N'A PAS consommé le mot-clé, `parse_global` le consomme.
//!
//! ## LIBNAME
//! - `libname ref 'chemin' ;`  → `GlobalStmt::Libname` (chemin = littéral
//!   chaîne ; relatif → résolu contre `Session::base_dir` à l'exécution).
//! - `libname ref clear ;`     → `GlobalStmt::LibnameClear`.
//!
//! ## TITLE
//! - `title 'texte' ;` / `titleN 'texte' ;` (N=1..9, suffixe dans
//!   l'ident) ; sans texte → efface. M1 : seul TITLE1 est rendu par le
//!   listing.
//!
//! ## OPTIONS
//! - `options name[=valeur]... ;` → liste brute. L'exécution (executor)
//!   applique `ls=` (40..=256) et ignore le reste avec WARNING
//!   "Option XXX is not yet supported".

use super::{StatementStream, footnote_level, title_level};
use crate::ast::{DatasetRef, GlobalStmt, OdsAction};
use crate::error::{Result, SasError};
use crate::token::{Span, StrSuffix, TokenKind};

mod lib;
mod ods;
mod titles;

pub use ods::parse_ods_statement;

use lib::*;
use titles::*;

/// Parse a global statement (LIBNAME, OPTIONS, or TITLEn).
///
/// The leading keyword token must still be in the stream (not yet consumed);
/// this function consumes it and the closing `;`.
pub fn parse_global(ts: &mut StatementStream) -> Result<GlobalStmt> {
    let head = ts.peek().clone();
    let kw = match head.ident() {
        Some(s) => s.to_ascii_lowercase(),
        None => {
            return Err(SasError::parse(
                "expected LIBNAME, FILENAME, OPTIONS, or TITLE keyword",
                head.span,
            ));
        }
    };

    if kw == "libname" {
        ts.next(); // consume `libname`
        parse_libname(ts)
    } else if kw == "filename" {
        ts.next(); // consume `filename`
        parse_filename(ts)
    } else if kw == "options" {
        ts.next(); // consume `options`
        parse_options(ts)
    } else if kw == "ods" {
        ts.next(); // consume `ods`
        parse_ods_statement(ts)
    } else if let Some(n) = title_level(&kw) {
        ts.next(); // consume `titleN`
        parse_title(ts, n)
    } else if let Some(n) = footnote_level(&kw) {
        ts.next(); // consume `footnoteN`
        parse_footnote(ts, n)
    } else {
        Err(SasError::parse(
            format!(
                "Expected LIBNAME, FILENAME, OPTIONS, ODS, TITLEn, or FOOTNOTEn; got '{}'",
                kw.to_uppercase()
            ),
            head.span,
        ))
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
