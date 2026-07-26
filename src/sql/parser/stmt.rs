use super::*;

/// Parse tout le contenu de `proc sql; ... quit;`.
///
/// Boucle de parsing des statements jusqu'à `quit`/`quit;` ou EOF. Chaque
/// statement se termine par `;`. `quit;` est consommé et arrête la boucle.
pub fn parse_sql_program(ts: &mut StatementStream) -> Result<SqlProgram> {
    let mut stmts = Vec::new();
    loop {
        // Statements vides : `;` isolé.
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            continue;
        }
        if ts.at_eof() {
            break;
        }
        if ts.peek().is_kw("quit") {
            ts.next();
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            break;
        }
        let stmt = parse_statement(ts)?;
        match stmt {
            Some(s) => {
                stmts.push(s);
                ts.expect_semi()?;
            }
            // Statement ignoré (RESET/TITLE/...) : déjà avancé jusqu'au `;`.
            None => {}
        }
    }
    Ok(SqlProgram { stmts })
}

/// Parse un statement PROC SQL. `Ok(None)` = statement reconnu mais ignoré
/// (déjà consommé jusqu'au `;` inclus).
pub(super) fn parse_statement(ts: &mut StatementStream) -> Result<Option<SqlStmt>> {
    let tok = ts.peek().clone();
    let Some(head) = tok.ident().map(|s| s.to_ascii_lowercase()) else {
        return Err(SasError::parse(
            "expected a PROC SQL statement keyword",
            tok.span,
        ));
    };
    match head.as_str() {
        "select" => Ok(Some(SqlStmt::Select(parse_select(ts)?))),
        "create" => Ok(Some(parse_create(ts)?)),
        "drop" => Ok(Some(parse_drop(ts)?)),
        "update" => Ok(Some(parse_update(ts)?)),
        "insert" => Ok(Some(parse_insert(ts)?)),
        "delete" => Ok(Some(parse_delete(ts)?)),
        "describe" => Ok(Some(parse_describe(ts)?)),
        // Statements PROC SQL non modélisés (RESET, TITLE, FOOTNOTE,
        // VALIDATE, ...) : on les saute proprement jusqu'au `;`.
        _ => {
            ts.skip_to_semi();
            Ok(None)
        }
    }
}

/// `CREATE TABLE <ref> AS <select>` ou `CREATE VIEW <ref> AS <select>`.
/// Le mot-clé après CREATE (table/view) discrimine. Tout autre objet
/// (`INDEX`, ...) → erreur propre.
pub(super) fn parse_create(ts: &mut StatementStream) -> Result<SqlStmt> {
    ts.next(); // CREATE
    if ts.peek().is_kw("view") {
        return parse_create_view(ts);
    }
    expect_kw(ts, "table")?;
    let table = ts.parse_dataset_ref()?;
    expect_kw(ts, "as")?;
    if !ts.peek().is_kw("select") {
        return Err(SasError::parse(
            "expected SELECT after CREATE TABLE ... AS",
            ts.peek().span,
        ));
    }
    let query = parse_select(ts)?;
    Ok(SqlStmt::CreateTableAs { table, query })
}

/// `CREATE VIEW <ref> AS <select>` (M20.4). Symétrique de CREATE TABLE AS,
/// le mot-clé VIEW étant déjà en tête (non consommé).
pub(super) fn parse_create_view(ts: &mut StatementStream) -> Result<SqlStmt> {
    expect_kw(ts, "view")?;
    let name = ts.parse_dataset_ref()?;
    expect_kw(ts, "as")?;
    if !ts.peek().is_kw("select") {
        return Err(SasError::parse(
            "expected SELECT after CREATE VIEW ... AS",
            ts.peek().span,
        ));
    }
    let query = parse_select(ts)?;
    Ok(SqlStmt::CreateView {
        name,
        query: Box::new(query),
    })
}

/// `DROP TABLE <ref> [, <ref> ...]` ou `DROP VIEW <ref> [, <ref> ...]`.
pub(super) fn parse_drop(ts: &mut StatementStream) -> Result<SqlStmt> {
    ts.next(); // DROP
    if ts.peek().is_kw("view") {
        ts.next(); // VIEW
        let mut refs = vec![ts.parse_dataset_ref()?];
        while ts.peek().kind == TokenKind::Comma {
            ts.next();
            refs.push(ts.parse_dataset_ref()?);
        }
        return Ok(SqlStmt::DropView(refs));
    }
    expect_kw(ts, "table")?;
    let mut refs = vec![ts.parse_dataset_ref()?];
    while ts.peek().kind == TokenKind::Comma {
        ts.next();
        refs.push(ts.parse_dataset_ref()?);
    }
    Ok(SqlStmt::DropTable(refs))
}

/// `UPDATE <ref> SET col1=expr1 [, col2=expr2 ...] [WHERE <sqlexpr>]`.
/// SET est obligatoire et exige au moins une assignation.
pub(super) fn parse_update(ts: &mut StatementStream) -> Result<SqlStmt> {
    ts.next(); // UPDATE
    let table = ts.parse_dataset_ref()?;
    expect_kw(ts, "set")?;
    let mut assignments = Vec::new();
    loop {
        let col_tok = ts.peek().clone();
        let Some(col) = col_tok.ident().map(str::to_string) else {
            return Err(SasError::parse(
                "expected a column name in the SET clause",
                col_tok.span,
            ));
        };
        ts.next();
        if ts.peek().kind != TokenKind::Eq {
            return Err(SasError::parse(
                "expected '=' in the SET clause",
                ts.peek().span,
            ));
        }
        ts.next(); // =
        let value = parse_sql_expr(ts)?;
        assignments.push((col, value));
        if ts.peek().kind == TokenKind::Comma {
            ts.next();
        } else {
            break;
        }
    }
    let where_ = if ts.peek().is_kw("where") {
        ts.next();
        Some(parse_sql_expr(ts)?)
    } else {
        None
    };
    Ok(SqlStmt::Update {
        table,
        assignments,
        where_,
    })
}

/// `DELETE FROM <ref> [WHERE <sqlexpr>]`.
pub(super) fn parse_delete(ts: &mut StatementStream) -> Result<SqlStmt> {
    ts.next(); // DELETE
    expect_kw(ts, "from")?;
    let table = ts.parse_dataset_ref()?;
    let where_ = if ts.peek().is_kw("where") {
        ts.next();
        Some(parse_sql_expr(ts)?)
    } else {
        None
    };
    Ok(SqlStmt::DeleteFrom { table, where_ })
}

/// `DESCRIBE TABLE <ref>`.
pub(super) fn parse_describe(ts: &mut StatementStream) -> Result<SqlStmt> {
    ts.next(); // DESCRIBE
    expect_kw(ts, "table")?;
    let table = ts.parse_dataset_ref()?;
    Ok(SqlStmt::Describe(table))
}

/// `INSERT INTO <ref> [(cols)] (VALUES (...) [VALUES (...)...] | <select>)`.
pub(super) fn parse_insert(ts: &mut StatementStream) -> Result<SqlStmt> {
    ts.next(); // INSERT
    expect_kw(ts, "into")?;
    let table = ts.parse_dataset_ref()?;
    // Liste de colonnes optionnelle `(c1, c2, ...)`.
    let mut columns = Vec::new();
    if ts.peek().kind == TokenKind::LParen {
        ts.next(); // (
        loop {
            let col_tok = ts.peek().clone();
            let Some(col) = col_tok.ident().map(str::to_string) else {
                return Err(SasError::parse(
                    "expected a column name in the INSERT column list",
                    col_tok.span,
                ));
            };
            ts.next();
            columns.push(col);
            match ts.peek().kind {
                TokenKind::Comma => {
                    ts.next();
                }
                TokenKind::RParen => {
                    ts.next();
                    break;
                }
                _ => {
                    return Err(SasError::parse(
                        "expected ',' or ')' in the INSERT column list",
                        ts.peek().span,
                    ));
                }
            }
        }
    }
    if ts.peek().is_kw("values") {
        let mut rows = Vec::new();
        while ts.peek().is_kw("values") {
            ts.next(); // VALUES
            rows.push(parse_values_group(ts)?);
        }
        Ok(SqlStmt::InsertValues {
            table,
            columns,
            rows,
        })
    } else if ts.peek().is_kw("select") {
        let query = parse_select(ts)?;
        Ok(SqlStmt::InsertSelect { table, query })
    } else {
        Err(SasError::parse(
            "expected VALUES or SELECT after INSERT INTO",
            ts.peek().span,
        ))
    }
}

/// Un groupe `( expr [, expr]* )` d'un INSERT ... VALUES. Le mot-clé VALUES
/// est déjà consommé. Les valeurs sont des `Expr` de base.
pub(super) fn parse_values_group(ts: &mut StatementStream) -> Result<Vec<Expr>> {
    if ts.peek().kind != TokenKind::LParen {
        return Err(SasError::parse("expected '(' after VALUES", ts.peek().span));
    }
    ts.next(); // (
    let mut vals = Vec::new();
    if ts.peek().kind != TokenKind::RParen {
        loop {
            vals.push(parse_expr(ts)?);
            match ts.peek().kind {
                TokenKind::Comma => {
                    ts.next();
                }
                _ => break,
            }
        }
    }
    if ts.peek().kind != TokenKind::RParen {
        return Err(SasError::parse(
            "expected ',' or ')' in the VALUES list",
            ts.peek().span,
        ));
    }
    ts.next(); // )
    Ok(vals)
}
