//! Formats et informats utilisateur définis par PROC FORMAT (jalons M4/M18.2).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc format; value sexfmt 1='Male' 2='Female' other='?'; run;`
//! produit un `UserFormat` à plages :
//! - plage = valeur unique, `low-<high` / `low<-high` (bornes exclusives
//!   côté `<`), `LOW`/`HIGH` symboliques, ou liste `1,2,3='x'`.
//! - `VALUE` (num→label), `VALUE $fmt` (char→label) ; `INVALUE` pour les
//!   informats utilisateur (M18.2).
//! - Résolution : première plage qui matche dans l'ordre de tri SAS des
//!   bornes ; sinon `other` ; sinon la valeur formatée en BEST./$.
//!
//! ## INVALUE (M18.2)
//! `invalue grade 'A'=4 'B'=3 'C'=2 'D'=1 'F'=0;`
//! produit un `UserInformat` : les CLÉS sont des chaînes (plages de chaînes,
//! comme les formats char), la VALEUR de résultat est `Value::Num` ou
//! `Value::Char` selon que le nom porte `$` ou non.
//! - `invalue name` → résultat numérique ; clés = chaînes brutes à matcher.
//! - `invalue $name` → résultat caractère.
//! - La valeur de résultat peut être : littéral numérique `=4`, littéral
//!   chaîne `='Small'`, ou le mot-clé `_SAME_` (copie l'entrée non modifiée).
//! - `other=value` : valeur de repli si aucune plage ne correspond.
//! - Ranges de chaînes : `'A'-'F'` inclusive, `low-'C'`, `'D'-high`, avec
//!   bornes exclusives `<` ; comparison via `str::trim_end()` (insensible aux
//!   blancs finaux, fidèle SAS).
//! - Valeur inconnue (aucune plage, pas d'other) → missing.

#![allow(unused_variables, dead_code)]

use crate::value::Value;


mod picture;
mod informat;

pub use informat::InformatRange;
pub use informat::InformatValue;
pub use informat::UserInformat;
pub use picture::PictureDirectives;
pub use picture::PictureRange;
pub use picture::UserPicture;


/// The three kinds of user-defined format object that PROC FORMAT can build.
///
/// - `Value`   — `VALUE name range='label';`   (num/char → display label) → [`UserFormat`].
/// - `Picture` — `PICTURE name range='template' (dirs);` (num → templated digits) → [`UserPicture`].
/// - `Invalue` — `INVALUE name 'key'=result;`  (string → [`Value`]) → [`UserInformat`].
///
/// The three carry structurally different payloads, so they are stored in
/// separate maps in the [`crate::formats::FormatCatalog`]; this enum documents
/// the shared taxonomy and tags each stored object with its kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatKind {
    Value,
    Picture,
    Invalue,
}

#[derive(Debug)]
pub enum Bound {
    Low,
    High,
    Num(f64),
    Char(String),
}

impl Clone for Bound {
    fn clone(&self) -> Self {
        match self {
            Bound::Low => Bound::Low,
            Bound::High => Bound::High,
            Bound::Num(n) => Bound::Num(*n),
            Bound::Char(s) => Bound::Char(s.clone()),
        }
    }
}

#[derive(Clone)]
pub struct Range {
    pub from: Bound,
    pub to: Bound,
    pub from_exclusive: bool,
    pub to_exclusive: bool,
    pub label: String,
}

#[derive(Clone)]
pub struct UserFormat {
    pub is_char: bool,
    pub ranges: Vec<Range>,
    pub other: Option<String>,
}

impl UserFormat {
    pub fn lookup(&self, v: &Value) -> Option<&str> {
        for range in &self.ranges {
            if self.range_matches(range, v) {
                return Some(&range.label);
            }
        }
        self.other.as_deref()
    }

    fn range_matches(&self, range: &Range, v: &Value) -> bool {
        if self.is_char {
            // Character format: match Value::Char against Bound::Char bounds.
            let s = match v {
                Value::Char(s) => s.trim_end(),
                _ => return false,
            };
            let from_ok = match &range.from {
                Bound::Low => true,
                Bound::High => false,
                Bound::Char(c) => {
                    let c = c.trim_end();
                    if range.from_exclusive { s > c } else { s >= c }
                }
                Bound::Num(_) => false,
            };
            if !from_ok {
                return false;
            }
            let to_ok = match &range.to {
                Bound::High => true,
                Bound::Low => false,
                Bound::Char(c) => {
                    let c = c.trim_end();
                    if range.to_exclusive { s < c } else { s <= c }
                }
                Bound::Num(_) => false,
            };
            to_ok
        } else {
            // Numeric format: match Value::Num against numeric bounds.
            // Missing values don't match numeric ranges unless there is a
            // special handling; here we treat them as no-match (falls to `other`).
            let n = match v {
                Value::Num(n) => *n,
                _ => return false,
            };
            let from_ok = match &range.from {
                Bound::Low => true,
                Bound::High => false,
                Bound::Num(lo) => {
                    if range.from_exclusive { n > *lo } else { n >= *lo }
                }
                Bound::Char(_) => false,
            };
            if !from_ok {
                return false;
            }
            let to_ok = match &range.to {
                Bound::High => true,
                Bound::Low => false,
                Bound::Num(hi) => {
                    if range.to_exclusive { n < *hi } else { n <= *hi }
                }
                Bound::Char(_) => false,
            };
            to_ok
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
