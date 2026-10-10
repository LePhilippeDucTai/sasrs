use super::*;

use crate::parser::ProcParseEffect;
use crate::source::SourceFile;

// ───────────────────────── Contract diagnostics (J02-P5) ─────────────────────────
//
// SAS/IML 13.2 User's Guide (SAS 9.4), Language Reference: « Control
// Statements », « Data Set and File Functions », « Traditional Graphics and
// Window functions », « Calling SAS statements or R Functions ». Every
// statement that sasrs does not implement used to be parsed as a malformed
// assignment (« expected '=' in an assignment, found Ident(…) ») or to fail
// only at run time (STORE, LOAD, FREE…). CONTRIBUTING §5: a statement that
// can change a result is an ERROR (step rejected at parse time), a
// display-only one a WARNING, an unknown one the 180-322 ERROR.

/// Valid SAS/IML statements that are not implemented, with the
/// roadmap-avancee unit that will implement them (None: not planned).
const UNSUPPORTED_STATEMENTS: &[(&str, Option<&str>)] = &[
    ("abort", None),
    ("closefile", None),
    ("delete", None),
    ("display", None),
    ("edit", None),
    ("endsubmit", None),
    ("file", None),
    ("find", None),
    ("finish", Some("J10-P2")),
    ("force", None),
    ("free", None),
    ("goto", None),
    ("index", None),
    ("infile", None),
    ("input", None),
    ("link", None),
    ("list", None),
    ("load", None),
    ("package", None),
    ("pause", None),
    ("purge", None),
    ("put", None),
    ("remove", None),
    ("replace", None),
    ("reset", None),
    ("resume", None),
    ("return", Some("J10-P2")),
    ("run", Some("J10-P2")),
    ("save", None),
    ("setin", None),
    ("setout", None),
    ("sort", None),
    ("start", Some("J10-P2")),
    ("stop", None),
    ("store", None),
    ("submit", None),
    ("summary", None),
    ("window", None),
];

/// SAS/IML statements whose only effect is on the printed output: MATTRIB
/// « associates printing attributes with matrices », SHOW « prints system
/// information ».
const DISPLAY_STATEMENTS: &[&str] = &["mattrib", "show"];

/// ERROR for a valid but unimplemented statement: the shared catalogue
/// message, plus the unit that will implement it when one is planned.
fn unsupported_statement(kw: &str, unit: Option<&str>) -> SasError {
    match unit {
        None => common::unsupported_statement("IML", kw),
        Some(_) => unsupported(&format!("The {} statement", kw.to_ascii_uppercase()), unit),
    }
}

impl Parser {
    pub(super) fn parse_program(&mut self) -> Result<ImlProgram> {
        let mut stmts = Vec::new();
        while self.peek() != &Tok::Eof {
            // Tolérer les `;` vides.
            if self.eat(&Tok::Semi) {
                continue;
            }
            stmts.extend(self.parse_stmt()?);
        }
        Ok(ImlProgram { stmts })
    }

    /// Parse un statement (sans le `;` final consommé, sauf flux qui gèrent
    /// `end`). `None` : instruction d'affichage ignorée (WARNING en attente).
    pub(super) fn parse_stmt(&mut self) -> Result<Option<ImlStmt>> {
        let tok = self.peek().clone();
        let Tok::Ident(kw) = tok else {
            return Err(SasError::runtime(
                "180-322: Statement is not valid or it is used out of proper order in PROC IML.",
            ));
        };
        let lower = kw.to_ascii_lowercase();
        let stmt = match lower.as_str() {
            "print" => {
                self.next();
                let items = self.parse_print_items()?;
                self.expect(&Tok::Semi, "';' after PRINT")?;
                ImlStmt::Print { items }
            }
            "if" => self.parse_if()?,
            "do" => self.parse_do()?,
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
                ImlStmt::Call { func: name, args }
            }
            "create" => self.parse_create()?,
            "append" => self.parse_append()?,
            "close" => self.parse_close()?,
            "use" => self.parse_use()?,
            "read" => self.parse_read()?,
            _ => match self.toks.get(self.pos + 1) {
                // Assignation : ident = expr ;
                Some(Tok::Eq) => {
                    self.next();
                    self.next(); // =
                    let expr = self.parse_expr()?;
                    self.expect(&Tok::Semi, "';' after an assignment")?;
                    ImlStmt::Assign { var: kw, expr }
                }
                Some(Tok::LBracket) => {
                    return Err(unsupported(
                        &format!(
                            "Assignment to a subscripted matrix ({}[...] = ...)",
                            kw.to_ascii_uppercase()
                        ),
                        Some("J10-P3"),
                    ));
                }
                _ => return self.parse_unimplemented_statement(&lower),
            },
        };
        Ok(Some(stmt))
    }

    /// Statement keyword that sasrs does not execute (cursor on the keyword).
    fn parse_unimplemented_statement(&mut self, kw: &str) -> Result<Option<ImlStmt>> {
        if let Some((_, unit)) = UNSUPPORTED_STATEMENTS.iter().find(|(k, _)| *k == kw) {
            return Err(unsupported_statement(kw, *unit));
        }
        if DISPLAY_STATEMENTS.contains(&kw) {
            self.warnings
                .push(common::ignored_display_statement("IML", kw));
            self.skip_statement();
            return Ok(None);
        }
        // Global statements cannot be routed to the global executor from the
        // raw IML body: TITLE/FOOTNOTE only change the display (WARNING);
        // OPTIONS, LIBNAME, FILENAME and ODS can change results (ERROR).
        if crate::parser::global::is_global_statement(kw) {
            if kw.starts_with("title") || kw.starts_with("footnote") {
                self.warnings
                    .push(common::ignored_display_statement("IML", kw));
                self.skip_statement();
                return Ok(None);
            }
            return Err(unsupported_statement(kw, None));
        }
        self.shared_statement_fallback()?;
        Ok(None)
    }

    /// Shared contract fallback of every PROC (`common::unhandled_proc_statement`),
    /// applied to the IML statement at the cursor: BY, WHERE, WEIGHT… → « not
    /// supported » ERROR, FORMAT/LABEL/ATTRIB → display WARNING (ATTRIB
    /// LENGTH=/INFORMAT= → ERROR), anything else → 180-322 ERROR. Only the
    /// identifiers and `=` of the statement are passed to the SAS token
    /// stream: they are all that the fallback inspects.
    fn shared_statement_fallback(&mut self) -> Result<()> {
        let mut text = String::new();
        let mut i = self.pos;
        while !matches!(self.toks[i], Tok::Semi | Tok::Eof) {
            match &self.toks[i] {
                Tok::Ident(s) => text.push_str(s),
                Tok::Eq => text.push('='),
                _ => {}
            }
            text.push(' ');
            i += 1;
        }
        text.push(';');
        let src = SourceFile::new(text);
        let mut ts = StatementStream::new(&src)?;
        common::unhandled_proc_statement(&mut ts, "IML")?;
        for effect in ts.take_proc_effects() {
            if let ProcParseEffect::Warning(message) = effect {
                self.warnings.push(message);
            }
        }
        self.skip_statement();
        Ok(())
    }

    /// Consume the current statement up to its `;` (included).
    fn skip_statement(&mut self) {
        while !matches!(self.peek(), Tok::Semi | Tok::Eof) {
            self.next();
        }
        self.eat(&Tok::Semi);
    }

    /// Parse un nom de dataset possiblement qualifié : `name` ou `lib.name`.
    /// Retourne la forme canonique en MAJUSCULES `LIB.NAME`, un nom à un
    /// niveau étant dans WORK (J02-P5 : `CREATE x` puis `CLOSE work.x`
    /// désignaient deux tables distinctes et la table n'était jamais écrite).
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
            Ok(format!("WORK.{}", first.to_uppercase()))
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

    /// `READ ALL VAR {vars} INTO mat;`. READ NEXT and the WHERE clause used to
    /// fail only at run time ; with the other unimplemented forms of the
    /// READ statement (SAS/IML 9.4) they are now rejected at parse time.
    pub(super) fn parse_read(&mut self) -> Result<ImlStmt> {
        self.next(); // read
        let mode = self.expect_ident("ALL or NEXT after READ")?;
        match mode.to_ascii_lowercase().as_str() {
            "all" => {}
            "next" => return Err(unsupported("READ NEXT", Some("J10-P3"))),
            "current" | "point" | "after" => {
                return Err(unsupported(
                    &format!("The READ {} range", mode.to_ascii_uppercase()),
                    None,
                ));
            }
            _ => return Err(SasError::runtime("IML: expected ALL or NEXT after READ")),
        }
        let var_kw = self.expect_ident("VAR after READ ALL")?;
        if !var_kw.eq_ignore_ascii_case("var") {
            return Err(SasError::runtime("IML: expected VAR after READ ALL"));
        }
        // VAR _NUM_ / _CHAR_ / _ALL_ ou une matrice de noms : formes SAS
        // valides, non implémentées.
        if let Tok::Ident(name) = self.peek().clone() {
            let unit = name.eq_ignore_ascii_case("_num_").then_some("J10-P3");
            return Err(unsupported(
                &format!("READ ALL VAR {}", name.to_ascii_uppercase()),
                unit,
            ));
        }
        // Liste de variables : `{ "x" "y" }` ou `{ x y }`.
        let vars = self.parse_var_list()?;
        // INTO mat ou WHERE ... .
        let kw = self.expect_ident("INTO or WHERE after the variable list")?;
        if kw.eq_ignore_ascii_case("where") {
            return Err(unsupported(
                "The WHERE clause of the READ statement",
                Some("J10-P3"),
            ));
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
                        "IML: unexpected {} in a variable list.",
                        other.describe()
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
                    if self.peek() == &Tok::LBracket {
                        self.parse_print_options()?;
                    }
                    items.push(ImlPrintItem::Var(name));
                }
                other => {
                    return Err(SasError::runtime(format!(
                        "IML: unexpected {} in the PRINT statement.",
                        other.describe()
                    )));
                }
            }
        }
        Ok(items)
    }

    /// `[COLNAME= ROWNAME= FORMAT= LABEL=]` after a matrix name in PRINT
    /// (SAS/IML 9.4, PRINT statement; abbreviations C=, R=, F=, L=). They only
    /// change the printed output and used to be dropped in silence: one
    /// display WARNING per option (CONTRIBUTING §5). Anything else between
    /// the brackets is an ERROR.
    fn parse_print_options(&mut self) -> Result<()> {
        self.expect(&Tok::LBracket, "'['")?;
        loop {
            let name = match (self.peek().clone(), self.toks.get(self.pos + 1)) {
                (Tok::Ident(name), Some(Tok::Eq)) => name,
                (other, _) => {
                    return Err(SasError::runtime(format!(
                        "IML: expected a PRINT option (COLNAME=, ROWNAME=, FORMAT= or LABEL=), \
                         found {}.",
                        other.describe()
                    )));
                }
            };
            let option = match name.to_ascii_lowercase().as_str() {
                "colname" | "c" => "COLNAME",
                "rowname" | "r" => "ROWNAME",
                "format" | "f" => "FORMAT",
                "label" | "l" => "LABEL",
                _ => {
                    return Err(SasError::runtime(format!(
                        "IML: unknown PRINT option '{}='; the PRINT options are COLNAME=, \
                         ROWNAME=, FORMAT= and LABEL=.",
                        name.to_ascii_uppercase()
                    )));
                }
            };
            self.next(); // name
            self.next(); // =
            // Value: up to the next `name =` or the closing `]` at depth 0.
            let mut depth = 0usize;
            let mut len = 0usize;
            loop {
                match self.peek() {
                    Tok::Semi | Tok::Eof => {
                        return Err(SasError::runtime(
                            "IML: missing ']' after the PRINT options.",
                        ));
                    }
                    Tok::RBracket if depth == 0 => break,
                    Tok::Ident(_)
                        if depth == 0
                            && len > 0
                            && self.toks.get(self.pos + 1) == Some(&Tok::Eq) =>
                    {
                        break;
                    }
                    Tok::LBrace | Tok::LParen | Tok::LBracket => depth += 1,
                    Tok::RBrace | Tok::RParen | Tok::RBracket => depth = depth.saturating_sub(1),
                    _ => {}
                }
                self.next();
                len += 1;
            }
            if len == 0 {
                return Err(SasError::runtime(format!(
                    "IML: the PRINT option {option}= requires a value."
                )));
            }
            self.warnings.push(format!(
                "The {option}= option of the PRINT statement is ignored in PROC IML; display \
                 customization is not supported."
            ));
            if self.eat(&Tok::RBracket) {
                return Ok(());
            }
        }
    }
}
