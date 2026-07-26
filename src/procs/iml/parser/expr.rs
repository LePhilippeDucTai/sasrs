use super::*;

impl Parser {
    // ── Expressions (par niveaux de précédence) ──

    pub(super) fn parse_expr(&mut self) -> Result<ImlExpr> {
        self.parse_compare()
    }

    pub(super) fn parse_compare(&mut self) -> Result<ImlExpr> {
        let left = self.parse_add()?;
        let op = match self.peek() {
            Tok::Eq => Some(ImlOp::Eq),
            Tok::Ne => Some(ImlOp::Ne),
            Tok::Lt => Some(ImlOp::Lt),
            Tok::Le => Some(ImlOp::Le),
            Tok::Gt => Some(ImlOp::Gt),
            Tok::Ge => Some(ImlOp::Ge),
            _ => None,
        };
        if let Some(op) = op {
            self.next();
            let right = self.parse_add()?;
            Ok(ImlExpr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            })
        } else {
            Ok(left)
        }
    }

    pub(super) fn parse_add(&mut self) -> Result<ImlExpr> {
        let mut left = self.parse_mul()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => ImlOp::Add,
                Tok::Minus => ImlOp::Sub,
                _ => break,
            };
            self.next();
            let right = self.parse_mul()?;
            left = ImlExpr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    pub(super) fn parse_mul(&mut self) -> Result<ImlExpr> {
        let mut left = self.parse_kron()?;
        loop {
            let op = match self.peek() {
                Tok::Star => ImlOp::Mul,
                Tok::Hash => ImlOp::Hadamard,
                Tok::Slash => ImlOp::Div,
                _ => break,
            };
            self.next();
            let right = self.parse_kron()?;
            left = ImlExpr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    pub(super) fn parse_kron(&mut self) -> Result<ImlExpr> {
        let mut left = self.parse_unary()?;
        while self.peek() == &Tok::At {
            self.next();
            let right = self.parse_unary()?;
            left = ImlExpr::BinOp {
                op: ImlOp::Kronecker,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    pub(super) fn parse_unary(&mut self) -> Result<ImlExpr> {
        if self.peek() == &Tok::Minus {
            self.next();
            let e = self.parse_unary()?;
            return Ok(ImlExpr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(e),
            });
        }
        self.parse_postfix()
    }

    /// Postfix : transposée `'` et indexation `[...]`, en boucle.
    pub(super) fn parse_postfix(&mut self) -> Result<ImlExpr> {
        let mut e = self.parse_primary()?;
        loop {
            match self.peek() {
                Tok::Quote => {
                    self.next();
                    e = ImlExpr::Transpose(Box::new(e));
                }
                Tok::LBracket => {
                    self.next();
                    let (row, col) = self.parse_subscript()?;
                    self.expect(&Tok::RBracket, "']'")?;
                    e = ImlExpr::Subscript {
                        mat: Box::new(e),
                        row,
                        col,
                    };
                }
                _ => break,
            }
        }
        Ok(e)
    }

    /// `[ row , col ]` ou `[ idx ]` (1-D, non supporté → erreur à l'exec).
    pub(super) fn parse_subscript(&mut self) -> Result<(ImlIndex, ImlIndex)> {
        let row = self.parse_index()?;
        if self.eat(&Tok::Comma) {
            let col = self.parse_index()?;
            Ok((row, col))
        } else {
            // Indexation 1-D : différée v1.
            Ok((row, ImlIndex::All))
        }
    }

    pub(super) fn parse_index(&mut self) -> Result<ImlIndex> {
        // `*` or an empty position (before `,` or `]`) both mean "all".
        if self.peek() == &Tok::Star {
            self.next();
            return Ok(ImlIndex::All);
        }
        if matches!(self.peek(), Tok::Comma | Tok::RBracket) {
            return Ok(ImlIndex::All);
        }
        let e = self.parse_add()?;
        if self.eat(&Tok::Colon) {
            let e2 = self.parse_add()?;
            return Ok(ImlIndex::Range(Box::new(e), Box::new(e2)));
        }
        Ok(ImlIndex::Scalar(Box::new(e)))
    }

    pub(super) fn parse_primary(&mut self) -> Result<ImlExpr> {
        match self.peek().clone() {
            Tok::Num(_) | Tok::Minus => {
                // Un nombre nu hors littéral n'est pas valide en IML pur, mais
                // on l'accepte comme matrice 1×1 pour les expressions de flux.
                if let Tok::Num(v) = self.peek().clone() {
                    self.next();
                    return Ok(ImlExpr::Literal(vec![vec![v]]));
                }
                unreachable!()
            }
            Tok::LBrace => self.parse_matrix_literal(),
            Tok::LParen => {
                self.next();
                let e = self.parse_expr()?;
                self.expect(&Tok::RParen, "')'")?;
                Ok(e)
            }
            Tok::Ident(name) => {
                self.next();
                if self.eat(&Tok::LParen) {
                    let args = self.parse_arg_list()?;
                    self.expect(&Tok::RParen, "')'")?;
                    Ok(ImlExpr::FnCall { name, args })
                } else {
                    Ok(ImlExpr::Var(name))
                }
            }
            other => Err(SasError::runtime(format!(
                "IML: unexpected token in an expression: {other:?}"
            ))),
        }
    }

    /// `{ a b , c d }` — espace = élément, virgule = nouvelle ligne.
    /// `{ "x" "y" }` — liste de chaînes (pour COLNAME=, READ VAR, etc.).
    pub(super) fn parse_matrix_literal(&mut self) -> Result<ImlExpr> {
        self.expect(&Tok::LBrace, "'{'")?;
        // Liste de chaînes : `{ "x" "y" ... }`.
        if matches!(self.peek(), Tok::Str(_)) {
            let mut strs = Vec::new();
            loop {
                match self.peek().clone() {
                    Tok::Str(s) => {
                        self.next();
                        strs.push(s);
                    }
                    Tok::RBrace => {
                        self.next();
                        break;
                    }
                    other => {
                        return Err(SasError::runtime(format!(
                            "IML: a string literal list may only contain strings, found {other:?}"
                        )));
                    }
                }
            }
            return Ok(ImlExpr::StrList(strs));
        }
        let mut rows: Vec<Vec<f64>> = Vec::new();
        let mut cur: Vec<f64> = Vec::new();
        loop {
            match self.peek().clone() {
                Tok::RBrace => {
                    self.next();
                    rows.push(cur);
                    break;
                }
                Tok::Comma => {
                    self.next();
                    rows.push(std::mem::take(&mut cur));
                }
                Tok::Minus => {
                    self.next();
                    let v = self.expect_number()?;
                    cur.push(-v);
                }
                Tok::Num(v) => {
                    self.next();
                    cur.push(v);
                }
                other => {
                    return Err(SasError::runtime(format!(
                        "IML: matrix literals support only numeric constants, found {other:?}"
                    )));
                }
            }
        }
        // Valider la rectangularité.
        let ncol = rows.first().map(|r| r.len()).unwrap_or(0);
        if rows.iter().any(|r| r.len() != ncol) {
            return Err(SasError::runtime(
                "IML: all rows of a matrix literal must have the same number of elements",
            ));
        }
        if ncol == 0 {
            return Err(SasError::runtime("IML: empty matrix literal"));
        }
        Ok(ImlExpr::Literal(rows))
    }
}
