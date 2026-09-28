//! Lecteur XLSX pour PROC IMPORT `DBMS=XLSX|EXCEL` (J08-P1).
//!
//! Lit le classeur via la crate `calamine` (aucune dépendance externe au
//! système : parseur pur Rust) et produit un `DataFrame` Polars restreint au
//! modèle de types SAS (Float64 / String), plus un format SAS par colonne
//! date (dates Excel sérial → dates SAS 1960, format `DATE9.` ou
//! `DATETIME20.`).
//!
//! ## Sémantique
//! - `SHEET=` : nom de feuille (exact puis insensible à la casse) ou numéro
//!   1-based ; défaut : la première feuille.
//! - `RANGE=` : plage A1 (`A1:C10`), éventuellement préfixée du nom de
//!   feuille (`Sheet1$A1:C10`) — le préfixe PRIME sur `SHEET=`. Une cellule
//!   seule (`A1`) est une plage à une cellule. Tout autre forme (plage
//!   nommée notamment) est refusée par une ERROR explicite.
//! - `GETNAMES=` : la première ligne de la plage donne les noms de variables
//!   (cellules vides/sanitisées → `VARn`).
//! - `GUESSINGROWS=` : fenêtre d'inférence des types — seules les `n`
//!   premières lignes de données décident numérique vs caractère ; les
//!   valeurs ultérieures non convertibles deviennent missing (comportement
//!   SAS : le type est figé par la fenêtre de scan).
//! - Missings : cellules vides → missing SAS (null), numérique ou caractère.

use crate::error::{Result, SasError};
use calamine::{Data, Reader, Xlsx, open_workbook};
use polars::prelude::*;

/// Lit le classeur `path` selon les options de `ast` (SHEET/RANGE/GETNAMES/
/// GUESSINGROWS) : DataFrame + format SAS par colonne (None = pas une date).
pub(super) fn read_xlsx(
    ast: &super::ImportAst,
    path: &std::path::Path,
) -> Result<(DataFrame, Vec<Option<String>>)> {
    let mut workbook: Xlsx<_> = open_workbook(path).map_err(|e| {
        SasError::runtime(format!(
            "PROC IMPORT: cannot open '{}': {}",
            ast.datafile, e
        ))
    })?;

    // --- RANGE= éventuel : le préfixe de feuille prime sur SHEET= ---
    let (range_sheet, cell_range) = match &ast.range {
        Some(spec) => {
            let (sheet, cells) = parse_range_spec(spec)?;
            (sheet, Some(cells))
        }
        None => (None, None),
    };

    // --- Résolution de la feuille ---
    let sheet_names = workbook.sheet_names();
    if sheet_names.is_empty() {
        return Err(SasError::runtime(format!(
            "PROC IMPORT: workbook '{}' contains no worksheet.",
            ast.datafile
        )));
    }
    let sheet_name =
        resolve_sheet_name(&sheet_names, ast.sheet.as_deref(), range_sheet.as_deref())?;

    let used = workbook.worksheet_range(&sheet_name).map_err(|e| {
        SasError::runtime(format!(
            "PROC IMPORT: error reading worksheet '{sheet_name}' of '{}': {}",
            ast.datafile, e
        ))
    })?;

    // --- Extraction de la grille (used range ∩ RANGE=) ---
    let grid = extract_grid(&used, cell_range)?;

    if grid.is_empty() || grid[0].is_empty() {
        return Err(SasError::runtime(format!(
            "PROC IMPORT: worksheet '{sheet_name}' of '{}' is empty.",
            ast.datafile
        )));
    }

    // --- Noms de variables ---
    let headers: Vec<String> = if ast.getnames {
        sanitize_headers(&grid[0])
    } else {
        (1..=grid[0].len()).map(|i| format!("VAR{i}")).collect()
    };
    let data_rows: &[Vec<Option<Data>>] = if ast.getnames { &grid[1..] } else { &grid[..] };

    // --- Typage colonne par colonne (fenêtre GUESSINGROWS) ---
    let scan = ast.guessingrows.unwrap_or(usize::MAX);
    let width = headers.len();
    let mut kinds: Vec<ColKind> = Vec::with_capacity(width);
    for c in 0..width {
        kinds.push(classify_column(data_rows, c, scan));
    }

    // --- Séries Polars ---
    let mut columns: Vec<Column> = Vec::with_capacity(width);
    let mut formats: Vec<Option<String>> = Vec::with_capacity(width);
    for (c, kind) in kinds.iter().enumerate() {
        let name = headers[c].as_str();
        match kind {
            ColKind::Numeric => {
                let ca: Float64Chunked = data_rows.iter().map(|row| cell_to_f64(&row[c])).collect();
                columns.push(ca.into_series().with_name(name.into()).into());
                formats.push(None);
            }
            ColKind::Date => {
                // Dates Excel sérial → jours/secondes SAS (époque 1960).
                let all_days = data_rows.iter().all(|row| {
                    cell_to_serial(&row[c])
                        .map(|s| (s - s.trunc()).abs() < 1e-9)
                        .unwrap_or(true)
                });
                let ca: Float64Chunked = data_rows
                    .iter()
                    .map(|row| {
                        cell_to_serial(&row[c]).map(|serial| {
                            if all_days {
                                crate::output::xlsx::excel_serial_to_sas_days(serial)
                            } else {
                                crate::output::xlsx::excel_serial_to_sas_seconds(serial)
                            }
                        })
                    })
                    .collect();
                columns.push(ca.into_series().with_name(name.into()).into());
                formats.push(Some(if all_days {
                    "DATE9.".to_string()
                } else {
                    "DATETIME20.".to_string()
                }));
            }
            ColKind::Text => {
                let ca: StringChunked = data_rows
                    .iter()
                    .map(|row| cell_to_string(&row[c]))
                    .collect();
                columns.push(ca.into_series().with_name(name.into()).into());
                formats.push(None);
            }
        }
    }

    let df = DataFrame::new(columns).map_err(|e| {
        SasError::runtime(format!("PROC IMPORT DBMS=XLSX: cannot build dataset: {e}"))
    })?;
    Ok((df, formats))
}

// ---------------------------------------------------------------------------
// Types internes
// ---------------------------------------------------------------------------

/// Classification d'une colonne par la fenêtre de scan.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ColKind {
    /// Numérique pur (aucune date dans la fenêtre).
    Numeric,
    /// Dates/timestamps Excel (au moins un DateTime dans la fenêtre).
    Date,
    /// Caractère (au moins une chaîne dans la fenêtre).
    Text,
}

/// Coordonnées de cellule 0-based (ligne, colonne).
type CellPos = (u32, u32);

/// Plage A1 résolue : `(début, fin)` 0-based incluses.
type CellSpan = (CellPos, CellPos);

/// Classifie la colonne `col` sur les `scan` premières lignes de données.
fn classify_column(rows: &[Vec<Option<Data>>], col: usize, scan: usize) -> ColKind {
    let mut kind = ColKind::Numeric;
    for row in rows.iter().take(scan) {
        if let Some(Some(t)) = row.get(col).and_then(|c| c.as_ref()).map(CellTaxonomy::of) {
            match t {
                CellTaxonomy::Text => return ColKind::Text,
                CellTaxonomy::Date => kind = ColKind::Date,
                CellTaxonomy::Number => {}
            }
        }
    }
    kind
}

/// Taxonomie d'une cellule non vide pour la classification de colonne.
enum CellTaxonomy {
    Number,
    Date,
    Text,
}

impl CellTaxonomy {
    fn of(cell: &Data) -> Option<Self> {
        match cell {
            Data::Empty => None,
            Data::Float(_) | Data::Int(_) | Data::Bool(_) => Some(Self::Number),
            Data::DateTime(_) => Some(Self::Date),
            Data::String(_) | Data::DateTimeIso(_) | Data::DurationIso(_) | Data::Error(_) => {
                Some(Self::Text)
            }
        }
    }
}

/// Valeur numérique d'une cellule (brute pour une colonne numérique).
/// NB : le système de dates 1904 (workbooks Mac legacy) n'est pas géré —
/// nos writer et lecteur utilisent le système 1900 standard.
fn cell_to_f64(cell: &Option<Data>) -> Option<f64> {
    match cell {
        Some(Data::Float(v)) => Some(*v),
        Some(Data::Int(v)) => Some(*v as f64),
        Some(Data::Bool(b)) => Some(if *b { 1.0 } else { 0.0 }),
        Some(Data::DateTime(dt)) => Some(dt.as_f64()),
        _ => None,
    }
}

/// Sérial Excel brut d'une cellule (colonne date).
fn cell_to_serial(cell: &Option<Data>) -> Option<f64> {
    match cell {
        Some(Data::DateTime(dt)) => Some(dt.as_f64()),
        Some(Data::Float(v)) => Some(*v),
        Some(Data::Int(v)) => Some(*v as f64),
        _ => None,
    }
}

/// Représentation chaîne d'une cellule dans une colonne caractère.
fn cell_to_string(cell: &Option<Data>) -> Option<String> {
    match cell {
        Some(Data::String(s)) => Some(s.clone()),
        Some(Data::Float(v)) => Some(format_float(*v)),
        Some(Data::Int(v)) => Some(v.to_string()),
        Some(Data::Bool(b)) => Some(if *b { "TRUE" } else { "FALSE" }.to_string()),
        Some(Data::DateTime(dt)) => Some(excel_serial_to_iso_date(dt.as_f64())),
        Some(Data::DateTimeIso(s)) | Some(Data::DurationIso(s)) => Some(s.clone()),
        Some(Data::Error(e)) => Some(format!("{e}")),
        None | Some(Data::Empty) => None,
    }
}

/// Convertit un sérial Excel (1900) en date ISO `YYYY-MM-DD` (algorithme
/// civil de Howard Hinnant, sans dépendance chrono).
fn excel_serial_to_iso_date(serial: f64) -> String {
    let days = serial.trunc() as i64;
    // Sérial Excel → jours depuis 1970-01-01 (25569 = 1970-01-01).
    let z = days - 25569;
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Formate un f64 sans `.0` traînant pour un entier (12 → "12", 12.5 → "12.5").
fn format_float(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

// ---------------------------------------------------------------------------
// SHEET= / RANGE=
// ---------------------------------------------------------------------------

/// Résout le nom de feuille à ouvrir : préfixe de RANGE= > SHEET= > première
/// feuille. `SHEET=` accepte un nom ou un numéro 1-based.
fn resolve_sheet_name(
    sheet_names: &[String],
    sheet: Option<&str>,
    range_sheet: Option<&str>,
) -> Result<String> {
    let wanted = range_sheet.or(sheet);
    let Some(wanted) = wanted else {
        return Ok(sheet_names[0].clone());
    };
    // Numéro 1-based ?
    if let Ok(idx) = wanted.parse::<usize>() {
        if range_sheet.is_some() {
            return Err(SasError::runtime(
                "PROC IMPORT: the sheet prefix of RANGE= must be a sheet NAME, not a number.",
            ));
        }
        return sheet_names.get(idx - 1).cloned().ok_or_else(|| {
            SasError::runtime(format!(
                "PROC IMPORT: SHEET={idx} is out of range (workbook has {} worksheet(s)).",
                sheet_names.len()
            ))
        });
    }
    // Nom exact puis insensible à la casse.
    if sheet_names.iter().any(|s| s == wanted) {
        return Ok(wanted.to_string());
    }
    let lower = wanted.to_ascii_lowercase();
    sheet_names
        .iter()
        .find(|s| s.to_ascii_lowercase() == lower)
        .cloned()
        .ok_or_else(|| {
            SasError::runtime(format!(
                "PROC IMPORT: worksheet '{wanted}' not found (available: {}).",
                sheet_names.join(", ")
            ))
        })
}

/// Parse `RANGE=` : `(feuille optionnelle, (début 0-based, fin 0-based))`.
/// Formes acceptées : `A1:C10`, `A1`, `Sheet1$A1:C10`. Une plage nommée ou
/// toute autre forme est une ERROR explicite.
fn parse_range_spec(spec: &str) -> Result<(Option<String>, CellSpan)> {
    let (sheet_part, a1_part) = match spec.split_once('$') {
        Some((s, r)) => (Some(s.to_string()), r),
        None => (None, spec),
    };
    let (start, end) = match a1_part.split_once(':') {
        Some((a, b)) => (parse_a1(a)?, parse_a1(b)?),
        None => {
            let cell = parse_a1(a1_part)?;
            (cell, cell)
        }
    };
    if start.0 > end.0 || start.1 > end.1 {
        return Err(SasError::runtime(format!(
            "PROC IMPORT: RANGE='{spec}' is inverted (start must precede end)."
        )));
    }
    Ok((sheet_part, (start, end)))
}

/// Parse une référence `A1` en (ligne 0-based, colonne 0-based).
fn parse_a1(cell: &str) -> Result<(u32, u32)> {
    let cell = cell.trim();
    let mut col: u32 = 0;
    let mut row_str = String::new();
    for ch in cell.chars() {
        if ch.is_ascii_alphabetic() {
            if !row_str.is_empty() {
                return Err(bad_a1(cell));
            }
            col = col
                .checked_mul(26)
                .and_then(|c| c.checked_add(ch.to_ascii_uppercase() as u32 - 'A' as u32 + 1))
                .ok_or_else(|| bad_a1(cell))?;
        } else if ch.is_ascii_digit() {
            row_str.push(ch);
        } else {
            return Err(bad_a1(cell));
        }
    }
    if col == 0 || row_str.is_empty() {
        return Err(bad_a1(cell));
    }
    let row: u32 = row_str.parse().map_err(|_| bad_a1(cell))?;
    if row == 0 {
        return Err(bad_a1(cell));
    }
    Ok((row - 1, col - 1))
}

fn bad_a1(cell: &str) -> SasError {
    SasError::runtime(format!(
        "PROC IMPORT: RANGE= accepts only A1-style ranges (e.g. A1:C10 or \
         Sheet1$A1:C10); '{cell}' is not one (named ranges are not supported)."
    ))
}

/// Extrait la grille de cellules : used range complet, ou son intersection
/// avec la plage demandée (cellules hors used range → vides/missing).
fn extract_grid(
    used: &calamine::Range<Data>,
    cell_range: Option<CellSpan>,
) -> Result<Vec<Vec<Option<Data>>>> {
    let (used_start, used_end) = (used.start().unwrap_or((0, 0)), used.end().unwrap_or((0, 0)));
    let (start, end) = cell_range.unwrap_or((used_start, used_end));
    let mut grid: Vec<Vec<Option<Data>>> = Vec::new();
    for r in start.0..=end.0 {
        let mut row = Vec::new();
        for c in start.1..=end.1 {
            let inside =
                r >= used_start.0 && r <= used_end.0 && c >= used_start.1 && c <= used_end.1;
            let cell = if inside {
                used.get((r as usize, c as usize)).cloned()
            } else {
                None
            };
            // calamine rend les cellules vides comme Data::Empty → missing.
            let cell = match cell {
                Some(Data::Empty) | None => None,
                other => other,
            };
            row.push(cell);
        }
        grid.push(row);
    }
    Ok(grid)
}

/// Sanitise les en-têtes GETNAMES : cellule vide → `VARn`, caractères non
/// valides pour un nom SAS → `_`, nom commençant par un chiffre → préfixe
/// `_` (convention VALIDVARNAME=V6 SAS).
fn sanitize_headers(header_row: &[Option<Data>]) -> Vec<String> {
    header_row
        .iter()
        .enumerate()
        .map(|(i, cell)| {
            let raw = cell_to_string(cell).unwrap_or_default();
            let raw = raw.trim();
            if raw.is_empty() {
                return format!("VAR{}", i + 1);
            }
            let mut name: String = raw
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                name.insert(0, '_');
            }
            name.truncate(32);
            name
        })
        .collect()
}
