//! Évaluateur d'expressions (tree-walking) sur le PDV.
//!
//! # Plan du fichier — voir PLAN.md  (difficulté : MOYENNE-ÉLEVÉE)
//!
//! ## Règles de coercition / missing (fidélité SAS)
//! - Arithmétique (`+ - * / **`) : si UN opérande est missing → résultat
//!   `.` + incrément `ctx.missing_generated` (PAS d'erreur).
//! - Division par zéro → `.` + note dédiée + `_ERROR_`.
//! - `0 ** 0` = 1 ; `(-2) ** 0.5` → missing + note (SAS).
//! - Comparaisons : via `Value::sas_cmp` → 1.0/0.0. ATTENTION : les
//!   missings SONT comparables (`. = .` vrai, `. < 0` vrai). Comparaison
//!   num/char → en SAS c'est une ERREUR de compilation ; ici : note +
//!   conversion automatique (cf. ci-dessous) pour rester permissif.
//! - `AND`/`OR`/`NOT` : `truthy()` de chaque opérande → 1.0/0.0 (pas de
//!   court-circuit nécessaire, pas d'effets de bord).
//! - `||` : opérandes num convertis en char via BEST12. JUSTIFIÉ À
//!   DROITE sur 12 (oui, avec les espaces de tête — fidèle à SAS) + note
//!   "Numeric values have been converted to character values..." UNE
//!   fois par étape.
//! - Conversion char→num automatique (char utilisé en contexte
//!   numérique) : trim puis parse f64 ; vide/invalide → `.` + note
//!   "Invalid numeric data" + `_ERROR_`. Note "Character values have
//!   been converted to numeric values..." une fois par étape.
//! - `IN` : égalités successives via sas_cmp.
//! - `Call` : déléguer à `functions::call` ; fonction inconnue → erreur
//!   de compilation en SAS ; ici ERROR à la première évaluation.
//!
//! ## EvalCtx
//! Collecte les notes uniques (conversions), les compteurs (missing
//! generated, division par zéro avec n° de ligne plus tard), et le flag
//! `_ERROR_` à reporter au PDV par l'exécuteur.

#![allow(unused_variables, dead_code)]

use super::pdv::Pdv;
use crate::ast::Expr;
use crate::value::Value;

#[derive(Default)]
pub struct EvalCtx {
    pub missing_generated: u32,
    pub division_by_zero: u32,
    pub note_num_to_char: bool,
    pub note_char_to_num: bool,
    pub invalid_data: u32,
    pub error_flag: bool,
    /// Erreur fatale (fonction inconnue...) — stoppe l'étape.
    pub fatal: Option<String>,
}

pub fn eval(expr: &Expr, pdv: &Pdv, ctx: &mut EvalCtx) -> Value {
    todo!("cf. règles en tête de fichier")
}
