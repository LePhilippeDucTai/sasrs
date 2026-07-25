use super::*;

/// Primaires : littéraux, missings, variables/appels, parenthèses.
pub(crate) fn parse_primary(ts: &mut StatementStream) -> Result<Expr> {
    let tok = ts.peek().clone();
    match &tok.kind {
        TokenKind::Num(n) => {
            ts.next();
            Ok(Expr::Num(*n))
        }
        TokenKind::Str { value, suffix } => {
            ts.next();
            literal_from_string(value, *suffix, tok.span)
        }
        TokenKind::Dot => parse_dot(ts),
        TokenKind::LParen => {
            ts.next();
            let inner = parse_expr(ts)?;
            if ts.peek().kind != TokenKind::RParen {
                return Err(SasError::parse(
                    "expected ')'",
                    ts.peek().span,
                ));
            }
            ts.next();
            Ok(inner)
        }
        TokenKind::Ident(name) => {
            let name = name.clone();
            ts.next();
            // `first.var` / `last.var` (insensible casse) : variables
            // automatiques de groupe BY → `Expr::Var("FIRST.<VAR>")`, nom
            // canonique MAJUSCULE. L'adjacence des tokens n'est PAS requise
            // (SAS tolère `first. age`). Tout AUTRE ident suivi d'un `.`
            // reste une erreur de syntaxe (pas de lib.member en expression) :
            // le `.` non consommé fait échouer l'appelant.
            // Appel de méthode d'objet hash en expression (M17.2) :
            // `h.find()`, `h.check()`, ... — `ident . ident (`. Renvoie un
            // `Expr::HashMethod` (code retour numérique). Détecté avant la
            // règle FIRST./LAST. (un hash ne s'appelle jamais `first`/`last`
            // sans parenthèses, et inversement).
            if ts.peek().kind == TokenKind::Dot
                && ts.peek2().ident().is_some()
                && ts.peek_nth(2).kind == TokenKind::LParen
                && !name.eq_ignore_ascii_case("first")
                && !name.eq_ignore_ascii_case("last")
            {
                ts.next(); // `.`
                let method = ts
                    .peek()
                    .ident()
                    .expect("checked an ident after '.'")
                    .to_string();
                ts.next(); // méthode
                ts.next(); // `(`
                let args =
                    crate::parser::datastep::parse_hash_args(ts, &name, &method)?;
                return Ok(Expr::HashMethod(Box::new(crate::ast::HashMethodCall {
                    object: name,
                    method,
                    args,
                })));
            }
            // Attribut d'objet hash sans parenthèses (M17.2) : `h.num_items`.
            // Détecté seulement pour l'attribut connu `num_items` (les autres
            // `ident.ident` sans parenthèse restent une erreur de syntaxe).
            if ts.peek().kind == TokenKind::Dot
                && ts
                    .peek2()
                    .ident()
                    .is_some_and(|m| m.eq_ignore_ascii_case("num_items"))
                && ts.peek_nth(2).kind != TokenKind::LParen
            {
                ts.next(); // `.`
                let method = ts.peek().ident().unwrap().to_string();
                ts.next(); // attribut
                return Ok(Expr::HashMethod(Box::new(crate::ast::HashMethodCall {
                    object: name,
                    method,
                    args: Vec::new(),
                })));
            }
            if ts.peek().kind == TokenKind::Dot
                && (name.eq_ignore_ascii_case("first") || name.eq_ignore_ascii_case("last"))
            {
                ts.next(); // `.`
                let var_tok = ts.peek().clone();
                let Some(v) = var_tok.ident().map(str::to_string) else {
                    return Err(SasError::parse(
                        format!(
                            "expected a BY variable name after {}.",
                            name.to_uppercase()
                        ),
                        var_tok.span,
                    ));
                };
                ts.next();
                return Ok(Expr::Var(format!(
                    "{}.{}",
                    name.to_uppercase(),
                    v.to_uppercase()
                )));
            }
            match ts.peek().kind {
                TokenKind::LParen => parse_call(ts, name),
                // `arr{i}` / `arr[i]` : référence d'array indexée. La forme
                // `arr(i)` reste un Call (ambiguïté résolue à l'évaluation).
                TokenKind::LBrace | TokenKind::LBracket => parse_index(ts, name),
                _ => Ok(Expr::Var(name)),
            }
        }
        _ => Err(SasError::parse(
            "expected an expression",
            tok.span,
        )),
    }
}

/// `Dot` en tête : missing ordinaire, ou missing spécial `.A`..`.Z` / `._`
/// si un ident d'UNE lettre/`_` lui est immédiatement adjacent (spans
/// jointifs). Sinon, le `Dot` seul est consommé et l'ident reste à parser
/// par l'appelant.
pub(crate) fn parse_dot(ts: &mut StatementStream) -> Result<Expr> {
    let dot_tok = ts.peek().clone();
    let dot_end = dot_tok.span.end;
    ts.next(); // consomme le '.'
    let next = ts.peek().clone();
    if let TokenKind::Ident(s) = &next.kind {
        // Adjacent : aucun espace entre le '.' et l'ident.
        if next.span.start == dot_end && s.chars().count() == 1 {
            let c = s.chars().next().unwrap();
            if let Some(kind) = MissingKind::from_letter(c) {
                ts.next(); // consomme l'ident de la lettre
                return Ok(Expr::Missing(kind));
            }
        }
    }
    Ok(Expr::Missing(MissingKind::Dot))
}

/// Lit un token de format/informat (ex. `dollar8.2`, `date9.`, `8.2`,
/// `$char10.`) en consommant le RUN contigu de tokens et en tranchant la
/// source brute — robuste quelle que soit la façon dont le lexer découpe
/// lettres/chiffres/points. Algorithme : on consomme les tokens
/// `Dollar | Ident | Num | Dot` tant qu'ils sont ADJACENTS (le `span.start`
/// de chacun touche le `span.end` du précédent), puis on rend la tranche
/// `src.text[start..end]`.
pub(crate) fn read_format_token(ts: &mut StatementStream) -> Result<String> {
    let start = ts.peek().span.start;
    let mut end = start;
    let mut consumed = false;
    loop {
        let tok = ts.peek();
        let is_fmt_piece = matches!(
            tok.kind,
            TokenKind::Dollar | TokenKind::Ident(_) | TokenKind::Num(_) | TokenKind::Dot
        );
        if !is_fmt_piece {
            break;
        }
        // Le premier morceau est accepté tel quel ; les suivants doivent être
        // adjacents (aucun espace dans la source).
        if consumed && tok.span.start != end {
            break;
        }
        end = tok.span.end;
        consumed = true;
        ts.next();
    }
    if !consumed {
        return Err(SasError::parse("expected a format", ts.peek().span));
    }
    Ok(ts.src.text[start..end].to_string())
}

/// Appel de fonction : `(` déjà en tête de stream, `name` déjà consommé.
pub(crate) fn parse_call(ts: &mut StatementStream, name: String) -> Result<Expr> {
    ts.next(); // (

    // PUT(value, format) / INPUT(source, informat) : le second argument est
    // un TOKEN de format (ex. `dollar8.2`, `date9.`) — pas une expression. On
    // parse le premier argument normalement, puis on lit le token de format
    // brut via `read_format_token` et on le pousse comme `Expr::Str`.
    let upper = name.to_uppercase();
    if upper == "PUT" || upper == "INPUT" {
        let value = parse_expr(ts)?;
        if ts.peek().kind != TokenKind::Comma {
            return Err(SasError::parse(
                format!("expected ',' then a format in the call to {upper}"),
                ts.peek().span,
            ));
        }
        ts.next(); // `,`
        let token = read_format_token(ts)?;
        if ts.peek().kind != TokenKind::RParen {
            return Err(SasError::parse(
                format!("expected ')' after the format in the call to {upper}"),
                ts.peek().span,
            ));
        }
        ts.next(); // `)`
        return Ok(Expr::Call {
            name,
            args: vec![value, Expr::Str(token)],
        });
    }

    let mut args = Vec::new();
    if ts.peek().kind != TokenKind::RParen {
        loop {
            args.push(parse_expr(ts)?);
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
            format!("expected ',' or ')' in call to {}", name.to_uppercase()),
            ts.peek().span,
        ));
    }
    ts.next(); // )
    Ok(Expr::Call { name, args })
}

/// Référence d'array indexée : `{`/`[` en tête de stream, `name` déjà
/// consommé. Le délimiteur fermant doit être assorti à l'ouvrant.
/// Un ou plusieurs indices séparés par des virgules (M16.2,
/// multi-dimensionnel) : `arr{i}` ou `arr{i, j, k}`.
pub(crate) fn parse_index(ts: &mut StatementStream, name: String) -> Result<Expr> {
    let open = ts.next(); // `{` ou `[`
    let closer = match open.kind {
        TokenKind::LBrace => TokenKind::RBrace,
        TokenKind::LBracket => TokenKind::RBracket,
        _ => unreachable!("parse_index called without an opening brace/bracket"),
    };
    let mut indices = Vec::new();
    loop {
        indices.push(parse_expr(ts)?);
        if ts.peek().kind == TokenKind::Comma {
            ts.next(); // `,`
            continue;
        }
        break;
    }
    if ts.peek().kind != closer {
        return Err(SasError::parse(
            format!(
                "expected '{}' to close the array subscript of {}",
                if closer == TokenKind::RBrace { "}" } else { "]" },
                name.to_uppercase()
            ),
            ts.peek().span,
        ));
    }
    ts.next(); // fermant
    Ok(Expr::Index { name, indices })
}
