use super::*;

// ----------------------------------------------------------------------------
// 6. SELECT list / aliases / CALCULATED
// ----------------------------------------------------------------------------

pub(super) fn project_select_list(
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
pub(super) fn output_name(it: &SelectItem, _query: &SelectStmt) -> Result<String> {
    if let Some(a) = &it.alias {
        return Ok(a.clone());
    }
    match &it.expr {
        SqlExpr::Base(SasExpr::Var(name)) => Ok(name.clone()),
        SqlExpr::Qualified { column, .. } => Ok(column.clone()),
        SqlExpr::Aggregate {
            func, arg, star, ..
        } => {
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
pub(super) fn order_output_name(e: &SqlExpr, query: &SelectStmt) -> Option<String> {
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

pub(super) fn apply_order_by(
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
