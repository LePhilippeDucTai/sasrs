use super::*;

// ── statements de corps réutilisables (M31.2 — déplacés depuis means.rs) ──────
//
// `parse_single_var` et `parse_by` (ex-`means::parse_by_list`) sont déplacés
// VERBATIM depuis `means.rs` (corps inchangés) ; `means.rs` les ré-exporte pour
// ses appelants existants (`univariate.rs`, `rank.rs`). `parse_weight`,
// `parse_var_list` et `parse_class` sont des aides génériques additives.

/// Parse un nom de variable unique pour un statement mono-variable (après que le
/// mot-clé `kw` a été consommé par l'appelant), jusqu'à son `;`.
/// `kw stat ;` — déplacé verbatim depuis `means::parse_single_var`.
pub(crate) fn parse_single_var(ts: &mut StatementStream, kw: &str) -> Result<String> {
    let tok = ts.peek().clone();
    let name = match tok.ident() {
        Some(n) => {
            ts.next();
            n.to_string()
        }
        None => {
            return Err(SasError::parse(
                format!("expected a variable name in the {kw} statement"),
                tok.span,
            ));
        }
    };
    // Consume the terminating `;` (tolerate trailing tokens by skipping).
    if ts.peek().kind == TokenKind::Semi {
        ts.next();
    } else {
        ts.skip_to_semi();
    }
    Ok(name)
}

/// Parse a BY statement body (after "by" consumed), through its `;`.
/// `by [descending] v1 [descending] v2 ... ;` — mirrors PROC SORT.
/// Déplacé verbatim depuis `means::parse_by_list`.
pub(crate) fn parse_by(ts: &mut StatementStream) -> Result<Vec<(String, bool)>> {
    let mut by: Vec<(String, bool)> = Vec::new();
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        let descending = if ts.peek().is_kw("descending") {
            ts.next();
            true
        } else {
            false
        };
        let tok = ts.peek().clone();
        match tok.ident() {
            Some(name) => {
                ts.next();
                by.push((name.to_string(), descending));
            }
            None => {
                return Err(SasError::parse(
                    "expected a variable name in the BY statement",
                    tok.span,
                ));
            }
        }
    }
    Ok(by)
}

/// `weight var ;` — raccourci de `parse_single_var(ts, "WEIGHT")`.
#[allow(dead_code)]
pub(crate) fn parse_weight(ts: &mut StatementStream) -> Result<String> {
    parse_single_var(ts, "WEIGHT")
}

/// Parse une liste de noms de variables terminée par `;` (`v1 v2 … ;`), comme
/// le statement VAR de `print.rs` (`parse_name_list()` + `expect_semi()`).
#[allow(dead_code)]
pub(crate) fn parse_var_list(ts: &mut StatementStream) -> Result<Vec<String>> {
    let names = ts.parse_name_list()?;
    ts.expect_semi()?;
    Ok(names)
}

/// `class v1 v2 … ;` — même grammaire qu'une liste de noms (`parse_var_list`).
#[allow(dead_code)]
pub(crate) fn parse_class(ts: &mut StatementStream) -> Result<Vec<String>> {
    parse_var_list(ts)
}
