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
//!
//! # Notes d'implémentation (M6 box 2)
//!
//! ## Résolution alias/colonnes qualifiées
//! Approche retenue, simple et robuste : on construit la frame FROM (+ ses
//! joins) en un espace de noms PLAT. À chaque table on associe (via le
//! schéma collecté de sa frame scannée) l'ensemble de SES noms de colonnes.
//! Une référence `alias.col` est résolue au nom NU `col` : on suppose que
//! les tables jointes ont des colonnes non-clés distinctes ; les clés de
//! jointure partagées sont coalescées (les joins equi de Polars, en
//! coalesce par défaut pour Inner/Left/Right, fusionnent la clé), donc une
//! seule colonne `col` survit. Si la même colonne non-clé existe des deux
//! côtés, Polars la suffixe `_right` ; on ne tente PAS de la désambiguïser
//! ici (hors périmètre M6, documenté). La résolution se contente donc de
//! renvoyer `col(column)`.
//!
//! ## Normalisation des missings spéciaux (NaN-payload → null)
//! `missing::nullify_specials` opère en EAGER. En lazy on réplique son
//! effet : juste après `scan`, pour chaque colonne Float64 du schéma on
//! applique `when(col.is_nan()).then(lit(NULL)).otherwise(col)`. Les
//! missings ordinaires `.` sont DÉJÀ des nulls. Invariant : avant toute
//! jointure / agrégation / comparaison, les missings spéciaux sont null.
//!
//! ## Couverture des set-ops
//! UNION [ALL] : `concat` vertical (+ `.unique` sauf ALL). EXCEPT et
//! INTERSECT (DISTINCT) : anti-/semi-join sur TOUTES les colonnes (avec
//! `join_nulls(true)`) + `.unique`. Les variantes ALL honorent EXACTEMENT la
//! multiplicité SAS : EXCEPT ALL conserve `max(0, n_gauche - n_droite)` copies
//! de chaque ligne, INTERSECT ALL en conserve `min(n_gauche, n_droite)`. On y
//! parvient sans itérer ligne à ligne en numérotant l'occurrence de chaque
//! ligne identique (rang cumulatif via une fenêtre `over` sur toutes les
//! colonnes) puis en faisant la jointure sur (colonnes + rang).

#![allow(unused_variables, dead_code)]

use crate::ast::Expr as SasExpr;
use crate::ast::{BinaryOp, UnaryOp};
use crate::error::{Result, SasError};
use crate::session::Session;
use crate::sql::ast::{JoinKind, SelectItem, SelectStmt, SetOp, SqlExpr};
use crate::value::MissingKind;
use polars::prelude::*;

mod expr;
mod grouping;
mod project;
mod setop;
mod source;
mod subquery;
use expr::*;
use grouping::*;
use project::*;
use setop::*;
use source::*;
use subquery::*;

/// Contexte de traduction : permet à `CALCULATED x` de retrouver
/// l'expression de l'alias `x` dans le select-list courant.
struct Ctx<'a> {
    /// (alias minuscule → SqlExpr) du select-list, pour CALCULATED.
    aliases: &'a [(String, SqlExpr)],
}

impl<'a> Ctx<'a> {
    fn empty() -> Ctx<'static> {
        Ctx { aliases: &[] }
    }
}

/// Traduit un prédicat SQL nu (sans CALCULATED ni agrégats) en expression
/// Polars. Utilisé par `DELETE FROM ... WHERE` (cf. sql/mod.rs), qui filtre
/// une frame déjà scannée et normalisée. Réutilise exactement la sémantique
/// des missings (`x = .` → is_null, etc.) du traducteur interne.
pub(crate) fn translate_predicate(pred: &SqlExpr) -> Result<Expr> {
    sql_expr_to_polars(pred, &Ctx::empty())
}

/// Traduit une expression SQL scalaire nue (sans CALCULATED ni agrégats) en
/// expression Polars, contexte vide. Utilisé par `UPDATE ... SET` (cf.
/// sql/mod.rs) pour évaluer chaque assignation contre la frame scannée.
pub(crate) fn translate_expr(e: &SqlExpr) -> Result<Expr> {
    sql_expr_to_polars(e, &Ctx::empty())
}

/// Réplique l'effet eager de `missing::nullify_specials` sur une LazyFrame :
/// pour chaque colonne Float64, NaN-payload (missings spéciaux) → null, afin
/// que les comparaisons Polars d'un `WHERE` voient bien les missings.
pub(crate) fn normalize_specials(mut lf: LazyFrame) -> Result<LazyFrame> {
    let schema = lf.collect_schema()?;
    let float_cols: Vec<String> = schema
        .iter()
        .filter(|(_, dt)| matches!(dt, DataType::Float64))
        .map(|(name, _)| name.to_string())
        .collect();
    for name in float_cols {
        lf = lf.with_column(
            when(col(name.clone()).is_nan())
                .then(lit(NULL))
                .otherwise(col(name.clone()))
                .alias(name.clone()),
        );
    }
    Ok(lf)
}

pub fn lower_select(query: &SelectStmt, session: &mut Session) -> Result<LazyFrame> {
    // 0. Sous-requêtes (M20.2) : résolution préalable des sous-requêtes
    // non-corrélées (scalaire / IN / EXISTS) en littéraux. Les sous-requêtes
    // corrélées sont détectées et signalées par une erreur documentée.
    let resolved = resolve_subqueries(query, session)?;
    let query = &resolved;

    // 1. FROM + joins.
    let mut lf = build_from(query, session)?;

    // 2. WHERE.
    if let Some(w) = &query.where_ {
        let pred = sql_expr_to_polars(w, &Ctx::empty())?;
        lf = lf.filter(pred);
    }

    // Liste des alias du select-list (pour CALCULATED).
    let aliases: Vec<(String, SqlExpr)> = query
        .items
        .iter()
        .filter_map(|it| {
            it.alias
                .as_ref()
                .map(|a| (a.to_ascii_lowercase(), it.expr.clone()))
        })
        .collect();
    let ctx = Ctx { aliases: &aliases };

    let has_agg = query.items.iter().any(|it| item_has_aggregate(&it.expr));
    let has_group = !query.group_by.is_empty();

    if has_group || (has_agg && all_items_aggregated(query)) {
        // 3.+4.+6. GROUP BY + agrégats + HAVING + projection finale. Tout est
        // fait ensemble : après agrégation, les colonnes agrégées et clés
        // existent par leur nom de sortie ; on ne peut plus ré-évaluer les
        // agrégats sur la frame réduite.
        lf = apply_group_by_project(query, lf, &ctx)?;

        // 7. DISTINCT puis 8. ORDER BY (sur les colonnes de sortie).
        if query.distinct {
            lf = lf.unique(None, UniqueKeepStrategy::Any);
        }
        if !query.order_by.is_empty() {
            lf = apply_order_by(query, lf, &ctx, false)?;
        }
    } else if has_agg {
        // 5. REMERGE : agrégats mélangés à des colonnes nues sans GROUP BY
        // couvrant. On calcule l'agrégat (total général) et on le rejoint à
        // chaque ligne (cross join), puis on projette.
        session
            .log
            .note("The query requires remerging summary statistics back with the original data.");
        lf = apply_remerge(query, lf, &ctx, session)?;
        if query.distinct {
            lf = lf.unique(None, UniqueKeepStrategy::Any);
        }
        if !query.order_by.is_empty() {
            lf = apply_order_by(query, lf, &ctx, false)?;
        }
    } else {
        // 6. SELECT list ordinaire. ORDER BY peut référencer des colonnes
        // SOURCE absentes du select-list (autorisé par SAS) : on trie AVANT
        // de projeter (les clés sont résolues sur les colonnes source / via
        // CALCULATED). Le tri étant stable et la projection/déduplication
        // préservant l'ordre, le résultat final reste correctement trié.
        if !query.order_by.is_empty() {
            lf = apply_order_by(query, lf, &ctx, true)?;
        }
        lf = project_select_list(query, lf, &ctx, session)?;
        if query.distinct {
            lf = lf.unique(None, UniqueKeepStrategy::Any);
        }
    }

    // 9. SET OPS.
    if let Some((op, all, rhs)) = &query.set_op {
        let rhs_lf = lower_select(rhs, session)?;
        lf = apply_set_op(lf, rhs_lf, op, *all)?;
    }

    Ok(lf)
}

#[cfg(test)]
mod tests;
