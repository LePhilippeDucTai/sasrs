use super::*;

impl Parser {
    pub(super) fn parse_if(&mut self) -> Result<ImlStmt> {
        self.next(); // if
        let cond = self.parse_expr()?;
        // then
        let then_kw = self.expect_ident("THEN")?;
        if !then_kw.eq_ignore_ascii_case("then") {
            return Err(SasError::runtime("IML: expected THEN after IF condition"));
        }
        let then_body = self.parse_then_or_block()?;
        let else_body = if matches!(self.peek(), Tok::Ident(s) if s.eq_ignore_ascii_case("else")) {
            self.next(); // else
            self.parse_then_or_block()?
        } else {
            Vec::new()
        };
        Ok(ImlStmt::If {
            cond,
            then_body,
            else_body,
        })
    }

    /// Après THEN/ELSE : soit `DO; ... END;`, soit un statement unique.
    pub(super) fn parse_then_or_block(&mut self) -> Result<Vec<ImlStmt>> {
        if matches!(self.peek(), Tok::Ident(s) if s.eq_ignore_ascii_case("do"))
            && self.peek_is_bare_do()
        {
            self.next(); // do
            self.expect(&Tok::Semi, "';' after DO")?;
            let body = self.parse_block_until_end()?;
            // Un `;` après END est optionnel ici (consommé par l'appelant ou non).
            self.eat(&Tok::Semi);
            Ok(body)
        } else {
            Ok(vec![self.parse_stmt()?])
        }
    }

    /// Vrai si le token courant `do` est un DO « nu » (`do;`) et non un DO
    /// itératif/while/until (`do i=...`, `do while(...)`, `do until(...)`).
    pub(super) fn peek_is_bare_do(&self) -> bool {
        // toks[pos] == do ; regarder toks[pos+1].
        matches!(self.toks.get(self.pos + 1), Some(Tok::Semi))
    }

    pub(super) fn parse_do(&mut self) -> Result<ImlStmt> {
        self.next(); // do
        // DO; (bloc nu) — non attendu au niveau statement, mais tolérons-le.
        if self.eat(&Tok::Semi) {
            let body = self.parse_block_until_end()?;
            self.expect(&Tok::Semi, "';' after END")?;
            // Bloc nu ≡ exécution séquentielle : on le rend comme un IF vrai.
            return Ok(ImlStmt::If {
                cond: ImlExpr::Literal(vec![vec![1.0]]),
                then_body: body,
                else_body: Vec::new(),
            });
        }
        // DO WHILE (cond) / DO UNTIL (cond)
        if let Tok::Ident(s) = self.peek().clone()
            && (s.eq_ignore_ascii_case("while") || s.eq_ignore_ascii_case("until"))
        {
            let is_while = s.eq_ignore_ascii_case("while");
            self.next();
            self.expect(&Tok::LParen, "'(' after DO WHILE/UNTIL")?;
            let cond = self.parse_expr()?;
            self.expect(&Tok::RParen, "')'")?;
            self.expect(&Tok::Semi, "';' after DO WHILE/UNTIL")?;
            let body = self.parse_block_until_end()?;
            self.expect(&Tok::Semi, "';' after END")?;
            return Ok(if is_while {
                ImlStmt::DoWhile { cond, body }
            } else {
                ImlStmt::DoUntil { cond, body }
            });
        }
        // DO i = from TO to [BY by];
        let var = self.expect_ident("a loop variable after DO")?;
        self.expect(&Tok::Eq, "'=' in a DO loop")?;
        let from = self.parse_expr()?;
        let to_kw = self.expect_ident("TO")?;
        if !to_kw.eq_ignore_ascii_case("to") {
            return Err(SasError::runtime("IML: expected TO in a DO loop"));
        }
        let to = self.parse_expr()?;
        let by = if matches!(self.peek(), Tok::Ident(s) if s.eq_ignore_ascii_case("by")) {
            self.next();
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect(&Tok::Semi, "';' after a DO loop header")?;
        let body = self.parse_block_until_end()?;
        self.expect(&Tok::Semi, "';' after END")?;
        Ok(ImlStmt::DoLoop {
            var,
            from,
            to,
            by,
            body,
        })
    }

    /// Parse des statements jusqu'à `END` (consommé), sans le `;` final.
    pub(super) fn parse_block_until_end(&mut self) -> Result<Vec<ImlStmt>> {
        let mut body = Vec::new();
        loop {
            if self.eat(&Tok::Semi) {
                continue;
            }
            if matches!(self.peek(), Tok::Ident(s) if s.eq_ignore_ascii_case("end")) {
                self.next(); // end
                return Ok(body);
            }
            if self.peek() == &Tok::Eof {
                return Err(SasError::runtime("IML: missing END for a DO block"));
            }
            body.push(self.parse_stmt()?);
        }
    }

    pub(super) fn parse_arg_list(&mut self) -> Result<Vec<ImlExpr>> {
        let mut args = Vec::new();
        if self.peek() == &Tok::RParen {
            return Ok(args);
        }
        loop {
            args.push(self.parse_expr()?);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        Ok(args)
    }
}
