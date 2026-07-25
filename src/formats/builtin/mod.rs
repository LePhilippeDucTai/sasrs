//! Formats/informats intégrés (jalon M4) — table-driven, idéal pour un
//! modèle économique, format par format avec tests unitaires exhaustifs.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! ## Formats numériques
//! - `w.d` : arrondi à d décimales, justifié droite sur w ; débordement →
//!   BESTw., puis `*` répétés si vraiment trop étroit (règle SAS).
//! - `BESTw.` : déjà approximé par `value::format_best` — déplacer ici la
//!   version finale (justifiée droite sur w pour PUT).
//! - `COMMAw.d` (séparateur milliers `,`), `DOLLARw.d` (`$` + virgules),
//!   `Zw.d` (zéros de tête), `Ew.` (scientifique), `PERCENTw.d`.
//! - Missings : `.` / `_` / `A`..`Z` justifiés droite.
//! ## Formats caractère
//! - `$w.` / `$CHARw.` : tronquer/padder à w (gauche).
//! ## Formats date/heure (valeur = jours ou secondes depuis 1960)
//! - `DATE9.` (01JAN2020), `DATE7.`, `DDMMYYw.`, `MMDDYYw.`, `YYMMDDw.`
//!   (séparateurs selon w : 8 sans, 10 avec), `MONYY7.`, `WORDDATEw.`,
//!   `DATETIMEw.d` (01JAN2020:12:34:56), `TIMEw.d` (hh:mm:ss).
//!   Conversion jours→date via chrono : 1960-01-01 + jours.
//! ## Informats
//! - `w.d` (parse f64 ; d implicite si pas de point décimal dans la
//!   source : 123 avec informat 5.2 → 1.23 — piège SAS célèbre),
//!   `COMMAw.d` (vire $ et ,), `DATE9.`/`MMDDYY10.`/`DDMMYY10.`/
//!   `YYMMDD10.` → jours depuis 1960, `TIMEw.` → secondes.
//!
//! `format_builtin`/`informat_builtin` renvoient None si le nom est
//! inconnu (le catalogue essaie alors les formats utilisateur).

#![allow(unused_variables, dead_code)]

use super::{right_justify, FormatSpec};
use crate::value::{format_best, Value};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Timelike};

mod datetime;
mod helpers;
mod informat;
mod numeric;

pub use informat::informat_builtin;

use datetime::*;
use helpers::*;
use numeric::*;

// ─────────────────────────────────────────────────────────────────────────────
// format_builtin
// ─────────────────────────────────────────────────────────────────────────────

/// Apply a builtin SAS format. Returns None if the format name is unknown.
///
/// NOTE: `Value::Missing` is handled by the catalog BEFORE this function is
/// called, so we never see it here for numeric formats. Character formats
/// still check for `Value::Char`.
pub fn format_builtin(v: &Value, spec: &FormatSpec) -> Option<String> {
    let name = spec.name.to_uppercase();

    // ── Character formats ────────────────────────────────────────────────────
    if let Some(s) = format_char_family(v, spec, &name) {
        return Some(s);
    }

    // ── Numeric formats — require a numeric value ────────────────────────────
    let fval = match v {
        Value::Num(n) => *n,
        // Missing handled by catalog; if we somehow get here, return the char.
        Value::Missing(k) => {
            let ch = k.display();
            let w = spec.w.unwrap_or(1) as usize;
            return Some(right_justify(&ch, w));
        }
        Value::Char(_) => return None, // numeric format on char → unknown
    };

    match name.as_str() {
        // ── w.d (plain numeric) ──────────────────────────────────────────────
        "" => Some(format_wd(fval, spec.w, spec.d)),

        // ── BESTw. ───────────────────────────────────────────────────────────
        "BEST" => fmt_best(fval, spec),

        // ── COMMAw.d ─────────────────────────────────────────────────────────
        "COMMA" => fmt_comma(fval, spec),

        // ── DOLLARw.d ────────────────────────────────────────────────────────
        "DOLLAR" => fmt_dollar(fval, spec),

        // ── Zw.d (zero-padded) ───────────────────────────────────────────────
        "Z" => fmt_z(fval, spec),

        // ── PERCENTw.d ───────────────────────────────────────────────────────
        // SAS behavior: multiply by 100, append %. For negative, SAS uses
        // parentheses like DOLLAR: (1.23%) — we implement the simpler -1.23%
        // (documented simplification).
        "PERCENT" => fmt_percent(fval, spec),

        // ── Ew. (scientific notation) ────────────────────────────────────────
        "E" => fmt_e(fval, spec),

        // ── Date formats ─────────────────────────────────────────────────────

        // DATE9. → 01JAN2020, DATE7. → 01JAN20
        "DATE" => fmt_date(fval, spec),

        // DDMMYYw.: w=8 → ddmmyy (no sep), w=10 → dd/mm/yyyy
        "DDMMYY" => fmt_ddmmyy(fval, spec),

        // MMDDYYw.: w=8 → mmddyy, w=10 → mm/dd/yyyy
        "MMDDYY" => fmt_mmddyy(fval, spec),

        // YYMMDDw.: w=8 → yymmdd, w=10 → yyyy/mm/dd
        "YYMMDD" => fmt_yymmdd(fval, spec),

        // MONYY7. → JAN2020
        "MONYY" => fmt_monyy(fval, spec),

        // WORDDATEw. → January 1, 2020
        "WORDDATE" => fmt_worddate(fval, spec),

        // DATETIMEw.d → 01JAN2020:12:34:56
        "DATETIME" => fmt_datetime(fval, spec),

        // TIMEw.d → hh:mm:ss
        "TIME" => fmt_time(fval, spec),

        // ── COMMAX: like COMMA but with period as thousands sep and comma as decimal
        // European-style: 1.234,56
        "COMMAX" => fmt_commax(fval, spec),

        // ── DOLLARX: like DOLLAR but European-decimal style ($1.234,56)
        "DOLLARX" => fmt_dollarx(fval, spec),

        // ── EURO: European style with € prefix (€1.234,56)
        "EURO" | "EUROX" => fmt_euro(fval, spec),

        // ── NEGPAREN: negative numbers in parentheses; positives as-is
        "NEGPAREN" => fmt_negparen(fval, spec),

        // ── HEX: integer to hexadecimal uppercase
        "HEX" => fmt_hex(fval, spec),

        // ── BINARY: integer to binary representation
        "BINARY" => fmt_binary(fval, spec),

        // ── OCTAL: integer to octal
        "OCTAL" => fmt_octal(fval, spec),

        // ── ROMAN: Roman numeral notation (1–3999)
        "ROMAN" => fmt_roman(fval, spec),

        // ── WORDS: English words representation
        "WORDS" => fmt_words(fval, spec),

        // ── FRACT: fractional notation (0.5 → 1/2)
        "FRACT" => fmt_fract(fval, spec),

        // ── SCIENTIFIC: scientific notation (SAS-style: 1.23E+02)
        "SCIENTIFIC" => fmt_scientific(fval, spec),

        // ── Date formats (M18.1 additions) ──────────────────────────────────

        // WEEKDATE: abbreviated day of week (MON, TUE, etc.)
        "WEEKDATE" => fmt_weekdate(fval, spec),

        // DOWNAME: full day name (Monday, Tuesday, etc.)
        "DOWNAME" => fmt_downame(fval, spec),

        // MONNAME: full month name (January, February, etc.)
        "MONNAME" => fmt_monname(fval, spec),

        // QTR: quarter number Q1–Q4
        "QTR" | "QTRR" => fmt_qtr(fval, spec),

        // YYQ: year + quarter (2024Q1)
        "YYQ" => fmt_yyq(fval, spec),

        // JULIAN: Julian day format YYYYDDD (day-of-year)
        "JULIAN" => fmt_julian(fval, spec),

        // B8601DA / B8601DT / B8601TM — ISO 8601 basic (no separators)
        "B8601DA" => fmt_b8601da(fval, spec),

        "B8601DT" => fmt_b8601dt(fval, spec),

        "B8601TM" => fmt_b8601tm(fval, spec),

        // E8601DA / E8601DT / E8601TM — ISO 8601 extended (with separators)
        "E8601DA" => fmt_e8601da(fval, spec),

        "E8601DT" => fmt_e8601dt(fval, spec),

        "E8601TM" => fmt_e8601tm(fval, spec),

        _ => None,
    }
}

/// Character formats ($, $CHAR, $F, $QUOTE, $HEX, $UPCASE).
/// Returns `None` when `name` is not a character format.
fn format_char_family(v: &Value, spec: &FormatSpec, name: &str) -> Option<String> {
    match name {
        "$" | "$CHAR" | "$F" => {
            let s = match v {
                Value::Char(c) => c.clone(),
                Value::Num(n) => format_best(*n, 12),
                Value::Missing(k) => k.display(),
            };
            return Some(match spec.w {
                None => s,
                Some(w) => {
                    let w = w as usize;
                    let mut out = s;
                    out.truncate(w);
                    while out.len() < w {
                        out.push(' ');
                    }
                    out
                }
            });
        }

        // $QUOTE: wrap value in double quotes  "hello"
        "$QUOTE" => {
            let inner = match v {
                Value::Char(c) => c.clone(),
                Value::Num(n) => format_best(*n, 12),
                Value::Missing(k) => k.display(),
            };
            let quoted = format!("\"{}\"", inner);
            return Some(match spec.w {
                None => quoted,
                Some(w) => {
                    let w = w as usize;
                    let mut out = quoted;
                    out.truncate(w);
                    while out.len() < w {
                        out.push(' ');
                    }
                    out
                }
            });
        }

        // $HEX: each byte of the string → two uppercase hex chars
        "$HEX" => {
            let s = match v {
                Value::Char(c) => c.clone(),
                Value::Num(n) => format_best(*n, 12),
                Value::Missing(k) => k.display(),
            };
            let hex: String = s.bytes().map(|b| format!("{:02X}", b)).collect();
            return Some(match spec.w {
                None => hex,
                Some(w) => {
                    let w = w as usize;
                    let mut out = hex;
                    out.truncate(w);
                    while out.len() < w {
                        out.push(' ');
                    }
                    out
                }
            });
        }

        // $UPCASE: uppercase the string
        "$UPCASE" => {
            let s = match v {
                Value::Char(c) => c.to_uppercase(),
                Value::Num(n) => format_best(*n, 12).to_uppercase(),
                Value::Missing(k) => k.display(),
            };
            return Some(match spec.w {
                None => s,
                Some(w) => {
                    let w = w as usize;
                    let mut out = s;
                    out.truncate(w);
                    while out.len() < w {
                        out.push(' ');
                    }
                    out
                }
            });
        }

        _ => {}
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
