use super::*;

/// Poursuit une expression SQL à partir d'un atome déjà construit (utilisé
/// après avoir lu un `a.col` qualifié dans le select-list). On rebranche au
/// niveau le plus haut en réinjectant l'atome comme membre gauche des
/// niveaux arithmétiques/comparaison/booléens.
pub(super) fn continue_expr_from(ts: &mut StatementStream, base: SqlExpr) -> Result<SqlExpr> {
    // Niveaux arithmétiques d'abord (mul/div, concat, add/sub) sur `base`,
    // puis comparaison, puis and/or.
    let after_mul = continue_mul_div(ts, base)?;
    let after_add = continue_add_sub(ts, after_mul)?;
    let after_cmp = continue_compare(ts, after_add)?;
    continue_and_or(ts, after_cmp)
}

pub(super) fn continue_concat(ts: &mut StatementStream, mut left: SqlExpr) -> Result<SqlExpr> {
    while ts.peek().kind == TokenKind::Concat {
        ts.next();
        let right = parse_sql_unary(ts)?;
        left = SqlExpr::Binary {
            op: BinaryOp::Concat,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

pub(super) fn continue_mul_div(ts: &mut StatementStream, base: SqlExpr) -> Result<SqlExpr> {
    let mut left = continue_concat(ts, base)?;
    loop {
        let op = match ts.peek().kind {
            TokenKind::Star => BinaryOp::Mul,
            TokenKind::Slash => BinaryOp::Div,
            _ => break,
        };
        ts.next();
        let right = parse_sql_concat(ts)?;
        left = SqlExpr::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

pub(super) fn continue_add_sub(ts: &mut StatementStream, mut left: SqlExpr) -> Result<SqlExpr> {
    loop {
        let op = match ts.peek().kind {
            TokenKind::Plus => BinaryOp::Add,
            TokenKind::Minus => BinaryOp::Sub,
            _ => break,
        };
        ts.next();
        let right = parse_sql_mul_div(ts)?;
        left = SqlExpr::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

pub(super) fn continue_compare(ts: &mut StatementStream, left: SqlExpr) -> Result<SqlExpr> {
    let negated = if ts.peek().kind == TokenKind::Not {
        if ts.peek2().is_kw("between")
            || ts.peek2().is_kw("like")
            || ts.peek2().is_kw("contains")
            || ts.peek2().is_kw("sounds")
            || ts.peek2().kind == TokenKind::In
        {
            ts.next();
            true
        } else {
            false
        }
    } else {
        false
    };
    if ts.peek().kind == TokenKind::In {
        return parse_sql_in(ts, left, negated);
    }
    if ts.peek().is_kw("between") {
        ts.next();
        let low = parse_sql_add_sub(ts)?;
        expect_and(ts)?;
        let high = parse_sql_add_sub(ts)?;
        return Ok(SqlExpr::Between {
            expr: Box::new(left),
            low: Box::new(low),
            high: Box::new(high),
            negated,
        });
    }
    if ts.peek().is_kw("is") {
        ts.next();
        let negated = if ts.peek().kind == TokenKind::Not {
            ts.next();
            true
        } else {
            false
        };
        if ts.peek().is_kw("null") || ts.peek().is_kw("missing") {
            ts.next();
        } else {
            return Err(SasError::parse(
                "expected NULL or MISSING after IS",
                ts.peek().span,
            ));
        }
        return Ok(SqlExpr::IsNull {
            expr: Box::new(left),
            negated,
        });
    }
    if ts.peek().is_kw("like") {
        ts.next();
        let pat_tok = ts.peek().clone();
        let pattern = match &pat_tok.kind {
            TokenKind::Str { value, .. } => value.clone(),
            _ => {
                return Err(SasError::parse(
                    "expected a string pattern after LIKE",
                    pat_tok.span,
                ));
            }
        };
        ts.next();
        return Ok(SqlExpr::Like {
            expr: Box::new(left),
            pattern,
            negated,
        });
    }
    if ts.peek().is_kw("contains") {
        ts.next();
        let pat_tok = ts.peek().clone();
        let pattern = match &pat_tok.kind {
            TokenKind::Str { value, .. } => value.clone(),
            _ => {
                return Err(SasError::parse(
                    "expected a string after CONTAINS",
                    pat_tok.span,
                ));
            }
        };
        ts.next();
        return Ok(SqlExpr::Contains {
            expr: Box::new(left),
            pattern,
            negated,
        });
    }
    if ts.peek().is_kw("sounds") {
        ts.next();
        expect_kw(ts, "like")?;
        let txt_tok = ts.peek().clone();
        let text = match &txt_tok.kind {
            TokenKind::Str { value, .. } => value.clone(),
            _ => {
                return Err(SasError::parse(
                    "expected a string after SOUNDS LIKE",
                    txt_tok.span,
                ));
            }
        };
        ts.next();
        return Ok(SqlExpr::SoundsLike {
            expr: Box::new(left),
            text,
            negated,
        });
    }
    let op = match ts.peek().kind {
        TokenKind::Eq => Some(BinaryOp::Eq),
        TokenKind::Ne => Some(BinaryOp::Ne),
        TokenKind::Lt => Some(BinaryOp::Lt),
        TokenKind::Le => Some(BinaryOp::Le),
        TokenKind::Gt => Some(BinaryOp::Gt),
        TokenKind::Ge => Some(BinaryOp::Ge),
        _ => None,
    };
    match op {
        Some(op) => {
            ts.next();
            let right = parse_sql_add_sub(ts)?;
            Ok(SqlExpr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            })
        }
        None => Ok(left),
    }
}

pub(super) fn continue_and_or(ts: &mut StatementStream, mut left: SqlExpr) -> Result<SqlExpr> {
    while ts.peek().kind == TokenKind::And {
        ts.next();
        let right = parse_sql_not(ts)?;
        left = SqlExpr::Binary {
            op: BinaryOp::And,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    while ts.peek().kind == TokenKind::Or {
        ts.next();
        let right = parse_sql_and(ts)?;
        left = SqlExpr::Binary {
            op: BinaryOp::Or,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Consomme un mot-clé contextuel attendu (insensible à la casse).
pub(super) fn expect_kw(ts: &mut StatementStream, kw: &str) -> Result<()> {
    if ts.peek().is_kw(kw) {
        ts.next();
        Ok(())
    } else {
        Err(SasError::parse(
            format!("expected '{}'", kw.to_uppercase()),
            ts.peek().span,
        ))
    }
}

pub(super) fn expect_rparen(ts: &mut StatementStream) -> Result<()> {
    if ts.peek().kind == TokenKind::RParen {
        ts.next();
        Ok(())
    } else {
        Err(SasError::parse("expected ')'", ts.peek().span))
    }
}

/// `AND` séparateur de BETWEEN. Lexé en `TokenKind::And` (pas un ident).
pub(super) fn expect_and(ts: &mut StatementStream) -> Result<()> {
    if ts.peek().kind == TokenKind::And {
        ts.next();
        Ok(())
    } else {
        Err(SasError::parse(
            "expected AND in the BETWEEN expression",
            ts.peek().span,
        ))
    }
}
