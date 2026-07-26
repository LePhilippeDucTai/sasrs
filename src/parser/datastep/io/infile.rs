use super::*;

/// `infile <source> [options] ;` (M14). La source est un littéral chemin
/// (`'fichier.txt'`) OU le mot-clé `datalines`/`cards` (source inline).
/// Options reconnues : `DELIMITER=`/`DLM=`, `DSD`, `FIRSTOBS=`, `OBS=`,
/// `MISSOVER`, `TRUNCOVER`, `STOPOVER`, `LRECL=`. Une option inconnue →
/// erreur claire.
pub(crate) fn parse_infile(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `infile`
    let src_tok = ts.peek().clone();
    let source = match &src_tok.kind {
        TokenKind::Str {
            value,
            suffix: StrSuffix::None | StrSuffix::Name,
        } => {
            let s = value.clone();
            ts.next();
            InfileSource::Path(s)
        }
        TokenKind::Ident(name)
            if name.eq_ignore_ascii_case("datalines") || name.eq_ignore_ascii_case("cards") =>
        {
            ts.next();
            InfileSource::Datalines
        }
        _ => {
            return Err(SasError::parse(
                "expected a quoted file path or DATALINES/CARDS after INFILE",
                src_tok.span,
            ));
        }
    };
    let mut options = InfileOptions::default();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi => {
                ts.next();
                return Ok(DsStmt::Infile { source, options });
            }
            TokenKind::Ident(name) => {
                let lower = name.to_ascii_lowercase();
                match lower.as_str() {
                    "dsd" => {
                        ts.next();
                        options.dsd = true;
                    }
                    "missover" => {
                        ts.next();
                        options.missover = true;
                    }
                    "truncover" => {
                        ts.next();
                        options.truncover = true;
                    }
                    "stopover" => {
                        ts.next();
                        options.stopover = true;
                    }
                    "delimiter" | "dlm" => {
                        ts.next();
                        expect_eq(ts, &lower)?;
                        options.delimiter = Some(parse_infile_delimiter(ts)?);
                    }
                    "firstobs" => {
                        ts.next();
                        expect_eq(ts, &lower)?;
                        options.firstobs = Some(parse_infile_count(ts, "FIRSTOBS")?);
                    }
                    "obs" => {
                        ts.next();
                        expect_eq(ts, &lower)?;
                        options.obs = Some(parse_infile_count(ts, "OBS")?);
                    }
                    "lrecl" => {
                        ts.next();
                        expect_eq(ts, &lower)?;
                        // LRECL est conservé mais reste un no-op fonctionnel.
                        options.lrecl = Some(parse_infile_count(ts, "LRECL")?);
                    }
                    _ => {
                        return Err(SasError::parse(
                            format!("INFILE option {} is not supported.", lower.to_uppercase()),
                            tok.span,
                        ));
                    }
                }
            }
            _ => {
                return Err(SasError::parse(
                    "expected an INFILE option or ';'",
                    tok.span,
                ));
            }
        }
    }
}

/// Consomme le `=` d'une option `nom=valeur`.
pub(crate) fn expect_eq(ts: &mut StatementStream, opt: &str) -> Result<()> {
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            format!(
                "expected '=' after the INFILE option {}",
                opt.to_uppercase()
            ),
            ts.peek().span,
        ));
    }
    ts.next();
    Ok(())
}

/// Valeur d'un `DELIMITER=`/`DLM=` : une chaîne littérale (`','`, `'09'x`
/// non géré) ou un identifiant/caractère isolé. On accepte une chaîne ou un
/// token simple ; on en garde la valeur textuelle.
pub(crate) fn parse_infile_delimiter(ts: &mut StatementStream) -> Result<String> {
    let tok = ts.peek().clone();
    match &tok.kind {
        TokenKind::Str {
            value,
            suffix: StrSuffix::None | StrSuffix::Name,
        } => {
            let s = value.clone();
            ts.next();
            Ok(s)
        }
        // Un identifiant nu (`dlm=x`) ou un caractère seul.
        TokenKind::Ident(s) => {
            let s = s.clone();
            ts.next();
            Ok(s)
        }
        _ => Err(SasError::parse(
            "expected a delimiter (quoted string or character) after DELIMITER=/DLM=",
            tok.span,
        )),
    }
}

/// Entier positif d'une option INFILE (`FIRSTOBS=`, `OBS=`, `LRECL=`).
pub(crate) fn parse_infile_count(ts: &mut StatementStream, opt: &str) -> Result<usize> {
    let tok = ts.peek().clone();
    let TokenKind::Num(n) = tok.kind else {
        return Err(SasError::parse(
            format!("expected a positive integer after {opt}="),
            tok.span,
        ));
    };
    if n.fract() != 0.0 || n < 1.0 {
        return Err(SasError::parse(
            format!("the value of {opt}= must be a positive integer"),
            tok.span,
        ));
    }
    ts.next();
    Ok(n as usize)
}

/// `datalines;` / `cards;` (M14). Le mot-clé a été lu par `parse_statement` ;
/// ici on consomme le `;` puis le token `DataLines` (émis par le lexer juste
/// après ce `;`). Les variantes `4` (`datalines4`/`cards4`) sont équivalentes
/// au parsing près (le terminateur a déjà été géré par le lexer).
pub(crate) fn parse_datalines(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `datalines` / `cards` / `datalines4` / `cards4`
    ts.expect_semi()?;
    // Le token suivant DOIT être le bloc verbatim capturé par le lexer.
    let tok = ts.peek().clone();
    if let TokenKind::DataLines(lines) = &tok.kind {
        let lines = lines.clone();
        ts.next();
        Ok(DsStmt::Datalines(lines))
    } else {
        // Aucun bloc (cas dégénéré) : datalines vide.
        Ok(DsStmt::Datalines(Vec::new()))
    }
}
