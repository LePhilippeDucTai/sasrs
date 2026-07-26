use super::*;

/// Helper M29.1 : exige un `=` après un nom d'option ODS GRAPHICS.
pub(super) fn expect_ods_graphics_eq(ts: &mut StatementStream, name: &str) -> Result<()> {
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            format!(
                "ODS GRAPHICS option {} requires a value (e.g. {}=...)",
                name.to_uppercase(),
                name.to_uppercase()
            ),
            ts.peek().span,
        ));
    }
    ts.next(); // consume `=`
    Ok(())
}

// ── TITLE ────────────────────────────────────────────────────────────────────

pub(super) fn parse_title(ts: &mut StatementStream, n: u8) -> Result<GlobalStmt> {
    // `title ;` or `titleN ;` — no text, clears the title.
    if ts.peek().kind == TokenKind::Semi {
        ts.expect_semi()?;
        return Ok(GlobalStmt::Title { n, text: None });
    }

    // Only a quoted string literal is accepted in M1.
    //
    // Note: SAS itself accepts unquoted text after TITLE (e.g. `title My Report;`),
    // but our M1 parser intentionally restricts this to quoted string literals only.
    // Unquoted text after TITLE is an error here; this keeps the AST unambiguous and
    // avoids complex multi-token text concatenation. Callers should quote their titles.
    let text_tok = ts.peek().clone();
    match &text_tok.kind {
        TokenKind::Str { value, suffix } => {
            if *suffix != StrSuffix::None {
                return Err(SasError::parse(
                    "TITLE text must be a plain string literal (no date/time suffix)",
                    text_tok.span,
                ));
            }
            let text = value.clone();
            ts.next(); // consume the string literal
            ts.expect_semi()?;
            Ok(GlobalStmt::Title {
                n,
                text: Some(text),
            })
        }
        _ => {
            // Unquoted text or any non-string token after TITLE.
            Err(SasError::parse(
                "TITLE text must be a quoted string literal, e.g. title 'My Report';",
                text_tok.span,
            ))
        }
    }
}

// ── FOOTNOTE ───────────────────────────────────────────────────────────────

/// Parse `FOOTNOTEn ['texte'];`. Même grammaire que TITLE : soit une chaîne
/// littérale simple, soit rien (efface le niveau). Le niveau `n` (1..9) est déjà
/// extrait par l'appelant.
pub(super) fn parse_footnote(ts: &mut StatementStream, n: u8) -> Result<GlobalStmt> {
    // `footnote ;` or `footnoteN ;` — no text, clears the footnote.
    if ts.peek().kind == TokenKind::Semi {
        ts.expect_semi()?;
        return Ok(GlobalStmt::Footnote { n, text: None });
    }

    let text_tok = ts.peek().clone();
    match &text_tok.kind {
        TokenKind::Str { value, suffix } => {
            if *suffix != StrSuffix::None {
                return Err(SasError::parse(
                    "FOOTNOTE text must be a plain string literal (no date/time suffix)",
                    text_tok.span,
                ));
            }
            let text = value.clone();
            ts.next(); // consume the string literal
            ts.expect_semi()?;
            Ok(GlobalStmt::Footnote {
                n,
                text: Some(text),
            })
        }
        _ => Err(SasError::parse(
            "FOOTNOTE text must be a quoted string literal, e.g. footnote 'My Note';",
            text_tok.span,
        )),
    }
}

// ── OPTIONS ──────────────────────────────────────────────────────────────────

pub(super) fn parse_options(ts: &mut StatementStream) -> Result<GlobalStmt> {
    let mut opts: Vec<(String, Option<String>)> = Vec::new();

    // Collect `name` or `name=value` pairs until `;`.
    loop {
        if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
            break;
        }

        // The option name must be an identifier.
        let name_tok = ts.peek().clone();
        let name = match name_tok.ident() {
            Some(s) => s.to_string(),
            None => {
                return Err(SasError::parse(
                    "Expected an option name (identifier) in OPTIONS statement",
                    name_tok.span,
                ));
            }
        };
        ts.next(); // consume the name

        // Check for an `=` (value follows) or just a flag.
        if ts.peek().kind == TokenKind::Eq {
            ts.next(); // consume `=`
            // FMTSEARCH= and MISSING= accept a parenthesised list `(a b c)`.
            // We handle this here rather than in `parse_option_value` to avoid
            // accepting `(` universally for all options.
            let name_lc = name.to_ascii_lowercase();
            if (name_lc == "fmtsearch" || name_lc == "missing")
                && ts.peek().kind == TokenKind::LParen
            {
                let value = parse_paren_list(ts)?;
                opts.push((name, Some(value)));
            } else {
                let val_tok = ts.peek().clone();
                let value = parse_option_value(ts, &val_tok.span)?;
                opts.push((name, Some(value)));
            }
        } else {
            // Boolean flag: `nocenter`, `center`, etc.
            opts.push((name, None));
        }
    }

    ts.expect_semi()?;
    Ok(GlobalStmt::Options(opts))
}

/// Parse a parenthesised list of identifiers for OPTIONS values such as
/// `FMTSEARCH=(lib1 lib2)` and `MISSING=(. .)`.
/// The leading `(` must still be in the stream; it is consumed here.
/// Returns the identifiers joined by spaces (e.g. `"lib1 lib2"`).
pub(super) fn parse_paren_list(ts: &mut StatementStream) -> Result<String> {
    let lparen = ts.peek().clone();
    ts.next(); // consume `(`
    let mut items: Vec<String> = Vec::new();
    loop {
        match ts.peek().kind {
            TokenKind::RParen | TokenKind::Semi | TokenKind::Eof => break,
            _ => {}
        }
        let tok = ts.peek().clone();
        match tok.ident() {
            Some(s) => {
                items.push(s.to_string());
                ts.next();
            }
            None => {
                return Err(SasError::parse(
                    "Expected an identifier or ')' inside parenthesised OPTIONS value",
                    tok.span,
                ));
            }
        }
    }
    if ts.peek().kind != TokenKind::RParen {
        return Err(SasError::parse(
            "Expected ')' to close parenthesised OPTIONS value",
            lparen.span,
        ));
    }
    ts.next(); // consume `)`
    Ok(items.join(" "))
}

/// Parse the value token after `=` in an OPTIONS pair.
/// Accepts: identifier, integer or float number, plain string literal.
pub(super) fn parse_option_value(ts: &mut StatementStream, _span: &Span) -> Result<String> {
    let val_tok = ts.peek().clone();
    match &val_tok.kind {
        TokenKind::Ident(s) => {
            let s = s.clone();
            ts.next();
            Ok(s)
        }
        TokenKind::Num(f) => {
            let f = *f;
            ts.next();
            // Format integers without a trailing ".0" for readability.
            if f.fract() == 0.0 && f.abs() < 1e15 {
                Ok(format!("{}", f as i64))
            } else {
                Ok(format!("{}", f))
            }
        }
        TokenKind::Str { value, suffix } => {
            if *suffix != StrSuffix::None {
                return Err(SasError::parse(
                    "OPTIONS value must be a plain string literal (no date/time suffix)",
                    val_tok.span,
                ));
            }
            let v = value.clone();
            ts.next();
            Ok(v)
        }
        _ => Err(SasError::parse(
            "Expected an identifier, number, or quoted string as OPTIONS value",
            val_tok.span,
        )),
    }
}
