use super::*;

/// `do ...; stmts end ;` — quatre formes :
/// - `do;` : bloc non itératif → `DsStmt::Block` (chemin M1 conservé) ;
/// - `do i = e1 [to e2] [by e3] [while(c)] [until(c)];` : itératif ;
/// - `do while(c);` / `do until(c);` : conditionnel pur.
///
/// `do i = 1, 5, 9;` (liste de valeurs, y compris à UNE valeur sans
/// clause TO/BY/WHILE/UNTIL) → ERROR "not yet implemented" propre.
/// `while` et `until` ne sont pas réservés : `do while = 1 to 2;` reste
/// un DO itératif d'index `while` (le `=` est inspecté avant le `(`).
pub(crate) fn parse_do(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `do`
    let head = ts.peek().clone();
    match &head.kind {
        // Forme non itérative : `do` immédiatement suivi de `;`.
        TokenKind::Semi => {
            ts.next(); // `;`
            Ok(DsStmt::Block(parse_do_body(ts)?))
        }
        TokenKind::Ident(name) => {
            let name = name.clone();
            let lower = name.to_ascii_lowercase();
            // `do over arr;` : boucle implicite sur un array. `over` n'est pas
            // un mot réservé — on ne le reconnaît que s'il est suivi d'un
            // identifiant d'array et d'un `;` (sinon `over` serait un index).
            if lower == "over" {
                if let TokenKind::Ident(arr) = &ts.peek_nth(1).kind {
                    if ts.peek_nth(2).kind == TokenKind::Semi {
                        let arr = arr.clone();
                        ts.next(); // `over`
                        ts.next(); // nom d'array
                        ts.expect_semi()?;
                        let body = parse_do_body(ts)?;
                        return Ok(DsStmt::DoOver { array: arr, body });
                    }
                }
            }
            ts.next(); // l'ident (index potentiel, ou while/until)
            if ts.peek().kind == TokenKind::Eq {
                // `do i = ...` : itératif ou liste de valeurs.
                validate_sas_name(&name, head.span)?;
                ts.next(); // `=`
                parse_iterative_do(ts, name)
            } else if (lower == "while" || lower == "until")
                && ts.peek().kind == TokenKind::LParen
            {
                // `do while(c);` / `do until(c);` : conditionnel pur.
                let cond = parse_paren_cond(ts)?;
                ts.expect_semi()?;
                let body = parse_do_body(ts)?;
                let (while_, until) = if lower == "while" {
                    (Some(cond), None)
                } else {
                    (None, Some(cond))
                };
                Ok(DsStmt::DoLoop {
                    index: None,
                    to: None,
                    by: None,
                    while_,
                    until,
                    body,
                })
            } else {
                Err(SasError::parse(
                    "expected '=', WHILE(...) or UNTIL(...) after DO",
                    head.span,
                ))
            }
        }
        _ => Err(SasError::parse(
            "expected ';', an index variable, WHILE(...) or UNTIL(...) after DO",
            head.span,
        )),
    }
}

/// Clauses d'un DO itératif après `do index =`. Deux formes possibles :
///
/// - Itératif classique : `from [to e] [by e] [while(c)] [until(c)]` (TO/BY
///   dans les deux ordres, UN seul de chaque) → `DsStmt::DoLoop`.
/// - Liste de valeurs (M16.3) : `v1, v2, v3` où chaque `vk` est une valeur
///   explicite OU une sous-liste `from to e [by k]`, séparées par des
///   virgules → `DsStmt::DoList`. Une valeur unique sans clause (`do i = 1;`)
///   est aussi une liste (à un élément).
///
/// On parse d'abord le premier segment (`from` + clauses éventuelles). S'il
/// n'y a NI virgule NI valeur unique nue, c'est le DO itératif classique
/// (qui seul porte WHILE/UNTIL). Sinon c'est une liste de valeurs.
pub(crate) fn parse_iterative_do(ts: &mut StatementStream, index_name: String) -> Result<DsStmt> {
    let from = super::expr::parse_expr(ts)?;
    let mut to: Option<Expr> = None;
    let mut by: Option<Expr> = None;
    let mut while_: Option<Expr> = None;
    let mut until: Option<Expr> = None;
    loop {
        let tok = ts.peek().clone();
        let Some(kw) = tok.ident().map(str::to_ascii_lowercase) else {
            break;
        };
        match kw.as_str() {
            "to" if to.is_none() => {
                ts.next();
                to = Some(super::expr::parse_expr(ts)?);
            }
            "by" if by.is_none() => {
                ts.next();
                by = Some(super::expr::parse_expr(ts)?);
            }
            "while" if while_.is_none() => {
                ts.next();
                while_ = Some(parse_paren_cond(ts)?);
            }
            "until" if until.is_none() => {
                ts.next();
                until = Some(parse_paren_cond(ts)?);
            }
            "to" | "by" | "while" | "until" => {
                return Err(SasError::parse(
                    format!("duplicate {} clause in the DO statement", kw.to_uppercase()),
                    tok.span,
                ));
            }
            _ => break,
        }
    }

    let has_comma = ts.peek().kind == TokenKind::Comma;

    // Forme itérative classique : au moins UNE clause TO/BY/WHILE/UNTIL et
    // PAS de virgule en suite. WHILE/UNTIL n'existent que dans cette forme.
    if !has_comma && (to.is_some() || by.is_some() || while_.is_some() || until.is_some()) {
        ts.expect_semi()?;
        let body = parse_do_body(ts)?;
        return Ok(DsStmt::DoLoop {
            index: Some((index_name, from)),
            to,
            by,
            while_,
            until,
            body,
        });
    }

    // Forme liste de valeurs (M16.3). WHILE/UNTIL y sont illégaux.
    if while_.is_some() || until.is_some() {
        return Err(SasError::parse(
            "WHILE/UNTIL are not allowed in a DO statement over a list of values.",
            ts.peek().span,
        ));
    }
    let mut items: Vec<DoListItem> = Vec::new();
    // Le premier segment est déjà parsé : valeur unique, ou sous-liste si TO
    // (le BY ne peut apparaître que conjointement à TO).
    items.push(make_do_list_item(from, to, by, ts.peek().span)?);
    while ts.peek().kind == TokenKind::Comma {
        ts.next(); // `,`
        let v = super::expr::parse_expr(ts)?;
        let (mut t, mut b): (Option<Expr>, Option<Expr>) = (None, None);
        loop {
            let tok = ts.peek().clone();
            let Some(kw) = tok.ident().map(str::to_ascii_lowercase) else {
                break;
            };
            match kw.as_str() {
                "to" if t.is_none() => {
                    ts.next();
                    t = Some(super::expr::parse_expr(ts)?);
                }
                "by" if b.is_none() => {
                    ts.next();
                    b = Some(super::expr::parse_expr(ts)?);
                }
                "to" | "by" => {
                    return Err(SasError::parse(
                        format!("duplicate {} clause in the DO statement", kw.to_uppercase()),
                        tok.span,
                    ));
                }
                _ => break,
            }
        }
        items.push(make_do_list_item(v, t, b, ts.peek().span)?);
    }
    // WHILE/UNTIL en fin de liste (`do i = 1, 3 while(x);`) sont illégaux.
    if let Some(kw) = ts.peek().ident().map(str::to_ascii_lowercase) {
        if kw == "while" || kw == "until" {
            return Err(SasError::parse(
                "WHILE/UNTIL are not allowed in a DO statement over a list of values.",
                ts.peek().span,
            ));
        }
    }
    ts.expect_semi()?;
    let body = parse_do_body(ts)?;
    Ok(DsStmt::DoList {
        index: index_name,
        items,
        body,
    })
}

/// Construit un `DoListItem` à partir d'un segment de liste de valeurs :
/// `from` seul → `Value` ; `from to to_ [by by_]` → `Range`. Un `BY` sans
/// `TO` est une erreur de syntaxe.
pub(crate) fn make_do_list_item(
    from: Expr,
    to: Option<Expr>,
    by: Option<Expr>,
    span: Span,
) -> Result<DoListItem> {
    match to {
        Some(t) => Ok(DoListItem::Range { from, to: t, by }),
        None => {
            if by.is_some() {
                return Err(SasError::parse(
                    "BY without TO in a DO statement value list.",
                    span,
                ));
            }
            Ok(DoListItem::Value(from))
        }
    }
}

/// `( expr )` après WHILE/UNTIL.
pub(crate) fn parse_paren_cond(ts: &mut StatementStream) -> Result<Expr> {
    let tok = ts.peek().clone();
    if tok.kind != TokenKind::LParen {
        return Err(SasError::parse(
            "expected '(' after WHILE/UNTIL in the DO statement",
            tok.span,
        ));
    }
    ts.next(); // `(`
    let cond = super::expr::parse_expr(ts)?;
    let tok = ts.peek().clone();
    if tok.kind != TokenKind::RParen {
        return Err(SasError::parse(
            "expected ')' after the WHILE/UNTIL condition",
            tok.span,
        ));
    }
    ts.next(); // `)`
    Ok(cond)
}

/// Corps d'un DO (toutes formes) : statements jusqu'au `end ;` (consommé).
pub(crate) fn parse_do_body(ts: &mut StatementStream) -> Result<Vec<DsStmt>> {
    let mut body = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Eof => {
                return Err(SasError::parse(
                    "missing END for DO block.",
                    tok.span,
                ));
            }
            TokenKind::Semi => {
                ts.next();
            }
            TokenKind::Star => {
                ts.skip_to_semi();
            }
            TokenKind::Ident(s) if s.eq_ignore_ascii_case("end") => {
                ts.next(); // `end`
                ts.expect_semi()?;
                return Ok(body);
            }
            TokenKind::Ident(s) => {
                let lower = s.to_ascii_lowercase();
                if lower == "run" || lower == "quit" || is_block_head_kw(&lower) {
                    // Frontière atteinte sans END : DO non clos.
                    return Err(SasError::parse(
                        "missing END for DO block.",
                        tok.span,
                    ));
                }
                body.push(parse_statement(ts)?);
            }
            _ => {
                return Err(SasError::parse(
                    "expected a DATA step statement inside DO block",
                    tok.span,
                ));
            }
        }
    }
}
