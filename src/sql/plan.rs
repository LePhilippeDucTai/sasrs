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
//! UNION [ALL] : `concat` vertical (+ `.unique` sauf ALL). EXCEPT [ALL] et
//! INTERSECT [ALL] : anti-/semi-join sur TOUTES les colonnes (avec
//! `join_nulls(true)`), `.unique` sauf ALL. (ALL ne duplique pas
//! fidèlement la multiplicité pour EXCEPT/INTERSECT — approximation
//! documentée ; UNION ALL est exact.)

#![allow(unused_variables, dead_code)]

use super::ast::{JoinKind, SelectItem, SelectStmt, SetOp, SqlExpr};
use crate::ast::{BinaryOp, UnaryOp};
use crate::ast::Expr as SasExpr;
use crate::error::{Result, SasError};
use crate::session::Session;
use crate::value::MissingKind;
use polars::prelude::*;

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

// ----------------------------------------------------------------------------
// 1. FROM + joins
// ----------------------------------------------------------------------------

fn scan_normalized(session: &Session, lib: &str, table: &str) -> Result<LazyFrame> {
    let provider = session.libs.get(lib)?;
    let lf = provider.scan(table)?;
    // Normalisation des missings spéciaux (NaN-payload → null) sur chaque
    // colonne Float64 — passe par l'unique implémentation `normalize_specials`
    // (cf. note d'en-tête : ne jamais réimplémenter ad hoc).
    normalize_specials(lf)
}

fn build_from(query: &SelectStmt, session: &mut Session) -> Result<LazyFrame> {
    let Some(first) = query.from.first() else {
        return Err(SasError::runtime(
            "PROC SQL: a SELECT must have a FROM clause.",
        ));
    };
    let mut lf = scan_normalized(
        session,
        &first.table.libref_or_work(),
        &first.table.name.to_uppercase(),
    )?;

    // Tables FROM additionnelles (séparées par des virgules) = cross join.
    for extra in query.from.iter().skip(1) {
        let rhs = scan_normalized(
            session,
            &extra.table.libref_or_work(),
            &extra.table.name.to_uppercase(),
        )?;
        lf = lf.join(
            rhs,
            [] as [Expr; 0],
            [] as [Expr; 0],
            JoinArgs::new(JoinType::Cross),
        );
    }

    // Joins explicites.
    for join in &query.joins {
        let rhs = scan_normalized(
            session,
            &join.table.table.libref_or_work(),
            &join.table.table.name.to_uppercase(),
        )?;
        lf = apply_join(lf, rhs, join)?;
    }

    Ok(lf)
}

fn apply_join(lf: LazyFrame, rhs: LazyFrame, join: &super::ast::Join) -> Result<LazyFrame> {
    let how = match join.kind {
        JoinKind::Inner => JoinType::Inner,
        JoinKind::Left => JoinType::Left,
        JoinKind::Right => JoinType::Right,
        JoinKind::Full => JoinType::Full,
        JoinKind::Cross => JoinType::Cross,
    };

    if matches!(join.kind, JoinKind::Cross) {
        let args = JoinArgs::new(JoinType::Cross);
        let mut out = lf.join(rhs, [] as [Expr; 0], [] as [Expr; 0], args);
        if let Some(on) = &join.on {
            let pred = sql_expr_to_polars(on, &Ctx::empty())?;
            out = out.filter(pred);
        }
        return Ok(out);
    }

    let Some(on) = &join.on else {
        return Err(SasError::runtime(
            "PROC SQL: this JOIN requires an ON clause.",
        ));
    };

    // Equi-join `a.k = b.k` : on extrait les colonnes de chaque côté. Tout
    // autre prédicat ON → cross join + filter (documenté).
    if let Some((lkey, rkey)) = as_equi_key(on) {
        let mut args = JoinArgs::new(how);
        args.join_nulls = true; // SAS apparie les missings entre eux.
        Ok(lf.join(rhs, [col(lkey)], [col(rkey)], args))
    } else {
        // ON non-equi : cross join puis filter.
        let pred = sql_expr_to_polars(on, &Ctx::empty())?;
        let args = JoinArgs::new(JoinType::Cross);
        Ok(lf.join(rhs, [] as [Expr; 0], [] as [Expr; 0], args).filter(pred))
    }
}

/// Si `on` est exactement `lhs = rhs` avec deux références de colonnes,
/// renvoie (nom_gauche, nom_droite).
fn as_equi_key(on: &SqlExpr) -> Option<(String, String)> {
    let SqlExpr::Binary { op, left, right } = on else {
        return None;
    };
    if *op != BinaryOp::Eq {
        return None;
    }
    let l = as_column_name(left)?;
    let r = as_column_name(right)?;
    Some((l, r))
}

fn as_column_name(e: &SqlExpr) -> Option<String> {
    match e {
        SqlExpr::Qualified { column, .. } => Some(column.clone()),
        SqlExpr::Base(SasExpr::Var(name)) => Some(name.clone()),
        _ => None,
    }
}

// ----------------------------------------------------------------------------
// 3. GROUP BY + agrégats
// ----------------------------------------------------------------------------

/// Résout un GROUP BY / ORDER BY positionnel : entier N → expression du
/// N-ième item du select-list (1-indexé).
fn resolve_positional<'a>(e: &'a SqlExpr, items: &'a [SelectItem]) -> Result<&'a SqlExpr> {
    if let SqlExpr::Base(SasExpr::Num(n)) = e {
        let idx = *n as usize;
        if *n >= 1.0 && idx <= items.len() && (*n - idx as f64).abs() < 1e-9 {
            return Ok(&items[idx - 1].expr);
        }
        return Err(SasError::runtime(format!(
            "PROC SQL: positional reference {n} is out of range."
        )));
    }
    Ok(e)
}

/// GROUP BY + agrégation + HAVING + projection finale, en une passe. Après
/// `group_by(keys).agg(aggs)`, la frame ne contient plus que les clés et les
/// colonnes agrégées (par leur nom de sortie). La projection finale et le
/// HAVING référencent donc ces colonnes par NOM (pas de ré-évaluation).
fn apply_group_by_project(query: &SelectStmt, lf: LazyFrame, ctx: &Ctx) -> Result<LazyFrame> {
    let mut keys: Vec<Expr> = Vec::new();
    for g in &query.group_by {
        let resolved = resolve_positional(g, &query.items)?;
        let name = group_key_output_name(resolved, query)?;
        keys.push(sql_expr_to_polars(resolved, ctx)?.alias(name));
    }

    // Inventaire des agrégats : chaque agrégat (du select-list ET du HAVING)
    // reçoit un nom de colonne. On déduplique par expression pour réutiliser
    // le même nom entre select-list et HAVING (ex. `count(*)`).
    let mut agg_exprs: Vec<Expr> = Vec::new();
    let mut agg_names: Vec<(SqlExpr, String)> = Vec::new();

    let mut intern = |sql: &SqlExpr, preferred: Option<String>| -> Result<String> {
        if let Some((_, n)) = agg_names.iter().find(|(e, _)| e == sql) {
            return Ok(n.clone());
        }
        let name = preferred.unwrap_or_else(|| format!("__agg_{}", agg_names.len()));
        agg_exprs.push(sql_expr_to_polars(sql, ctx)?.alias(name.clone()));
        agg_names.push((sql.clone(), name.clone()));
        Ok(name)
    };

    // Agrégats du select-list (nom de sortie préféré).
    for it in &query.items {
        for a in collect_aggregates(&it.expr) {
            let preferred = if &it.expr == a {
                Some(output_name(it, query)?)
            } else {
                None
            };
            intern(a, preferred)?;
        }
    }
    // Agrégats du HAVING.
    if let Some(h) = &query.having {
        for a in collect_aggregates(h) {
            intern(a, None)?;
        }
    }

    let mut out = lf.group_by(keys).agg(agg_exprs);

    // HAVING : référence les agrégats par leur colonne.
    if let Some(h) = &query.having {
        let pred = sql_expr_with_aggs(h, ctx, &agg_names)?;
        out = out.filter(pred);
    }

    // Projection finale : select-list, agrégats → col(nom).
    if query.items.len() == 1 && matches!(query.items[0].expr, SqlExpr::Star) {
        return Ok(out);
    }
    let mut proj: Vec<Expr> = Vec::new();
    for it in &query.items {
        let name = output_name(it, query)?;
        let e = sql_expr_with_aggs(&it.expr, ctx, &agg_names)?;
        proj.push(e.alias(name));
    }
    Ok(out.select(proj))
}

/// Collecte les nœuds Aggregate d'une expression (peu profonde).
fn collect_aggregates(e: &SqlExpr) -> Vec<&SqlExpr> {
    let mut out = Vec::new();
    fn rec<'a>(e: &'a SqlExpr, out: &mut Vec<&'a SqlExpr>) {
        match e {
            SqlExpr::Aggregate { .. } => out.push(e),
            SqlExpr::Binary { left, right, .. } => {
                rec(left, out);
                rec(right, out);
            }
            SqlExpr::Unary { expr, .. } => rec(expr, out),
            SqlExpr::Between {
                expr, low, high, ..
            } => {
                rec(expr, out);
                rec(low, out);
                rec(high, out);
            }
            SqlExpr::IsNull { expr, .. } => rec(expr, out),
            SqlExpr::Like { expr, .. } => rec(expr, out),
            _ => {}
        }
    }
    rec(e, &mut out);
    out
}

/// Traduit une expression en référençant les agrégats déjà calculés (par
/// nom de colonne) au lieu de les recalculer.
fn sql_expr_with_aggs(
    e: &SqlExpr,
    ctx: &Ctx,
    aggs: &[(SqlExpr, String)],
) -> Result<Expr> {
    if let SqlExpr::Aggregate { .. } = e {
        if let Some((_, name)) = aggs.iter().find(|(a, _)| a == e) {
            return Ok(col(name.clone()));
        }
    }
    match e {
        SqlExpr::Binary { op, left, right } => {
            let l = sql_expr_with_aggs(left, ctx, aggs)?;
            let r = sql_expr_with_aggs(right, ctx, aggs)?;
            Ok(apply_binop(*op, l, r))
        }
        SqlExpr::Unary { op, expr } => {
            let a = sql_expr_with_aggs(expr, ctx, aggs)?;
            Ok(match op {
                UnaryOp::Minus => lit(0.0) - a,
                UnaryOp::Plus => a,
                UnaryOp::Not => a.not(),
            })
        }
        SqlExpr::Between {
            expr,
            low,
            high,
            negated,
        } => {
            let a = sql_expr_with_aggs(expr, ctx, aggs)?;
            let lo = sql_expr_with_aggs(low, ctx, aggs)?;
            let hi = sql_expr_with_aggs(high, ctx, aggs)?;
            let between = a.clone().gt_eq(lo).and(a.lt_eq(hi));
            Ok(if *negated { between.not() } else { between })
        }
        SqlExpr::IsNull { expr, negated } => {
            let a = sql_expr_with_aggs(expr, ctx, aggs)?;
            Ok(if *negated { a.is_not_null() } else { a.is_null() })
        }
        // Pas d'agrégat à l'intérieur : traduction normale.
        _ => sql_expr_to_polars(e, ctx),
    }
}

/// Nom de sortie d'une clé de group-by (utilisé pour aligner avec le
/// select-list lors de la projection finale). Pour une simple colonne, c'est
/// le nom de la colonne.
fn group_key_output_name(e: &SqlExpr, query: &SelectStmt) -> Result<String> {
    match e {
        SqlExpr::Base(SasExpr::Var(name)) => Ok(name.clone()),
        SqlExpr::Qualified { column, .. } => Ok(column.clone()),
        _ => Ok(format!("__grpkey_{}", query.group_by.len())),
    }
}

// ----------------------------------------------------------------------------
// 5. REMERGE
// ----------------------------------------------------------------------------

fn apply_remerge(
    query: &SelectStmt,
    lf: LazyFrame,
    ctx: &Ctx,
    session: &mut Session,
) -> Result<LazyFrame> {
    // Total général : un seul groupe. On calcule chaque agrégat en une frame
    // d'une ligne (nommée par un nom interne), puis cross join à toutes les
    // lignes d'origine. La projection finale référence l'agrégat par sa
    // colonne (et les colonnes nues telles quelles).
    let mut agg_exprs: Vec<Expr> = Vec::new();
    let mut agg_names: Vec<(SqlExpr, String)> = Vec::new();
    for it in &query.items {
        for a in collect_aggregates(&it.expr) {
            if agg_names.iter().any(|(e, _)| e == a) {
                continue;
            }
            let name = format!("__agg_{}", agg_names.len());
            agg_exprs.push(sql_expr_to_polars(a, ctx)?.alias(name.clone()));
            agg_names.push((a.clone(), name));
        }
    }
    let totals = lf.clone().select(agg_exprs);
    let merged = lf.join(
        totals,
        [] as [Expr; 0],
        [] as [Expr; 0],
        JoinArgs::new(JoinType::Cross),
    );

    // Projection finale.
    if query.items.len() == 1 && matches!(query.items[0].expr, SqlExpr::Star) {
        return Ok(merged);
    }
    let mut proj: Vec<Expr> = Vec::new();
    for it in &query.items {
        match &it.expr {
            SqlExpr::Star | SqlExpr::QualifiedStar(_) => proj.push(col("*")),
            _ => {
                let name = output_name(it, query)?;
                proj.push(sql_expr_with_aggs(&it.expr, ctx, &agg_names)?.alias(name));
            }
        }
    }
    Ok(merged.select(proj))
}

// ----------------------------------------------------------------------------
// 6. SELECT list / aliases / CALCULATED
// ----------------------------------------------------------------------------

fn project_select_list(
    query: &SelectStmt,
    mut lf: LazyFrame,
    ctx: &Ctx,
    _session: &mut Session,
) -> Result<LazyFrame> {
    // `*` seul → pas de projection (toutes les colonnes).
    if query.items.len() == 1 && matches!(query.items[0].expr, SqlExpr::Star) {
        return Ok(lf);
    }

    let mut exprs: Vec<Expr> = Vec::new();
    for it in &query.items {
        match &it.expr {
            SqlExpr::Star => {
                exprs.push(col("*"));
            }
            SqlExpr::QualifiedStar(_) => {
                // Espace de noms plat : `alias.*` ≈ toutes les colonnes
                // (les colonnes des autres tables ont des noms distincts).
                exprs.push(col("*"));
            }
            _ => {
                let name = output_name(it, query)?;
                exprs.push(sql_expr_to_polars(&it.expr, ctx)?.alias(name));
            }
        }
    }
    lf = lf.select(exprs);
    Ok(lf)
}

/// Nom de sortie d'un item : alias explicite, sinon nom de colonne nue,
/// sinon nom dérivé de l'agrégat / expression.
fn output_name(it: &SelectItem, _query: &SelectStmt) -> Result<String> {
    if let Some(a) = &it.alias {
        return Ok(a.clone());
    }
    match &it.expr {
        SqlExpr::Base(SasExpr::Var(name)) => Ok(name.clone()),
        SqlExpr::Qualified { column, .. } => Ok(column.clone()),
        SqlExpr::Aggregate { func, arg, star, .. } => {
            // COUNT(*) → _TEMA001 façon SAS ; on garde un nom simple.
            if *star {
                Ok(func.to_ascii_uppercase())
            } else if let Some(a) = arg {
                match a.as_ref() {
                    SqlExpr::Base(SasExpr::Var(v)) => Ok(format!("_{}", func.to_ascii_uppercase())),
                    SqlExpr::Qualified { column, .. } => {
                        Ok(format!("_{}", func.to_ascii_uppercase()))
                    }
                    _ => Ok(func.to_ascii_uppercase()),
                }
            } else {
                Ok(func.to_ascii_uppercase())
            }
        }
        _ => Ok("_col".to_string()),
    }
}

// ----------------------------------------------------------------------------
// 8. ORDER BY
// ----------------------------------------------------------------------------

/// Si une clé d'ORDER BY (après projection) désigne une colonne de SORTIE
/// existante (par nom de colonne nue ou alias), renvoie ce nom.
fn order_output_name(e: &SqlExpr, query: &SelectStmt) -> Option<String> {
    let target = match e {
        SqlExpr::Base(SasExpr::Var(name)) => name.clone(),
        SqlExpr::Qualified { column, .. } => column.clone(),
        SqlExpr::Calculated(name) => name.clone(),
        _ => return None,
    };
    for it in &query.items {
        if let Ok(n) = output_name(it, query) {
            if n.eq_ignore_ascii_case(&target) {
                return Some(n);
            }
        }
    }
    None
}

fn apply_order_by(
    query: &SelectStmt,
    lf: LazyFrame,
    ctx: &Ctx,
    pre_projection: bool,
) -> Result<LazyFrame> {
    let mut by: Vec<Expr> = Vec::new();
    let mut desc: Vec<bool> = Vec::new();
    for (e, d) in &query.order_by {
        // Référence positionnelle `order by N` : la N-ième colonne de sortie.
        if let SqlExpr::Base(SasExpr::Num(n)) = e {
            let idx = *n as usize;
            if *n >= 1.0 && idx <= query.items.len() && (*n - idx as f64).abs() < 1e-9 {
                if pre_projection {
                    // Avant projection : trier sur l'EXPRESSION source.
                    by.push(sql_expr_to_polars(&query.items[idx - 1].expr, ctx)?);
                } else {
                    let name = output_name(&query.items[idx - 1], query)?;
                    by.push(col(name));
                }
                desc.push(*d);
                continue;
            }
            return Err(SasError::runtime(format!(
                "PROC SQL: ORDER BY position {n} is out of range."
            )));
        }
        // Référence par alias de sortie (après projection) : col(alias).
        if !pre_projection {
            if let Some(name) = order_output_name(e, query) {
                by.push(col(name));
                desc.push(*d);
                continue;
            }
        }
        by.push(sql_expr_to_polars(e, ctx)?);
        desc.push(*d);
    }
    // ORDER BY SAS : missings EN PREMIER (nulls first), tri STABLE.
    let opts = SortMultipleOptions::default()
        .with_order_descending_multi(desc)
        .with_nulls_last(false)
        .with_maintain_order(true);
    Ok(lf.sort_by_exprs(by, opts))
}

// ----------------------------------------------------------------------------
// 9. SET OPS
// ----------------------------------------------------------------------------

fn apply_set_op(lhs: LazyFrame, rhs: LazyFrame, op: &SetOp, all: bool) -> Result<LazyFrame> {
    match op {
        SetOp::Union => {
            let out = concat([lhs, rhs], UnionArgs::default())?;
            if all {
                Ok(out)
            } else {
                Ok(out.unique(None, UniqueKeepStrategy::Any))
            }
        }
        SetOp::Except => {
            // Anti-join sur toutes les colonnes du lhs.
            let on = lhs_columns(&lhs)?;
            let mut args = JoinArgs::new(JoinType::Anti);
            args.join_nulls = true;
            let on_l: Vec<Expr> = on.iter().map(|c| col(c.clone())).collect();
            let out = lhs.join(rhs, &on_l, &on_l, args);
            if all {
                Ok(out)
            } else {
                Ok(out.unique(None, UniqueKeepStrategy::Any))
            }
        }
        SetOp::Intersect => {
            let on = lhs_columns(&lhs)?;
            let mut args = JoinArgs::new(JoinType::Semi);
            args.join_nulls = true;
            let on_l: Vec<Expr> = on.iter().map(|c| col(c.clone())).collect();
            let out = lhs.join(rhs, &on_l, &on_l, args);
            if all {
                Ok(out)
            } else {
                Ok(out.unique(None, UniqueKeepStrategy::Any))
            }
        }
    }
}

fn lhs_columns(lf: &LazyFrame) -> Result<Vec<String>> {
    let mut lf = lf.clone();
    let schema = lf.collect_schema()?;
    Ok(schema.iter_names().map(|n| n.to_string()).collect())
}

// ----------------------------------------------------------------------------
// Helpers sur les agrégats
// ----------------------------------------------------------------------------

fn item_has_aggregate(e: &SqlExpr) -> bool {
    match e {
        SqlExpr::Aggregate { .. } => true,
        SqlExpr::Binary { left, right, .. } => {
            item_has_aggregate(left) || item_has_aggregate(right)
        }
        SqlExpr::Unary { expr, .. } => item_has_aggregate(expr),
        SqlExpr::Between {
            expr, low, high, ..
        } => item_has_aggregate(expr) || item_has_aggregate(low) || item_has_aggregate(high),
        SqlExpr::IsNull { expr, .. } => item_has_aggregate(expr),
        SqlExpr::Like { expr, .. } => item_has_aggregate(expr),
        SqlExpr::Calculated(_)
        | SqlExpr::Base(_)
        | SqlExpr::Star
        | SqlExpr::QualifiedStar(_)
        | SqlExpr::Qualified { .. } => false,
    }
}

/// Vrai si CHAQUE item du select-list est soit un agrégat, soit une clé du
/// GROUP BY (cas standard sans remerge).
fn all_items_aggregated(query: &SelectStmt) -> bool {
    let group_cols: Vec<String> = query
        .group_by
        .iter()
        .filter_map(|g| as_column_name(g))
        .collect();
    query.items.iter().all(|it| {
        if item_has_aggregate(&it.expr) {
            return true;
        }
        match &it.expr {
            SqlExpr::Base(SasExpr::Var(name)) => {
                group_cols.iter().any(|g| g.eq_ignore_ascii_case(name))
            }
            SqlExpr::Qualified { column, .. } => {
                group_cols.iter().any(|g| g.eq_ignore_ascii_case(column))
            }
            SqlExpr::Star | SqlExpr::QualifiedStar(_) => false,
            // Constantes/expressions sans colonne nue : OK.
            _ => !references_bare_column(&it.expr),
        }
    })
}

fn references_bare_column(e: &SqlExpr) -> bool {
    match e {
        SqlExpr::Base(SasExpr::Var(_)) | SqlExpr::Qualified { .. } => true,
        SqlExpr::Base(_) => false,
        SqlExpr::Aggregate { .. } => false,
        SqlExpr::Binary { left, right, .. } => {
            references_bare_column(left) || references_bare_column(right)
        }
        SqlExpr::Unary { expr, .. } => references_bare_column(expr),
        SqlExpr::Between {
            expr, low, high, ..
        } => {
            references_bare_column(expr)
                || references_bare_column(low)
                || references_bare_column(high)
        }
        SqlExpr::IsNull { expr, .. } => references_bare_column(expr),
        SqlExpr::Like { expr, .. } => references_bare_column(expr),
        SqlExpr::Calculated(_) => false,
        SqlExpr::Star | SqlExpr::QualifiedStar(_) => false,
    }
}

// ----------------------------------------------------------------------------
// Traducteur SqlExpr → polars::prelude::Expr
// ----------------------------------------------------------------------------

fn sql_expr_to_polars(e: &SqlExpr, ctx: &Ctx) -> Result<Expr> {
    match e {
        SqlExpr::Base(b) => base_expr_to_polars(b, ctx),
        SqlExpr::Star => Err(SasError::runtime(
            "PROC SQL: '*' is only valid in a select-list.",
        )),
        SqlExpr::QualifiedStar(_) => Err(SasError::runtime(
            "PROC SQL: 'table.*' is only valid in a select-list.",
        )),
        SqlExpr::Qualified { column, .. } => Ok(col(column.clone())),
        SqlExpr::Calculated(name) => {
            let key = name.to_ascii_lowercase();
            let target = ctx
                .aliases
                .iter()
                .find(|(a, _)| *a == key)
                .map(|(_, ex)| ex.clone())
                .ok_or_else(|| {
                    SasError::runtime(format!(
                        "PROC SQL: CALCULATED {} refers to an unknown column.",
                        name.to_uppercase()
                    ))
                })?;
            sql_expr_to_polars(&target, ctx)
        }
        SqlExpr::Aggregate {
            func,
            distinct,
            arg,
            star,
        } => aggregate_to_polars(func, *distinct, arg.as_deref(), *star, ctx),
        SqlExpr::Between {
            expr,
            low,
            high,
            negated,
        } => {
            let a = sql_expr_to_polars(expr, ctx)?;
            let lo = sql_expr_to_polars(low, ctx)?;
            let hi = sql_expr_to_polars(high, ctx)?;
            let between = a.clone().gt_eq(lo).and(a.lt_eq(hi));
            Ok(if *negated { between.not() } else { between })
        }
        SqlExpr::IsNull { expr, negated } => {
            let a = sql_expr_to_polars(expr, ctx)?;
            Ok(if *negated { a.is_not_null() } else { a.is_null() })
        }
        SqlExpr::Like {
            expr,
            pattern,
            negated,
        } => {
            let a = sql_expr_to_polars(expr, ctx)?;
            let m = like_to_match(a, pattern)?;
            Ok(if *negated { m.not() } else { m })
        }
        SqlExpr::Binary { op, left, right } => binary_to_polars(*op, left, right, ctx),
        SqlExpr::Unary { op, expr } => {
            let a = sql_expr_to_polars(expr, ctx)?;
            Ok(match op {
                UnaryOp::Minus => lit(0.0) - a,
                UnaryOp::Plus => a,
                UnaryOp::Not => a.not(),
            })
        }
    }
}

/// Traduit un `Expr` (feuille « base ») en Polars.
fn base_expr_to_polars(e: &SasExpr, ctx: &Ctx) -> Result<Expr> {
    match e {
        SasExpr::Num(n) => Ok(lit(*n)),
        SasExpr::Str(s) => Ok(lit(s.clone())),
        SasExpr::Missing(MissingKind::Dot) => Ok(lit(NULL)),
        // Tout missing en tant que littéral → null (les spéciaux sont
        // déjà normalisés en null sur les colonnes).
        SasExpr::Missing(_) => Ok(lit(NULL)),
        SasExpr::Var(name) => Ok(col(name.clone())),
        SasExpr::Binary { op, left, right } => {
            base_binary_to_polars(*op, left, right, ctx)
        }
        SasExpr::Unary { op, expr } => {
            let a = base_expr_to_polars(expr, ctx)?;
            Ok(match op {
                UnaryOp::Minus => lit(0.0) - a,
                UnaryOp::Plus => a,
                UnaryOp::Not => a.not(),
            })
        }
        SasExpr::In { expr, list } => {
            let a = base_expr_to_polars(expr, ctx)?;
            let items: Vec<Expr> = list
                .iter()
                .map(|x| base_expr_to_polars(x, ctx))
                .collect::<Result<_>>()?;
            Ok(a.is_in(concat_list(items)?))
        }
        SasExpr::Call { name, .. } => Err(SasError::runtime(format!(
            "PROC SQL: function {}() is not supported yet.",
            name.to_uppercase()
        ))),
        SasExpr::Index { name, .. } => Err(SasError::runtime(format!(
            "PROC SQL: array reference {} is not supported in SQL.",
            name.to_uppercase()
        ))),
        SasExpr::HashMethod(call) => Err(SasError::runtime(format!(
            "PROC SQL: hash method call on {} is not supported in SQL.",
            call.object.to_uppercase()
        ))),
    }
}

/// Comparaison `a = .` / `a <> .` → is_null / is_not_null.
fn is_missing_literal(e: &SasExpr) -> bool {
    matches!(e, SasExpr::Missing(_))
}

fn base_binary_to_polars(op: BinaryOp, left: &SasExpr, right: &SasExpr, ctx: &Ctx) -> Result<Expr> {
    // Egalité/inégalité contre un littéral missing.
    if matches!(op, BinaryOp::Eq) {
        if is_missing_literal(right) {
            return Ok(base_expr_to_polars(left, ctx)?.is_null());
        }
        if is_missing_literal(left) {
            return Ok(base_expr_to_polars(right, ctx)?.is_null());
        }
    }
    if matches!(op, BinaryOp::Ne) {
        if is_missing_literal(right) {
            return Ok(base_expr_to_polars(left, ctx)?.is_not_null());
        }
        if is_missing_literal(left) {
            return Ok(base_expr_to_polars(right, ctx)?.is_not_null());
        }
    }
    let l = base_expr_to_polars(left, ctx)?;
    let r = base_expr_to_polars(right, ctx)?;
    Ok(apply_binop(op, l, r))
}

fn binary_to_polars(op: BinaryOp, left: &SqlExpr, right: &SqlExpr, ctx: &Ctx) -> Result<Expr> {
    // Missing literal en SqlExpr::Base.
    if matches!(op, BinaryOp::Eq | BinaryOp::Ne) {
        let l_missing = matches!(left, SqlExpr::Base(b) if is_missing_literal(b));
        let r_missing = matches!(right, SqlExpr::Base(b) if is_missing_literal(b));
        if r_missing {
            let a = sql_expr_to_polars(left, ctx)?;
            return Ok(if op == BinaryOp::Eq {
                a.is_null()
            } else {
                a.is_not_null()
            });
        }
        if l_missing {
            let a = sql_expr_to_polars(right, ctx)?;
            return Ok(if op == BinaryOp::Eq {
                a.is_null()
            } else {
                a.is_not_null()
            });
        }
    }
    let l = sql_expr_to_polars(left, ctx)?;
    let r = sql_expr_to_polars(right, ctx)?;
    Ok(apply_binop(op, l, r))
}

fn apply_binop(op: BinaryOp, l: Expr, r: Expr) -> Expr {
    match op {
        BinaryOp::Add => l + r,
        BinaryOp::Sub => l - r,
        BinaryOp::Mul => l * r,
        BinaryOp::Div => l / r,
        BinaryOp::Power => l.pow(r),
        BinaryOp::Concat => l.cast(DataType::String) + r.cast(DataType::String),
        BinaryOp::Lt => l.lt(r),
        BinaryOp::Le => l.lt_eq(r),
        BinaryOp::Gt => l.gt(r),
        BinaryOp::Ge => l.gt_eq(r),
        BinaryOp::Eq => l.eq(r),
        BinaryOp::Ne => l.neq(r),
        BinaryOp::And => l.and(r),
        BinaryOp::Or => l.or(r),
    }
}

fn aggregate_to_polars(
    func: &str,
    distinct: bool,
    arg: Option<&SqlExpr>,
    star: bool,
    ctx: &Ctx,
) -> Result<Expr> {
    let f = func.to_ascii_lowercase();
    match f.as_str() {
        "count" => {
            if star || arg.is_none() {
                Ok(len())
            } else {
                let a = sql_expr_to_polars(arg.unwrap(), ctx)?;
                if distinct {
                    Ok(a.n_unique())
                } else {
                    Ok(a.count())
                }
            }
        }
        "sum" | "avg" | "mean" | "min" | "max" => {
            let arg = arg.ok_or_else(|| {
                SasError::runtime(format!(
                    "PROC SQL: aggregate {}() requires an argument.",
                    func.to_uppercase()
                ))
            })?;
            let a = sql_expr_to_polars(arg, ctx)?;
            let a = if distinct { a.unique() } else { a };
            Ok(match f.as_str() {
                "sum" => a.sum(),
                "avg" | "mean" => a.mean(),
                "min" => a.min(),
                "max" => a.max(),
                _ => unreachable!(),
            })
        }
        other => Err(SasError::runtime(format!(
            "PROC SQL: aggregate function {}() is not supported.",
            other.to_uppercase()
        ))),
    }
}

/// Traduit un prédicat SQL `expr LIKE pattern` en expression Polars.
///
/// Sémantique SAS du LIKE (cf. SAS SQL) :
///   - `%`  : correspond à zéro caractère ou plus,
///   - `_`  : correspond à exactement un caractère,
///   - tout autre caractère se compare littéralement,
///   - la comparaison est **sensible à la casse** (contrairement à `=` SAS
///     qui l'est aussi sur les char ; SAS ne fait PAS de upcase ici),
///   - une valeur missing (null) ne matche jamais → résultat null/false.
///
/// On n'utilise PAS la feature `regex` de Polars (non activée). Pour couvrir
/// l'intégralité des motifs (y compris `_`, les `%` internes et la forme
/// substring `%abc%`), on optimise les cas courants en primitives Polars
/// (`eq` / `starts_with` / `ends_with` / `contains_literal`) et on retombe sur
/// un matcher SAS maison appliqué via `Expr::map` pour les cas généraux.
fn like_to_match(a: Expr, pattern: &str) -> Result<Expr> {
    // Cas spéciaux purement composés de jokers `%` → tout non-missing matche.
    // (`%`, `%%`, ... = "zéro ou plus" répété = "n'importe quoi".)
    if !pattern.is_empty() && pattern.chars().all(|c| c == '%') {
        return Ok(a.clone().is_not_null());
    }

    // Optimisations : motifs sans `_` et sans plusieurs `%` internes.
    // On les traduit en primitives Polars natives (plus rapides, vectorisées).
    // Pour la forme `%abc%`, on retombe sur le matcher maison pour éviter
    // les dépendances regex.
    if !pattern.contains('_') {
        let leading = pattern.starts_with('%');
        let trailing = pattern.ends_with('%');
        let core = pattern.trim_matches('%');
        if !core.contains('%') && (leading, trailing) != (true, true) {
            let core = core.to_string();
            return Ok(match (leading, trailing) {
                // Pas de joker du tout → égalité exacte.
                (false, false) => a.eq(lit(core)),
                // `abc%` → commence par "abc".
                (false, true) => a.str().starts_with(lit(core)),
                // `%abc` → finit par "abc".
                (true, false) => a.str().ends_with(lit(core)),
                // `%abc%` → gérée par le matcher maison ci-dessous.
                (true, true) => unreachable!(),
            });
        }
    }

    // Cas général (joker `_`, ou plusieurs `%` internes) : matcher SAS maison
    // appliqué élément par élément via une UDF Polars renvoyant un booléen.
    let pat = pattern.to_string();
    Ok(a.map(
        move |col: Column| {
            let s = col.str()?;
            let out: BooleanChunked = s
                .iter()
                .map(|opt| opt.map(|v| sas_like_match(v, &pat)))
                .collect();
            Ok(Some(out.into_column()))
        },
        GetOutput::from_type(DataType::Boolean),
    ))
}

/// Matcher SAS `LIKE` pour une seule valeur (sensible à la casse) :
/// `%` = 0+ caractères, `_` = exactement 1 caractère, le reste littéral.
/// Implémentation par backtracking glob classique (sur les `char`, pour gérer
/// l'UTF-8 correctement).
fn sas_like_match(text: &str, pattern: &str) -> bool {
    let t: Vec<char> = text.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    // i : index dans le texte, j : index dans le motif.
    let (mut i, mut j) = (0usize, 0usize);
    // Dernier `%` rencontré et position du texte au moment de ce `%` : permet
    // le backtracking (avancer d'un caractère dans le texte si la suite échoue).
    let mut star_j: Option<usize> = None;
    let mut star_i = 0usize;
    while i < t.len() {
        if j < p.len() && (p[j] == t[i] || p[j] == '_') {
            i += 1;
            j += 1;
        } else if j < p.len() && p[j] == '%' {
            star_j = Some(j);
            star_i = i;
            j += 1;
        } else if let Some(sj) = star_j {
            // Échec : le dernier `%` absorbe un caractère de plus.
            j = sj + 1;
            star_i += 1;
            i = star_i;
        } else {
            return false;
        }
    }
    // Texte épuisé : le reste du motif doit être uniquement des `%`.
    while j < p.len() && p[j] == '%' {
        j += 1;
    }
    j == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::missing::encode_special;
    use crate::parser::StatementStream;
    use crate::session::Session;
    use crate::source::SourceFile;
    use crate::sql::ast::{SelectStmt, SqlStmt};
    use crate::sql::parser::parse_sql_program;
    use crate::value::{MissingKind, VarType};
    use polars::df;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn first_select(src: &str) -> SelectStmt {
        let file = SourceFile::new(src);
        let mut ts = StatementStream::new(&file).unwrap();
        let prog = parse_sql_program(&mut ts).unwrap();
        match prog.stmts.into_iter().next().unwrap() {
            SqlStmt::Select(s) => s,
            other => panic!("expected SELECT, got {other:?}"),
        }
    }

    fn run(src: &str, session: &mut Session) -> DataFrame {
        let sel = first_select(src);
        lower_select(&sel, session).unwrap().collect().unwrap()
    }

    /// Écrit une table dans WORK.
    fn write_table(session: &mut Session, name: &str, df: DataFrame, vars: Vec<VarMeta>) {
        let ds = SasDataset { df, vars };
        session.libs.get("WORK").unwrap().write(name, &ds).unwrap();
    }

    fn num(name: &str) -> VarMeta {
        VarMeta {
            name: name.into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        }
    }
    fn chr(name: &str, len: usize) -> VarMeta {
        VarMeta {
            name: name.into(),
            ty: VarType::Char,
            length: len,
            format: None,
            label: None,
        }
    }

    fn write_people(session: &mut Session) {
        let df = df![
            "name" => ["Al", "Bo", "Cy", "Di"],
            "sex"  => ["M", "M", "F", "F"],
            "age"  => [10.0_f64, 14.0, 13.0, 11.0],
            "height" => [50.0_f64, 60.0, 55.0, 52.0],
        ]
        .unwrap();
        write_table(
            session,
            "T",
            df,
            vec![chr("name", 8), chr("sex", 1), num("age"), num("height")],
        );
    }

    #[test]
    fn where_filter_numeric() {
        let mut s = make_session();
        write_people(&mut s);
        let df = run("select name, age from t where age > 12;", &mut s);
        assert_eq!(df.height(), 2);
        let ages: Vec<f64> = df.column("age").unwrap().f64().unwrap().into_no_null_iter().collect();
        assert_eq!(ages, vec![14.0, 13.0]);
    }

    #[test]
    fn where_equals_missing_is_null() {
        let mut s = make_session();
        let df = df![
            "x" => [Some(1.0_f64), None, Some(3.0)],
            "y" => [10.0_f64, 20.0, 30.0],
        ]
        .unwrap();
        write_table(&mut s, "T", df, vec![num("x"), num("y")]);
        let out = run("select y from t where x = .;", &mut s);
        assert_eq!(out.height(), 1);
        let ys: Vec<f64> = out.column("y").unwrap().f64().unwrap().into_no_null_iter().collect();
        assert_eq!(ys, vec![20.0]);
    }

    #[test]
    fn where_special_missing_normalized_to_null() {
        // Une colonne contient un missing spécial (.A) : `x = .` doit le
        // capturer (normalisation NaN-payload → null avant comparaison).
        let mut s = make_session();
        let df = df![
            "x" => [Some(1.0_f64), Some(encode_special(MissingKind::Letter(0))), Some(3.0)],
            "y" => [10.0_f64, 20.0, 30.0],
        ]
        .unwrap();
        write_table(&mut s, "T", df, vec![num("x"), num("y")]);
        let out = run("select y from t where x = .;", &mut s);
        let ys: Vec<f64> = out.column("y").unwrap().f64().unwrap().into_no_null_iter().collect();
        assert_eq!(ys, vec![20.0]);
    }

    #[test]
    fn group_by_aggregates() {
        let mut s = make_session();
        write_people(&mut s);
        let out = run(
            "select sex, count(*) as n, avg(height) as a from t group by sex;",
            &mut s,
        );
        assert_eq!(out.height(), 2);
        // Vérifie les valeurs par sexe.
        let sexes: Vec<String> = out
            .column("sex")
            .unwrap()
            .str()
            .unwrap()
            .iter()
            .map(|o| o.unwrap().to_string())
            .collect();
        let ns: Vec<u32> = out
            .column("n")
            .unwrap()
            .u32()
            .unwrap()
            .into_no_null_iter()
            .collect();
        // Chaque groupe a 2 lignes.
        for (i, sx) in sexes.iter().enumerate() {
            assert_eq!(ns[i], 2, "sex {sx}");
        }
        // avg(height) : F = (55+52)/2 = 53.5 ; M = (50+60)/2 = 55.
        let avgs: Vec<f64> = out
            .column("a")
            .unwrap()
            .f64()
            .unwrap()
            .into_no_null_iter()
            .collect();
        for (i, sx) in sexes.iter().enumerate() {
            if sx == "F" {
                assert!((avgs[i] - 53.5).abs() < 1e-9);
            } else {
                assert!((avgs[i] - 55.0).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn remerge_grand_total_and_note() {
        let mut s = make_session();
        write_people(&mut s);
        let out = run("select name, max(height) as mx from t;", &mut s);
        // Une ligne par observation d'origine, mx constant = 60.
        assert_eq!(out.height(), 4);
        let mxs: Vec<f64> = out
            .column("mx")
            .unwrap()
            .f64()
            .unwrap()
            .into_no_null_iter()
            .collect();
        assert!(mxs.iter().all(|v| (*v - 60.0).abs() < 1e-9));
        let log = s.log.into_string();
        assert!(
            log.contains("The query requires remerging summary statistics back with the original data."),
            "log: {log}"
        );
    }

    #[test]
    fn order_by_missing_first() {
        let mut s = make_session();
        let df = df![
            "x" => [Some(3.0_f64), None, Some(1.0), Some(2.0)],
        ]
        .unwrap();
        write_table(&mut s, "T", df, vec![num("x")]);
        let out = run("select x from t order by x;", &mut s);
        let col = out.column("x").unwrap().f64().unwrap();
        // null en premier, puis 1, 2, 3.
        assert_eq!(col.get(0), None);
        assert_eq!(col.get(1), Some(1.0));
        assert_eq!(col.get(2), Some(2.0));
        assert_eq!(col.get(3), Some(3.0));
    }

    #[test]
    fn order_by_descending() {
        let mut s = make_session();
        write_people(&mut s);
        let out = run("select age from t order by age desc;", &mut s);
        let ages: Vec<f64> = out.column("age").unwrap().f64().unwrap().into_no_null_iter().collect();
        assert_eq!(ages, vec![14.0, 13.0, 11.0, 10.0]);
    }

    #[test]
    fn join_with_missing_key_matches() {
        // join_nulls(true) : les clés missing s'apparient.
        let mut s = make_session();
        let left = df![
            "k" => [Some(1.0_f64), None, Some(2.0)],
            "a" => [10.0_f64, 20.0, 30.0],
        ]
        .unwrap();
        write_table(&mut s, "L", left, vec![num("k"), num("a")]);
        let right = df![
            "k" => [Some(1.0_f64), None],
            "b" => [100.0_f64, 200.0],
        ]
        .unwrap();
        write_table(&mut s, "R", right, vec![num("k"), num("b")]);
        let out = run(
            "select l.a, r.b from l inner join r on l.k = r.k;",
            &mut s,
        );
        // k=1 (a=10,b=100) et k=null (a=20,b=200) → 2 lignes.
        assert_eq!(out.height(), 2);
        let bs: Vec<f64> = out.column("b").unwrap().f64().unwrap().into_no_null_iter().collect();
        assert!(bs.contains(&100.0) && bs.contains(&200.0));
    }

    #[test]
    fn distinct_dedups_rows() {
        let mut s = make_session();
        let df = df![
            "x" => [1.0_f64, 1.0, 2.0, 2.0, 2.0],
        ]
        .unwrap();
        write_table(&mut s, "T", df, vec![num("x")]);
        let out = run("select distinct x from t;", &mut s);
        assert_eq!(out.height(), 2);
    }

    #[test]
    fn select_star() {
        let mut s = make_session();
        write_people(&mut s);
        let out = run("select * from t;", &mut s);
        assert_eq!(out.width(), 4);
        assert_eq!(out.height(), 4);
    }

    #[test]
    fn calculated_reexpands_alias() {
        let mut s = make_session();
        write_people(&mut s);
        // bmi-like : alias `dbl` = age*2, puis CALCULATED dbl + 1.
        let out = run(
            "select age*2 as dbl, calculated dbl + 1 as plus from t order by age;",
            &mut s,
        );
        let dbl: Vec<f64> = out.column("dbl").unwrap().f64().unwrap().into_no_null_iter().collect();
        let plus: Vec<f64> = out.column("plus").unwrap().f64().unwrap().into_no_null_iter().collect();
        for (d, p) in dbl.iter().zip(plus.iter()) {
            assert!((p - (d + 1.0)).abs() < 1e-9);
        }
    }

    #[test]
    fn union_all_and_distinct() {
        let mut s = make_session();
        let a = df!["x" => [1.0_f64, 2.0]].unwrap();
        let b = df!["x" => [2.0_f64, 3.0]].unwrap();
        write_table(&mut s, "A", a, vec![num("x")]);
        write_table(&mut s, "B", b, vec![num("x")]);
        let all = run("select x from a union all select x from b;", &mut s);
        assert_eq!(all.height(), 4);
        let uniq = run("select x from a union select x from b;", &mut s);
        assert_eq!(uniq.height(), 3);
    }

    #[test]
    fn like_pattern_match() {
        let mut s = make_session();
        let df = df![
            "name" => ["Alice", "Bob", "Albert", "Carol"],
        ]
        .unwrap();
        write_table(&mut s, "T", df, vec![chr("name", 8)]);
        let out = run("select name from t where name like 'Al%';", &mut s);
        let names: Vec<String> = out
            .column("name")
            .unwrap()
            .str()
            .unwrap()
            .iter()
            .map(|o| o.unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["Alice".to_string(), "Albert".to_string()]);
    }

    #[test]
    fn having_filters_groups() {
        let mut s = make_session();
        write_people(&mut s);
        let out = run(
            "select sex, count(*) as n from t group by sex having count(*) > 1;",
            &mut s,
        );
        // Les deux groupes ont 2 → tous passent.
        assert_eq!(out.height(), 2);
    }

    #[test]
    fn between_filter() {
        let mut s = make_session();
        write_people(&mut s);
        let out = run("select name from t where age between 11 and 13;", &mut s);
        assert_eq!(out.height(), 2);
    }
}
