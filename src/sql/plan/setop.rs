use super::*;

// ----------------------------------------------------------------------------
// 9. SET OPS
// ----------------------------------------------------------------------------

pub(super) fn apply_set_op(
    lhs: LazyFrame,
    rhs: LazyFrame,
    op: &SetOp,
    all: bool,
) -> Result<LazyFrame> {
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
            let on = lhs_columns(&lhs)?;
            if all {
                // EXCEPT ALL : conserver max(0, n_gauche - n_droite) copies de
                // chaque ligne. On numérote l'occurrence de chaque ligne
                // identique (rang 1, 2, ...) des deux côtés, puis on
                // anti-jointe sur (colonnes + rang). Une ligne de gauche de
                // rang k survit ssi la droite n'a PAS de ligne identique de
                // rang k, c.-à-d. n_droite < k.
                set_op_all(lhs, rhs, &on, JoinType::Anti)
            } else {
                let mut args = JoinArgs::new(JoinType::Anti);
                args.join_nulls = true;
                let on_l: Vec<Expr> = on.iter().map(|c| col(c.clone())).collect();
                let out = lhs.join(rhs, &on_l, &on_l, args);
                Ok(out.unique(None, UniqueKeepStrategy::Any))
            }
        }
        SetOp::Intersect => {
            let on = lhs_columns(&lhs)?;
            if all {
                // INTERSECT ALL : conserver min(n_gauche, n_droite) copies. Une
                // ligne de gauche de rang k survit ssi la droite a une ligne
                // identique de rang k (n_droite >= k) → semi-join sur
                // (colonnes + rang).
                set_op_all(lhs, rhs, &on, JoinType::Semi)
            } else {
                let mut args = JoinArgs::new(JoinType::Semi);
                args.join_nulls = true;
                let on_l: Vec<Expr> = on.iter().map(|c| col(c.clone())).collect();
                let out = lhs.join(rhs, &on_l, &on_l, args);
                Ok(out.unique(None, UniqueKeepStrategy::Any))
            }
        }
    }
}

/// Nom de la colonne interne portant le rang d'occurrence. Préfixe improbable
/// pour ne pas entrer en collision avec une vraie variable SAS (max 32 car.,
/// jamais d'espace ni de `#`).
pub(super) const OCC_RANK_COL: &str = "# sasrs occ rank #";

/// Implémente EXCEPT ALL / INTERSECT ALL en respectant la multiplicité exacte.
///
/// Idée : pour chaque ligne, on calcule son rang d'occurrence parmi les lignes
/// identiques (1 pour la première copie, 2 pour la deuxième, ...) via une
/// fenêtre `cum_sum().over(toutes les colonnes)`. On joint alors gauche et
/// droite sur (toutes les colonnes + rang) :
///   - `Anti` (EXCEPT ALL)  → gardent les (ligne, rang) absents à droite,
///     soit `max(0, n_gauche - n_droite)` copies ;
///   - `Semi` (INTERSECT ALL) → gardent les (ligne, rang) présents à droite,
///     soit `min(n_gauche, n_droite)` copies.
/// La colonne de rang est retirée du résultat. `join_nulls(true)` assure que
/// `. = .` matche (sémantique SAS).
pub(super) fn set_op_all(
    lhs: LazyFrame,
    rhs: LazyFrame,
    on: &[String],
    how: JoinType,
) -> Result<LazyFrame> {
    let partition: Vec<Expr> = on.iter().map(|c| col(c.clone())).collect();
    // Rang d'occurrence = somme cumulée, partitionnée par toutes les colonnes,
    // d'une constante 1 MATÉRIALISÉE en colonne. (Un `lit(1)` scalaire ne se
    // diffuse pas correctement dans `over` : Polars exige une expression de la
    // longueur du groupe ; on passe donc par une vraie colonne `col(ONE)`.)
    // `cum_sum` sur des entiers non-nuls donne 1, 2, 3... pour les lignes
    // identiques.
    const ONE_COL: &str = "# sasrs one #";
    let rank_expr = col(ONE_COL)
        .cum_sum(false)
        .over(partition.clone())
        .alias(OCC_RANK_COL);
    let lhs_r = lhs
        .with_column(lit(1i32).alias(ONE_COL))
        .with_column(rank_expr.clone())
        .drop([col(ONE_COL)]);
    let rhs_r = rhs
        .with_column(lit(1i32).alias(ONE_COL))
        .with_column(rank_expr)
        .drop([col(ONE_COL)]);

    let mut on_cols: Vec<Expr> = partition;
    on_cols.push(col(OCC_RANK_COL));

    let mut args = JoinArgs::new(how);
    args.join_nulls = true;
    let out = lhs_r.join(rhs_r, &on_cols, &on_cols, args);
    // La colonne de rang ne doit pas apparaître dans le résultat.
    Ok(out.drop([col(OCC_RANK_COL)]))
}

pub(super) fn lhs_columns(lf: &LazyFrame) -> Result<Vec<String>> {
    let mut lf = lf.clone();
    let schema = lf.collect_schema()?;
    Ok(schema.iter_names().map(|n| n.to_string()).collect())
}
