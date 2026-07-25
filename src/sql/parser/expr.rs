use super::*;

// ── Expressions SqlExpr ──────────────────────────────────────────────────
//
// Échelle de précédence (faible → fort) :
//   or → and → not → comparaison (+ BETWEEN/IS/LIKE) → add_sub → mul_div
//      → concat → unary(+/-) → atome.

/// Point d'entrée d'une expression SQL.
pub(super) fn parse_sql_expr(ts: &mut StatementStream) -> Result<SqlExpr> {
    parse_sql_or(ts)
}

pub(super) fn parse_sql_or(ts: &mut StatementStream) -> Result<SqlExpr> {
    let mut left = parse_sql_and(ts)?;
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

pub(super) fn parse_sql_and(ts: &mut StatementStream) -> Result<SqlExpr> {
    let mut left = parse_sql_not(ts)?;
    while ts.peek().kind == TokenKind::And {
        ts.next();
        let right = parse_sql_not(ts)?;
        left = SqlExpr::Binary {
            op: BinaryOp::And,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

/// `NOT` préfixe (au niveau booléen).
pub(super) fn parse_sql_not(ts: &mut StatementStream) -> Result<SqlExpr> {
    if ts.peek().kind == TokenKind::Not {
        ts.next();
        let expr = parse_sql_not(ts)?;
        return Ok(SqlExpr::Unary {
            op: UnaryOp::Not,
            expr: Box::new(expr),
        });
    }
    parse_sql_compare(ts)
}

/// Comparaison `= <> != < <= > >=`, plus les postfixes non-associatifs
/// `[NOT] BETWEEN a AND b`, `IS [NOT] NULL|MISSING`, `[NOT] LIKE 'p'`.
pub(super) fn parse_sql_compare(ts: &mut StatementStream) -> Result<SqlExpr> {
    let left = parse_sql_add_sub(ts)?;

    // Postfixes SQL : BETWEEN / IS / LIKE / IN, éventuellement précédés de
    // NOT (lexé en `TokenKind::Not`, PAS un ident).
    let negated = if ts.peek().kind == TokenKind::Not {
        // `x NOT BETWEEN ...`, `x NOT LIKE ...`, `x NOT IN ...`. On ne
        // consomme NOT que si un postfixe reconnu suit.
        if ts.peek2().is_kw("between")
            || ts.peek2().is_kw("like")
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

    // `x [NOT] IN ( ... )` — une parenthèse suivie de SELECT = sous-requête
    // interdite, sinon liste de littéraux (réutilise parse_expr).
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
    if ts.peek().is_kw("is") {
        ts.next();
        let is_negated = if ts.peek().kind == TokenKind::Not {
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
            negated: is_negated,
        });
    }

    // Comparaison binaire ordinaire (non associative).
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

pub(super) fn parse_sql_add_sub(ts: &mut StatementStream) -> Result<SqlExpr> {
    let mut left = parse_sql_mul_div(ts)?;
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

pub(super) fn parse_sql_mul_div(ts: &mut StatementStream) -> Result<SqlExpr> {
    let mut left = parse_sql_concat(ts)?;
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

pub(super) fn parse_sql_concat(ts: &mut StatementStream) -> Result<SqlExpr> {
    let mut left = parse_sql_unary(ts)?;
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

/// Préfixes arithmétiques `+` / `-`.
pub(super) fn parse_sql_unary(ts: &mut StatementStream) -> Result<SqlExpr> {
    let op = match ts.peek().kind {
        TokenKind::Plus => Some(UnaryOp::Plus),
        TokenKind::Minus => Some(UnaryOp::Minus),
        _ => None,
    };
    match op {
        Some(op) => {
            ts.next();
            let expr = parse_sql_unary(ts)?;
            Ok(SqlExpr::Unary {
                op,
                expr: Box::new(expr),
            })
        }
        None => parse_sql_atom(ts),
    }
}

/// Atome : CALCULATED, agrégat, `a.x` qualifié, parenthèses, ou base `Expr`
/// (littéral / variable / appel de fonction non agrégé).
pub(super) fn parse_sql_atom(ts: &mut StatementStream) -> Result<SqlExpr> {
    let tok = ts.peek().clone();

    // `( SELECT ... )` — sous-requête scalaire (M20.2).
    if tok.kind == TokenKind::LParen && ts.peek2().is_kw("select") {
        ts.next(); // (
        let query = parse_select(ts)?;
        expect_rparen(ts)?;
        return Ok(SqlExpr::Subquery(Box::new(query)));
    }

    // `( <sqlexpr> )` — parenthèses ordinaires.
    if tok.kind == TokenKind::LParen {
        ts.next(); // (
        let inner = parse_sql_expr(ts)?;
        if ts.peek().kind != TokenKind::RParen {
            return Err(SasError::parse("expected ')'", ts.peek().span));
        }
        ts.next(); // )
        return Ok(inner);
    }

    if let TokenKind::Ident(name) = &tok.kind {
        let lower = name.to_ascii_lowercase();
        let name = name.clone();

        // `EXISTS ( SELECT ... )` (M20.2). Le `NOT` préfixe est géré au niveau
        // booléen (parse_sql_not) → `NOT (EXISTS ...)`.
        if lower == "exists" && ts.peek2().kind == TokenKind::LParen {
            ts.next(); // EXISTS
            ts.next(); // (
            if !ts.peek().is_kw("select") {
                return Err(SasError::parse(
                    "expected SELECT after EXISTS (",
                    ts.peek().span,
                ));
            }
            let query = parse_select(ts)?;
            expect_rparen(ts)?;
            return Ok(SqlExpr::Exists {
                query: Box::new(query),
                negated: false,
            });
        }

        // CALCULATED <ident>.
        if lower == "calculated" {
            ts.next(); // CALCULATED
            let id_tok = ts.peek().clone();
            let Some(col) = id_tok.ident().map(str::to_string) else {
                return Err(SasError::parse(
                    "expected a column name after CALCULATED",
                    id_tok.span,
                ));
            };
            ts.next();
            return Ok(SqlExpr::Calculated(col));
        }

        // Agrégats : COUNT/SUM/AVG/MIN/MAX (+ COUNT(*) / DISTINCT).
        if is_aggregate(&lower) && ts.peek2().kind == TokenKind::LParen {
            return parse_aggregate(ts, &lower);
        }

        // `a.x` qualifié (ident `.` ident). Pas de lib.table dans une
        // expression scalaire.
        if ts.peek2().kind == TokenKind::Dot {
            ts.next(); // ident
            ts.next(); // dot
            let col_tok = ts.peek().clone();
            let Some(col) = col_tok.ident().map(str::to_string) else {
                return Err(SasError::parse(
                    "expected a column name after '.'",
                    col_tok.span,
                ));
            };
            ts.next();
            return Ok(SqlExpr::Qualified {
                table: name,
                column: col,
            });
        }

        // Appel de fonction non agrégé `f(args)` : on délègue le parsing des
        // arguments à parse_expr (sous-arbre Expr autonome), en réutilisant
        // parse_primary via une expression complète. Plus simple : déléguer
        // toute l'expression de base à parse_expr.
        // Variable simple ou appel : déléguer à parse_expr pour récupérer un
        // Expr de base maximal (mais sans avaler les opérateurs SQL : on
        // n'arrive ici qu'avec un atome, parse_expr lira un atome+postfix de
        // base — variable, call, index, power — ce qui est correct).
        return parse_base_atom(ts);
    }

    // Littéraux (Num, Str, Dot=missing, ...) : déléguer à parse_expr (atome).
    parse_base_atom(ts)
}

/// `expr [NOT] IN ( <liste de littéraux> )`. Le token `IN` est en tête. Une
/// `(` suivie de SELECT = sous-requête interdite. La liste de valeurs réutilise
/// `parse_expr` (littéraux de base). Représenté comme `SqlExpr::Base(Expr::In)`
/// avec un membre gauche aplati (Qualified → "table.column").
pub(super) fn parse_sql_in(ts: &mut StatementStream, left: SqlExpr, negated: bool) -> Result<SqlExpr> {
    ts.next(); // IN
    if ts.peek().kind != TokenKind::LParen {
        return Err(SasError::parse("expected '(' after IN", ts.peek().span));
    }
    // `expr [NOT] IN ( SELECT ... )` — sous-requête de liste (M20.2).
    if ts.peek2().is_kw("select") {
        ts.next(); // (
        let query = parse_select(ts)?;
        expect_rparen(ts)?;
        return Ok(SqlExpr::InSubquery {
            expr: Box::new(left),
            query: Box::new(query),
            negated,
        });
    }
    ts.next(); // (
    let mut list = Vec::new();
    if ts.peek().kind != TokenKind::RParen {
        loop {
            list.push(parse_expr(ts)?);
            match ts.peek().kind {
                TokenKind::Comma => {
                    ts.next();
                }
                _ => break,
            }
        }
    }
    expect_rparen(ts)?;
    let base_left = sql_expr_to_base(&left).ok_or_else(|| {
        SasError::parse(
            "unsupported left-hand side for IN",
            ts.peek().span,
        )
    })?;
    let in_expr = Expr::In {
        expr: Box::new(base_left),
        list,
    };
    if negated {
        Ok(SqlExpr::Unary {
            op: UnaryOp::Not,
            expr: Box::new(SqlExpr::Base(in_expr)),
        })
    } else {
        Ok(SqlExpr::Base(in_expr))
    }
}

/// Aplatit un SqlExpr « scalaire simple » en Expr (pour le membre gauche d'un
/// IN). `Base` → tel quel ; `Qualified{t,c}` → `Var("t.c")`.
pub(super) fn sql_expr_to_base(e: &SqlExpr) -> Option<Expr> {
    match e {
        SqlExpr::Base(b) => Some(b.clone()),
        SqlExpr::Qualified { table, column } => Some(Expr::Var(format!("{table}.{column}"))),
        _ => None,
    }
}
