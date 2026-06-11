//! Parser d'expressions SAS (Pratt / precedence climbing), partagé par
//! l'étape DATA, les WHERE et (en partie) PROC SQL.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! ## Précédence SAS (du plus FORT au plus FAIBLE liage) — attention,
//! SAS est inhabituel :
//! 1. `**` (associatif à DROITE), préfixes `+` `-` `NOT` — oui, NOT lie
//!    très fort en SAS : `not x = 1` ≡ `(not x) = 1` !
//! 2. `*` `/`
//! 3. `+` `-` (binaires)
//! 4. `||` (concaténation)
//! 5. comparaisons `=` `ne` `<` `<=` `>` `>=` et `IN (v1, v2, ...)`
//!    (non associatives ; pas de chaînage a<b<c en M1)
//! 6. `AND`
//! 7. `OR`
//!
//! ## Primaires
//! - `TokenKind::Num` → `Expr::Num`
//! - `TokenKind::Str` : suffixe `None`→`Expr::Str` ; `Date`→ jours depuis
//!   1960-01-01 (parser `ddMONyyyy`, ex. `01jan2020`, via chrono ;
//!   `'...'d` invalide → erreur de parse) ; `Time`→ secondes depuis
//!   minuit (`hh:mm[:ss]`) ; `DateTime`→ secondes depuis 1960
//!   (`ddMONyyyy:hh:mm:ss`) ; `Name`→ `Expr::Str`.
//! - `Dot` → `Expr::Missing(Dot)` ; `Dot` immédiatement suivi d'un ident
//!   d'une lettre ou `_` (adjacent : vérifier les spans !) → missing
//!   spécial `.A`..`.Z` / `._`.
//! - `Ident` suivi de `(` → `Expr::Call { name, args }` (args séparés par
//!   des virgules, éventuellement vides : `today()`).
//! - `Ident` sinon → `Expr::Var`.
//! - `( expr )`.
//!
//! ## IN
//! `expr IN ( item [, item]* )` ; items = littéraux num/str (M1).
//! Produit `Expr::In { expr, list }`.
//!
//! ## Tests unitaires à écrire
//! - `2**3**2` = 512 (droite-associatif) une fois évalué.
//! - `not x = 1` parse comme `(not x) = 1`.
//! - `'01jan1960'd` → Num(0.0) ; `'02jan1960'd` → Num(1.0).
//! - `.a` adjacent → Missing(Letter(0)) ; `. a` (espace) → erreur ou
//!   Missing puis Var (le contexte appelant tranchera) — choisir Missing
//!   ordinaire si non adjacent.

#![allow(unused_variables, dead_code)]

use super::StatementStream;
use crate::ast::Expr;
use crate::error::Result;

pub fn parse_expr(ts: &mut StatementStream) -> Result<Expr> {
    todo!("precedence climbing, niveaux décrits en tête de fichier")
}
