use super::*;

// ── PICTURE parsing (M18.3) ──────────────────────────────────────────────────

/// Parse one PICTURE statement (after the "picture" keyword has been consumed):
///   <picname>  <range>='template' [(dirs)]  [<range>='template' [(dirs)]] ... ;
///
/// PICTURE formats are always numeric (no `$`). Each range maps to a picture
/// template string, optionally followed by parenthesised directives
/// (`PREFIX=` / `MULT=` / `FILL=`).
pub(super) fn parse_picture_stmt(ts: &mut StatementStream) -> Result<(String, UserPicture)> {
    let name_tok = ts.peek().clone();
    let name = match name_tok.ident() {
        Some(n) => n.to_string(),
        None => {
            return Err(SasError::parse(
                "expected a picture name after PICTURE",
                name_tok.span,
            ));
        }
    };
    ts.next();

    let mut ranges: Vec<PictureRange> = Vec::new();
    let mut other: Option<(String, PictureDirectives)> = None;

    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("run") || ts.peek().is_kw("quit") {
            break;
        }

        // OTHER keyword → fallback template.
        if ts.peek().is_kw("other") {
            ts.next();
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse(
                    "expected '=' after OTHER in PICTURE",
                    ts.peek().span,
                ));
            }
            ts.next(); // `=`
            let template = parse_string_literal(ts)?;
            let directives = parse_picture_directives(ts)?;
            other = Some((template, directives));
            continue;
        }

        // A group of numeric ranges sharing one template (comma list).
        let group = parse_picture_range_group(ts)?;

        if ts.peek().kind != TokenKind::Eq {
            return Err(SasError::parse(
                "expected '=' after range specification in PICTURE",
                ts.peek().span,
            ));
        }
        ts.next(); // `=`

        let template = parse_string_literal(ts)?;
        let directives = parse_picture_directives(ts)?;

        for (from, to, from_excl, to_excl) in group {
            ranges.push(PictureRange {
                from,
                to,
                from_exclusive: from_excl,
                to_exclusive: to_excl,
                template: template.clone(),
                directives: directives.clone(),
            });
        }
    }

    Ok((name, UserPicture { ranges, other }))
}

/// Parse a comma-separated group of numeric picture ranges (no template yet).
/// Returns tuples `(from, to, from_exclusive, to_exclusive)`.
pub(super) fn parse_picture_range_group(
    ts: &mut StatementStream,
) -> Result<Vec<(Bound, Bound, bool, bool)>> {
    let mut out = Vec::new();
    loop {
        // Reuse the numeric VALUE range parser (is_char = false).
        let r = parse_single_range(ts, false)?;
        out.push((r.from, r.to, r.from_exclusive, r.to_exclusive));
        if ts.peek().kind == TokenKind::Comma {
            ts.next();
        } else {
            break;
        }
    }
    Ok(out)
}

/// Parse the optional `(PREFIX='...' MULT=n FILL='c' ...)` directive list that
/// follows a picture template. Returns defaults when no `(` is present.
/// Directives are space-separated `KEY=VALUE` pairs.
pub(super) fn parse_picture_directives(ts: &mut StatementStream) -> Result<PictureDirectives> {
    let mut dir = PictureDirectives::default();
    if ts.peek().kind != TokenKind::LParen {
        return Ok(dir);
    }
    ts.next(); // `(`

    loop {
        if ts.peek().kind == TokenKind::RParen {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            return Err(SasError::parse(
                "unterminated directive list in PICTURE (missing ')')",
                ts.peek().span,
            ));
        }

        let key_tok = ts.peek().clone();
        let key = match key_tok.ident() {
            Some(k) => k.to_lowercase(),
            None => {
                return Err(SasError::parse(
                    "expected a directive name (PREFIX/MULT/FILL) in PICTURE",
                    key_tok.span,
                ));
            }
        };
        ts.next();

        if ts.peek().kind != TokenKind::Eq {
            return Err(SasError::parse(
                "expected '=' after directive name in PICTURE",
                ts.peek().span,
            ));
        }
        ts.next(); // `=`

        match key.as_str() {
            "prefix" => {
                dir.prefix = Some(parse_string_literal(ts)?);
            }
            "fill" => {
                let s = parse_string_literal(ts)?;
                dir.fill = s.chars().next();
            }
            "mult" | "multiplier" => {
                // MULT=n — a (possibly negative, possibly decimal) number.
                let negative = if ts.peek().kind == TokenKind::Minus
                    && matches!(ts.peek2().kind, TokenKind::Num(_))
                {
                    ts.next();
                    true
                } else {
                    false
                };
                match ts.peek().kind.clone() {
                    TokenKind::Num(n) => {
                        ts.next();
                        dir.mult = Some(if negative { -n } else { n });
                    }
                    _ => {
                        return Err(SasError::parse(
                            "expected a number after MULT= in PICTURE",
                            ts.peek().span,
                        ));
                    }
                }
            }
            other => {
                return Err(SasError::parse(
                    format!("unsupported PICTURE directive '{other}'"),
                    key_tok.span,
                ));
            }
        }
    }

    Ok(dir)
}
