use super::*;

impl Parser {
    pub(super) fn parse_program(&mut self) -> Result<ImlProgram> {
        let mut stmts = Vec::new();
        while self.peek() != &Tok::Eof {
            // Tolérer les `;` vides.
            if self.eat(&Tok::Semi) {
                continue;
            }
            stmts.push(self.parse_stmt()?);
        }
        Ok(ImlProgram { stmts })
    }

    /// Parse un statement (sans le `;` final consommé, sauf flux qui gèrent `end`).
    pub(super) fn parse_stmt(&mut self) -> Result<ImlStmt> {
        let tok = self.peek().clone();
        let Tok::Ident(kw) = tok else {
            return Err(SasError::runtime(format!(
                "IML: expected a statement, found {:?}",
                self.peek()
            )));
        };
        match kw.to_ascii_lowercase().as_str() {
            "print" => {
                self.next();
                let items = self.parse_print_items()?;
                self.expect(&Tok::Semi, "';' after PRINT")?;
                Ok(ImlStmt::Print { items })
            }
            "if" => self.parse_if(),
            "do" => self.parse_do(),
            "call" => {
                self.next();
                let name = self.expect_ident("a routine name after CALL")?;
                let args = if self.eat(&Tok::LParen) {
                    let a = self.parse_arg_list()?;
                    self.expect(&Tok::RParen, "')'")?;
                    a
                } else {
                    Vec::new()
                };
                self.expect(&Tok::Semi, "';' after CALL")?;
                Ok(ImlStmt::Call { func: name, args })
            }
            "create" => self.parse_create(),
            "append" => self.parse_append(),
            "close" => self.parse_close(),
            "use" => self.parse_use(),
            "read" => self.parse_read(),
            // Statements I/O différés : erreur propre à l'exécution.
            "store" | "load" | "show" => {
                self.next();
                while self.peek() != &Tok::Semi && self.peek() != &Tok::Eof {
                    self.next();
                }
                self.expect(&Tok::Semi, "';'")?;
                Ok(ImlStmt::UnsupportedIo {
                    msg: format!("{} is not yet implemented in PROC IML", kw.to_uppercase()),
                })
            }
            // Autres statements de gestion : consommés sans effet (best-effort).
            "edit" | "reset" | "free" | "remove" => {
                self.next();
                while self.peek() != &Tok::Semi && self.peek() != &Tok::Eof {
                    self.next();
                }
                self.expect(&Tok::Semi, "';'")?;
                Ok(ImlStmt::UnsupportedIo {
                    msg: format!(
                        "the {} statement is not yet implemented in PROC IML",
                        kw.to_uppercase()
                    ),
                })
            }
            _ => {
                // Assignation : ident [subscript] = expr ;
                let var = self.expect_ident("a variable name")?;
                self.expect(&Tok::Eq, "'=' in an assignment")?;
                let expr = self.parse_expr()?;
                self.expect(&Tok::Semi, "';' after an assignment")?;
                Ok(ImlStmt::Assign { var, expr })
            }
        }
    }

    /// Parse un nom de dataset possiblement qualifié : `name` ou `lib.name`.
    /// Retourne la forme canonique en MAJUSCULES (`LIB.NAME` ou `NAME`).
    pub(super) fn parse_dataset_name(&mut self, what: &str) -> Result<String> {
        let first = self.expect_ident(what)?;
        if self.eat(&Tok::Dot) {
            let second = self.expect_ident("a dataset name after '.'")?;
            Ok(format!(
                "{}.{}",
                first.to_uppercase(),
                second.to_uppercase()
            ))
        } else {
            Ok(first.to_uppercase())
        }
    }

    /// `CREATE ds FROM mat [COLNAME=cn];`
    pub(super) fn parse_create(&mut self) -> Result<ImlStmt> {
        self.next(); // create
        let ds = self.parse_dataset_name("a dataset name after CREATE")?;
        let from_kw = self.expect_ident("FROM")?;
        if !from_kw.eq_ignore_ascii_case("from") {
            return Err(SasError::runtime(
                "IML: expected FROM in a CREATE statement",
            ));
        }
        let from = self
            .expect_ident("a matrix name after FROM")?
            .to_uppercase();
        // Option [COLNAME=cn] ou [colname=cn].
        let mut colname = None;
        if self.eat(&Tok::LBracket) {
            let opt = self.expect_ident("an option name (COLNAME)")?;
            if !opt.eq_ignore_ascii_case("colname") {
                return Err(SasError::runtime(format!(
                    "IML: unsupported CREATE option '{opt}' (only COLNAME= is supported)"
                )));
            }
            self.expect(&Tok::Eq, "'=' after COLNAME")?;
            colname = Some(self.parse_primary()?);
            self.expect(&Tok::RBracket, "']'")?;
        }
        self.expect(&Tok::Semi, "';' after CREATE")?;
        Ok(ImlStmt::Create { ds, from, colname })
    }

    /// `APPEND FROM mat;`
    pub(super) fn parse_append(&mut self) -> Result<ImlStmt> {
        self.next(); // append
        let from_kw = self.expect_ident("FROM")?;
        if !from_kw.eq_ignore_ascii_case("from") {
            return Err(SasError::runtime(
                "IML: expected FROM in an APPEND statement",
            ));
        }
        let from = self
            .expect_ident("a matrix name after FROM")?
            .to_uppercase();
        self.expect(&Tok::Semi, "';' after APPEND")?;
        Ok(ImlStmt::Append { from })
    }

    /// `CLOSE ds;`
    pub(super) fn parse_close(&mut self) -> Result<ImlStmt> {
        self.next(); // close
        let ds = self.parse_dataset_name("a dataset name after CLOSE")?;
        self.expect(&Tok::Semi, "';' after CLOSE")?;
        Ok(ImlStmt::Close { ds })
    }

    /// `USE ds;`
    pub(super) fn parse_use(&mut self) -> Result<ImlStmt> {
        self.next(); // use
        let ds = self.parse_dataset_name("a dataset name after USE")?;
        self.expect(&Tok::Semi, "';' after USE")?;
        Ok(ImlStmt::Use { ds })
    }

    /// `READ ALL VAR {vars} INTO mat;` (autres formes → erreur propre).
    pub(super) fn parse_read(&mut self) -> Result<ImlStmt> {
        self.next(); // read
        let mode = self.expect_ident("ALL or NEXT after READ")?;
        if mode.eq_ignore_ascii_case("next") {
            while self.peek() != &Tok::Semi && self.peek() != &Tok::Eof {
                self.next();
            }
            self.expect(&Tok::Semi, "';'")?;
            return Ok(ImlStmt::UnsupportedIo {
                msg: "READ NEXT not yet implemented; use READ ALL instead".to_string(),
            });
        }
        if !mode.eq_ignore_ascii_case("all") {
            return Err(SasError::runtime("IML: expected ALL or NEXT after READ"));
        }
        let var_kw = self.expect_ident("VAR after READ ALL")?;
        if !var_kw.eq_ignore_ascii_case("var") {
            return Err(SasError::runtime("IML: expected VAR after READ ALL"));
        }
        // Liste de variables : `{ "x" "y" }` ou `{ x y }`.
        let vars = self.parse_var_list()?;
        // INTO mat ou WHERE ... .
        let kw = self.expect_ident("INTO or WHERE after the variable list")?;
        if kw.eq_ignore_ascii_case("where") {
            while self.peek() != &Tok::Semi && self.peek() != &Tok::Eof {
                self.next();
            }
            self.expect(&Tok::Semi, "';'")?;
            return Ok(ImlStmt::UnsupportedIo {
                msg: "WHERE clause in READ not yet implemented".to_string(),
            });
        }
        if !kw.eq_ignore_ascii_case("into") {
            return Err(SasError::runtime(
                "IML: expected INTO after the variable list",
            ));
        }
        let into = self
            .expect_ident("a matrix name after INTO")?
            .to_uppercase();
        self.expect(&Tok::Semi, "';' after READ")?;
        Ok(ImlStmt::ReadAll { vars, into })
    }

    /// Liste de variables `{ "x" "y" }` ou `{ x y }` (noms en MAJUSCULES).
    pub(super) fn parse_var_list(&mut self) -> Result<Vec<String>> {
        self.expect(&Tok::LBrace, "'{' to begin a variable list")?;
        let mut out = Vec::new();
        loop {
            match self.peek().clone() {
                Tok::Str(s) => {
                    self.next();
                    out.push(s.to_uppercase());
                }
                Tok::Ident(s) => {
                    self.next();
                    out.push(s.to_uppercase());
                }
                Tok::RBrace => {
                    self.next();
                    break;
                }
                other => {
                    return Err(SasError::runtime(format!(
                        "IML: unexpected token in a variable list: {other:?}"
                    )));
                }
            }
        }
        if out.is_empty() {
            return Err(SasError::runtime("IML: empty variable list in READ"));
        }
        Ok(out)
    }

    pub(super) fn parse_print_items(&mut self) -> Result<Vec<ImlPrintItem>> {
        let mut items = Vec::new();
        while self.peek() != &Tok::Semi && self.peek() != &Tok::Eof {
            match self.peek().clone() {
                Tok::Str(s) => {
                    self.next();
                    items.push(ImlPrintItem::StringLiteral(s));
                }
                Tok::Ident(name) => {
                    self.next();
                    // Option [label='...'] : parser et ignorer.
                    if self.eat(&Tok::LBracket) {
                        let mut depth = 1;
                        while depth > 0 && self.peek() != &Tok::Eof {
                            match self.next() {
                                Tok::LBracket => depth += 1,
                                Tok::RBracket => depth -= 1,
                                _ => {}
                            }
                        }
                    }
                    items.push(ImlPrintItem::Var(name));
                }
                other => {
                    return Err(SasError::runtime(format!(
                        "IML: unexpected token in PRINT: {other:?}"
                    )));
                }
            }
        }
        Ok(items)
    }
}
