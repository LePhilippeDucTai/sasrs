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
//! - Sous-requêtes : HORS périmètre M6 (ERROR propre "subqueries not
//!   yet supported") — lever la limite en M8.
//!
//! ## Tests
//! Assertions d'AST pures (pas besoin de l'exécution) : une requête par
//! forme syntaxique, y compris les ratés (messages d'erreur SAS-like).

#![allow(unused_variables, dead_code)]

use super::ast::SqlProgram;
use crate::error::Result;
use crate::parser::StatementStream;

/// Parse tout le contenu de `proc sql; ... quit;`.
pub fn parse_sql_program(ts: &mut StatementStream) -> Result<SqlProgram> {
    todo!("cf. plan en tête de fichier")
}
