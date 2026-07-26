use super::*;

// ── SELECT ───────────────────────────────────────────────────────────────

/// `SELECT [DISTINCT] <list> FROM <from-list> [<joins>] [WHERE] [GROUP BY]
/// [HAVING] [ORDER BY] [<set-op> SELECT ...]`.
pub(super) fn parse_select(ts: &mut StatementStream) -> Result<SelectStmt> {
    expect_kw(ts, "select")?;
    let distinct = if ts.peek().is_kw("distinct") {
        ts.next();
        true
    } else {
        false
    };
    let items = parse_select_list(ts)?;

    // `SELECT ... INTO :macrovar ...` — clause INTO non supportée.
    if ts.peek().is_kw("into") {
        return Err(SasError::parse(
            "The INTO clause is not yet supported.",
            ts.peek().span,
        ));
    }

    expect_kw(ts, "from")?;
    let from = parse_from_list(ts)?;
    let joins = parse_joins(ts)?;

    let where_ = if ts.peek().is_kw("where") {
        ts.next();
        Some(parse_sql_expr(ts)?)
    } else {
        None
    };

    let group_by = if ts.peek().is_kw("group") {
        ts.next();
        expect_kw(ts, "by")?;
        parse_sql_expr_list(ts)?
    } else {
        Vec::new()
    };

    let having = if ts.peek().is_kw("having") {
        ts.next();
        Some(parse_sql_expr(ts)?)
    } else {
        None
    };

    let order_by = if ts.peek().is_kw("order") {
        ts.next();
        expect_kw(ts, "by")?;
        parse_order_list(ts)?
    } else {
        Vec::new()
    };

    let set_op = parse_set_op_tail(ts)?;

    Ok(SelectStmt {
        distinct,
        items,
        from,
        joins,
        where_,
        group_by,
        having,
        order_by,
        set_op,
    })
}

/// select-list : `*` | `alias.*` | `<sqlexpr> [[AS] alias]`, séparés par `,`.
pub(super) fn parse_select_list(ts: &mut StatementStream) -> Result<Vec<SelectItem>> {
    let mut items = Vec::new();
    loop {
        items.push(parse_select_item(ts)?);
        if ts.peek().kind == TokenKind::Comma {
            ts.next();
        } else {
            break;
        }
    }
    Ok(items)
}

pub(super) fn parse_select_item(ts: &mut StatementStream) -> Result<SelectItem> {
    // `select x into :macrovar ...` — clause INTO non supportée. Détectée si
    // l'item est suivi de `INTO`, mais le cas typique met INTO juste après
    // SELECT ; on le repère aussi au niveau de l'item courant.
    if ts.peek().is_kw("into") {
        return Err(SasError::parse(
            "The INTO clause is not yet supported.",
            ts.peek().span,
        ));
    }

    // `*` — toutes les colonnes.
    if ts.peek().kind == TokenKind::Star {
        ts.next();
        return Ok(SelectItem {
            expr: SqlExpr::Star,
            alias: None,
        });
    }

    // `alias.*` : ident `.` `*`.
    if let TokenKind::Ident(name) = &ts.peek().kind {
        if ts.peek2().kind == TokenKind::Dot {
            let name = name.clone();
            // Lookahead manuel : ident `.` `*` ?
            // On clone pour inspecter le 3e token sans le consommer : pas de
            // peek3, donc on consomme prudemment l'ident + dot puis on teste.
            // Pour éviter une mauvaise consommation, on bascule plutôt vers
            // l'expression qui gère déjà `a.col` ; le cas `a.*` est traité ici
            // en vérifiant le `*` après avoir consommé ident et dot.
            // -> Implémenté dans parse_sql_atom via un drapeau ? Plus simple :
            //    on consomme ident + dot puis on regarde `*`.
            ts.next(); // ident
            ts.next(); // dot
            if ts.peek().kind == TokenKind::Star {
                ts.next(); // *
                let alias = maybe_alias(ts)?;
                return Ok(SelectItem {
                    expr: SqlExpr::QualifiedStar(name),
                    alias,
                });
            }
            // Sinon c'est `a.col` : on a déjà consommé ident + dot, il reste
            // la colonne. On la lit et on construit un Qualified, puis on
            // poursuit l'expression via la suite postfixée.
            let col_tok = ts.peek().clone();
            let Some(col) = col_tok.ident().map(str::to_string) else {
                return Err(SasError::parse(
                    "expected a column name after '.'",
                    col_tok.span,
                ));
            };
            ts.next();
            let base = SqlExpr::Qualified {
                table: name,
                column: col,
            };
            let expr = continue_expr_from(ts, base)?;
            let alias = maybe_alias(ts)?;
            return Ok(SelectItem { expr, alias });
        }
    }

    let expr = parse_sql_expr(ts)?;
    let alias = maybe_alias(ts)?;
    Ok(SelectItem { expr, alias })
}

/// Alias optionnel d'un item de select : `AS nom` ou `nom` nu. Le `nom` nu
/// n'est consommé que si c'est un ident qui n'introduit PAS une clause
/// suivante (from/where/...). En tête d'item un `*` ou expression a déjà été
/// lu, donc tout ident restant qui n'est pas un mot-clé de clause est un
/// alias.
pub(super) fn maybe_alias(ts: &mut StatementStream) -> Result<Option<String>> {
    if ts.peek().is_kw("as") {
        ts.next();
        let tok = ts.peek().clone();
        let Some(name) = tok.ident().map(str::to_string) else {
            return Err(SasError::parse("expected an alias name after AS", tok.span));
        };
        ts.next();
        return Ok(Some(name));
    }
    // Alias nu : un ident qui n'est pas un mot-clé de clause/jointure.
    if let TokenKind::Ident(s) = &ts.peek().kind {
        if !is_clause_kw(s) {
            let name = s.clone();
            ts.next();
            return Ok(Some(name));
        }
    }
    Ok(None)
}

/// Mots-clés qui terminent un item / une liste et ne peuvent pas être pris
/// pour un alias nu.
pub(super) fn is_clause_kw(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "from"
            | "where"
            | "group"
            | "having"
            | "order"
            | "union"
            | "except"
            | "intersect"
            | "on"
            | "inner"
            | "left"
            | "right"
            | "full"
            | "cross"
            | "join"
            | "as"
            | "and"
            | "or"
            | "asc"
            | "desc"
            | "into"
    )
}

/// from-list : `<from-item> [, <from-item>]*`.
pub(super) fn parse_from_list(ts: &mut StatementStream) -> Result<Vec<FromItem>> {
    let mut items = vec![parse_from_item(ts)?];
    while ts.peek().kind == TokenKind::Comma {
        ts.next();
        items.push(parse_from_item(ts)?);
    }
    Ok(items)
}

/// from-item : `lib.table | table [[AS] alias]` ou `( SELECT ... ) [[AS] alias]`
/// (sous-requête en FROM, M20.4). Le placeholder `table` d'une sous-requête
/// prend pour nom l'alias (ou un nom synthétique), jamais résolu physiquement.
pub(super) fn parse_from_item(ts: &mut StatementStream) -> Result<FromItem> {
    if ts.peek().kind == TokenKind::LParen {
        ts.next(); // (
        if !ts.peek().is_kw("select") {
            return Err(SasError::parse(
                "expected SELECT in the FROM subquery",
                ts.peek().span,
            ));
        }
        let query = parse_select(ts)?;
        expect_rparen(ts)?;
        let alias = maybe_table_alias(ts)?;
        let placeholder = alias.clone().unwrap_or_else(|| "__derived__".to_string());
        return Ok(FromItem {
            table: crate::ast::DatasetRef {
                libref: None,
                name: placeholder,
            },
            alias,
            subquery: Some(Box::new(query)),
        });
    }
    let table = ts.parse_dataset_ref()?;
    let alias = maybe_table_alias(ts)?;
    Ok(FromItem {
        table,
        alias,
        subquery: None,
    })
}

/// Alias d'une table : `AS nom` ou `nom` nu (pas un mot-clé de clause).
pub(super) fn maybe_table_alias(ts: &mut StatementStream) -> Result<Option<String>> {
    if ts.peek().is_kw("as") {
        ts.next();
        let tok = ts.peek().clone();
        let Some(name) = tok.ident().map(str::to_string) else {
            return Err(SasError::parse("expected an alias name after AS", tok.span));
        };
        ts.next();
        return Ok(Some(name));
    }
    if let TokenKind::Ident(s) = &ts.peek().kind {
        if !is_clause_kw(s) {
            let name = s.clone();
            ts.next();
            return Ok(Some(name));
        }
    }
    Ok(None)
}

/// Jointures : `[INNER|LEFT [OUTER]|RIGHT [OUTER]|FULL [OUTER]|CROSS] JOIN
/// <from-item> [ON <sqlexpr>]`, accumulées.
pub(super) fn parse_joins(ts: &mut StatementStream) -> Result<Vec<Join>> {
    let mut joins = Vec::new();
    loop {
        let kind = match peek_join_kind(ts) {
            Some(k) => k,
            None => break,
        };
        consume_join_prefix(ts)?;
        let table = parse_from_item(ts)?;
        let on = if ts.peek().is_kw("on") {
            ts.next();
            Some(parse_sql_expr(ts)?)
        } else {
            None
        };
        joins.push(Join { kind, table, on });
    }
    Ok(joins)
}

/// Détecte si une jointure commence ici (sans consommer).
pub(super) fn peek_join_kind(ts: &StatementStream) -> Option<JoinKind> {
    let s = ts.peek().ident()?;
    match s.to_ascii_lowercase().as_str() {
        "join" => Some(JoinKind::Inner),
        "inner" => Some(JoinKind::Inner),
        "left" => Some(JoinKind::Left),
        "right" => Some(JoinKind::Right),
        "full" => Some(JoinKind::Full),
        "cross" => Some(JoinKind::Cross),
        _ => None,
    }
}

/// Consomme le préfixe de jointure (`INNER JOIN`, `LEFT [OUTER] JOIN`, ...).
pub(super) fn consume_join_prefix(ts: &mut StatementStream) -> Result<()> {
    if ts.peek().is_kw("join") {
        ts.next();
        return Ok(());
    }
    // mot-clé de type (inner/left/right/full/cross) déjà détecté.
    ts.next();
    // `OUTER` optionnel après LEFT/RIGHT/FULL.
    if ts.peek().is_kw("outer") {
        ts.next();
    }
    expect_kw(ts, "join")?;
    Ok(())
}

/// set-op tail : `UNION|EXCEPT|INTERSECT [ALL] SELECT ...`.
pub(super) fn parse_set_op_tail(
    ts: &mut StatementStream,
) -> Result<Option<(SetOp, bool, Box<SelectStmt>)>> {
    let op = match ts.peek().ident().map(|s| s.to_ascii_lowercase()) {
        Some(ref s) if s == "union" => SetOp::Union,
        Some(ref s) if s == "except" => SetOp::Except,
        Some(ref s) if s == "intersect" => SetOp::Intersect,
        _ => return Ok(None),
    };
    ts.next(); // l'opérateur
    let all = if ts.peek().is_kw("all") {
        ts.next();
        true
    } else {
        false
    };
    let rhs = parse_select(ts)?;
    Ok(Some((op, all, Box::new(rhs))))
}

/// Liste d'expressions SQL séparées par `,` (GROUP BY).
pub(super) fn parse_sql_expr_list(ts: &mut StatementStream) -> Result<Vec<SqlExpr>> {
    let mut list = vec![parse_sql_expr(ts)?];
    while ts.peek().kind == TokenKind::Comma {
        ts.next();
        list.push(parse_sql_expr(ts)?);
    }
    Ok(list)
}

/// ORDER BY : liste de `(SqlExpr, desc)` avec `ASC`/`DESC` optionnel.
pub(super) fn parse_order_list(ts: &mut StatementStream) -> Result<Vec<(SqlExpr, bool)>> {
    let mut list = Vec::new();
    loop {
        let e = parse_sql_expr(ts)?;
        let desc = if ts.peek().is_kw("desc") {
            ts.next();
            true
        } else if ts.peek().is_kw("asc") {
            ts.next();
            false
        } else {
            false
        };
        list.push((e, desc));
        if ts.peek().kind == TokenKind::Comma {
            ts.next();
        } else {
            break;
        }
    }
    Ok(list)
}
