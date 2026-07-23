//! Statements de contrôle : IF/THEN/ELSE, SELECT, DO (itératif ou non), GOTO, LINK.

use super::*;

/// `goto label;` / `go to label;` (M16.6). Le mot-clé de tête (`goto` ou `go`)
/// a déjà été identifié ; pour `go`, on consomme le `to` qui suit. La cible est
/// un identifiant unique (résolu en MAJUSCULES à la compilation).
pub(super) fn parse_goto(ts: &mut StatementStream, head: &str) -> Result<DsStmt> {
    let kw_tok = ts.peek().clone();
    ts.next(); // `goto` ou `go`
    if head == "go" {
        // Forme `go to label;` : le token suivant DOIT être `to`.
        match ts.peek().ident() {
            Some(w) if w.eq_ignore_ascii_case("to") => {
                ts.next();
            }
            _ => {
                return Err(SasError::parse(
                    "expected TO after GO (use `go to label;` or `goto label;`)",
                    ts.peek().span,
                ));
            }
        }
    }
    let label_tok = ts.peek().clone();
    let label = match label_tok.ident() {
        Some(s) => s.to_string(),
        None => {
            return Err(SasError::parse(
                "expected a statement label after GOTO",
                kw_tok.span,
            ));
        }
    };
    ts.next(); // label
    ts.expect_semi()?;
    Ok(DsStmt::Goto(label))
}

/// `link label;` (M16.6). La cible est un identifiant unique (résolu en
/// MAJUSCULES à la compilation).
pub(super) fn parse_link(ts: &mut StatementStream) -> Result<DsStmt> {
    let kw_tok = ts.peek().clone();
    ts.next(); // `link`
    let label_tok = ts.peek().clone();
    let label = match label_tok.ident() {
        Some(s) => s.to_string(),
        None => {
            return Err(SasError::parse(
                "expected a statement label after LINK",
                kw_tok.span,
            ));
        }
    };
    ts.next(); // label
    ts.expect_semi()?;
    Ok(DsStmt::Link(label))
}

/// `if expr then stmt [else stmt]` ou `if expr ;` (subsetting).
pub(super) fn parse_if(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `if`
    let cond = super::expr::parse_expr(ts)?;
    if ts.peek().is_kw("then") {
        ts.next(); // `then`
        let then_branch = Box::new(parse_branch_statement(ts)?);
        let else_branch = if ts.peek().is_kw("else") {
            ts.next(); // `else`
            Some(Box::new(parse_branch_statement(ts)?))
        } else {
            None
        };
        Ok(DsStmt::If {
            cond,
            then_branch,
            else_branch,
        })
    } else if ts.peek().kind == TokenKind::Semi {
        ts.next(); // `;`
        Ok(DsStmt::SubsettingIf(cond))
    } else {
        Err(SasError::parse(
            "expected THEN or ';' after the IF condition",
            ts.peek().span,
        ))
    }
}

/// Une branche de IF/THEN ou IF/ELSE : UN statement (récursion). Les
/// frontières de bloc ou `run`/`quit` ne peuvent pas servir de branche.
fn parse_branch_statement(ts: &mut StatementStream) -> Result<DsStmt> {
    let tok = ts.peek().clone();
    if let Some(s) = tok.ident() {
        let lower = s.to_ascii_lowercase();
        if lower == "run" || lower == "quit" || is_block_head_kw(&lower) {
            return Err(SasError::parse(
                "expected a statement after THEN/ELSE",
                tok.span,
            ));
        }
    }
    parse_statement(ts)
}

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
pub(super) fn parse_select(ts: &mut StatementStream) -> Result<DsStmt> {
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
                    return Err(SasError::parse(
                        "missing END for SELECT block.",
                        tok.span,
                    ));
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
fn parse_when_values(ts: &mut StatementStream, selector_form: bool) -> Result<Vec<Expr>> {
    let open = ts.peek().clone();
    if open.kind != TokenKind::LParen {
        return Err(SasError::parse(
            "expected '(' after WHEN",
            open.span,
        ));
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
fn parse_select_branch(ts: &mut StatementStream) -> Result<DsStmt> {
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

/// `do ...; stmts end ;` — quatre formes :
/// - `do;` : bloc non itératif → `DsStmt::Block` (chemin M1 conservé) ;
/// - `do i = e1 [to e2] [by e3] [while(c)] [until(c)];` : itératif ;
/// - `do while(c);` / `do until(c);` : conditionnel pur.
///
/// `do i = 1, 5, 9;` (liste de valeurs, y compris à UNE valeur sans
/// clause TO/BY/WHILE/UNTIL) → ERROR "not yet implemented" propre.
/// `while` et `until` ne sont pas réservés : `do while = 1 to 2;` reste
/// un DO itératif d'index `while` (le `=` est inspecté avant le `(`).
pub(super) fn parse_do(ts: &mut StatementStream) -> Result<DsStmt> {
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
fn parse_iterative_do(ts: &mut StatementStream, index_name: String) -> Result<DsStmt> {
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
fn make_do_list_item(
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
fn parse_paren_cond(ts: &mut StatementStream) -> Result<Expr> {
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
fn parse_do_body(ts: &mut StatementStream) -> Result<Vec<DsStmt>> {
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
