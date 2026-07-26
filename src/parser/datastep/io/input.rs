use super::*;

/// `input <items> ;` (M14). Modes pris en charge :
/// - liste : `name $ age` ;
/// - colonne : `name $ 1-10 age 11-13` ;
/// - formaté : `name $char10. d date9.` ;
/// - pointeurs `@n`, `+n`, `/`, hold `@`/`@@`, modificateur `:`.
///
/// On lit les tokens jusqu'au `;` final (consommé). Le `$` se rapporte à la
/// variable qui PRÉCÈDE (forme `name $`).
pub(crate) fn parse_input(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `input`
    let mut items: Vec<InputItem> = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi => {
                ts.next();
                return Ok(DsStmt::Input(items));
            }
            // `@@` (double hold) ou `@n` (pointeur de colonne) ou `@` (hold).
            TokenKind::At => {
                let at_end = tok.span.end;
                ts.next(); // `@`
                if ts.peek().kind == TokenKind::At {
                    ts.next(); // second `@`
                    items.push(InputItem::HoldLineDouble);
                } else if let TokenKind::Num(n) = ts.peek().kind {
                    // `@n` : pointeur ADJACENT (`@5`) ou espacé (`@ 5`) —
                    // SAS tolère les deux.
                    if n.fract() != 0.0 || n < 1.0 {
                        return Err(SasError::parse(
                            "the column pointer @n must be a positive integer",
                            ts.peek().span,
                        ));
                    }
                    ts.next();
                    items.push(InputItem::ColumnPointer(n as usize));
                } else {
                    // `@` final (hold simple) — doit être suivi du `;`.
                    let _ = at_end;
                    items.push(InputItem::HoldLine);
                }
            }
            // `+n` : avance relative du curseur.
            TokenKind::Plus => {
                ts.next(); // `+`
                let n_tok = ts.peek().clone();
                let TokenKind::Num(n) = n_tok.kind else {
                    return Err(SasError::parse(
                        "expected a positive integer after '+' in the INPUT statement",
                        n_tok.span,
                    ));
                };
                if n.fract() != 0.0 || n < 0.0 {
                    return Err(SasError::parse(
                        "the column skip +n must be a non-negative integer",
                        n_tok.span,
                    ));
                }
                ts.next();
                items.push(InputItem::SkipColumns(n as usize));
            }
            // `/` : passage à la ligne d'entrée suivante.
            TokenKind::Slash => {
                ts.next();
                items.push(InputItem::NextLine);
            }
            // Un nom de variable, éventuellement suivi de `$`, de colonnes
            // `a-b`, d'un `:`-modificateur et/ou d'un informat.
            TokenKind::Ident(name) => {
                let name = name.clone();
                validate_sas_name(&name, tok.span)?;
                ts.next();
                let item = parse_input_var(ts, name)?;
                items.push(item);
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable name, column pointer or ';' in the INPUT statement",
                    tok.span,
                ));
            }
        }
    }
}

/// Suffixe d'une variable INPUT : `[$] [a-b | [:] informat]`.
pub(crate) fn parse_input_var(ts: &mut StatementStream, name: String) -> Result<InputItem> {
    let mut is_char = false;
    // `$` : variable caractère. Deux cas :
    // - `$char10.` / `$10.` : le `$` ouvre un INFORMAT caractère (adjacent à
    //   un Ident/Num) — on NE le consomme PAS ici, `read_format_token` le
    //   lira en entier.
    // - `$` isolé (suivi d'un espace, de colonnes, ou de la variable
    //   suivante) : simple marqueur caractère du mode liste/colonne.
    if ts.peek().kind == TokenKind::Dollar && !dollar_begins_informat(ts) {
        ts.next();
        is_char = true;
    }
    // `:` modificateur d'informat en mode liste.
    let mut list_modifier = false;
    if ts.peek().kind == TokenKind::Colon {
        ts.next();
        list_modifier = true;
    }
    // Mode colonne : `a-b` (a et b entiers, a-b adjacents au `-`).
    if let TokenKind::Num(a) = ts.peek().kind {
        // Distinguer `a-b` (colonnes) d'un informat `8.` : un informat a un
        // `.` ; les colonnes ont un `-`. On regarde le token suivant.
        if a.fract() == 0.0 && a >= 1.0 && ts.peek2().kind == TokenKind::Minus {
            ts.next(); // a
            ts.next(); // `-`
            let b_tok = ts.peek().clone();
            let TokenKind::Num(b) = b_tok.kind else {
                return Err(SasError::parse(
                    "expected the end column after '-' in the INPUT statement",
                    b_tok.span,
                ));
            };
            if b.fract() != 0.0 || b < a {
                return Err(SasError::parse(
                    "invalid column range in the INPUT statement",
                    b_tok.span,
                ));
            }
            ts.next();
            return Ok(InputItem::Var {
                name,
                is_char,
                cols: Some((a as usize, b as usize)),
                informat: None,
                list_modifier,
            });
        }
    }
    // Mode formaté : un informat suit (token de format `date9.`, `8.2`,
    // `$char10.`, etc.). On le détecte par adjacence (comme FORMAT).
    if input_informat_follows(ts) {
        let token = super::expr::read_format_token(ts)?;
        return Ok(InputItem::Var {
            name,
            is_char,
            cols: None,
            informat: Some(token),
            list_modifier,
        });
    }
    // Mode liste pur.
    Ok(InputItem::Var {
        name,
        is_char,
        cols: None,
        informat: None,
        list_modifier,
    })
}

/// Vrai si le `$` courant ouvre un informat caractère (`$char10.`, `$10.`,
/// `$.`) : le token ADJACENT est un Ident ou un Num (qui formera le reste de
/// l'informat). Un `$` isolé (suivi d'espace ou d'un nombre non adjacent =
/// colonnes) reste un simple marqueur caractère.
pub(crate) fn dollar_begins_informat(ts: &StatementStream) -> bool {
    let cur = ts.peek();
    let next = ts.peek2();
    next.span.start == cur.span.end && matches!(next.kind, TokenKind::Ident(_) | TokenKind::Num(_))
}

/// Vrai si un informat suit (mode formaté) : un `$`, un nombre porteur d'un
/// point décimal (`5.2`, lexé en `Num(5.2)`) ou suivi d'un `.` adjacent
/// (`8.`), ou un Ident adjacent à un morceau de format (`date9.`). Le cas du
/// nombre suivi d'un `-` (plage de colonnes) est déjà traité plus haut.
pub(crate) fn input_informat_follows(ts: &StatementStream) -> bool {
    let cur = ts.peek();
    match &cur.kind {
        // `$char10.` : le `$` ouvre un informat caractère.
        TokenKind::Dollar => true,
        TokenKind::Num(n) => {
            // `5.2` : le point décimal est DANS le token (partie fractionnaire).
            if n.fract() != 0.0 {
                return true;
            }
            // `8.` : un `.` adjacent suit le nombre entier.
            let next = ts.peek2();
            next.span.start == cur.span.end && next.kind == TokenKind::Dot
        }
        // `date9.` : un Ident dont le morceau suivant adjacent est un format.
        TokenKind::Ident(_) => ident_begins_format(ts),
        _ => false,
    }
}
