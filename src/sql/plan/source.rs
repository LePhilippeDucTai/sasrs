use super::*;

// ----------------------------------------------------------------------------
// 1. FROM + joins
// ----------------------------------------------------------------------------

pub(super) fn scan_normalized(session: &Session, lib: &str, table: &str) -> Result<LazyFrame> {
    // Dictionary tables (M20.3) : `DICTIONARY.TABLES/COLUMNS/MACROS` et leurs
    // vues `sashelp.v*` sont matérialisées à la volée depuis l'état de session,
    // puis injectées dans le pipeline standard (WHERE/SELECT/ORDER BY normaux).
    // Leurs colonnes numériques sont déjà des Float64 sans NaN-payload, donc on
    // saute `normalize_specials` (no-op) et on rend la frame telle quelle.
    if let Some(kind) = crate::sql::dictionary::dictionary_kind(lib, table) {
        return crate::sql::dictionary::build_dictionary(session, kind);
    }
    let provider = session.libs.get(lib)?;
    let lf = provider.scan(table)?;
    // Normalisation des missings spéciaux (NaN-payload → null) sur chaque
    // colonne Float64 — passe par l'unique implémentation `normalize_specials`
    // (cf. note d'en-tête : ne jamais réimplémenter ad hoc).
    normalize_specials(lf)
}

/// Scanne une source de `FROM`/JOIN : soit une VUE SQL stockée en session
/// (M20.4), soit une table physique via `scan_normalized`. Une vue est
/// reconnue dans l'espace WORK (libref absent ou `WORK`) par son nom
/// UPPERCASE présent dans `Session.views` ; sa requête stockée est abaissée
/// récursivement (vues imbriquées admises). La frame résultat est déjà
/// coercée/normalisée par `lower_select`, on n'y rejoue pas `normalize_specials`.
/// Scanne une source de `FROM`/JOIN : sous-requête en FROM (M20.4), vue SQL
/// stockée, ou table physique. Une sous-requête (`FROM (SELECT ...) alias`)
/// est abaissée récursivement. Une vue est reconnue dans l'espace WORK
/// (libref absent / `WORK`) par son nom UPPERCASE présent dans
/// `Session.views`. Sinon → `scan_normalized` (table physique / dictionnaire).
pub(super) fn scan_source(
    session: &mut Session,
    item: &crate::sql::ast::FromItem,
) -> Result<LazyFrame> {
    if let Some(sub) = &item.subquery {
        return lower_select(sub, session);
    }
    let lib = item.table.libref_or_work();
    let name = item.table.name.to_uppercase();
    if lib == "WORK"
        && let Some(view_query) = session.views.get(&name).cloned()
    {
        return lower_select(&view_query, session);
    }
    scan_normalized(session, &lib, &name)
}

pub(super) fn build_from(query: &SelectStmt, session: &mut Session) -> Result<LazyFrame> {
    let Some(first) = query.from.first() else {
        return Err(SasError::runtime(
            "PROC SQL: a SELECT must have a FROM clause.",
        ));
    };
    let mut lf = scan_source(session, first)?;

    // Tables FROM additionnelles (séparées par des virgules) = cross join.
    for extra in query.from.iter().skip(1) {
        let rhs = scan_source(session, extra)?;
        lf = lf.join(
            rhs,
            [] as [Expr; 0],
            [] as [Expr; 0],
            JoinArgs::new(JoinType::Cross),
        );
    }

    // Joins explicites.
    for join in &query.joins {
        let rhs = scan_source(session, &join.table)?;
        lf = apply_join(lf, rhs, join)?;
    }

    Ok(lf)
}

pub(super) fn apply_join(
    lf: LazyFrame,
    rhs: LazyFrame,
    join: &crate::sql::ast::Join,
) -> Result<LazyFrame> {
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
        Ok(lf
            .join(rhs, [] as [Expr; 0], [] as [Expr; 0], args)
            .filter(pred))
    }
}

/// Si `on` est exactement `lhs = rhs` avec deux références de colonnes,
/// renvoie (nom_gauche, nom_droite).
pub(super) fn as_equi_key(on: &SqlExpr) -> Option<(String, String)> {
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

pub(super) fn as_column_name(e: &SqlExpr) -> Option<String> {
    match e {
        SqlExpr::Qualified { column, .. } => Some(column.clone()),
        SqlExpr::Base(SasExpr::Var(name)) => Some(name.clone()),
        _ => None,
    }
}
