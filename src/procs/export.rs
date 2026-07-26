//! PROC EXPORT (jalon M14.3).
//!
//! Écrit une table SAS (parquet) vers un fichier texte délimité (CSV/TAB/DLM)
//! via le writer CSV de Polars.
//!
//! # Syntaxe prise en charge
//!
//! ```sas
//! proc export data=lib.table outfile='chemin' dbms=CSV [replace];
//!     delimiter='x';  /* ou dlm='x' */
//! run;
//! ```
//!
//! ## DBMS pris en charge
//! - `CSV`  → séparateur virgule (`,`)
//! - `TAB`  → séparateur tabulation (`\t`)
//! - `DLM`  → séparateur fourni par `DELIMITER=`/`DLM=` (défaut espace ` `)
//!
//! ## DBMS différés (erreur propre)
//! - `XLSX`, `EXCEL` → `SasError::runtime(...)` avec message explicite.
//!
//! ## REPLACE
//! Option flag : si le fichier existe déjà, il est écrasé (comportement
//! documenté ; SAS 9.4 renverrait une erreur sans REPLACE, mais notre
//! implémentation écrase toujours — documenté).
//!
//! ## NOTE de fin
//! `"N records were written to the file 'chemin'."` (SAS 9.4 wording)
//!
//! ## Invariants
//! - L'en-tête CSV (noms de colonnes) est TOUJOURS écrit (comportement SAS
//!   par défaut pour PROC EXPORT DBMS=CSV/TAB/DLM).
//! - Le dataset source est lu via `provider.read()` → `SasDataset` →
//!   `SasDataset::df` est un `DataFrame` Polars prêt à passer au writer CSV.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::expect_eq;
use crate::procs::common::parse_string_or_ident;
use crate::session::Session;
use crate::token::TokenKind;
use polars::prelude::*;
use std::fs::File;

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

/// DBMS reconnu par PROC EXPORT.
#[derive(Debug, Clone, PartialEq)]
pub enum ExportDbms {
    /// DBMS=CSV  → séparateur `,`
    Csv,
    /// DBMS=TAB  → séparateur `\t`
    Tab,
    /// DBMS=DLM  → séparateur fourni par `delimiter=` (défaut ` `)
    Dlm,
}

/// AST de PROC EXPORT.
pub struct ExportAst {
    /// Dataset source (`DATA=`).
    pub data: Option<DatasetRef>,
    /// Chemin du fichier de sortie (`OUTFILE=`).
    pub outfile: String,
    /// Moteur d'écriture.
    pub dbms: ExportDbms,
    /// `REPLACE` présent ? (documenté : on écrase toujours).
    pub replace: bool,
    /// Séparateur explicite (`DELIMITER=`/`DLM=` dans le corps).
    pub delimiter: Option<u8>,
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse `proc export ...` jusqu'à `run;`/`quit;`. Appelé APRÈS que
/// `proc export` a été consommé par le dispatcher.
pub fn parse(ts: &mut StatementStream) -> Result<ExportAst> {
    let mut data: Option<DatasetRef> = None;
    let mut outfile: Option<String> = None;
    let mut dbms: Option<ExportDbms> = None;
    let mut replace = false;

    // --- Options sur le statement PROC EXPORT (jusqu'au `;`) ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            common::consume_option_eq(ts, "DATA")?;
            data = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("outfile") {
            common::consume_option_eq(ts, "OUTFILE")?;
            outfile = Some(parse_string_or_ident(ts, "OUTFILE")?);
        } else if ts.peek().is_kw("dbms") {
            common::consume_option_eq(ts, "DBMS")?;
            let tok = ts.peek().clone();
            let name = tok
                .ident()
                .ok_or_else(|| SasError::parse("expected a DBMS name after DBMS=", tok.span))?
                .to_ascii_uppercase();
            ts.next();
            dbms = Some(parse_dbms(&name, tok.span)?);
        } else if ts.peek().is_kw("replace") {
            ts.next();
            replace = true;
        } else {
            // option inconnue → ignorer (récupération)
            ts.next();
        }
    }

    // --- Sous-statements jusqu'à run;/quit; ---
    let mut delimiter: Option<u8> = None;

    loop {
        while ts.peek().kind == TokenKind::Semi {
            ts.next();
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("run") || ts.peek().is_kw("quit") {
            ts.next();
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            break;
        }
        let kw_tok = ts.peek().clone();
        let kw = match kw_tok.ident() {
            Some(s) => s.to_ascii_lowercase(),
            None => {
                ts.skip_to_semi();
                continue;
            }
        };
        ts.next();

        match kw.as_str() {
            "delimiter" | "dlm" => {
                expect_eq(ts, "DELIMITER")?;
                let s = parse_string_or_ident(ts, "DELIMITER")?;
                delimiter = parse_delimiter_char(&s, kw_tok.span)?;
                ts.skip_to_semi();
            }
            _ => {
                ts.skip_to_semi();
            }
        }
    }

    let outfile = outfile.ok_or_else(|| SasError::runtime("PROC EXPORT: OUTFILE= is required."))?;
    let dbms = dbms.unwrap_or(ExportDbms::Csv);

    Ok(ExportAst {
        data,
        outfile,
        dbms,
        replace,
        delimiter,
    })
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

/// Execute PROC EXPORT. Appelé par `procs::execute_proc`.
pub fn execute(ast: &ExportAst, session: &mut Session) -> Result<()> {
    // --- Résoudre le dataset source ---
    let (ds, _, _) = common::open_input(&ast.data, session)?;

    let n_obs = ds.n_obs();

    // --- Résoudre le séparateur ---
    let sep = resolve_separator(ast);

    // --- Écrire le fichier CSV (chemin relatif résolu sous base_dir) ---
    let out_path = session.resolve_path(&ast.outfile);
    let mut file = File::create(&out_path).map_err(|e| {
        SasError::runtime(format!("PROC EXPORT: cannot create '{}': {e}", ast.outfile))
    })?;

    let mut df_clone = ds.df.clone();
    CsvWriter::new(&mut file)
        .include_header(true)
        .with_separator(sep)
        .finish(&mut df_clone)
        .map_err(|e| {
            SasError::runtime(format!("PROC EXPORT: error writing '{}': {e}", ast.outfile))
        })?;

    // --- NOTE de fin ---
    session.log.note(&format!(
        "{} records were written to the file '{}'.",
        n_obs, ast.outfile
    ));

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers internes
// ---------------------------------------------------------------------------

/// Résout le séparateur en octet selon DBMS + DELIMITER éventuel.
fn resolve_separator(ast: &ExportAst) -> u8 {
    match &ast.dbms {
        ExportDbms::Csv => b',',
        ExportDbms::Tab => b'\t',
        ExportDbms::Dlm => ast.delimiter.unwrap_or(b' '),
    }
}

/// Parse un DBMS par son nom en majuscules ; renvoie une erreur propre pour
/// les DBMS différés (XLSX/EXCEL).
fn parse_dbms(name: &str, span: crate::token::Span) -> Result<ExportDbms> {
    match name {
        "CSV" => Ok(ExportDbms::Csv),
        "TAB" => Ok(ExportDbms::Tab),
        "DLM" | "DLMSTR" => Ok(ExportDbms::Dlm),
        "XLSX" | "EXCEL" | "XLS" => Err(SasError::runtime(format!(
            "PROC EXPORT with DBMS={name} is not yet implemented in this build \
             (the calamine/rust_xlsxwriter crates are not available)."
        ))),
        other => Err(SasError::parse(
            format!("Unknown DBMS '{other}' for PROC EXPORT."),
            span,
        )),
    }
}

/// Parse un caractère délimiteur depuis une chaîne.
fn parse_delimiter_char(s: &str, span: crate::token::Span) -> Result<Option<u8>> {
    let s = match s.to_ascii_uppercase().as_str() {
        "TAB" | "09X" => return Ok(Some(b'\t')),
        "SPACE" | "20X" => return Ok(Some(b' ')),
        "COMMA" | "2CX" => return Ok(Some(b',')),
        "PIPE" | "7CX" => return Ok(Some(b'|')),
        "SEMICOLON" | "3BX" => return Ok(Some(b';')),
        _ => s,
    };
    if s.is_empty() {
        return Ok(None);
    }
    let bytes = s.as_bytes();
    if bytes.len() == 1 {
        return Ok(Some(bytes[0]));
    }
    Err(SasError::parse(
        format!("DELIMITER value '{s}' must be a single ASCII character or a recognized mnemonic."),
        span,
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
