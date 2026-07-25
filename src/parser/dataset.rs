use super::*;

impl<'a> StatementStream<'a> {
    /// `ident [ . ident ]` → DatasetRef. Valide les noms SAS (≤32 chars).
    pub fn parse_dataset_ref(&mut self) -> Result<DatasetRef> {
        let tok = self.peek().clone();
        let Some(first) = tok.ident().map(str::to_string) else {
            return Err(SasError::parse("expected a dataset name", tok.span));
        };
        self.next();
        validate_sas_name(&first, tok.span)?;
        if self.peek().kind != TokenKind::Dot {
            return Ok(DatasetRef {
                libref: None,
                name: first,
            });
        }
        self.next(); // '.'
        let member_tok = self.peek().clone();
        let Some(member) = member_tok.ident().map(str::to_string) else {
            return Err(SasError::parse(
                "expected a member name after '.'",
                member_tok.span,
            ));
        };
        self.next();
        validate_sas_name(&member, member_tok.span)?;
        let first_upper = first.to_uppercase();
        // Allow special system librefs (DICTIONARY, SASHELP) that exceed 8 characters.
        if first.len() > 8 && !matches!(first_upper.as_str(), "DICTIONARY" | "SASHELP") {
            return Err(SasError::parse(
                format!("The libref {} exceeds 8 characters.", first_upper),
                tok.span,
            ));
        }
        Ok(DatasetRef {
            libref: Some(first),
            name: member,
        })
    }

    /// `ref [( options )]` → `DatasetSpec`. Options de dataset (M2,
    /// insensibles à la casse, séparées par des espaces) :
    /// - `keep = liste` / `drop = liste` : liste de noms ou de plages
    ///   numérotées `x1-x3` (même expansion que pour ARRAY). Deux formes :
    ///   parenthésée `keep=(x y)` (lue jusqu'au `)` interne) ou nue
    ///   `keep=x y` — la liste nue s'arrête au prochain Ident SUIVI de `=`
    ///   (c'est l'option suivante) ou au `)` fermant (simplification
    ///   documentée : SAS lui-même n'accepte pas les parenthèses internes,
    ///   on accepte les deux).
    /// - `rename = (old=new [old2=new2...])` : TOUJOURS parenthésé.
    /// - `where = (expr)` : parenthésé, une expression.
    /// - option inconnue → erreur "Dataset option X is not supported.".
    pub fn parse_dataset_spec(&mut self) -> Result<DatasetSpec> {
        let dref = self.parse_dataset_ref()?;
        let mut options = DatasetOptions::default();
        if self.peek().kind != TokenKind::LParen {
            return Ok(DatasetSpec { dref, options });
        }
        self.next(); // `(`
        loop {
            let tok = self.peek().clone();
            // Le nom d'option : un Ident, ou le mot-clé `in` (lexé en
            // `TokenKind::In`, l'opérateur — réutilisé ici comme option IN=).
            let opt_name: Option<String> = match &tok.kind {
                TokenKind::RParen => {
                    self.next();
                    break;
                }
                TokenKind::Ident(name) => Some(name.to_ascii_lowercase()),
                TokenKind::In => Some("in".to_string()),
                _ => None,
            };
            let Some(lower) = opt_name else {
                return Err(SasError::parse(
                    "expected a dataset option or ')'",
                    tok.span,
                ));
            };
            if !matches!(lower.as_str(), "keep" | "drop" | "rename" | "where" | "in") {
                return Err(SasError::parse(
                    format!("Dataset option {} is not supported.", lower.to_uppercase()),
                    tok.span,
                ));
            }
            self.next(); // le nom d'option
            if self.peek().kind != TokenKind::Eq {
                return Err(SasError::parse(
                    format!(
                        "expected '=' after the dataset option {}",
                        lower.to_uppercase()
                    ),
                    self.peek().span,
                ));
            }
            self.next(); // `=`
            match lower.as_str() {
                "keep" => {
                    let list = self.parse_option_name_list("KEEP")?;
                    options.keep.get_or_insert_with(Vec::new).extend(list);
                }
                "drop" => {
                    let list = self.parse_option_name_list("DROP")?;
                    options.drop.get_or_insert_with(Vec::new).extend(list);
                }
                "rename" => options.rename.extend(self.parse_rename_pairs()?),
                "where" => options.where_ = Some(self.parse_where_option()?),
                // `in=nom` (M3) : nom de variable automatique 0/1. Sa
                // validité (MERGE input seulement) est tranchée à la
                // compilation.
                "in" => {
                    let nm_tok = self.peek().clone();
                    let Some(nm) = nm_tok.ident().map(str::to_string) else {
                        return Err(SasError::parse(
                            "expected a variable name after the IN= dataset option",
                            nm_tok.span,
                        ));
                    };
                    validate_sas_name(&nm, nm_tok.span)?;
                    self.next();
                    options.in_ = Some(nm);
                }
                _ => unreachable!("filtered above"),
            }
        }
        Ok(DatasetSpec { dref, options })
    }

    /// Valeur d'un `keep=` / `drop=` : forme parenthésée `( noms )` ou nue
    /// `noms` (arrêt sur Ident-suivi-de-`=` ou `)`). Plages `x1-x3`
    /// expansées. Au moins un nom.
    pub(super) fn parse_option_name_list(&mut self, opt: &str) -> Result<Vec<String>> {
        let mut names = Vec::new();
        let parenthesized = self.peek().kind == TokenKind::LParen;
        if parenthesized {
            self.next(); // `(`
        }
        loop {
            let tok = self.peek().clone();
            match &tok.kind {
                TokenKind::RParen if parenthesized => {
                    self.next();
                    break;
                }
                // Forme nue : le `)` fermant des options termine la liste
                // SANS être consommé.
                TokenKind::RParen => break,
                TokenKind::Ident(_) => {
                    // Forme nue : un Ident suivi de `=` est l'option
                    // SUIVANTE, pas un nom de la liste.
                    if !parenthesized && self.peek2().kind == TokenKind::Eq {
                        break;
                    }
                    self.parse_option_list_item(&mut names)?;
                }
                _ => {
                    return Err(SasError::parse(
                        format!("expected a variable name in the {opt}= dataset option"),
                        tok.span,
                    ));
                }
            }
        }
        if names.is_empty() {
            return Err(SasError::parse(
                format!("expected a variable name in the {opt}= dataset option"),
                self.peek().span,
            ));
        }
        Ok(names)
    }

    /// Un élément de liste keep=/drop= : nom simple ou plage `x1-x3`
    /// (expansée via la même routine que pour ARRAY).
    pub(super) fn parse_option_list_item(&mut self, out: &mut Vec<String>) -> Result<()> {
        let tok = self.peek().clone();
        let name = tok.ident().expect("caller matched an Ident").to_string();
        validate_sas_name(&name, tok.span)?;
        self.next();
        if self.peek().kind == TokenKind::Minus {
            self.next(); // `-`
            let end_tok = self.peek().clone();
            let Some(end_name) = end_tok.ident().map(str::to_string) else {
                return Err(SasError::parse(
                    "expected a variable name after '-' in the dataset option",
                    end_tok.span,
                ));
            };
            validate_sas_name(&end_name, end_tok.span)?;
            self.next();
            datastep::expand_numbered_range(&name, &end_name, tok.span.merge(end_tok.span), out)?;
        } else {
            out.push(name);
        }
        Ok(())
    }

    /// `rename = (old=new [old2=new2...])` — TOUJOURS parenthésé, au moins
    /// une paire.
    pub(super) fn parse_rename_pairs(&mut self) -> Result<Vec<(String, String)>> {
        if self.peek().kind != TokenKind::LParen {
            return Err(SasError::parse(
                "The RENAME= dataset option requires a parenthesized list of old=new pairs.",
                self.peek().span,
            ));
        }
        self.next(); // `(`
        let mut pairs = Vec::new();
        loop {
            let tok = self.peek().clone();
            match &tok.kind {
                TokenKind::RParen => {
                    self.next();
                    break;
                }
                TokenKind::Ident(old) => {
                    let old = old.clone();
                    validate_sas_name(&old, tok.span)?;
                    self.next();
                    if self.peek().kind != TokenKind::Eq {
                        return Err(SasError::parse(
                            "expected '=' between the old and new names in RENAME=",
                            self.peek().span,
                        ));
                    }
                    self.next(); // `=`
                    let new_tok = self.peek().clone();
                    let Some(new) = new_tok.ident().map(str::to_string) else {
                        return Err(SasError::parse(
                            "expected a new variable name after '=' in RENAME=",
                            new_tok.span,
                        ));
                    };
                    validate_sas_name(&new, new_tok.span)?;
                    self.next();
                    pairs.push((old, new));
                }
                _ => {
                    return Err(SasError::parse(
                        "expected an old=new pair or ')' in RENAME=",
                        tok.span,
                    ));
                }
            }
        }
        if pairs.is_empty() {
            return Err(SasError::parse(
                "expected at least one old=new pair in RENAME=",
                self.peek().span,
            ));
        }
        Ok(pairs)
    }

    /// `where = (expr)` — parenthésé, une expression.
    pub(super) fn parse_where_option(&mut self) -> Result<crate::ast::Expr> {
        if self.peek().kind != TokenKind::LParen {
            return Err(SasError::parse(
                "The WHERE= dataset option requires a parenthesized expression.",
                self.peek().span,
            ));
        }
        self.next(); // `(`
        let cond = expr::parse_expr(self)?;
        if self.peek().kind != TokenKind::RParen {
            return Err(SasError::parse(
                "expected ')' after the WHERE= expression",
                self.peek().span,
            ));
        }
        self.next(); // `)`
        Ok(cond)
    }

    /// Liste de noms de variables jusqu'au `;` (non consommé). Au moins un.
    pub fn parse_name_list(&mut self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        while let Some(name) = self.peek().ident().map(str::to_string) {
            validate_sas_name(&name, self.peek().span)?;
            self.next();
            names.push(name);
        }
        if names.is_empty() {
            return Err(SasError::parse(
                "expected a variable name",
                self.peek().span,
            ));
        }
        Ok(names)
    }
}
