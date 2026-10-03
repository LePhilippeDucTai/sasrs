use super::*;

// ----------------------------------------------------------------------------
// 1. FROM + joins
// ----------------------------------------------------------------------------

pub(super) fn scan_normalized(session: &mut Session, lib: &str, table: &str) -> Result<LazyFrame> {
    // Dictionary tables (M20.3) : `DICTIONARY.TABLES/COLUMNS/MACROS` et leurs
    // vues `sashelp.v*` sont matérialisées à la volée depuis l'état de session,
    // puis injectées dans le pipeline standard (WHERE/SELECT/ORDER BY normaux).
    // Leurs colonnes numériques sont déjà des Float64 sans NaN-payload, donc on
    // saute `normalize_specials` (no-op) et on rend la frame telle quelle.
    if let Some(kind) = crate::sql::dictionary::dictionary_kind(lib, table) {
        return crate::sql::dictionary::build_dictionary(session, kind);
    }
    let provider = session.libs.get(lib)?;
    // J04-P2/J04-P4 : les lectures SQL passent par `scan_with_notes` pour ne
    // pas perdre les diagnostics de coercition (WARNING 2**53, sidecar
    // illisible…) que les fournisseurs émettent à la lecture.
    let (lf, notes) = provider.scan_with_notes(table)?;
    for note in notes {
        session.log.forward(&note);
    }
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

/// Issue #15 : construit FROM (+ joins) en appliquant le prédicat WHERE
/// comme condition de jointure quand c'est possible. Pour `from a, b
/// where a.k = b.k`, SAS fait une jointure interne sur les clés — PAS un
/// produit cartésien filtré. On décompose donc le WHERE en conjonctions
/// et, pour chaque table FROM additionnelle, un conjoint equi
/// `gauche.col = droite.col` (colonnes résolues de part et d'autre) est
/// consommé comme clé de jointure ; les conjoints restants sont rendus
/// comme WHERE résiduel à appliquer en filtre.
pub(super) fn build_from(
    query: &SelectStmt,
    session: &mut Session,
) -> Result<(LazyFrame, Option<SqlExpr>)> {
    let Some(first) = query.from.first() else {
        return Err(SasError::runtime(
            "PROC SQL: a SELECT must have a FROM clause.",
        ));
    };
    let mut lf = scan_source(session, first)?;
    let mut left_cols = frame_columns(lf.clone())?;
    let mut conjuncts: Vec<SqlExpr> = Vec::new();
    if let Some(w) = &query.where_ {
        split_conjuncts(w, &mut conjuncts);
    }

    // Tables FROM additionnelles (séparées par des virgules) : jointure
    // interne si un conjoint du WHERE relie les deux côtés (sémantique
    // SAS), sinon cross join (le WHERE résiduel filtrera).
    for extra in query.from.iter().skip(1) {
        let rhs = scan_source(session, extra)?;
        let right_cols = frame_columns(rhs.clone())?;
        if let Some((i, lkey, rkey)) = equi_conjunct_between(&conjuncts, &left_cols, &right_cols) {
            let mut args = JoinArgs::new(JoinType::Inner);
            args.join_nulls = true; // SAS apparie les missings entre eux.
            // Ordre SAS : celui de la table de gauche (et des lignes
            // appariées de droite dans leur ordre d'origine).
            args.maintain_order = MaintainOrderJoin::LeftRight;
            lf = lf.join(rhs, [col(lkey)], [col(rkey)], args);
            conjuncts.remove(i);
        } else {
            lf = lf.join(
                rhs,
                [] as [Expr; 0],
                [] as [Expr; 0],
                JoinArgs::new(JoinType::Cross),
            );
        }
        left_cols.extend(right_cols);
    }

    // Joins explicites.
    for join in &query.joins {
        let rhs = scan_source(session, &join.table)?;
        lf = apply_join(lf, rhs, join)?;
    }

    // WHERE résiduel : conjoints non consommés, ré-AND-és.
    let residual = conjuncts.into_iter().reduce(|a, b| SqlExpr::Binary {
        op: BinaryOp::And,
        left: Box::new(a),
        right: Box::new(b),
    });
    Ok((lf, residual))
}

/// Colonnes du schéma d'une frame (noms réels, pour la résolution
/// insensible à la casse des clés de jointure).
fn frame_columns(mut lf: LazyFrame) -> Result<Vec<String>> {
    Ok(lf
        .collect_schema()?
        .iter_names()
        .map(|n| n.to_string())
        .collect())
}

/// Aplatit une conjonction AND en liste de conjonctions.
fn split_conjuncts(e: &SqlExpr, out: &mut Vec<SqlExpr>) {
    if let SqlExpr::Binary {
        op: BinaryOp::And,
        left,
        right,
    } = e
    {
        split_conjuncts(left, out);
        split_conjuncts(right, out);
    } else {
        out.push(e.clone());
    }
}

/// Cherche un conjoint equi du WHERE dont un côté résout contre une
/// colonne de l'accumulé gauche et l'autre contre une colonne du côté
/// droit. Renvoie (index du conjoint, clé gauche, clé droite) avec les
/// noms RÉELS des colonnes de chaque schéma. Un prédicat qualifié des
/// deux côtés par la MÊME table (`c.id = c.id`) n'est pas une jointure.
fn equi_conjunct_between(
    conjuncts: &[SqlExpr],
    left_cols: &[String],
    right_cols: &[String],
) -> Option<(usize, String, String)> {
    let find =
        |cols: &[String], name: &str| cols.iter().find(|c| c.eq_ignore_ascii_case(name)).cloned();
    for (i, c) in conjuncts.iter().enumerate() {
        let Some((ltable, lcol, rtable, rcol)) = as_equi_key_qualified(c) else {
            continue;
        };
        // Même table qualifiée des deux côtés → comparaison intra-table.
        if let (Some(lt), Some(rt)) = (ltable.as_deref(), rtable.as_deref())
            && lt.eq_ignore_ascii_case(rt)
        {
            continue;
        }
        if let (Some(lk), Some(rk)) = (find(left_cols, &lcol), find(right_cols, &rcol)) {
            return Some((i, lk, rk));
        }
        if let (Some(rk), Some(lk)) = (find(right_cols, &lcol), find(left_cols, &rcol)) {
            return Some((i, lk, rk));
        }
    }
    None
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

/// Variante qualifiée de `as_equi_key` : renvoie (table gauche, colonne
/// gauche, table droite, colonne droite) — les tables sont `None` pour
/// une référence nue.
pub(super) fn as_equi_key_qualified(
    on: &SqlExpr,
) -> Option<(Option<String>, String, Option<String>, String)> {
    let SqlExpr::Binary { op, left, right } = on else {
        return None;
    };
    if *op != BinaryOp::Eq {
        return None;
    }
    let (lt, lc) = as_qualified_column(left)?;
    let (rt, rc) = as_qualified_column(right)?;
    Some((lt, lc, rt, rc))
}

fn as_qualified_column(e: &SqlExpr) -> Option<(Option<String>, String)> {
    match e {
        SqlExpr::Qualified { table, column } => Some((Some(table.clone()), column.clone())),
        SqlExpr::Base(SasExpr::Var(name)) => Some((None, name.clone())),
        _ => None,
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
