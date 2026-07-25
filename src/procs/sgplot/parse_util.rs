use super::*;

// ───────────────────────── Parser helpers ─────────────────────────

pub(super) fn expect_eq(ts: &mut StatementStream, opt: &str) -> Result<()> {
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            format!("expected '=' after {opt}"),
            ts.peek().span,
        ));
    }
    ts.next();
    Ok(())
}

/// Lit un nom de variable (identifiant). Erreur propre sinon.
pub(super) fn expect_ident(ts: &mut StatementStream, ctx: &str) -> Result<String> {
    match ts.peek().ident().map(str::to_string) {
        Some(s) => {
            ts.next();
            Ok(s)
        }
        None => Err(SasError::parse(
            format!("expected an identifier {ctx}"),
            ts.peek().span,
        )),
    }
}

/// Lit une valeur numérique littérale. Erreur propre sinon.
pub(super) fn expect_number(ts: &mut StatementStream, ctx: &str) -> Result<f64> {
    match ts.peek().kind {
        TokenKind::Num(f) => {
            ts.next();
            Ok(f)
        }
        _ => Err(SasError::parse(
            format!("expected a number {ctx}"),
            ts.peek().span,
        )),
    }
}

/// Lit une valeur de chaîne (string littérale ou identifiant).
pub(super) fn read_value(ts: &mut StatementStream) -> Option<String> {
    match &ts.peek().kind {
        TokenKind::Str { value, .. } => {
            let v = value.clone();
            ts.next();
            Some(v)
        }
        TokenKind::Ident(s) => {
            let v = s.clone();
            ts.next();
            Some(v)
        }
        TokenKind::Num(f) => {
            let f = *f;
            ts.next();
            Some(if f.fract() == 0.0 {
                format!("{}", f as i64)
            } else {
                format!("{f}")
            })
        }
        _ => None,
    }
}

/// Parse une liste parenthésée d'options `name=value` (MARKERATTRS, LINEATTRS).
/// Consomme `( ... )`. Renvoie les paires sous forme brute (UPPERCASE name).
pub(super) fn parse_paren_attrs(ts: &mut StatementStream) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if ts.peek().kind != TokenKind::LParen {
        return out;
    }
    ts.next(); // (
    loop {
        match &ts.peek().kind {
            TokenKind::RParen => {
                ts.next();
                break;
            }
            TokenKind::Eof | TokenKind::Semi => break,
            _ => {}
        }
        let name = match ts.peek().ident().map(|s| s.to_ascii_lowercase()) {
            Some(n) => {
                ts.next();
                n
            }
            None => {
                ts.next();
                continue;
            }
        };
        if ts.peek().kind == TokenKind::Eq {
            ts.next();
            if let Some(v) = read_value(ts) {
                out.push((name, v));
            }
        }
    }
    out
}
