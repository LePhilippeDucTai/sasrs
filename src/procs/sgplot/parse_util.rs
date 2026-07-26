use super::*;

// ───────────────────────── Parser helpers ─────────────────────────

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
