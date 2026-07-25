use super::*;

// ----------------------------------------------------------------------------
// 3. GROUP BY + agrégats
// ----------------------------------------------------------------------------

/// Résout un GROUP BY / ORDER BY positionnel : entier N → expression du
/// N-ième item du select-list (1-indexé).
pub(super) fn resolve_positional<'a>(e: &'a SqlExpr, items: &'a [SelectItem]) -> Result<&'a SqlExpr> {
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
pub(super) fn apply_group_by_project(query: &SelectStmt, lf: LazyFrame, ctx: &Ctx) -> Result<LazyFrame> {
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

    // Sans clé de GROUP BY (agrégation sur toute la table → une seule ligne),
    // `group_by([])` est invalide pour Polars : on projette directement les
    // agrégats. C'est le cas d'une sous-requête scalaire `(select avg(x) ...)`.
    let mut out = if keys.is_empty() {
        lf.select(agg_exprs)
    } else {
        lf.group_by(keys).agg(agg_exprs)
    };

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
pub(super) fn collect_aggregates(e: &SqlExpr) -> Vec<&SqlExpr> {
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
pub(super) fn sql_expr_with_aggs(
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
pub(super) fn group_key_output_name(e: &SqlExpr, query: &SelectStmt) -> Result<String> {
    match e {
        SqlExpr::Base(SasExpr::Var(name)) => Ok(name.clone()),
        SqlExpr::Qualified { column, .. } => Ok(column.clone()),
        _ => Ok(format!("__grpkey_{}", query.group_by.len())),
    }
}

// ----------------------------------------------------------------------------
// 5. REMERGE
// ----------------------------------------------------------------------------

pub(super) fn apply_remerge(
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
// Helpers sur les agrégats
// ----------------------------------------------------------------------------

pub(super) fn item_has_aggregate(e: &SqlExpr) -> bool {
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
        // Résolues en littéraux avant l'abaissement.
        SqlExpr::Subquery(_) | SqlExpr::InSubquery { .. } | SqlExpr::Exists { .. } => false,
    }
}

/// Vrai si CHAQUE item du select-list est soit un agrégat, soit une clé du
/// GROUP BY (cas standard sans remerge).
pub(super) fn all_items_aggregated(query: &SelectStmt) -> bool {
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

pub(super) fn references_bare_column(e: &SqlExpr) -> bool {
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
        // Résolues en littéraux avant l'abaissement.
        SqlExpr::Subquery(_) | SqlExpr::InSubquery { .. } | SqlExpr::Exists { .. } => false,
    }
}
