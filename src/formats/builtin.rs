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

use super::FormatSpec;
use crate::value::Value;

pub fn format_builtin(v: &Value, spec: &FormatSpec) -> Option<String> {
    todo!("cf. liste en tête de fichier")
}

pub fn informat_builtin(s: &str, spec: &FormatSpec) -> Option<Value> {
    todo!()
}
