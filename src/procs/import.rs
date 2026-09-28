//! PROC IMPORT (jalon M14.3).
//!
//! Lit un fichier texte délimité (CSV/TAB/DLM) via le lecteur CSV de Polars
//! et l'importe dans une table SAS (parquet) via `SasDataset::from_dataframe`.
//!
//! # Syntaxe prise en charge
//!
//! ```sas
//! proc import datafile='chemin' out=lib.table dbms=CSV [replace];
//!     getnames=yes|no;
//!     delimiter='x';   /* ou dlm='x' */
//!     /* GUESSINGROWS is rejected: inference length is not configurable. */
//! run;
//! ```
//!
//! ## DBMS pris en charge
//! - `CSV`  → séparateur virgule (`,`)
//! - `TAB`  → séparateur tabulation (`\t`)
//! - `DLM`  → séparateur fourni par `DELIMITER=`/`DLM=` (défaut espace ` `)
//! - `XLSX` / `EXCEL` → classeur Excel lu via la crate `calamine` (J08-P1) :
//!   `SHEET=` (nom ou numéro 1-based), `RANGE=` (plage A1 style `A1:C10`,
//!   éventuellement préfixée `Sheet1$A1:C10`), `GETNAMES=`, `GUESSINGROWS=`
//!   (fenêtre d'inférence de types), dates Excel sérial → dates SAS 1960
//!   (format `DATE9.`/`DATETIME20.`).
//!
//! ## DBMS différés (erreur propre)
//! - `XLS` (binaire legacy) → `SasError::runtime(...)` avec message explicite.
//!
//! ## GETNAMES
//! - `YES` (défaut) : la première ligne donne les noms de colonnes.
//! - `NO` : noms automatiques `VAR1`, `VAR2`, … (style SAS ; Polars produit
//!   `column_1`… qui est renommé ici).
//!
//! ## REPLACE
//! Option flag : documenté mais non appliqué — on écrase toujours (comportement
//! documenté ; SAS 9.4 renvoie une erreur sans REPLACE si la table existe).
//!
//! ## Invariants
//! - `SasDataset::from_dataframe` est appelé systématiquement → coercition
//!   de types, i64 > 2^53 WARNING, dates → f64+format.
//! - NOTE de fin : "The data set LIB.TABLE has N observations and M variables."
//!   (pluriel invariable — fidèle à SAS, cf. PLAN.md §Checklist piège 7).

use crate::ast::DatasetRef;
use crate::dataset::SasDataset;
use crate::error::{Result, SasError};
use crate::parser::StatementStream;
use crate::procs::common;
use crate::procs::common::expect_eq;
use crate::procs::common::parse_string_or_ident;
use crate::session::Session;
use crate::token::TokenKind;
use polars::prelude::*;

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

/// DBMS (système de fichier source) reconnu par PROC IMPORT.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportDbms {
    /// DBMS=CSV  → séparateur `,`
    Csv,
    /// DBMS=TAB  → séparateur `\t`
    Tab,
    /// DBMS=DLM  → séparateur fourni par `delimiter=` (défaut ` `)
    Dlm,
    /// DBMS=XLSX / EXCEL → classeur Excel via calamine (J08-P1)
    Xlsx,
}

/// AST de PROC IMPORT.
pub struct ImportAst {
    /// Chemin du fichier source (`DATAFILE=`).
    pub datafile: String,
    /// Dataset de sortie (`OUT=`).
    pub out: DatasetRef,
    /// Moteur de lecture.
    pub dbms: ImportDbms,
    /// `REPLACE` présent ? (documenté : on écrase toujours).
    pub replace: bool,
    /// `GETNAMES=YES` (défaut) : la 1re ligne donne les noms.
    pub getnames: bool,
    /// Séparateur explicite (DELIMITER=/DLM= dans le corps).
    pub delimiter: Option<u8>,
    /// `GUESSINGROWS=` (XLSX : fenêtre d'inférence des types ; CSV : refusé).
    pub guessingrows: Option<usize>,
    /// `SHEET=` (XLSX) : nom de feuille ou numéro 1-based.
    pub sheet: Option<String>,
    /// `RANGE=` (XLSX) : plage A1 (`A1:C10` ou `Sheet1$A1:C10`).
    pub range: Option<String>,
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse `proc import ...` jusqu'à `run;`/`quit;`. Appelé APRÈS que
/// `proc import` a été consommé par le dispatcher.
pub fn parse(ts: &mut StatementStream) -> Result<ImportAst> {
    let mut datafile: Option<String> = None;
    let mut out: Option<DatasetRef> = None;
    let mut dbms: Option<ImportDbms> = None;
    let mut replace = false;

    // --- Options sur le statement PROC IMPORT (jusqu'au `;`) ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("datafile") || ts.peek().is_kw("filename") {
            common::consume_option_eq(ts, "DATAFILE")?;
            datafile = Some(parse_string_or_ident(ts, "DATAFILE")?);
        } else if ts.peek().is_kw("out") {
            common::consume_option_eq(ts, "OUT")?;
            out = Some(ts.parse_dataset_ref()?);
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
    let mut getnames = true;
    let mut delimiter: Option<u8> = None;
    let mut guessingrows: Option<usize> = None;
    let mut sheet: Option<String> = None;
    let mut range: Option<String> = None;

    common::parse_proc_body(ts, "IMPORT", |ts, kw| {
        let kw_tok = ts.peek().clone();
        match kw {
            "getnames" => {
                ts.next();
                expect_eq(ts, "GETNAMES")?;
                let val_tok = ts.peek().clone();
                let val = val_tok
                    .ident()
                    .ok_or_else(|| {
                        SasError::parse("expected YES or NO after GETNAMES=", val_tok.span)
                    })?
                    .to_ascii_uppercase();
                ts.next();
                getnames = val != "NO";
                ts.expect_semi()?;
            }
            "delimiter" | "dlm" => {
                ts.next();
                expect_eq(ts, "DELIMITER")?;
                let s = parse_string_or_ident(ts, "DELIMITER")?;
                delimiter = parse_delimiter_char(&s, kw_tok.span)?;
                ts.expect_semi()?;
            }
            "guessingrows" => {
                ts.next();
                expect_eq(ts, "GUESSINGROWS")?;
                let val_tok = ts.peek().clone();
                // Honoré pour DBMS=XLSX (fenêtre d'inférence de types) ;
                // refusé explicitement pour les DBMS texte (Polars infère
                // seul — contrat §5 : jamais de repli silencieux).
                if dbms.as_ref() != Some(&ImportDbms::Xlsx) {
                    return Err(common::unsupported_statement("IMPORT", "GUESSINGROWS"));
                }
                let val = common::read_value(ts).ok_or_else(|| {
                    SasError::parse(
                        "expected a positive integer or MAX after GUESSINGROWS=",
                        val_tok.span,
                    )
                })?;
                guessingrows = Some(parse_guessingrows(&val, val_tok.span)?);
                ts.expect_semi()?;
            }
            "sheet" => {
                ts.next();
                expect_eq(ts, "SHEET")?;
                // Nom de feuille ou numéro 1-based (TokenKind::Num accepté).
                let s = common::read_value(ts).ok_or_else(|| {
                    SasError::parse("expected a sheet name or number after SHEET=", kw_tok.span)
                })?;
                if dbms.as_ref() != Some(&ImportDbms::Xlsx) {
                    return Err(SasError::runtime(
                        "PROC IMPORT: SHEET= is only valid with DBMS=XLSX.",
                    ));
                }
                sheet = Some(s);
                ts.expect_semi()?;
            }
            "range" => {
                ts.next();
                expect_eq(ts, "RANGE")?;
                let s = parse_string_or_ident(ts, "RANGE")?;
                if dbms.as_ref() != Some(&ImportDbms::Xlsx) {
                    return Err(SasError::runtime(
                        "PROC IMPORT: RANGE= is only valid with DBMS=XLSX.",
                    ));
                }
                range = Some(s);
                ts.expect_semi()?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    })?;

    let datafile =
        datafile.ok_or_else(|| SasError::runtime("PROC IMPORT: DATAFILE= is required."))?;
    let out = out.ok_or_else(|| SasError::runtime("PROC IMPORT: OUT= is required."))?;
    let dbms = dbms.unwrap_or(ImportDbms::Csv);

    Ok(ImportAst {
        datafile,
        out,
        dbms,
        replace,
        getnames,
        delimiter,
        guessingrows,
        sheet,
        range,
    })
}

/// Parse la valeur de `GUESSINGROWS=` : entier positif ou `MAX` (toutes les
/// lignes, comportement par défaut de notre lecteur XLSX).
fn parse_guessingrows(s: &str, span: crate::token::Span) -> Result<usize> {
    let upper = s.to_ascii_uppercase();
    if upper == "MAX" {
        return Ok(usize::MAX);
    }
    upper
        .parse::<usize>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| {
            SasError::parse(
                format!(
                    "GUESSINGROWS value '{s}' must be a positive integer or MAX (SAS 9.4: 1 to 2147483647)."
                ),
                span,
            )
        })
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

/// Execute PROC IMPORT. Appelé par `procs::execute_proc`.
pub fn execute(ast: &ImportAst, session: &mut Session) -> Result<()> {
    // --- Lire le DataFrame ---
    let path = session.resolve_path(&ast.datafile);
    let (df, extra_formats) = match &ast.dbms {
        ImportDbms::Xlsx => xlsx::read_xlsx(ast, &path)?,
        ImportDbms::Csv | ImportDbms::Tab | ImportDbms::Dlm => {
            let sep = resolve_separator(ast)?;
            let df = CsvReadOptions::default()
                .with_has_header(ast.getnames)
                .with_parse_options(CsvParseOptions::default().with_separator(sep))
                .try_into_reader_with_file_path(Some(path))
                .map_err(|e| {
                    SasError::runtime(format!(
                        "PROC IMPORT: cannot open '{}': {}",
                        ast.datafile, e
                    ))
                })?
                .finish()
                .map_err(|e| {
                    SasError::runtime(format!(
                        "PROC IMPORT: error reading '{}': {}",
                        ast.datafile, e
                    ))
                })?;
            (df, Vec::new())
        }
    };

    // --- Renommer les colonnes si GETNAMES=NO (Polars → VAR1, VAR2, …) ---
    // XLSX produit déjà les noms VARn (ou les en-têtes sanitises) lui-même.
    let df = if !ast.getnames && !matches!(ast.dbms, ImportDbms::Xlsx) {
        rename_to_var_n(df)?
    } else {
        df
    };

    // --- Coercition vers le modèle de types SAS ---
    let (mut ds, notes) = SasDataset::from_dataframe(df)?;
    // XLSX : les colonnes dates converties depuis le sérial Excel portent
    // leur format SAS (DATE9./DATETIME20.) — from_dataframe ne peut pas le
    // déduire d'un simple Float64.
    for (idx, fmt) in extra_formats.iter().enumerate() {
        if let Some(f) = fmt {
            ds.vars[idx].format = Some(f.clone());
        }
    }
    for note in &notes {
        session.log.forward(note);
    }

    let n_obs = ds.n_obs();
    let n_vars = ds.n_vars();

    // --- Écrire dans la bibliothèque cible ---
    let out_libref = ast.out.libref_or_work();
    let out_table = ast.out.name.to_uppercase();
    let display = ast.out.display();

    let provider = session.libs.get(&out_libref)?;
    provider.write(&out_table, &ds)?;

    // --- Mettre à jour _LAST_ et émettre la NOTE ---
    session.last_dataset = Some(display.clone());
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_obs, n_vars
    ));

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers internes
// ---------------------------------------------------------------------------

/// Résout le séparateur en octet selon DBMS + DELIMITER éventuel.
/// DBMS=XLSX n'utilise pas de séparateur (appelé uniquement pour CSV/TAB/DLM).
fn resolve_separator(ast: &ImportAst) -> Result<u8> {
    match &ast.dbms {
        ImportDbms::Csv => Ok(b','),
        ImportDbms::Tab => Ok(b'\t'),
        ImportDbms::Dlm => {
            // DELIMITER= fourni → l'utiliser ; sinon espace (défaut SAS DLM)
            Ok(ast.delimiter.unwrap_or(b' '))
        }
        ImportDbms::Xlsx => Ok(b','),
    }
}

/// Renomme les colonnes Polars `column_1`…`column_N` en `VAR1`…`VARN`
/// lorsque `GETNAMES=NO`.
fn rename_to_var_n(mut df: DataFrame) -> Result<DataFrame> {
    let n = df.width();
    let old_names: Vec<String> = df
        .get_column_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    for (i, old) in old_names.iter().enumerate() {
        let new_name = format!("VAR{}", i + 1);
        df.rename(old, new_name.as_str().into())
            .map_err(|e| SasError::runtime(format!("PROC IMPORT: rename column: {e}")))?;
    }
    let _ = n; // silence unused warning
    Ok(df)
}

/// Parse un DBMS par son nom en majuscules ; renvoie une erreur propre pour
/// les DBMS différés (XLS binaire legacy).
fn parse_dbms(name: &str, span: crate::token::Span) -> Result<ImportDbms> {
    match name {
        "CSV" => Ok(ImportDbms::Csv),
        "TAB" => Ok(ImportDbms::Tab),
        "DLM" | "DLMSTR" => Ok(ImportDbms::Dlm),
        "XLSX" | "EXCEL" => Ok(ImportDbms::Xlsx),
        "XLS" => Err(SasError::runtime(
            "PROC IMPORT with DBMS=XLS is not supported in this build \
             (legacy binary .xls format; use DBMS=XLSX for Excel workbooks).",
        )),
        other => Err(SasError::parse(
            format!("Unknown DBMS '{other}' for PROC IMPORT."),
            span,
        )),
    }
}

/// Parse un caractère délimiteur depuis une chaîne (potentiellement de
/// longueur 1 pour une casse simple, ou représentation mnémonique courante).
/// Renvoie une erreur si la chaîne est vide ou contient plus d'un octet ASCII.
fn parse_delimiter_char(s: &str, span: crate::token::Span) -> Result<Option<u8>> {
    // Mnémoniques courants
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

mod xlsx;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod dbms_xlsx_tests;
