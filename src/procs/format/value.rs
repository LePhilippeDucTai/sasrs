use super::*;

/// Parse one VALUE statement (after the "value" keyword has been consumed):
///   [$]<fmtname>  <range>='label'  [<range>='label' ...] [other='label'] ;
pub(super) fn parse_value_stmt(ts: &mut StatementStream) -> Result<(String, UserFormat)> {
    // --- format name: optional `$` then identifier ---
    let is_char = ts.peek().kind == TokenKind::Dollar;
    let dollar_span = ts.peek().span;
    if is_char {
        ts.next(); // consume `$`
    }

    let name_tok = ts.peek().clone();
    let base_name = match name_tok.ident() {
        Some(n) => n.to_string(),
        None => {
            return Err(SasError::parse(
                "expected a format name after VALUE",
                name_tok.span,
            ));
        }
    };
    ts.next();

    // Build the stored name: include the `$` prefix for char formats.
    let name = if is_char {
        let _ = dollar_span; // used above for is_char detection
        format!("${}", base_name)
    } else {
        base_name
    };

    // --- parse range='label' pairs until `;` ---
    let mut ranges: Vec<Range> = Vec::new();
    let mut other: Option<String> = None;

    loop {
        // Skip stray semicolons within the statement (there should not be any,
        // but be defensive). A real `;` ends the statement.
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        // `run` / `quit` as step terminator (in case `;` was already consumed).
        if ts.peek().is_kw("run") || ts.peek().is_kw("quit") {
            break;
        }

        // OTHER keyword.
        if ts.peek().is_kw("other") {
            ts.next(); // consume "other"
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse("expected '=' after OTHER", ts.peek().span));
            }
            ts.next(); // consume `=`
            let lbl = parse_string_literal(ts)?;
            other = Some(lbl);
            continue;
        }

        // Parse one or more bounds that share a label (comma list or range).
        // Collect all ranges for this label-group.
        let group_ranges = parse_range_group(ts, is_char)?;

        // Now expect `=` then label.
        if ts.peek().kind != TokenKind::Eq {
            return Err(SasError::parse(
                "expected '=' after range specification",
                ts.peek().span,
            ));
        }
        ts.next(); // consume `=`

        let label = parse_string_literal(ts)?;

        // Assign label to every range in the group.
        for mut r in group_ranges {
            r.label = label.clone();
            ranges.push(r);
        }
    }

    let uf = UserFormat {
        is_char,
        ranges,
        other,
    };
    Ok((name, uf))
}

/// Parse a comma-separated list of range specs that share a single label.
/// Each element is a bound or a bound-range (`a-b`, `low-<b`, etc.).
/// Returns a Vec of Range with empty labels (caller fills them in).
pub(super) fn parse_range_group(ts: &mut StatementStream, is_char: bool) -> Result<Vec<Range>> {
    let mut out: Vec<Range> = Vec::new();

    loop {
        let r = parse_single_range(ts, is_char)?;
        out.push(r);

        // If next token is a comma, consume it and continue with another range.
        if ts.peek().kind == TokenKind::Comma {
            ts.next();
            // After comma there must be another range before `=`.
        } else {
            break;
        }
    }

    Ok(out)
}

/// Parse a single bound or range:
///   single:  `1`  or  `'PAR'`
///   range:   `1-3`  |  `low-<5`  |  `5<-high`  |  `1<-<100`
///            `low-5`  |  `1-high`  etc.
///
/// Exclusivity encoding (`<` next to `-`):
///   `a-<b`  → from_exclusive=false, to_exclusive=true
///   `a<-b`  → from_exclusive=true,  to_exclusive=false
///   `a<-<b` → from_exclusive=true,  to_exclusive=true
pub(super) fn parse_single_range(ts: &mut StatementStream, is_char: bool) -> Result<Range> {
    // Parse the "from" bound.
    let (from_bound, from_lt_before_minus) = parse_bound_with_lt(ts, is_char)?;

    // Check if there is a `-` or `<-` token sequence starting a range.
    // After the from bound:
    //   Case A: Next is `-`       → simple `-`, to_exclusive stays false by default.
    //   Case B: Next is `<` then `-` (already consumed `<` as from_lt_before_minus)
    //      → from_exclusive=true, to_exclusive depends on `<` after `-`.
    //
    // from_lt_before_minus == true means we already consumed a `<` between the
    // from value and the `-` (i.e. `5 < - high`).

    // Now decide if there's a range at all.
    // If the next token is `=` or `,` or `;` or EOF or step-boundary → single value.
    let has_range = match ts.peek().kind {
        TokenKind::Minus => true,
        _ => false,
    };

    if !has_range {
        // Single value: from == to, no exclusivity.
        // from_lt_before_minus being true here would be a parse error, but
        // we'll just ignore it and treat as single value.
        let to = from_bound.clone();
        return Ok(Range {
            from: from_bound,
            to,
            from_exclusive: false,
            to_exclusive: false,
            label: String::new(),
        });
    }

    // Consume the `-`.
    ts.next(); // `-`

    // Check for `<` immediately after `-` → to_exclusive = true.
    let to_exclusive = if ts.peek().kind == TokenKind::Lt {
        ts.next(); // consume `<`
        true
    } else {
        false
    };

    // Parse the "to" bound.
    let (to_bound, _) = parse_bound_with_lt(ts, is_char)?;

    Ok(Range {
        from: from_bound,
        to: to_bound,
        from_exclusive: from_lt_before_minus,
        to_exclusive,
        label: String::new(),
    })
}

/// Parse a bound token, also detecting a leading `<` before the `-` dash
/// (which would indicate from_exclusive=true for a range like `5<-high`).
///
/// Returns `(Bound, had_lt_before_dash)`.
/// The `had_lt_before_dash` is true when we see `<` and the *next* token is
/// `-` (so `5<-high`). We consume the `<` in that case.
pub(super) fn parse_bound_with_lt(
    ts: &mut StatementStream,
    is_char: bool,
) -> Result<(Bound, bool)> {
    // Check for a leading `<` that precedes `-` (from_exclusive pattern).
    // We need lookahead: peek is `<` and peek2 is `-`.
    // Actually the `<` comes AFTER the bound value in `5<-high`:
    //   tokens: Num(5), Lt, Minus, Ident("high")
    // So we parse the bound normally, then check for `<` before `-`.

    let bound = parse_bound(ts, is_char)?;

    // After the bound, check for `<` immediately followed by `-` → from_exclusive.
    let had_lt = if ts.peek().kind == TokenKind::Lt && ts.peek2().kind == TokenKind::Minus {
        ts.next(); // consume `<`
        true
    } else {
        false
    };

    Ok((bound, had_lt))
}

/// Parse a single bound value (LOW, HIGH, a number, or a quoted string).
pub(super) fn parse_bound(ts: &mut StatementStream, is_char: bool) -> Result<Bound> {
    if ts.peek().is_kw("low") {
        ts.next();
        return Ok(Bound::Low);
    }
    if ts.peek().is_kw("high") {
        ts.next();
        return Ok(Bound::High);
    }

    if is_char {
        // Character bound: must be a string literal.
        let s = parse_string_literal(ts)?;
        return Ok(Bound::Char(s));
    }

    // Numeric bound: a number literal.
    // Handle optional leading minus sign (negative numbers).
    let negative = if ts.peek().kind == TokenKind::Minus {
        // But only if the next-next is a number (not another operator).
        // Peek2 check: is the token after `-` a Num?
        if matches!(ts.peek2().kind, TokenKind::Num(_)) {
            ts.next(); // consume `-`
            true
        } else {
            false
        }
    } else {
        false
    };

    match ts.peek().kind.clone() {
        TokenKind::Num(n) => {
            ts.next();
            let v = if negative { -n } else { n };
            Ok(Bound::Num(v))
        }
        _ => Err(SasError::parse(
            "expected a numeric bound (number, LOW, or HIGH)",
            ts.peek().span,
        )),
    }
}

/// Parse a quoted string literal and return its content.
pub(super) fn parse_string_literal(ts: &mut StatementStream) -> Result<String> {
    let tok = ts.peek().clone();
    match &tok.kind {
        TokenKind::Str { value, .. } => {
            let s = value.clone();
            ts.next();
            Ok(s)
        }
        _ => Err(SasError::parse(
            "expected a quoted string literal",
            tok.span,
        )),
    }
}
