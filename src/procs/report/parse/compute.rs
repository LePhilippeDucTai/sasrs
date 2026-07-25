use super::*;

/// Parse a `break` / `rbreak` statement, after the keyword was consumed.
/// `break after <var> [/ summarize ...];`  |  `rbreak after [/ summarize ...];`
pub(crate) fn parse_break(ts: &mut StatementStream, is_rbreak: bool) -> Result<Break> {
    // Optional `after` / `before` location keyword. v1 treats both as the
    // summary line placed AFTER the range (the meaningful case); `before` is
    // accepted and documented as placed-after.
    if ts.peek().is_kw("after") || ts.peek().is_kw("before") {
        ts.next();
    }

    // For BREAK, a group variable name follows (absent for RBREAK).
    let var = if !is_rbreak {
        match ts.peek().ident().map(str::to_string) {
            Some(v) if ts.peek().kind != TokenKind::Slash => {
                ts.next();
                Some(v)
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable name after BREAK",
                    ts.peek().span,
                ));
            }
        }
    } else {
        None
    };

    // Optional `/ <options>`. v1 understands SUMMARIZE; OL/DOL/SKIP/PAGE and
    // similar cosmetic options are accepted and ignored (documented).
    let mut summarize = false;
    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        loop {
            match &ts.peek().kind {
                TokenKind::Semi | TokenKind::Eof => break,
                TokenKind::Ident(raw) => {
                    if raw.eq_ignore_ascii_case("summarize") {
                        summarize = true;
                    }
                    // Other options (ol, dol, skip, page, suppress, ...) are
                    // cosmetic presentation flags → accepted, no-op in v1.
                    ts.next();
                }
                _ => {
                    ts.next();
                }
            }
        }
    }
    ts.expect_semi()?;
    Ok(Break { var, summarize })
}

/// Parse a `compute <target>; ... endcomp;` block, after `compute` consumed.
pub(crate) fn parse_compute(ts: &mut StatementStream) -> Result<Compute> {
    // Target: a column name or `after`/`before`.
    let target = match ts.peek().ident().map(str::to_string) {
        Some(t) => {
            ts.next();
            t
        }
        None => {
            return Err(SasError::parse(
                "expected a target after COMPUTE",
                ts.peek().span,
            ));
        }
    };
    // Skip any trailing options on the compute statement (e.g. `/ character`)
    // up to the `;`.
    while !matches!(ts.peek().kind, TokenKind::Semi | TokenKind::Eof) {
        ts.next();
    }
    ts.expect_semi()?;

    let mut stmts: Vec<ComputeStmt> = Vec::new();
    loop {
        while ts.peek().kind == TokenKind::Semi {
            ts.next();
        }
        if ts.peek().kind == TokenKind::Eof {
            return Err(SasError::parse(
                "expected ENDCOMP to close COMPUTE block",
                ts.peek().span,
            ));
        }
        if ts.peek().is_kw("endcomp") {
            ts.next();
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            break;
        }
        if ts.peek().is_kw("line") {
            ts.next();
            stmts.push(ComputeStmt::Line(parse_line_items(ts)?));
            ts.expect_semi()?;
        } else if let Some(col) = ts.peek().ident().map(str::to_string) {
            // Expect `<col> = <expr>;`. Anything else inside a COMPUTE is
            // deferred CLEANLY (no panic): error with a clear message.
            ts.next();
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::runtime(format!(
                    "PROC REPORT v1 supports only simple '<col> = <expr>;' \
                     assignments and LINE statements inside COMPUTE (got '{}').",
                    col.to_uppercase()
                )));
            }
            ts.next(); // '='
            let expr = crate::parser::expr::parse_expr(ts)?;
            ts.expect_semi()?;
            stmts.push(ComputeStmt::Assign { col, expr });
        } else {
            return Err(SasError::parse(
                "unexpected token inside COMPUTE block",
                ts.peek().span,
            ));
        }
    }
    Ok(Compute { target, stmts })
}

/// Parse the items of a `line` statement up to (but not consuming) the `;`.
/// Supports string literals, `@<col>` pointers (rendered as padding to that
/// column), bare expressions (column references / numbers), and an optional
/// trailing SAS format on an expression (`line @5 total best8.;`, M33.5).
pub(crate) fn parse_line_items(ts: &mut StatementStream) -> Result<Vec<LineItem>> {
    let mut items = Vec::new();
    loop {
        match &ts.peek().kind {
            TokenKind::Semi | TokenKind::Eof => break,
            TokenKind::Str { value, .. } => {
                items.push(LineItem::Literal(value.clone()));
                ts.next();
            }
            TokenKind::At => {
                // `@<col>` column pointer: pad the line out to column `col`.
                ts.next();
                if let TokenKind::Num(n) = ts.peek().kind {
                    ts.next();
                    items.push(LineItem::Pointer(n.max(1.0) as usize));
                }
                // A bare `@` without a column is ignored (lenient).
            }
            _ => {
                // Parse a bare expression (column reference, number, ...).
                let e = crate::parser::expr::parse_expr(ts)?;
                // Optional trailing SAS format token (e.g. `best8.`): a format
                // is recognized only when the next token starts a format whose
                // text contains a '.' (so plain identifiers stay expressions).
                let fmt = if peek_is_line_format(ts) {
                    Some(crate::parser::expr::read_format_token(ts)?)
                } else {
                    None
                };
                items.push(LineItem::Expr(e, fmt));
            }
        }
    }
    Ok(items)
}

/// True when the next token begins a SAS format used as a LINE item suffix.
/// We only accept tokens whose joined format text contains a '.', so bare
/// identifiers (another expression item) are not mistaken for a format.
pub(crate) fn peek_is_line_format(ts: &StatementStream) -> bool {
    // A format suffix begins with an identifier (e.g. `best8.`, `dollar8.2`)
    // or `$`; a leading bare number like `8.2` is also a format. We confirm by
    // requiring the following token to be a Dot or a Num adjacent to it (the
    // shape of `best8.` / `8.2`). Two-token lookahead suffices.
    match &ts.peek().kind {
        TokenKind::Ident(_) => {
            // e.g. `best8.` → ident "best8" then Dot, or ident "best" then num.
            matches!(ts.peek2().kind, TokenKind::Dot)
                || matches!(ts.peek2().kind, TokenKind::Num(_))
        }
        TokenKind::Dollar => true,
        _ => false,
    }
}
