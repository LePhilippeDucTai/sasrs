use super::*;

// ── INVALUE parsing (M18.2) ──────────────────────────────────────────────────

/// Parse one INVALUE statement (after the "invalue" keyword has been consumed):
///   [$]<inforname>  'key'=value  ['key'=value ...] [other=value] ;
///
/// Keys are always character strings (quoted); the result (`value`) is:
///   - a numeric literal   → `InformatValue::Num(f64)`
///   - a quoted string     → `InformatValue::Char(String)`
///   - `_SAME_` keyword    → `InformatValue::Same`
///   - `.`/`._`/`.A`..`.Z`→ `InformatValue::Missing(kind_str)`
pub(super) fn parse_invalue_stmt(ts: &mut StatementStream) -> Result<(String, UserInformat)> {
    // --- informat name: optional `$` then identifier ---
    let is_char_result = ts.peek().kind == TokenKind::Dollar;
    if is_char_result {
        ts.next(); // consume `$`
    }

    let name_tok = ts.peek().clone();
    let base_name = match name_tok.ident() {
        Some(n) => n.to_string(),
        None => {
            return Err(SasError::parse(
                "expected an informat name after INVALUE",
                name_tok.span,
            ));
        }
    };
    ts.next();

    // Build stored name (include `$` prefix for char informats).
    let name = if is_char_result {
        format!("${}", base_name)
    } else {
        base_name
    };

    // --- parse 'key'=result pairs until `;` ---
    let mut ranges: Vec<InformatRange> = Vec::new();
    let mut other: Option<InformatValue> = None;

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

        // OTHER keyword.
        if ts.peek().is_kw("other") {
            ts.next(); // consume "other"
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse(
                    "expected '=' after OTHER in INVALUE",
                    ts.peek().span,
                ));
            }
            ts.next(); // consume `=`
            let iv = parse_informat_value(ts)?;
            other = Some(iv);
            continue;
        }

        // Parse a group of key ranges sharing a single result value.
        // Keys are always character strings (quoted strings or LOW/HIGH).
        let group_ranges = parse_invalue_range_group(ts)?;

        if ts.peek().kind != TokenKind::Eq {
            return Err(SasError::parse(
                "expected '=' after key range specification in INVALUE",
                ts.peek().span,
            ));
        }
        ts.next(); // consume `=`

        let result = parse_informat_value(ts)?;

        for mut r in group_ranges {
            r.result = result.clone();
            ranges.push(r);
        }
    }

    let ui = UserInformat {
        is_char_result,
        ranges,
        other,
    };
    Ok((name, ui))
}

/// Parse a comma-separated group of character key ranges for INVALUE.
/// Returns Vec<InformatRange> with placeholder `result` values (caller fills in).
pub(super) fn parse_invalue_range_group(ts: &mut StatementStream) -> Result<Vec<InformatRange>> {
    let mut out: Vec<InformatRange> = Vec::new();
    loop {
        let r = parse_invalue_single_range(ts)?;
        out.push(r);
        if ts.peek().kind == TokenKind::Comma {
            ts.next();
        } else {
            break;
        }
    }
    Ok(out)
}

/// Parse a single key range for INVALUE: `'A'`, `'A'-'Z'`, `low-'C'`, etc.
pub(super) fn parse_invalue_single_range(ts: &mut StatementStream) -> Result<InformatRange> {
    // INVALUE keys are always char-mode bounds.
    let (from_bound, from_lt_before_minus) = parse_invalue_bound_with_lt(ts)?;

    let has_range = ts.peek().kind == TokenKind::Minus;

    if !has_range {
        let to = from_bound.clone();
        return Ok(InformatRange {
            from: from_bound,
            to,
            from_exclusive: false,
            to_exclusive: false,
            result: InformatValue::Same, // placeholder, caller replaces
        });
    }

    ts.next(); // consume `-`

    let to_exclusive = if ts.peek().kind == TokenKind::Lt {
        ts.next(); // consume `<`
        true
    } else {
        false
    };

    let (to_bound, _) = parse_invalue_bound_with_lt(ts)?;

    Ok(InformatRange {
        from: from_bound,
        to: to_bound,
        from_exclusive: from_lt_before_minus,
        to_exclusive,
        result: InformatValue::Same, // placeholder, caller replaces
    })
}

/// Parse an INVALUE key bound with optional leading `<` (from_exclusive marker).
pub(super) fn parse_invalue_bound_with_lt(ts: &mut StatementStream) -> Result<(Bound, bool)> {
    let bound = parse_invalue_bound(ts)?;
    // Check for `<` immediately followed by `-` (from_exclusive pattern).
    let had_lt = if ts.peek().kind == TokenKind::Lt && ts.peek2().kind == TokenKind::Minus {
        ts.next(); // consume `<`
        true
    } else {
        false
    };
    Ok((bound, had_lt))
}

/// Parse one INVALUE key bound: LOW, HIGH, or a quoted string.
pub(super) fn parse_invalue_bound(ts: &mut StatementStream) -> Result<Bound> {
    if ts.peek().is_kw("low") {
        ts.next();
        return Ok(Bound::Low);
    }
    if ts.peek().is_kw("high") {
        ts.next();
        return Ok(Bound::High);
    }
    // Must be a quoted string.
    let s = parse_string_literal(ts)?;
    Ok(Bound::Char(s))
}

/// Parse the result value on the right-hand side of `=` in an INVALUE mapping:
///   numeric literal  → `InformatValue::Num`
///   quoted string    → `InformatValue::Char`
///   `_SAME_`         → `InformatValue::Same`
///   `.` / `._` / `.A`..`.Z` → `InformatValue::Missing`
pub(super) fn parse_informat_value(ts: &mut StatementStream) -> Result<InformatValue> {
    // `_SAME_` keyword (identifier).
    if let Some(id) = ts.peek().ident()
        && id.eq_ignore_ascii_case("_same_")
    {
        ts.next();
        return Ok(InformatValue::Same);
    }

    // Missing value: starts with `.`
    if ts.peek().kind == TokenKind::Dot {
        ts.next(); // consume `.`
        // Check for special suffix: `_` or letter.
        if let Some(id) = ts.peek().ident() {
            let s = id.to_uppercase();
            if s == "_" || (s.len() == 1 && s.chars().next().unwrap().is_ascii_uppercase()) {
                ts.next();
                return Ok(InformatValue::Missing(s));
            }
        }
        return Ok(InformatValue::Missing(".".to_string()));
    }

    // Quoted string → character result.
    if let TokenKind::Str { value, .. } = &ts.peek().kind.clone() {
        let s = value.clone();
        ts.next();
        return Ok(InformatValue::Char(s));
    }

    // Numeric literal (possibly negative).
    let negative = if ts.peek().kind == TokenKind::Minus {
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
            Ok(InformatValue::Num(v))
        }
        _ => Err(SasError::parse(
            "expected a result value (number, quoted string, _SAME_, or missing) in INVALUE",
            ts.peek().span,
        )),
    }
}
