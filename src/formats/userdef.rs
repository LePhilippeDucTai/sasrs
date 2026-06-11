//! Formats utilisateur définis par PROC FORMAT (jalon M4).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc format; value sexfmt 1='Male' 2='Female' other='?'; run;`
//! produit un `UserFormat` à plages :
//! - plage = valeur unique, `low-<high` / `low<-high` (bornes exclusives
//!   côté `<`), `LOW`/`HIGH` symboliques, ou liste `1,2,3='x'`.
//! - `VALUE` (num→label), `VALUE $fmt` (char→label) ; `INVALUE` pour les
//!   informats utilisateur (M4+).
//! - Résolution : première plage qui matche dans l'ordre de tri SAS des
//!   bornes ; sinon `other` ; sinon la valeur formatée en BEST./$.

#![allow(unused_variables, dead_code)]

use crate::value::Value;

pub enum Bound {
    Low,
    High,
    Num(f64),
    Char(String),
}

pub struct Range {
    pub from: Bound,
    pub to: Bound,
    pub from_exclusive: bool,
    pub to_exclusive: bool,
    pub label: String,
}

pub struct UserFormat {
    pub is_char: bool,
    pub ranges: Vec<Range>,
    pub other: Option<String>,
}

impl UserFormat {
    pub fn lookup(&self, v: &Value) -> Option<&str> {
        todo!("première plage qui matche ; cf. tête de fichier")
    }
}
