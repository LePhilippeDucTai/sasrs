//! Statement ARRAY : déclaration, plages numérotées `x1-x3`, assignation indexée.

use super::*;

/// Fin commune d'une assignation indexée : l'indice est parsé, il reste
/// `= expr ;`.
pub(super) fn parse_assign_indexed_tail(
    ts: &mut StatementStream,
    array: String,
    indices: Vec<Expr>,
) -> Result<DsStmt> {
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            format!(
                "expected '=' after the array reference {}",
                array.to_uppercase()
            ),
            ts.peek().span,
        ));
    }
    ts.next(); // `=`
    let expr = super::expr::parse_expr(ts)?;
    ts.expect_semi()?;
    Ok(DsStmt::AssignIndexed {
        array,
        indices,
        expr,
    })
}

/// `array arr{3} x y z;` — déclaration d'array 1-D (M2). Délimiteurs de
/// dimension interchangeables (`{}`, `[]`, `()` — fermant assorti).
/// Formes : `{n}` taille explicite, `{*}` taille déduite de la liste ;
/// `$ [len]` array caractère (longueur défaut 8) ; liste de variables
/// optionnelle (vide → éléments auto-nommés à la compilation), plages
/// numérotées `x1-x3` expansées ICI. M16.2 ajoute : dimensions multiples
/// `{2,3}`, valeurs initiales `(1, 2, 3)` en ordre row-major, `_TEMPORARY_`
/// et listes spéciales `_NUMERIC_`/`_CHARACTER_`/`_ALL_`.
pub(super) fn parse_array(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `array`
    let name_tok = ts.peek().clone();
    let Some(name) = name_tok.ident().map(str::to_string) else {
        return Err(SasError::parse(
            "expected an array name in the ARRAY statement",
            name_tok.span,
        ));
    };
    validate_sas_name(&name, name_tok.span)?;
    ts.next();

    let dims = parse_array_dims(ts)?;
    let char_len = parse_array_char_len(ts)?;
    parse_array_elements(ts, name, dims, char_len)
}

/// Phase 1 de `parse_array` — dimensions : `{n}`, `{n, m, ...}`, `[n]`,
/// `(n)` ou `{*}` (délimiteur fermant assorti). `None` ⟺ `{*}`.
fn parse_array_dims(ts: &mut StatementStream) -> Result<Option<Vec<usize>>> {
    // ── Dimensions : `{n}`, `{n, m, ...}`, `[n]`, `(n)` ou `{*}` ─────────
    let open = ts.peek().clone();
    let closer = match open.kind {
        TokenKind::LBrace => TokenKind::RBrace,
        TokenKind::LBracket => TokenKind::RBracket,
        TokenKind::LParen => TokenKind::RParen,
        _ => {
            return Err(SasError::parse(
                "expected '{', '[' or '(' after the array name",
                open.span,
            ));
        }
    };
    ts.next(); // ouvrant
    // `dims = None` ⟺ une seule dimension `{*}` (taille déduite de la
    // liste) ; sinon une ou plusieurs bornes supérieures explicites.
    let mut dims: Option<Vec<usize>> = None;
    {
        let mut collected: Vec<usize> = Vec::new();
        loop {
            let dim_tok = ts.peek().clone();
            match dim_tok.kind {
                TokenKind::Star => {
                    if !collected.is_empty() {
                        return Err(SasError::parse(
                            "'*' is only allowed as the sole array dimension",
                            dim_tok.span,
                        ));
                    }
                    ts.next();
                    // `dims` reste None : taille déduite de la liste (1-D).
                }
                TokenKind::Num(n) => {
                    if n.fract() != 0.0 || n < 1.0 {
                        return Err(SasError::parse(
                            "the array dimension must be a positive integer",
                            dim_tok.span,
                        ));
                    }
                    ts.next();
                    collected.push(n as usize);
                }
                _ => {
                    return Err(SasError::parse(
                        "expected a dimension or '*' in the ARRAY statement",
                        dim_tok.span,
                    ));
                }
            }
            if ts.peek().kind == TokenKind::Comma {
                ts.next(); // `,`
                continue;
            }
            break;
        }
        if !collected.is_empty() {
            dims = Some(collected);
        }
    }
    if ts.peek().kind != closer {
        return Err(SasError::parse(
            "expected the matching closing delimiter of the array dimension",
            ts.peek().span,
        ));
    }
    ts.next(); // fermant
    Ok(dims)
}

/// Phase 2 de `parse_array` — `$ [len]` optionnel : array caractère,
/// longueur défaut 8. `None` = array numérique.
fn parse_array_char_len(ts: &mut StatementStream) -> Result<Option<usize>> {
    // ── `$ [len]` : array caractère, longueur défaut 8 ──────────────────
    let mut char_len: Option<usize> = None;
    if ts.peek().kind == TokenKind::Dollar {
        ts.next(); // `$`
        char_len = Some(8);
        if let TokenKind::Num(n) = ts.peek().kind {
            let num_span = ts.peek().span;
            if n.fract() != 0.0 || n < 1.0 {
                return Err(SasError::parse(
                    "the length in an ARRAY statement must be a positive integer",
                    num_span,
                ));
            }
            ts.next();
            char_len = Some(n as usize);
        }
    }
    Ok(char_len)
}

/// Phase 3 de `parse_array` — liste de variables (plages `x1-x3` expansées),
/// mots-clés spéciaux (`_TEMPORARY_`, `_NUMERIC_`/`_CHARACTER_`/`_ALL_`) et
/// valeurs initiales `(1, 2, 3)`, jusqu'au `;` (consommé) ; construit le
/// `DsStmt::Array` final.
fn parse_array_elements(
    ts: &mut StatementStream,
    name: String,
    dims: Option<Vec<usize>>,
    char_len: Option<usize>,
) -> Result<DsStmt> {
    // ── Liste de variables / mots-clés spéciaux / valeurs initiales ──────
    let mut vars: Vec<String> = Vec::new();
    let mut temporary = false;
    let mut special: Option<crate::ast::ArraySpecial> = None;
    let mut initial: Vec<Expr> = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi => {
                ts.next();
                return Ok(DsStmt::Array {
                    name,
                    dims,
                    char_len,
                    vars,
                    initial,
                    temporary,
                    special,
                });
            }
            TokenKind::Ident(v) => {
                let v = v.clone();
                let lower = v.to_ascii_lowercase();
                match lower.as_str() {
                    "_temporary_" => {
                        ts.next();
                        temporary = true;
                        continue;
                    }
                    "_numeric_" | "_character_" | "_all_" => {
                        if special.is_some() || !vars.is_empty() {
                            return Err(SasError::parse(
                                "a special list (_NUMERIC_/_CHARACTER_/_ALL_) cannot be \
                                 mixed with named array elements",
                                tok.span,
                            ));
                        }
                        ts.next();
                        special = Some(match lower.as_str() {
                            "_numeric_" => crate::ast::ArraySpecial::Numeric,
                            "_character_" => crate::ast::ArraySpecial::Character,
                            _ => crate::ast::ArraySpecial::All,
                        });
                        continue;
                    }
                    _ => {}
                }
                validate_sas_name(&v, tok.span)?;
                ts.next();
                if ts.peek().kind == TokenKind::Minus {
                    // Plage numérotée `x1-x3`.
                    ts.next(); // `-`
                    let end_tok = ts.peek().clone();
                    let Some(end_name) = end_tok.ident().map(str::to_string) else {
                        return Err(SasError::parse(
                            "expected a variable name after '-' in the ARRAY statement",
                            end_tok.span,
                        ));
                    };
                    validate_sas_name(&end_name, end_tok.span)?;
                    ts.next();
                    expand_numbered_range(&v, &end_name, tok.span.merge(end_tok.span), &mut vars)?;
                } else {
                    vars.push(v);
                }
            }
            // `(1, 2, 3)` : valeurs initiales (row-major). Parenthèses ; les
            // valeurs peuvent être séparées par des virgules OU des espaces
            // (SAS accepte les deux). On parse des expressions (littéraux
            // numériques/chaînes, missings, négatifs).
            TokenKind::LParen => {
                if !initial.is_empty() {
                    return Err(SasError::parse(
                        "duplicate initial-value list in the ARRAY statement",
                        tok.span,
                    ));
                }
                ts.next(); // `(`
                if ts.peek().kind == TokenKind::RParen {
                    return Err(SasError::parse(
                        "the array initial-value list cannot be empty",
                        ts.peek().span,
                    ));
                }
                loop {
                    initial.push(super::expr::parse_expr(ts)?);
                    if ts.peek().kind == TokenKind::Comma {
                        ts.next(); // séparateur virgule optionnel
                    }
                    if ts.peek().kind == TokenKind::RParen {
                        break;
                    }
                    if ts.peek().kind == TokenKind::Semi {
                        return Err(SasError::parse(
                            "expected ')' to close the array initial-value list",
                            ts.peek().span,
                        ));
                    }
                }
                ts.next(); // `)`
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable name in the ARRAY statement",
                    tok.span,
                ));
            }
        }
    }
}

/// Découpe `x12` en (`x`, `12`) : préfixe + suffixe numérique FINAL.
/// `None` si le nom ne se termine pas par un chiffre (ou n'a pas de
/// préfixe).
fn split_numbered(name: &str) -> Option<(&str, &str)> {
    let bytes = name.as_bytes();
    let mut i = bytes.len();
    while i > 0 && bytes[i - 1].is_ascii_digit() {
        i -= 1;
    }
    if i == bytes.len() || i == 0 {
        return None;
    }
    Some((&name[..i], &name[i..]))
}

/// Expanse la plage numérotée `x1-x3` en x1 x2 x3 : même préfixe (insensible
/// à la casse — la casse du premier nom est conservée), suffixes numériques,
/// bornes croissantes ; la largeur du suffixe de départ est conservée
/// (`x01-x03` → x01 x02 x03). Sinon, erreur claire. Partagée entre ARRAY et
/// les listes des options de dataset KEEP=/DROP= (`pub(super)`).
pub(in crate::parser) fn expand_numbered_range(
    start: &str,
    end: &str,
    span: Span,
    out: &mut Vec<String>,
) -> Result<()> {
    let err = || {
        SasError::parse(
            format!(
                "invalid variable range {start}-{end} \
                 (expected the same prefix with increasing numeric suffixes, e.g. x1-x3)"
            ),
            span,
        )
    };
    let (Some((p1, s1)), Some((p2, s2))) = (split_numbered(start), split_numbered(end)) else {
        return Err(err());
    };
    if !p1.eq_ignore_ascii_case(p2) {
        return Err(err());
    }
    let (Ok(a), Ok(b)) = (s1.parse::<u64>(), s2.parse::<u64>()) else {
        return Err(err());
    };
    if a > b {
        return Err(err());
    }
    let width = s1.len();
    for n in a..=b {
        out.push(format!("{p1}{n:0width$}"));
    }
    Ok(())
}
