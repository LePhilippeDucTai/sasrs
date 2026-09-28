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
//! - `XLSX` → classeur Excel via le writer XLSX pur Rust partagé
//!   (`src/output/xlsx.rs`, J08-P1) ; `SHEET=` nomme la feuille (défaut : le
//!   nom du dataset source). Les variables numériques deviennent des cellules
//!   numériques ; les formats date/datetime/time sont écrits comme dates
//!   Excel (style `yyyy-mm-dd`) pour un aller-retour IMPORT sans perte ;
//!   les missings deviennent des cellules vides.
//!
//! ## DBMS différés (erreur propre)
//! - `EXCEL` (legacy) → `SasError::runtime(...)` avec message explicite.
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
    /// DBMS=XLSX → classeur Excel (writer partagé `output::xlsx`)
    Xlsx,
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
    /// `SHEET=` (XLSX uniquement) : nom de la feuille de sortie.
    pub sheet: Option<String>,
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
    let mut sheet: Option<String> = None;

    common::parse_proc_body(ts, "EXPORT", |ts, kw| {
        let kw_tok = ts.peek().clone();
        match kw {
            "delimiter" | "dlm" => {
                ts.next();
                expect_eq(ts, "DELIMITER")?;
                let s = parse_string_or_ident(ts, "DELIMITER")?;
                delimiter = parse_delimiter_char(&s, kw_tok.span)?;
                ts.expect_semi()?;
            }
            "sheet" => {
                ts.next();
                expect_eq(ts, "SHEET")?;
                let s = parse_string_or_ident(ts, "SHEET")?;
                if dbms.as_ref() != Some(&ExportDbms::Xlsx) {
                    return Err(SasError::runtime(
                        "PROC EXPORT: SHEET= is only valid with DBMS=XLSX.",
                    ));
                }
                if s.is_empty() {
                    return Err(SasError::runtime(
                        "PROC EXPORT: SHEET= must not be an empty sheet name.",
                    ));
                }
                sheet = Some(s);
                ts.expect_semi()?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    })?;

    let outfile = outfile.ok_or_else(|| SasError::runtime("PROC EXPORT: OUTFILE= is required."))?;
    let dbms = dbms.unwrap_or(ExportDbms::Csv);

    Ok(ExportAst {
        data,
        outfile,
        dbms,
        replace,
        delimiter,
        sheet,
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

    // --- Écrire le fichier (chemin relatif résolu sous base_dir) ---
    let out_path = session.resolve_path(&ast.outfile);

    match &ast.dbms {
        ExportDbms::Xlsx => execute_xlsx(ast, &ds, &out_path)?,
        ExportDbms::Csv | ExportDbms::Tab | ExportDbms::Dlm => {
            let sep = resolve_separator(ast);
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
        }
    }

    // --- NOTE de fin ---
    session.log.note(&format!(
        "{} records were written to the file '{}'.",
        n_obs, ast.outfile
    ));

    Ok(())
}

// ---------------------------------------------------------------------------
// DBMS=XLSX (J08-P1) — writer XLSX partagé (src/output/xlsx.rs)
// ---------------------------------------------------------------------------

/// Nom de famille d'un format SAS : préfixe alphabétique du nom, la largeur
/// et les décimales retirées (`DATE9.` → `DATE`, `DATETIME20.` → `DATETIME`).
fn format_family(fmt: &str) -> String {
    fmt.to_ascii_uppercase()
        .chars()
        .take_while(|c| !c.is_ascii_digit())
        .collect()
}

/// Familles de formats dont la valeur numérique SAS est une DATE (jours
/// depuis 1960) — écrites comme dates Excel stylées.
fn is_date_format(fmt: &str) -> bool {
    let name = format_family(fmt);
    matches!(
        name.as_str(),
        "DATE"
            | "DATEAMPM"
            | "YYMMDD"
            | "YYMMDDD"
            | "YYMMDDN"
            | "YYMMDDS"
            | "YYMMDDP"
            | "YYMMDDC"
            | "MMDDYY"
            | "MMDDYYD"
            | "MMDDYYN"
            | "MMDDYYS"
            | "MMDDYYP"
            | "MMDDYYC"
            | "DDMMYY"
            | "DDMMYYD"
            | "DDMMYYN"
            | "DDMMYYS"
            | "DDMMYYP"
            | "DDMMYYC"
            | "YYQ"
            | "YYMON"
            | "MONYY"
            | "WEEKDATE"
            | "WORDDATE"
            | "NENGO"
            | "EURDFDE"
            | "MINGUO"
            | "JULDAY"
            | "JULIAN"
            | "DOWNAME"
            | "WEEKDATX"
            | "WORDDATX"
    )
}

/// Familles de formats dont la valeur numérique SAS est un DATETIME (secondes
/// depuis 1960-01-01 00:00:00).
fn is_datetime_format(fmt: &str) -> bool {
    let name = format_family(fmt);
    matches!(
        name.as_str(),
        "DATETIME" | "DTDATE" | "DTMONYY" | "DTYYQC" | "E8601DT" | "B8601DT"
    )
}

/// Écrit le classeur XLSX via le writer partagé : une feuille (SHEET= ou le
/// nom du dataset), en-têtes = noms de variables, cellules typées (numériques
/// pour les numériques, dates stylées selon le format SAS, chaînes sinon,
/// cellules vides pour les missings).
fn execute_xlsx(
    ast: &ExportAst,
    ds: &crate::dataset::SasDataset,
    out_path: &std::path::Path,
) -> Result<()> {
    use crate::output::xlsx::{
        XlsxCell, XlsxSheet, sas_days_to_excel_serial, sas_seconds_to_excel_serial,
    };

    let sheet_name = ast.sheet.clone().unwrap_or_else(|| {
        ast.data
            .as_ref()
            .map(|d| d.name.clone())
            .unwrap_or_default()
    });
    if sheet_name.is_empty() {
        return Err(SasError::runtime(
            "PROC EXPORT DBMS=XLSX: cannot derive a sheet name (no DATA= and no SHEET=).",
        ));
    }
    // SAS limite les noms de feuilles à 31 caractères.
    let sheet_name = sanitize_sheet_name(&sheet_name);

    let headers: Vec<String> = ds.vars.iter().map(|v| v.name.clone()).collect();

    let mut rows: Vec<Vec<Option<XlsxCell>>> = Vec::with_capacity(ds.df.height());
    for row_idx in 0..ds.df.height() {
        let mut row: Vec<Option<XlsxCell>> = Vec::with_capacity(ds.vars.len());
        for v in ds.vars.iter() {
            let cell = if v.ty == crate::value::VarType::Char {
                ds.df
                    .column(&v.name)
                    .ok()
                    .and_then(|c| c.as_materialized_series().str().ok())
                    .and_then(|ca| ca.get(row_idx))
                    .map(|s| XlsxCell::Str(s.to_string()))
            } else {
                ds.df
                    .column(&v.name)
                    .ok()
                    .and_then(|c| c.as_materialized_series().f64().ok())
                    .and_then(|ca| ca.get(row_idx))
                    .map(|value| match v.format.as_deref() {
                        Some(f) if is_date_format(f) => {
                            XlsxCell::Date(sas_days_to_excel_serial(value))
                        }
                        Some(f) if is_datetime_format(f) => {
                            XlsxCell::Date(sas_seconds_to_excel_serial(value))
                        }
                        // Les autres formats (dont TIME.) restent des nombres
                        // bruts : la VALEUR survit au round-trip, pas le style.
                        _ => XlsxCell::Num(value),
                    })
            };
            row.push(cell);
        }
        rows.push(row);
    }

    let sheet = XlsxSheet {
        name: sheet_name,
        headers,
        rows,
    };
    let bytes = crate::output::xlsx::xlsx_build_typed(&[sheet]);
    std::fs::write(out_path, &bytes).map_err(|e| {
        SasError::runtime(format!("PROC EXPORT: cannot create '{}': {e}", ast.outfile))
    })?;
    Ok(())
}

/// Nettoie un nom de feuille Excel : caractères interdits (`[ ] : * ? / \`)
/// remplacés, troncature à 31 caractères (limite Excel/SAS).
fn sanitize_sheet_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '[' | ']' | ':' | '*' | '?' | '/' | '\\' => '_',
            other => other,
        })
        .collect();
    cleaned.chars().take(31).collect()
}

// ---------------------------------------------------------------------------
// Helpers internes
// ---------------------------------------------------------------------------

/// Résout le séparateur en octet selon DBMS + DELIMITER éventuel.
/// DBMS=XLSX n'utilise pas de séparateur (appelé uniquement pour CSV/TAB/DLM).
fn resolve_separator(ast: &ExportAst) -> u8 {
    match &ast.dbms {
        ExportDbms::Csv => b',',
        ExportDbms::Tab => b'\t',
        ExportDbms::Dlm => ast.delimiter.unwrap_or(b' '),
        ExportDbms::Xlsx => b',',
    }
}

/// Parse un DBMS par son nom en majuscules ; renvoie une erreur propre pour
/// les DBMS différés (XLSX/EXCEL).
fn parse_dbms(name: &str, span: crate::token::Span) -> Result<ExportDbms> {
    match name {
        "CSV" => Ok(ExportDbms::Csv),
        "TAB" => Ok(ExportDbms::Tab),
        "DLM" | "DLMSTR" => Ok(ExportDbms::Dlm),
        "XLSX" => Ok(ExportDbms::Xlsx),
        "EXCEL" | "XLS" => Err(SasError::runtime(format!(
            "PROC EXPORT with DBMS={name} is not supported in this build \
             (legacy binary .xls formats; use DBMS=XLSX for Excel workbooks)."
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
