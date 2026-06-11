//! Abaissement SQL → Polars LazyFrame (jalon M6).
//!
//! # Plan du fichier — voir PLAN.md  (difficulté : ÉLEVÉE — sémantique)
//!
//! ## Pipeline
//! `FROM` → `LibraryProvider::scan` + `missing::nullify_specials`
//! (IMPÉRATIF : les NaN-payload doivent devenir des nulls avant tout
//! calcul Polars) → joins → filter(where) → group_by/agg → having →
//! select/aliases → distinct → sort(order by) → set-ops.
//!
//! ## Spécificités SAS à répliquer
//! 1. `CALCULATED x` : ré-expansion de l'expression de l'alias (les
//!    select Polars sont parallèles, pas séquentiels).
//! 2. REMERGE : si le select-list mélange agrégats et colonnes nues sans
//!    GROUP BY couvrant → calculer la frame agrégée puis la REJOINDRE
//!    aux lignes d'origine sur les clés (cross join du total général si
//!    pas de GROUP BY), et émettre la NOTE SAS exacte : "The query
//!    requires remerging summary statistics back with the original
//!    data."
//! 3. Missings : `where x = .` ≡ `x is null` → traduire les
//!    comparaisons à un littéral missing en `is_null()` ; jointures :
//!    SAS apparie les clés missing entre elles → `join_nulls(true)`.
//! 4. Comparaisons char : ignorer les blancs finaux (trim_end les deux
//!    côtés, cohérent avec `Value::sas_cmp`).
//! 5. ORDER BY : missings en premier (ordre SAS), tri stable.
//!
//! ## Sortie
//! CREATE TABLE : collect → ré-attacher des VarMeta (types depuis le
//! schéma, formats hérités si colonne copiée telle quelle) → write +
//! NOTE "Table WORK.X created, with N rows and M columns)." ; SELECT nu
//! → rendu listing (réutiliser listing::write_table).

#![allow(unused_variables, dead_code)]

use super::ast::SelectStmt;
use crate::error::Result;
use crate::session::Session;
use polars::prelude::LazyFrame;

pub fn lower_select(query: &SelectStmt, session: &mut Session) -> Result<LazyFrame> {
    todo!("cf. pipeline en tête de fichier")
}
