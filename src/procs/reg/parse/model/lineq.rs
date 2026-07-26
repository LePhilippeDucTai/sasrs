use super::*;

/// MQ5.1 — parse the value of a `SELECTION=method` MODEL option (the
/// `selection` keyword has not been consumed yet) and build the initial
/// `Selection` request with the method's default SLE/SLS.
pub(crate) fn parse_selection_value(ts: &mut StatementStream) -> Result<Selection> {
    common::expect_eq(ts, "SELECTION")?;
    let method_name = ts.peek().ident().map(str::to_string).ok_or_else(|| {
        SasError::parse("expected selection method after SELECTION=", ts.peek().span)
    })?;
    ts.next();
    let method = match method_name.to_ascii_lowercase().as_str() {
        "forward" => SelMethod::Forward,
        "backward" => SelMethod::Backward,
        "stepwise" => SelMethod::Stepwise,
        "rsquare" => SelMethod::RSquare,
        "adjrsq" => SelMethod::AdjRsq,
        "cp" => SelMethod::Cp,
        "maxr" => SelMethod::MaxR,
        "minr" => SelMethod::MinR,
        "none" => SelMethod::None,
        other => {
            return Err(SasError::parse(
                format!("unsupported SELECTION method '{}'", other),
                ts.peek().span,
            ));
        }
    };
    // Defaults depend on the method. The all-subsets and
    // R²-improvement methods don't use SLE/SLS; keep
    // harmless defaults so the struct is always valid.
    let (def_sle, def_sls) = match method {
        SelMethod::Forward => (0.50, 0.10),
        SelMethod::Backward => (0.50, 0.10),
        SelMethod::Stepwise => (0.15, 0.15),
        _ => (0.50, 0.10),
    };
    Ok(Selection {
        method,
        slentry: def_sle,
        slstay: def_sls,
        best: None,
        include: 0,
        start: None,
        stop: None,
        details: false,
        stb: false,
    })
}

// ───────────────────────── Linear-equation parsing (M36.1) ─────────────────────────

/// Parse a comma-separated list of linear equations (`eq [, eq ...]`),
/// stopping at the terminating `;`.
pub(crate) fn parse_lin_eqs(ts: &mut StatementStream) -> Result<Vec<LinEq>> {
    let mut eqs = Vec::new();
    loop {
        eqs.push(parse_lin_eq(ts)?);
        if ts.peek().kind == TokenKind::Comma {
            ts.next();
            continue;
        }
        break;
    }
    Ok(eqs)
}

/// Parse the (optional) equation list of an MTEST statement (M36.10). Each
/// comma-separated entry is a linear combination of regressors; unlike
/// TEST/RESTRICT the `= rhs` is OPTIONAL and defaults to `= 0` (MTEST tests
/// linear combinations of the parameters against zero across all responses).
/// Stops at the terminating `;` or a `/` option separator. An empty list
/// (the statement is just `MTEST;`) yields no equations, meaning the default
/// "all non-intercept coefficients = 0" hypothesis.
pub(crate) fn parse_mtest_equations(ts: &mut StatementStream) -> Result<Vec<LinEq>> {
    let mut eqs = Vec::new();
    loop {
        // Terminators with no (further) equation.
        if matches!(
            ts.peek().kind,
            TokenKind::Semi | TokenKind::Eof | TokenKind::Slash
        ) {
            break;
        }
        // Left side: signed sum of regressor terms (and bare constants).
        let mut terms: Vec<(f64, String)> = Vec::new();
        let mut lhs_const = 0.0;
        parse_lin_side(ts, 1.0, &mut terms, &mut lhs_const)?;
        let mut rhs = -lhs_const;
        // Optional `= rhs`.
        if ts.peek().kind == TokenKind::Eq {
            ts.next();
            let mut rhs_terms: Vec<(f64, String)> = Vec::new();
            let mut rhs_const = 0.0;
            parse_lin_side(ts, 1.0, &mut rhs_terms, &mut rhs_const)?;
            for (c, v) in rhs_terms {
                terms.push((-c, v));
            }
            rhs += rhs_const;
        }
        // Merge duplicate variables.
        let mut merged: Vec<(f64, String)> = Vec::new();
        for (c, v) in terms {
            if let Some(e) = merged.iter_mut().find(|(_, name)| *name == v) {
                e.0 += c;
            } else {
                merged.push((c, v));
            }
        }
        eqs.push(LinEq { terms: merged, rhs });
        if ts.peek().kind == TokenKind::Comma {
            ts.next();
            continue;
        }
        break;
    }
    Ok(eqs)
}

/// Parse one linear equation `lhs = rhs` and normalise it so every variable
/// term sits on the left and the net constant on the right:
/// Σ coef·var = rhs. Variable names are uppercased; `INTERCEPT` is preserved
/// as the reserved name `"INTERCEPT"`.
pub(crate) fn parse_lin_eq(ts: &mut StatementStream) -> Result<LinEq> {
    // Left side: accumulate terms with their natural sign.
    let mut terms: Vec<(f64, String)> = Vec::new();
    let mut rhs = 0.0; // net constant: starts on the LHS (subtracted later).
    let mut lhs_const = 0.0;
    parse_lin_side(ts, 1.0, &mut terms, &mut lhs_const)?;

    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            "expected '=' in TEST/RESTRICT equation",
            ts.peek().span,
        ));
    }
    ts.next(); // '='

    // Right side: variables flip sign (move to LHS), constants stay on RHS.
    let mut rhs_terms: Vec<(f64, String)> = Vec::new();
    let mut rhs_const = 0.0;
    parse_lin_side(ts, 1.0, &mut rhs_terms, &mut rhs_const)?;
    for (c, v) in rhs_terms {
        terms.push((-c, v));
    }
    // Net constant = rhs_const - lhs_const.
    rhs += rhs_const - lhs_const;

    // Merge duplicate variables.
    let mut merged: Vec<(f64, String)> = Vec::new();
    for (c, v) in terms {
        if let Some(e) = merged.iter_mut().find(|(_, name)| *name == v) {
            e.0 += c;
        } else {
            merged.push((c, v));
        }
    }
    Ok(LinEq { terms: merged, rhs })
}

/// Parse one side of an equation: a sum of signed terms up to `=`, `,` or `;`.
/// Variable terms are pushed into `terms` (scaled by `sign`); bare constants
/// accumulate into `konst`.
pub(crate) fn parse_lin_side(
    ts: &mut StatementStream,
    sign: f64,
    terms: &mut Vec<(f64, String)>,
    konst: &mut f64,
) -> Result<()> {
    let mut pending = sign; // sign accumulated from a run of leading +/-.
    loop {
        match ts.peek().kind {
            TokenKind::Eq | TokenKind::Comma | TokenKind::Semi | TokenKind::Eof => break,
            TokenKind::Plus => {
                ts.next();
                continue;
            }
            TokenKind::Minus => {
                pending = -pending;
                ts.next();
                continue;
            }
            _ => {}
        }
        // A term: optional numeric coefficient, optional `*`, then a name; or a
        // bare constant; or a bare name (coef 1).
        let mut coef = pending;
        let mut have_num = false;
        if let TokenKind::Num(v) = ts.peek().kind {
            coef = pending * v;
            have_num = true;
            ts.next();
            if ts.peek().kind == TokenKind::Star {
                ts.next();
            }
        }
        if let Some(name) = ts.peek().ident().map(str::to_string) {
            ts.next();
            terms.push((coef, name.to_ascii_uppercase()));
        } else if have_num {
            // Bare constant (no variable followed the number).
            *konst += coef;
        } else {
            return Err(SasError::parse(
                "expected variable or constant in TEST/RESTRICT equation",
                ts.peek().span,
            ));
        }
        // Reset the sign for the next term.
        pending = sign;
    }
    Ok(())
}
