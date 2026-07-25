use super::*;

/// `file <dest> ;` (M14.2). La destination est un littéral chemin
/// (`'sortie.txt'`) OU le mot-clé `log` / `print`. Toute autre forme →
/// erreur claire. (Les options FILE — `LRECL=`, `MOD`... — ne sont pas
/// supportées.)
pub(crate) fn parse_file(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `file`
    let tok = ts.peek().clone();
    let dest = match &tok.kind {
        TokenKind::Str {
            value,
            suffix: StrSuffix::None | StrSuffix::Name,
        } => {
            let s = value.clone();
            ts.next();
            PutDest::Path(s)
        }
        TokenKind::Ident(name) if name.eq_ignore_ascii_case("log") => {
            ts.next();
            PutDest::Log
        }
        TokenKind::Ident(name) if name.eq_ignore_ascii_case("print") => {
            ts.next();
            PutDest::Print
        }
        _ => {
            return Err(SasError::parse(
                "expected a quoted file path, LOG or PRINT after FILE",
                tok.span,
            ));
        }
    };
    ts.expect_semi()?;
    Ok(DsStmt::File { dest })
}

/// `put <items> ;` (M14.2). Miroir de sortie d'`input`. Modes pris en
/// charge :
/// - liste : `put name age` (format d'affichage de chaque variable) ;
/// - nommé : `put name= age=` (`name=VALEUR`) ;
/// - littéral : `put 'Report for' name` ;
/// - formaté : `put x 8.2` / `put d date9.` ;
/// - pointeurs `@n`, `+n`, `/`, hold `@`/`@@`, et `put _all_;`.
///
/// On lit les tokens jusqu'au `;` final (consommé).
pub(crate) fn parse_put(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `put`
    let mut items: Vec<PutItem> = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi => {
                ts.next();
                return Ok(DsStmt::Put(items));
            }
            // `@@` (double hold), `@n` (pointeur de colonne) ou `@` (hold).
            TokenKind::At => {
                ts.next(); // `@`
                if ts.peek().kind == TokenKind::At {
                    ts.next(); // second `@`
                    items.push(PutItem::HoldLineDouble);
                } else if let TokenKind::Num(n) = ts.peek().kind {
                    if n.fract() != 0.0 || n < 1.0 {
                        return Err(SasError::parse(
                            "the column pointer @n must be a positive integer",
                            ts.peek().span,
                        ));
                    }
                    ts.next();
                    items.push(PutItem::ColumnPointer(n as usize));
                } else {
                    // `@` final (hold simple) — doit être suivi du `;`.
                    items.push(PutItem::HoldLine);
                }
            }
            // `+n` : avance relative du curseur.
            TokenKind::Plus => {
                ts.next(); // `+`
                let n_tok = ts.peek().clone();
                let TokenKind::Num(n) = n_tok.kind else {
                    return Err(SasError::parse(
                        "expected a positive integer after '+' in the PUT statement",
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
                items.push(PutItem::SkipColumns(n as usize));
            }
            // `/` : passage à la ligne de sortie suivante.
            TokenKind::Slash => {
                ts.next();
                items.push(PutItem::NextLine);
            }
            // Un littéral chaîne : écrit verbatim.
            TokenKind::Str {
                value,
                suffix: StrSuffix::None | StrSuffix::Name,
            } => {
                let s = value.clone();
                ts.next();
                items.push(PutItem::Literal(s));
            }
            // Un nom de variable : forme nommée (`name=`), formatée
            // (`name 8.2`) ou liste (`name`). `_all_` est un cas spécial.
            TokenKind::Ident(name) => {
                let name = name.clone();
                if name.eq_ignore_ascii_case("_all_") {
                    ts.next();
                    items.push(PutItem::All);
                    continue;
                }
                validate_sas_name(&name, tok.span)?;
                ts.next();
                items.push(parse_put_var(ts, name)?);
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable, a literal, a column pointer or ';' in the PUT statement",
                    tok.span,
                ));
            }
        }
    }
}

/// Suffixe d'une variable PUT : `[= | format]`.
/// - `name=` : forme nommée (`name=VALEUR`). On distingue du début d'une
///   assignation : dans un PUT, `name=` n'est jamais suivi d'une expression
///   significative — l'item suivant est un autre item PUT ou le `;`.
/// - `name fmt.` : forme formatée (format adjacent comme dans FORMAT/INPUT).
/// - sinon : forme liste (format d'affichage par défaut de la variable).
pub(crate) fn parse_put_var(ts: &mut StatementStream, name: String) -> Result<PutItem> {
    // Forme nommée `name=`.
    if ts.peek().kind == TokenKind::Eq {
        ts.next(); // `=`
        return Ok(PutItem::NamedVar(name));
    }
    // Forme formatée : un format suit (token adjacent `$`, `8.2`, `date9.`).
    if put_format_follows(ts) {
        let token = super::expr::read_format_token(ts)?;
        return Ok(PutItem::Var {
            name,
            format: Some(token),
        });
    }
    // Forme liste pure.
    Ok(PutItem::Var { name, format: None })
}

/// Vrai si un format suit (forme formatée d'un item PUT). Identique à
/// `input_informat_follows` : un `$`, un nombre fractionnaire (`5.2`) ou
/// entier suivi d'un `.` adjacent (`8.`), ou un Ident adjacent à un morceau
/// de format (`date9.`).
pub(crate) fn put_format_follows(ts: &StatementStream) -> bool {
    let cur = ts.peek();
    match &cur.kind {
        TokenKind::Dollar => true,
        TokenKind::Num(n) => {
            if n.fract() != 0.0 {
                return true;
            }
            let next = ts.peek2();
            next.span.start == cur.span.end && next.kind == TokenKind::Dot
        }
        TokenKind::Ident(_) => ident_begins_format(ts),
        _ => false,
    }
}
