use super::*;

/// `select [(expr)]; when (...) stmt; ... [otherwise stmt;] end;` (M16.1).
///
/// Forme SÉLECTEUR : `select (expr);` — l'expression entre parenthèses est
/// évaluée une fois, puis chaque `when (v1, v2, ...)` compare le sélecteur à
/// la liste de valeurs (sémantique `=` de SAS). Forme BOOLÉENNE :
/// `select;` — chaque `when (cond)` est une condition booléenne, la première
/// vraie l'emporte. `otherwise` est optionnelle ; `end;` clôt le bloc.
///
/// Le corps d'un WHEN/OTHERWISE est UN statement (comme une branche THEN) :
/// `do; ... end;` pour plusieurs statements. Un WHEN/OTHERWISE sans corps
/// (immédiatement suivi de `;`) est licite (no-op) en SAS.
pub(crate) fn parse_select(ts: &mut StatementStream) -> Result<DsStmt> {
    let select_tok = ts.peek().clone();
    ts.next(); // `select`

    // Forme sélecteur : `(expr)` optionnel avant le `;`.
    let selector = if ts.peek().kind == TokenKind::LParen {
        ts.next(); // `(`
        let expr = super::expr::parse_expr(ts)?;
        if ts.peek().kind != TokenKind::RParen {
            return Err(SasError::parse(
                "expected ')' after the SELECT expression",
                ts.peek().span,
            ));
        }
        ts.next(); // `)`
        Some(expr)
    } else {
        None
    };
    ts.expect_semi()?;

    let mut whens: Vec<WhenClause> = Vec::new();
    let mut otherwise: Option<Box<DsStmt>> = None;
    let selector_form = selector.is_some();

    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Eof => {
                return Err(SasError::parse(
                    "missing END for SELECT block.",
                    select_tok.span,
                ));
            }
            // `;` superflus entre clauses (et un éventuel commentaire `*` géré
            // par le lexer en amont).
            TokenKind::Semi => {
                ts.next();
            }
            TokenKind::Ident(s) if s.eq_ignore_ascii_case("when") => {
                if otherwise.is_some() {
                    return Err(SasError::parse(
                        "WHEN is not allowed after OTHERWISE in a SELECT block.",
                        tok.span,
                    ));
                }
                ts.next(); // `when`
                let values = parse_when_values(ts, selector_form)?;
                let body = Box::new(parse_select_branch(ts)?);
                whens.push(WhenClause { values, body });
            }
            TokenKind::Ident(s) if s.eq_ignore_ascii_case("otherwise") => {
                if otherwise.is_some() {
                    return Err(SasError::parse(
                        "Only one OTHERWISE clause is allowed in a SELECT block.",
                        tok.span,
                    ));
                }
                ts.next(); // `otherwise`
                otherwise = Some(Box::new(parse_select_branch(ts)?));
            }
            TokenKind::Ident(s) if s.eq_ignore_ascii_case("end") => {
                ts.next(); // `end`
                ts.expect_semi()?;
                return Ok(DsStmt::Select {
                    selector,
                    whens,
                    otherwise,
                });
            }
            TokenKind::Ident(s) => {
                let lower = s.to_ascii_lowercase();
                if lower == "run" || lower == "quit" || is_block_head_kw(&lower) {
                    return Err(SasError::parse("missing END for SELECT block.", tok.span));
                }
                return Err(SasError::parse(
                    "expected WHEN, OTHERWISE or END inside the SELECT block",
                    tok.span,
                ));
            }
            _ => {
                return Err(SasError::parse(
                    "expected WHEN, OTHERWISE or END inside the SELECT block",
                    tok.span,
                ));
            }
        }
    }
}

/// `( v1 [, v2 ...] )` après un WHEN. En forme booléenne (sans sélecteur),
/// une seule expression (la condition) est autorisée. La liste vide `when ()`
/// est rejetée.
pub(crate) fn parse_when_values(
    ts: &mut StatementStream,
    selector_form: bool,
) -> Result<Vec<Expr>> {
    let open = ts.peek().clone();
    if open.kind != TokenKind::LParen {
        return Err(SasError::parse("expected '(' after WHEN", open.span));
    }
    ts.next(); // `(`
    if ts.peek().kind == TokenKind::RParen {
        return Err(SasError::parse(
            "expected at least one value in the WHEN list",
            ts.peek().span,
        ));
    }
    let mut values = vec![super::expr::parse_expr(ts)?];
    while ts.peek().kind == TokenKind::Comma {
        if !selector_form {
            return Err(SasError::parse(
                "a WHEN condition in a boolean SELECT (no selector) takes a single expression",
                ts.peek().span,
            ));
        }
        ts.next(); // `,`
        values.push(super::expr::parse_expr(ts)?);
    }
    if ts.peek().kind != TokenKind::RParen {
        return Err(SasError::parse(
            "expected ',' or ')' in the WHEN list",
            ts.peek().span,
        ));
    }
    ts.next(); // `)`
    Ok(values)
}

/// Corps d'un WHEN/OTHERWISE : UN statement, ou rien (`;` immédiat → no-op,
/// rendu comme un bloc vide). `run`/`quit`/frontière de bloc ne peuvent pas
/// servir de corps.
pub(crate) fn parse_select_branch(ts: &mut StatementStream) -> Result<DsStmt> {
    if ts.peek().kind == TokenKind::Semi {
        ts.next(); // `;` — corps vide
        return Ok(DsStmt::Block(Vec::new()));
    }
    let tok = ts.peek().clone();
    if let Some(s) = tok.ident() {
        let lower = s.to_ascii_lowercase();
        if lower == "run" || lower == "quit" || lower == "end" || is_block_head_kw(&lower) {
            return Err(SasError::parse(
                "expected a statement after WHEN/OTHERWISE",
                tok.span,
            ));
        }
    }
    parse_statement(ts)
}
