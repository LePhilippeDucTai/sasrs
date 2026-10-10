use super::*;

/// BREAK/RBREAK options that only change the look of the break lines (SAS 9.4
/// BREAK statement): overlines, underlines, blank line, page break, value of
/// the break variable suppressed. Not rendered (J11-P2).
const BREAK_DISPLAY_OPTIONS: &[&str] = &["dol", "dul", "ol", "page", "skip", "suppress", "ul"];

/// BREAK/RBREAK options `opt=value` limited to ODS/window rendering.
const BREAK_DISPLAY_VALUE_OPTIONS: &[&str] = &["color", "contents", "style"];

/// Parse a `break` / `rbreak` statement, after the keyword was consumed.
/// `break after <var> [/ options];`  |  `rbreak after [/ options];`
///
/// J02-P7 — the location is required (SAS 9.4: `BREAK location
/// break-variable`); BEFORE used to be rendered as AFTER and is an ERROR until
/// J11-P2. SUMMARIZE is honored; the display options give a WARNING each (they
/// were accepted without a word, and any other token silently skipped).
pub(crate) fn parse_break(ts: &mut StatementStream, is_rbreak: bool) -> Result<Break> {
    let stmt = if is_rbreak { "RBREAK" } else { "BREAK" };
    if ts.peek().is_kw("before") {
        return Err(contract::unsupported_construct(
            &format!("{stmt} BEFORE"),
            Some("J11-P2"),
            ts.peek().span,
        ));
    }
    if !ts.peek().is_kw("after") {
        return Err(SasError::parse(
            format!("expected AFTER or BEFORE after {stmt}"),
            ts.peek().span,
        ));
    }
    ts.next();

    // For BREAK, a group variable name follows (absent for RBREAK).
    let var = if !is_rbreak {
        match ts.peek().ident().map(str::to_string) {
            Some(v) => {
                ts.next();
                Some(v)
            }
            None => {
                return Err(SasError::parse(
                    "expected a variable name after BREAK",
                    ts.peek().span,
                ));
            }
        }
    } else {
        None
    };

    let mut summarize = false;
    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        loop {
            if matches!(ts.peek().kind, TokenKind::Semi | TokenKind::Eof) {
                break;
            }
            let Some(opt) = ts.peek().ident().map(str::to_ascii_lowercase) else {
                return Err(SasError::parse(
                    format!("unexpected token in the {stmt} statement"),
                    ts.peek().span,
                ));
            };
            if opt == "summarize" {
                summarize = true;
                ts.next();
            } else if BREAK_DISPLAY_OPTIONS.contains(&opt.as_str()) {
                ts.warn_ignored_display(contract::ignored_display_option(
                    stmt,
                    &opt.to_ascii_uppercase(),
                    Some("J11-P2"),
                ));
                ts.next();
            } else if BREAK_DISPLAY_VALUE_OPTIONS.contains(&opt.as_str())
                && ts.peek2().kind == TokenKind::Eq
            {
                ts.warn_ignored_display(contract::ignored_display_option(
                    stmt,
                    &format!("{}=", opt.to_ascii_uppercase()),
                    None,
                ));
                ts.next();
                skip_display_value(ts);
            } else {
                return Err(SasError::parse(
                    format!(
                        "Unknown or unsupported {stmt} option '{}' in PROC REPORT.",
                        opt.to_ascii_uppercase()
                    ),
                    ts.peek().span,
                ));
            }
        }
    }
    ts.expect_semi()?;
    Ok(Break { var, summarize })
}

/// Consume `= value` of a display-only option: a quoted string, a name, a
/// number, a `[...]`/`{...}` style list or a parenthesized list.
fn skip_display_value(ts: &mut StatementStream) {
    if ts.peek().kind != TokenKind::Eq {
        return;
    }
    ts.next();
    // Optional style element name before the attribute list (`STYLE=Header[…]`).
    if matches!(
        ts.peek().kind,
        TokenKind::Ident(_) | TokenKind::Str { .. } | TokenKind::Num(_)
    ) {
        ts.next();
    }
    let close = match ts.peek().kind {
        TokenKind::LBracket => TokenKind::RBracket,
        TokenKind::LBrace => TokenKind::RBrace,
        TokenKind::LParen => {
            ts.skip_balanced_parens();
            return;
        }
        _ => return,
    };
    while !matches!(ts.peek().kind, TokenKind::Semi | TokenKind::Eof) {
        let done = ts.peek().kind == close;
        ts.next();
        if done {
            break;
        }
    }
}

/// Parse a `compute <target>; ... endcomp;` block, after `compute` consumed.
///
/// J02-P7 — only the report-level `COMPUTE AFTER` (LINE statements) and the
/// COMPUTE block of a report item (assignments) are implemented. COMPUTE
/// BEFORE, COMPUTE AFTER <var> (it used to run once, at report level), a LINE
/// statement in the block of a report item and an assignment in COMPUTE AFTER
/// (it used to rewrite every row) are ERRORs until J11-P2. Function calls in
/// an expression are an ERROR until J03-P3 (they evaluated to missing).
pub(crate) fn parse_compute(ts: &mut StatementStream) -> Result<Compute> {
    // Target: a column name or `after`/`before`.
    let target_span = ts.peek().span;
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
    let location = if target.eq_ignore_ascii_case("before") {
        Some("BEFORE")
    } else if target.eq_ignore_ascii_case("after") {
        Some("AFTER")
    } else {
        None
    };
    if let Some(loc) = location {
        // `compute before|after <var>;` or `_page_`: break-level block.
        let target_var = ts.peek().ident().map(str::to_ascii_uppercase);
        if loc == "BEFORE" || target_var.is_some() {
            let what = match target_var {
                Some(v) => format!("COMPUTE {loc} {v}"),
                None => format!("COMPUTE {loc}"),
            };
            return Err(contract::unsupported_construct(
                &what,
                Some("J11-P2"),
                target_span,
            ));
        }
    }
    parse_compute_options(ts)?;
    ts.expect_semi()?;

    let item = location.is_none();
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
            if item {
                return Err(contract::unsupported_construct(
                    &format!(
                        "A LINE statement in the COMPUTE block of report item {}",
                        target.to_ascii_uppercase()
                    ),
                    Some("J11-P2"),
                    ts.peek().span,
                ));
            }
            ts.next();
            stmts.push(ComputeStmt::Line(parse_line_items(ts)?));
            ts.expect_semi()?;
        } else if let Some(col) = ts.peek().ident().map(str::to_string) {
            // Expect `<col> = <expr>;`. Anything else inside a COMPUTE is
            // deferred CLEANLY (no panic): error with a clear message.
            let span = ts.peek().span;
            ts.next();
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::runtime(format!(
                    "PROC REPORT v1 supports only simple '<col> = <expr>;' \
                     assignments and LINE statements inside COMPUTE (got '{}').",
                    col.to_uppercase()
                )));
            }
            if !item {
                return Err(contract::unsupported_construct(
                    "An assignment in a COMPUTE AFTER block",
                    Some("J11-P2"),
                    span,
                ));
            }
            ts.next(); // '='
            let expr = parse_compute_expr(ts)?;
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

/// Options of the COMPUTE statement (SAS 9.4: `/ CHARACTER <LENGTH=n>` for a
/// computed item, `/ STYLE=` for the ODS rendering). They used to be skipped up
/// to the `;`: a CHARACTER item stayed numeric (right-aligned, missing in OUT=).
fn parse_compute_options(ts: &mut StatementStream) -> Result<()> {
    if ts.peek().kind != TokenKind::Slash {
        return Ok(());
    }
    ts.next();
    while !matches!(ts.peek().kind, TokenKind::Semi | TokenKind::Eof) {
        let Some(opt) = ts.peek().ident().map(str::to_ascii_lowercase) else {
            return Err(SasError::parse(
                "unexpected token in the COMPUTE statement",
                ts.peek().span,
            ));
        };
        match opt.as_str() {
            "character" | "char" | "length" => {
                let shown = if opt == "length" {
                    "LENGTH="
                } else {
                    "CHARACTER"
                };
                return Err(contract::unsupported_construct(
                    &format!("The {shown} option of the COMPUTE statement"),
                    None,
                    ts.peek().span,
                ));
            }
            "style" if ts.peek2().kind == TokenKind::Eq => {
                ts.warn_ignored_display(contract::ignored_display_option(
                    "COMPUTE", "STYLE=", None,
                ));
                ts.next();
                skip_display_value(ts);
            }
            _ => {
                return Err(SasError::parse(
                    format!(
                        "Unknown or unsupported COMPUTE option '{}' in PROC REPORT.",
                        opt.to_ascii_uppercase()
                    ),
                    ts.peek().span,
                ));
            }
        }
    }
    Ok(())
}

/// Parse an expression of a COMPUTE block and reject what the local evaluator
/// cannot compute (function calls, array references, compound names).
fn parse_compute_expr(ts: &mut StatementStream) -> Result<Expr> {
    let span = ts.peek().span;
    let expr = crate::parser::expr::parse_expr(ts)?;
    contract::check_expr(&expr, "in a COMPUTE block", span)?;
    contract::check_break_variable(&expr, span)?;
    reject_compound_name(ts, &expr)?;
    Ok(expr)
}

/// `sales.sum` — the SAS reference to an analysis variable by its compound
/// name `variable.statistic` (SAS 9.4 COMPUTE statement). The expression parser
/// stops at the `.`: in an assignment this was a bare syntax error, in a LINE
/// statement the pieces were printed as `sales`, a missing value and `sum`.
fn reject_compound_name(ts: &StatementStream, expr: &Expr) -> Result<()> {
    if ts.peek().kind != TokenKind::Dot {
        return Ok(());
    }
    let Some(stat) = ts.peek2().ident() else {
        return Ok(());
    };
    let var = rightmost_var(expr).unwrap_or_default();
    Err(contract::unsupported_construct(
        &format!(
            "The compound name {}.{}",
            var.to_ascii_uppercase(),
            stat.to_ascii_uppercase()
        ),
        None,
        ts.peek().span,
    ))
}

/// The variable that ends an expression (the parser stopped right after it).
fn rightmost_var(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Var(name) => Some(name),
        Expr::Unary { expr, .. } => rightmost_var(expr),
        Expr::Binary { right, .. } => rightmost_var(right),
        _ => None,
    }
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
                let e = parse_compute_expr(ts)?;
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
