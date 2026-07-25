use super::*;

// ----------------------------------------------------------------------------
// 0. Sous-requêtes (M20.2)
// ----------------------------------------------------------------------------
//
// Stratégie : pré-passe qui réécrit le `SelectStmt` en remplaçant chaque
// sous-requête NON-CORRÉLÉE par un littéral :
//   - scalaire `(SELECT ...)`      → la valeur unique (Num/Str/Missing) ;
//   - `x IN (SELECT ...)`          → `x IN (v1, v2, ...)` (liste matérialisée) ;
//   - `[NOT] EXISTS (SELECT ...)`  → booléen constant (1=true / 0=false).
// Les sous-requêtes corrélées (qui référencent une colonne d'une table de la
// requête EXTÉRIEURE) ne sont pas matérialisables ainsi : on lève une erreur
// documentée.

/// Réécrit récursivement `query` en résolvant ses sous-requêtes non-corrélées.
pub(super) fn resolve_subqueries(query: &SelectStmt, session: &mut Session) -> Result<SelectStmt> {
    let outer = visible_names(query);
    let mut out = query.clone();

    if let Some(w) = &out.where_ {
        let mut w = w.clone();
        rewrite_sql_expr(&mut w, &outer, session)?;
        out.where_ = Some(w);
    }
    for it in &mut out.items {
        rewrite_sql_expr(&mut it.expr, &outer, session)?;
    }
    if let Some(h) = &out.having {
        let mut h = h.clone();
        rewrite_sql_expr(&mut h, &outer, session)?;
        out.having = Some(h);
    }
    for j in &mut out.joins {
        if let Some(on) = &j.on {
            let mut on = on.clone();
            rewrite_sql_expr(&mut on, &outer, session)?;
            j.on = Some(on);
        }
    }
    // Sous-requêtes en FROM (M20.4) : résolues récursivement avant
    // l'abaissement (elles peuvent elles-mêmes contenir des sous-requêtes).
    for fi in &mut out.from {
        if let Some(sub) = &fi.subquery {
            let resolved = resolve_subqueries(sub, session)?;
            fi.subquery = Some(Box::new(resolved));
        }
    }
    for j in &mut out.joins {
        if let Some(sub) = &j.table.subquery {
            let resolved = resolve_subqueries(sub, session)?;
            j.table.subquery = Some(Box::new(resolved));
        }
    }
    if let Some((op, all, rhs)) = &out.set_op {
        let rhs2 = resolve_subqueries(rhs, session)?;
        out.set_op = Some((op.clone(), *all, Box::new(rhs2)));
    }
    Ok(out)
}

/// Ensemble (minuscule) des alias et noms de tables d'une requête, pour la
/// détection de corrélation.
pub(super) fn visible_names(query: &SelectStmt) -> Vec<String> {
    let mut names = Vec::new();
    let mut add = |fi: &crate::sql::ast::FromItem| {
        if let Some(a) = &fi.alias {
            names.push(a.to_ascii_lowercase());
        }
        names.push(fi.table.name.to_ascii_lowercase());
    };
    for fi in &query.from {
        add(fi);
    }
    for j in &query.joins {
        add(&j.table);
    }
    names
}

/// Réécrit en place les sous-requêtes d'une expression. `outer` = noms visibles
/// dans la requête englobante (pour détecter la corrélation).
pub(super) fn rewrite_sql_expr(e: &mut SqlExpr, outer: &[String], session: &mut Session) -> Result<()> {
    match e {
        SqlExpr::Subquery(q) => {
            ensure_not_correlated(q, outer)?;
            let lit = eval_scalar_subquery(q, session)?;
            *e = lit;
        }
        SqlExpr::InSubquery {
            expr,
            query,
            negated,
        } => {
            rewrite_sql_expr(expr, outer, session)?;
            ensure_not_correlated(query, outer)?;
            let list = eval_column_subquery(query, session)?;
            // On émet une chaîne d'égalités `expr = v1 OR expr = v2 OR ...`
            // (membre droit matérialisé) plutôt que `Expr::In` : cela évite les
            // problèmes de diffusion (`is_in` sur une liste d'une seule ligne ne
            // se diffuse pas sur N lignes) et respecte la sémantique missing
            // (un `expr = .` se traduit en is_null via le traducteur). Une liste
            // vide → faux partout (`1 = 0`).
            let mut pred: Option<SqlExpr> = None;
            for v in list {
                let eq = SqlExpr::Binary {
                    op: BinaryOp::Eq,
                    left: expr.clone(),
                    right: Box::new(SqlExpr::Base(v)),
                };
                pred = Some(match pred {
                    None => eq,
                    Some(acc) => SqlExpr::Binary {
                        op: BinaryOp::Or,
                        left: Box::new(acc),
                        right: Box::new(eq),
                    },
                });
            }
            let mut built = pred.unwrap_or(SqlExpr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(SqlExpr::Base(SasExpr::Num(1.0))),
                right: Box::new(SqlExpr::Base(SasExpr::Num(0.0))),
            });
            if *negated {
                built = SqlExpr::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(built),
                };
            }
            *e = built;
        }
        SqlExpr::Exists { query, negated } => {
            ensure_not_correlated(query, outer)?;
            let any = subquery_has_rows(query, session)?;
            let truth = any ^ *negated;
            // Booléen constant pour un prédicat WHERE : `1 = 1` (vrai, conserve
            // toutes les lignes) / `1 = 0` (faux, les élimine). On NE peut PAS
            // utiliser un littéral numérique nu — un filtre Polars exige une
            // expression BOOLÉENNE, pas un float.
            let rhs = if truth { 1.0 } else { 0.0 };
            *e = SqlExpr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(SqlExpr::Base(SasExpr::Num(1.0))),
                right: Box::new(SqlExpr::Base(SasExpr::Num(rhs))),
            };
        }
        SqlExpr::Binary { left, right, .. } => {
            rewrite_sql_expr(left, outer, session)?;
            rewrite_sql_expr(right, outer, session)?;
        }
        SqlExpr::Unary { expr, .. } => rewrite_sql_expr(expr, outer, session)?,
        SqlExpr::Between {
            expr, low, high, ..
        } => {
            rewrite_sql_expr(expr, outer, session)?;
            rewrite_sql_expr(low, outer, session)?;
            rewrite_sql_expr(high, outer, session)?;
        }
        SqlExpr::IsNull { expr, .. } => rewrite_sql_expr(expr, outer, session)?,
        SqlExpr::Like { expr, .. } => rewrite_sql_expr(expr, outer, session)?,
        SqlExpr::Aggregate { arg: Some(a), .. } => rewrite_sql_expr(a, outer, session)?,
        SqlExpr::Aggregate { .. }
        | SqlExpr::Base(_)
        | SqlExpr::Star
        | SqlExpr::QualifiedStar(_)
        | SqlExpr::Qualified { .. }
        | SqlExpr::Calculated(_) => {}
    }
    Ok(())
}

/// Lève une erreur documentée si la sous-requête est corrélée, c.-à-d. si elle
/// référence un alias/table de la requête EXTÉRIEURE qu'elle ne redéfinit pas
/// localement.
pub(super) fn ensure_not_correlated(query: &SelectStmt, outer: &[String]) -> Result<()> {
    let local = visible_names(query);
    let mut refs = Vec::new();
    collect_qualified_tables(query, &mut refs);
    for r in refs {
        let r = r.to_ascii_lowercase();
        if outer.contains(&r) && !local.contains(&r) {
            return Err(SasError::runtime(
                "PROC SQL: correlated subqueries are not supported yet \
                 (the subquery references a column from an outer query).",
            ));
        }
    }
    Ok(())
}

/// Collecte les préfixes de table des références qualifiées `table.col` d'une
/// requête, récursivement dans les sous-requêtes imbriquées.
pub(super) fn collect_qualified_tables(query: &SelectStmt, out: &mut Vec<String>) {
    fn walk(e: &SqlExpr, out: &mut Vec<String>) {
        match e {
            SqlExpr::Qualified { table, .. } => out.push(table.clone()),
            SqlExpr::Binary { left, right, .. } => {
                walk(left, out);
                walk(right, out);
            }
            SqlExpr::Unary { expr, .. } => walk(expr, out),
            SqlExpr::Between {
                expr, low, high, ..
            } => {
                walk(expr, out);
                walk(low, out);
                walk(high, out);
            }
            SqlExpr::IsNull { expr, .. } | SqlExpr::Like { expr, .. } => walk(expr, out),
            SqlExpr::Aggregate { arg: Some(a), .. } => walk(a, out),
            SqlExpr::Subquery(q) => collect_qualified_tables(q, out),
            SqlExpr::InSubquery { expr, query, .. } => {
                walk(expr, out);
                collect_qualified_tables(query, out);
            }
            SqlExpr::Exists { query, .. } => collect_qualified_tables(query, out),
            _ => {}
        }
    }
    for it in &query.items {
        walk(&it.expr, out);
    }
    if let Some(w) = &query.where_ {
        walk(w, out);
    }
    if let Some(h) = &query.having {
        walk(h, out);
    }
    for j in &query.joins {
        if let Some(on) = &j.on {
            walk(on, out);
        }
    }
}

/// Évalue une sous-requête scalaire : une seule colonne ; on prend la valeur de
/// la PREMIÈRE ligne (absence de ligne → missing).
pub(super) fn eval_scalar_subquery(query: &SelectStmt, session: &mut Session) -> Result<SqlExpr> {
    let df = lower_select(query, session)?.collect()?;
    if df.width() != 1 {
        return Err(SasError::runtime(
            "PROC SQL: a scalar subquery must select exactly one column.",
        ));
    }
    let col = df.get_columns()[0].as_materialized_series();
    if col.is_empty() {
        return Ok(SqlExpr::Base(SasExpr::Missing(MissingKind::Dot)));
    }
    any_value_to_base(col, 0)
}

/// Évalue une sous-requête de colonne (membre droit d'un IN) en une liste de
/// littéraux `Expr`. Les nulls sont ignorés.
pub(super) fn eval_column_subquery(query: &SelectStmt, session: &mut Session) -> Result<Vec<SasExpr>> {
    let df = lower_select(query, session)?.collect()?;
    if df.width() != 1 {
        return Err(SasError::runtime(
            "PROC SQL: a subquery used with IN must select exactly one column.",
        ));
    }
    let col = df.get_columns()[0].as_materialized_series();
    let mut list = Vec::with_capacity(col.len());
    for i in 0..col.len() {
        // Les missings ne sont pas inclus dans la liste IN.
        match any_value_to_base(col, i)? {
            SqlExpr::Base(SasExpr::Missing(_)) => {}
            SqlExpr::Base(b) => list.push(b),
            _ => {}
        }
    }
    Ok(list)
}

/// Vrai si la sous-requête renvoie au moins une ligne (pour EXISTS).
pub(super) fn subquery_has_rows(query: &SelectStmt, session: &mut Session) -> Result<bool> {
    let df = lower_select(query, session)?.limit(1).collect()?;
    Ok(df.height() > 0)
}

/// Convertit la valeur (ligne `idx`) d'une série en littéral `SqlExpr::Base`.
pub(super) fn any_value_to_base(series: &Series, idx: usize) -> Result<SqlExpr> {
    use polars::prelude::AnyValue;
    let av = series
        .get(idx)
        .map_err(|e| SasError::runtime(format!("PROC SQL: failed to read subquery value: {e}")))?;
    let base = match av {
        AnyValue::Null => SasExpr::Missing(MissingKind::Dot),
        AnyValue::Float64(f) => {
            if f.is_nan() {
                SasExpr::Missing(MissingKind::Dot)
            } else {
                SasExpr::Num(f)
            }
        }
        AnyValue::Float32(f) => SasExpr::Num(f as f64),
        AnyValue::Int64(n) => SasExpr::Num(n as f64),
        AnyValue::Int32(n) => SasExpr::Num(n as f64),
        AnyValue::UInt32(n) => SasExpr::Num(n as f64),
        AnyValue::UInt64(n) => SasExpr::Num(n as f64),
        AnyValue::Boolean(b) => SasExpr::Num(if b { 1.0 } else { 0.0 }),
        AnyValue::String(s) => SasExpr::Str(s.to_string()),
        AnyValue::StringOwned(s) => SasExpr::Str(s.to_string()),
        other => {
            return Err(SasError::runtime(format!(
                "PROC SQL: subquery produced an unsupported value type ({other:?})."
            )));
        }
    };
    Ok(SqlExpr::Base(base))
}
