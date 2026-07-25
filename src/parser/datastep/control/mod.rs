//! Statements de contrôle : IF/THEN/ELSE, SELECT, DO (itératif ou non), GOTO, LINK.

use super::*;


mod select;
mod loops;

pub(crate) use select::*;
pub(crate) use loops::*;

/// `goto label;` / `go to label;` (M16.6). Le mot-clé de tête (`goto` ou `go`)
/// a déjà été identifié ; pour `go`, on consomme le `to` qui suit. La cible est
/// un identifiant unique (résolu en MAJUSCULES à la compilation).
pub(crate) fn parse_goto(ts: &mut StatementStream, head: &str) -> Result<DsStmt> {
    let kw_tok = ts.peek().clone();
    ts.next(); // `goto` ou `go`
    if head == "go" {
        // Forme `go to label;` : le token suivant DOIT être `to`.
        match ts.peek().ident() {
            Some(w) if w.eq_ignore_ascii_case("to") => {
                ts.next();
            }
            _ => {
                return Err(SasError::parse(
                    "expected TO after GO (use `go to label;` or `goto label;`)",
                    ts.peek().span,
                ));
            }
        }
    }
    let label_tok = ts.peek().clone();
    let label = match label_tok.ident() {
        Some(s) => s.to_string(),
        None => {
            return Err(SasError::parse(
                "expected a statement label after GOTO",
                kw_tok.span,
            ));
        }
    };
    ts.next(); // label
    ts.expect_semi()?;
    Ok(DsStmt::Goto(label))
}

/// `link label;` (M16.6). La cible est un identifiant unique (résolu en
/// MAJUSCULES à la compilation).
pub(crate) fn parse_link(ts: &mut StatementStream) -> Result<DsStmt> {
    let kw_tok = ts.peek().clone();
    ts.next(); // `link`
    let label_tok = ts.peek().clone();
    let label = match label_tok.ident() {
        Some(s) => s.to_string(),
        None => {
            return Err(SasError::parse(
                "expected a statement label after LINK",
                kw_tok.span,
            ));
        }
    };
    ts.next(); // label
    ts.expect_semi()?;
    Ok(DsStmt::Link(label))
}

/// `if expr then stmt [else stmt]` ou `if expr ;` (subsetting).
pub(crate) fn parse_if(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `if`
    let cond = super::expr::parse_expr(ts)?;
    if ts.peek().is_kw("then") {
        ts.next(); // `then`
        let then_branch = Box::new(parse_branch_statement(ts)?);
        let else_branch = if ts.peek().is_kw("else") {
            ts.next(); // `else`
            Some(Box::new(parse_branch_statement(ts)?))
        } else {
            None
        };
        Ok(DsStmt::If {
            cond,
            then_branch,
            else_branch,
        })
    } else if ts.peek().kind == TokenKind::Semi {
        ts.next(); // `;`
        Ok(DsStmt::SubsettingIf(cond))
    } else {
        Err(SasError::parse(
            "expected THEN or ';' after the IF condition",
            ts.peek().span,
        ))
    }
}

/// Une branche de IF/THEN ou IF/ELSE : UN statement (récursion). Les
/// frontières de bloc ou `run`/`quit` ne peuvent pas servir de branche.
fn parse_branch_statement(ts: &mut StatementStream) -> Result<DsStmt> {
    let tok = ts.peek().clone();
    if let Some(s) = tok.ident() {
        let lower = s.to_ascii_lowercase();
        if lower == "run" || lower == "quit" || is_block_head_kw(&lower) {
            return Err(SasError::parse(
                "expected a statement after THEN/ELSE",
                tok.span,
            ));
        }
    }
    parse_statement(ts)
}
