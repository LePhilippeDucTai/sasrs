//! Parser récursif descendant du dialecte SQL de SAS (jalon M6).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! Travaille sur le même flux de tokens que le reste (`StatementStream`),
//! la grammaire SQL n'ayant besoin d'aucun token supplémentaire.
//!
//! ## Points de grammaire
//! - Mots-clés contextuels (select, from, where, group, by, having,
//!   order, on, as, inner, left, right, full, join, union, except,
//!   intersect, all, distinct, calculated, between, is, null, missing,
//!   like, case...) matchés par `is_kw`, jamais réservés globalement.
//! - Expressions : reprendre la grammaire de `parser::expr` ÉTENDUE
//!   (précédence SQL standard) avec les nœuds SqlExpr (qualified refs
//!   `a.x` — ATTENTION à désambiguïser de `lib.table` selon le
//!   contexte, CALCULATED, agrégats, BETWEEN/IS NULL/LIKE).
//! - GROUP BY positionnel (`group by 1, 2`) : entiers littéraux.
//! - `select *` et `a.*`.
//! - Sous-requêtes (M20.2) : scalaires `(SELECT ...)`, `IN (SELECT ...)` et
//!   `[NOT] EXISTS (SELECT ...)` non-corrélées sont parsées en nœuds dédiés
//!   (`SqlExpr::Subquery` / `InSubquery` / `Exists`) puis résolues à
//!   l'abaissement. Les sous-requêtes en FROM restent hors périmètre.
//!
//! ## Approche d'implémentation des expressions
//! On NE délègue PAS bloc à `parser::expr::parse_expr` pour l'ensemble
//! d'une expression SQL : les nœuds spécifiques SQL (CALCULATED, `a.x`
//! qualifié, agrégats `COUNT(*)`, BETWEEN / IS [NOT] NULL|MISSING / LIKE)
//! peuvent apparaître AU MILIEU d'une expression arithmétique/booléenne,
//! ce qui imposerait de réécrire le résultat. On déroule donc à la main
//! une échelle de précédence au niveau `SqlExpr` :
//!   or → and → not → comparaison (+ BETWEEN / IS / LIKE, non assoc.)
//!     → add_sub → mul_div → concat → unary(+/-) → atome
//! Les atomes purement « base » (littéraux, appels de fonction non
//! agrégés, variables) sont construits soit directement, soit en
//! délégant le parsing des ARGUMENTS d'appel à `parse_expr` (lui-même un
//! sous-arbre `Expr` autonome). Les opérandes des agrégats sont parsés au
//! niveau SqlExpr complet pour autoriser `count(distinct a.x)`.
//!
//! ## Représentation de `*`
//! `select *`  → `SelectItem { expr: SqlExpr::Star, alias: None }`.
//! `select a.*` → `SelectItem { expr: SqlExpr::QualifiedStar("a"), .. }`.
//!
//! ## Tests
//! Assertions d'AST pures (pas besoin de l'exécution) : une requête par
//! forme syntaxique, y compris les ratés (messages d'erreur SAS-like).

#![allow(unused_variables, dead_code)]

use super::ast::{
    FromItem, Join, JoinKind, SelectItem, SelectStmt, SetOp, SqlExpr, SqlProgram, SqlStmt,
};
use crate::ast::{BinaryOp, Expr, UnaryOp};
use crate::error::{Result, SasError};
use crate::parser::expr::parse_expr;
use crate::parser::StatementStream;
use crate::token::TokenKind;


mod stmt;
mod select;
mod expr;
mod base;
mod continuation;

pub use stmt::parse_sql_program;

use select::*;
use expr::*;
use base::*;
use continuation::*;

#[cfg(test)]
mod tests;
