//! Statements d'E/S : datasets (SET/MERGE/UPDATE/MODIFY/BY) et texte (INFILE/INPUT/FILE/PUT/DATALINES).

use super::*;
use super::attrs::ident_begins_format;

/// `set spec [spec]* ;` — un ou plusieurs datasets (M3), chacun avec ses
/// options de dataset.
pub(super) fn parse_set(ts: &mut StatementStream) -> Result<DsStmt> {
    let set_tok = ts.peek().clone();
    ts.next(); // `set`
    if ts.peek().kind == TokenKind::Semi {
        // `set;` sans dataset : non supporté.
        return Err(SasError::parse(
            "Statement SET without a dataset is not yet implemented.",
            set_tok.span,
        ));
    }
    let mut specs = Vec::new();
    // Liste des datasets : un identifiant SUIVI de `=` est une option de
    // niveau statement (`end=`/`nobs=`/`point=`), pas un dataset → on arrête
    // la liste et on bascule sur le parsing des options.
    while ts.peek().ident().is_some() && ts.peek2().kind != TokenKind::Eq {
        specs.push(ts.parse_dataset_spec()?);
    }
    let options = parse_set_options(ts)?;
    ts.expect_semi()?;
    Ok(DsStmt::Set { specs, options })
}

/// Options de niveau statement du SET (M16.4) : `end=v`, `nobs=v`, `point=v`,
/// dans n'importe quel ordre, chacune au plus une fois. Toute autre clé →
/// erreur de parsing.
fn parse_set_options(ts: &mut StatementStream) -> Result<crate::ast::SetOptions> {
    let mut options = crate::ast::SetOptions::default();
    while let Some(kw) = ts.peek().ident() {
        if ts.peek2().kind != TokenKind::Eq {
            break;
        }
        let kw = kw.to_ascii_lowercase();
        let kw_tok = ts.peek().clone();
        ts.next(); // keyword
        ts.next(); // `=`
        let var_tok = ts.peek().clone();
        let var = match var_tok.ident() {
            Some(v) => v.to_string(),
            None => {
                return Err(SasError::parse(
                    "expected a variable name after SET option",
                    var_tok.span,
                ));
            }
        };
        ts.next(); // variable name
        let slot = match kw.as_str() {
            "end" => &mut options.end,
            "nobs" => &mut options.nobs,
            "point" => &mut options.point,
            other => {
                return Err(SasError::parse(
                    format!("unknown SET option {other}="),
                    kw_tok.span,
                ));
            }
        };
        if slot.is_some() {
            return Err(SasError::parse(
                format!("SET option {kw}= specified more than once"),
                kw_tok.span,
            ));
        }
        *slot = Some(var);
    }
    Ok(options)
}

/// `merge spec [spec]* ;` — un ou plusieurs datasets (M3), chacun avec ses
/// options de dataset (dont `in=`). Match-merge SAS par BY. La validité (un
/// seul SET/MERGE par étape, présence d'un BY, tri) est tranchée à la
/// compilation/exécution.
pub(super) fn parse_merge(ts: &mut StatementStream) -> Result<DsStmt> {
    let merge_tok = ts.peek().clone();
    ts.next(); // `merge`
    if ts.peek().kind == TokenKind::Semi {
        return Err(SasError::parse(
            "Statement MERGE without a dataset is not yet implemented.",
            merge_tok.span,
        ));
    }
    let mut specs = Vec::new();
    while ts.peek().ident().is_some() {
        specs.push(ts.parse_dataset_spec()?);
    }
    ts.expect_semi()?;
    Ok(DsStmt::Merge(specs))
}

/// `update master[(where=(...))] transaction key=k1 k2;` (M16.5) — fusion
/// maître/transaction. Le maître et la transaction sont deux références de
/// dataset ; seul le maître accepte des options de dataset (en pratique
/// `(where=(...))`, dont on extrait l'expression). `key=` est OBLIGATOIRE et
/// porte une liste (≥1) de noms de variables clé séparés par des espaces.
pub(super) fn parse_update(ts: &mut StatementStream) -> Result<DsStmt> {
    let upd_tok = ts.peek().clone();
    ts.next(); // `update`
    if ts.peek().kind == TokenKind::Semi {
        return Err(SasError::parse(
            "Statement UPDATE without a dataset is not yet implemented.",
            upd_tok.span,
        ));
    }
    // Le maître peut porter des options de dataset (where=). On ne retient
    // que `where=` (keep/drop/rename/in= sur UPDATE non supportés → erreur).
    let master_spec = ts.parse_dataset_spec()?;
    let opts = &master_spec.options;
    if opts.keep.is_some()
        || opts.drop.is_some()
        || !opts.rename.is_empty()
        || opts.in_.is_some()
    {
        return Err(SasError::parse(
            "Only the WHERE= data set option is supported on the UPDATE master data set.",
            upd_tok.span,
        ));
    }
    let master_where = master_spec.options.where_.clone();
    let master = master_spec.dref;
    // La transaction : une simple référence (pas d'options).
    let transaction = ts.parse_dataset_ref()?;
    // `key=` obligatoire.
    let key_vars = parse_key_option(ts)?;
    if key_vars.is_empty() {
        return Err(SasError::parse(
            "An UPDATE statement requires a KEY= option with at least one variable.",
            upd_tok.span,
        ));
    }
    ts.expect_semi()?;
    Ok(DsStmt::Update {
        master,
        master_where,
        transaction,
        key_vars,
    })
}

/// `modify dataset [key=k1 k2] [point=p] [nobs=n];` (M16.5) — modification en
/// place. Le dataset est une référence simple ; `key=` (optionnel) porte la
/// liste de clés ; `point=`/`nobs=` (optionnels) nomment des variables comme
/// pour SET. Les options apparaissent dans n'importe quel ordre.
pub(super) fn parse_modify(ts: &mut StatementStream) -> Result<DsStmt> {
    let mod_tok = ts.peek().clone();
    ts.next(); // `modify`
    if ts.peek().kind == TokenKind::Semi {
        return Err(SasError::parse(
            "Statement MODIFY without a dataset is not yet implemented.",
            mod_tok.span,
        ));
    }
    let dataset = ts.parse_dataset_ref()?;
    let mut key_vars: Vec<String> = Vec::new();
    let mut point: Option<String> = None;
    let mut nobs: Option<String> = None;
    // Options `key=`/`point=`/`nobs=` (chacune au plus une fois).
    while let Some(kw) = ts.peek().ident() {
        if ts.peek2().kind != TokenKind::Eq {
            break;
        }
        let kw = kw.to_ascii_lowercase();
        let kw_tok = ts.peek().clone();
        match kw.as_str() {
            "key" => {
                if !key_vars.is_empty() {
                    return Err(SasError::parse(
                        "MODIFY option KEY= specified more than once",
                        kw_tok.span,
                    ));
                }
                key_vars = parse_key_option(ts)?;
            }
            "point" | "nobs" => {
                ts.next(); // keyword
                ts.next(); // `=`
                let var_tok = ts.peek().clone();
                let Some(v) = var_tok.ident().map(str::to_string) else {
                    return Err(SasError::parse(
                        "expected a variable name after MODIFY option",
                        var_tok.span,
                    ));
                };
                ts.next();
                let slot = if kw == "point" { &mut point } else { &mut nobs };
                if slot.is_some() {
                    return Err(SasError::parse(
                        format!("MODIFY option {kw}= specified more than once"),
                        kw_tok.span,
                    ));
                }
                *slot = Some(v);
            }
            other => {
                return Err(SasError::parse(
                    format!("unknown MODIFY option {other}="),
                    kw_tok.span,
                ));
            }
        }
    }
    ts.expect_semi()?;
    Ok(DsStmt::Modify {
        dataset,
        key_vars,
        point,
        nobs,
    })
}

/// Parse l'option `key=v1 v2 ...` (M16.5) : consomme `key`, `=`, puis une
/// liste de noms de variables (au moins zéro ; l'appelant impose le minimum).
/// La liste s'arrête au prochain Ident SUIVI de `=` (option suivante) ou au
/// `;`. À l'entrée, `ts.peek()` doit être `key` ; sinon liste vide.
fn parse_key_option(ts: &mut StatementStream) -> Result<Vec<String>> {
    if !ts.peek().is_kw("key") {
        return Ok(Vec::new());
    }
    ts.next(); // `key`
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            "expected '=' after KEY",
            ts.peek().span,
        ));
    }
    ts.next(); // `=`
    let mut vars = Vec::new();
    while let Some(name) = ts.peek().ident() {
        // Un Ident suivi de `=` est l'option suivante (point=/nobs=), pas une
        // variable clé.
        if ts.peek2().kind == TokenKind::Eq {
            break;
        }
        let name = name.to_string();
        let span = ts.peek().span;
        validate_sas_name(&name, span)?;
        vars.push(name);
        ts.next();
    }
    Ok(vars)
}

/// `infile <source> [options] ;` (M14). La source est un littéral chemin
/// (`'fichier.txt'`) OU le mot-clé `datalines`/`cards` (source inline).
/// Options reconnues : `DELIMITER=`/`DLM=`, `DSD`, `FIRSTOBS=`, `OBS=`,
/// `MISSOVER`, `TRUNCOVER`, `STOPOVER`, `LRECL=`. Une option inconnue →
/// erreur claire.
pub(super) fn parse_infile(ts: &mut StatementStream) -> Result<DsStmt> {
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
fn expect_eq(ts: &mut StatementStream, opt: &str) -> Result<()> {
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            format!("expected '=' after the INFILE option {}", opt.to_uppercase()),
            ts.peek().span,
        ));
    }
    ts.next();
    Ok(())
}

/// Valeur d'un `DELIMITER=`/`DLM=` : une chaîne littérale (`','`, `'09'x`
/// non géré) ou un identifiant/caractère isolé. On accepte une chaîne ou un
/// token simple ; on en garde la valeur textuelle.
fn parse_infile_delimiter(ts: &mut StatementStream) -> Result<String> {
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
fn parse_infile_count(ts: &mut StatementStream, opt: &str) -> Result<usize> {
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

/// `input <items> ;` (M14). Modes pris en charge :
/// - liste : `name $ age` ;
/// - colonne : `name $ 1-10 age 11-13` ;
/// - formaté : `name $char10. d date9.` ;
/// - pointeurs `@n`, `+n`, `/`, hold `@`/`@@`, modificateur `:`.
///
/// On lit les tokens jusqu'au `;` final (consommé). Le `$` se rapporte à la
/// variable qui PRÉCÈDE (forme `name $`).
pub(super) fn parse_input(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `input`
    let mut items: Vec<InputItem> = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi => {
                ts.next();
                return Ok(DsStmt::Input(items));
            }
            // `@@` (double hold) ou `@n` (pointeur de colonne) ou `@` (hold).
            TokenKind::At => {
                let at_end = tok.span.end;
                ts.next(); // `@`
                if ts.peek().kind == TokenKind::At {
                    ts.next(); // second `@`
                    items.push(InputItem::HoldLineDouble);
                } else if let TokenKind::Num(n) = ts.peek().kind {
                    // `@n` : pointeur ADJACENT (`@5`) ou espacé (`@ 5`) —
                    // SAS tolère les deux.
                    if n.fract() != 0.0 || n < 1.0 {
                        return Err(SasError::parse(
                            "the column pointer @n must be a positive integer",
                            ts.peek().span,
                        ));
                    }
                    ts.next();
                    items.push(InputItem::ColumnPointer(n as usize));
                } else {
                    // `@` final (hold simple) — doit être suivi du `;`.
                    let _ = at_end;
                    items.push(InputItem::HoldLine);
                }
            }
            // `+n` : avance relative du curseur.
            TokenKind::Plus => {
                ts.next(); // `+`
                let n_tok = ts.peek().clone();
                let TokenKind::Num(n) = n_tok.kind else {
                    return Err(SasError::parse(
                        "expected a positive integer after '+' in the INPUT statement",
                        n_tok.span,
                    ));
                };
                if n.fract() != 0.0 || n < 0.0 {
                    return Err(SasError::parse(
                        "the column skip +n must be a non-negative integer",
                        n_tok.span,
                    ));
                }
                ts.next();
                items.push(InputItem::SkipColumns(n as usize));
            }
            // `/` : passage à la ligne d'entrée suivante.
            TokenKind::Slash => {
                ts.next();
                items.push(InputItem::NextLine);
            }
            // Un nom de variable, éventuellement suivi de `$`, de colonnes
            // `a-b`, d'un `:`-modificateur et/ou d'un informat.
            TokenKind::Ident(name) => {
                let name = name.clone();
                validate_sas_name(&name, tok.span)?;
                ts.next();
                let item = parse_input_var(ts, name)?;
                items.push(item);
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable name, column pointer or ';' in the INPUT statement",
                    tok.span,
                ));
            }
        }
    }
}

/// Suffixe d'une variable INPUT : `[$] [a-b | [:] informat]`.
fn parse_input_var(ts: &mut StatementStream, name: String) -> Result<InputItem> {
    let mut is_char = false;
    // `$` : variable caractère. Deux cas :
    // - `$char10.` / `$10.` : le `$` ouvre un INFORMAT caractère (adjacent à
    //   un Ident/Num) — on NE le consomme PAS ici, `read_format_token` le
    //   lira en entier.
    // - `$` isolé (suivi d'un espace, de colonnes, ou de la variable
    //   suivante) : simple marqueur caractère du mode liste/colonne.
    if ts.peek().kind == TokenKind::Dollar && !dollar_begins_informat(ts) {
        ts.next();
        is_char = true;
    }
    // `:` modificateur d'informat en mode liste.
    let mut list_modifier = false;
    if ts.peek().kind == TokenKind::Colon {
        ts.next();
        list_modifier = true;
    }
    // Mode colonne : `a-b` (a et b entiers, a-b adjacents au `-`).
    if let TokenKind::Num(a) = ts.peek().kind {
        // Distinguer `a-b` (colonnes) d'un informat `8.` : un informat a un
        // `.` ; les colonnes ont un `-`. On regarde le token suivant.
        if a.fract() == 0.0 && a >= 1.0 && ts.peek2().kind == TokenKind::Minus {
            ts.next(); // a
            ts.next(); // `-`
            let b_tok = ts.peek().clone();
            let TokenKind::Num(b) = b_tok.kind else {
                return Err(SasError::parse(
                    "expected the end column after '-' in the INPUT statement",
                    b_tok.span,
                ));
            };
            if b.fract() != 0.0 || b < a {
                return Err(SasError::parse(
                    "invalid column range in the INPUT statement",
                    b_tok.span,
                ));
            }
            ts.next();
            return Ok(InputItem::Var {
                name,
                is_char,
                cols: Some((a as usize, b as usize)),
                informat: None,
                list_modifier,
            });
        }
    }
    // Mode formaté : un informat suit (token de format `date9.`, `8.2`,
    // `$char10.`, etc.). On le détecte par adjacence (comme FORMAT).
    if input_informat_follows(ts) {
        let token = super::expr::read_format_token(ts)?;
        return Ok(InputItem::Var {
            name,
            is_char,
            cols: None,
            informat: Some(token),
            list_modifier,
        });
    }
    // Mode liste pur.
    Ok(InputItem::Var {
        name,
        is_char,
        cols: None,
        informat: None,
        list_modifier,
    })
}

/// Vrai si le `$` courant ouvre un informat caractère (`$char10.`, `$10.`,
/// `$.`) : le token ADJACENT est un Ident ou un Num (qui formera le reste de
/// l'informat). Un `$` isolé (suivi d'espace ou d'un nombre non adjacent =
/// colonnes) reste un simple marqueur caractère.
fn dollar_begins_informat(ts: &StatementStream) -> bool {
    let cur = ts.peek();
    let next = ts.peek2();
    next.span.start == cur.span.end
        && matches!(next.kind, TokenKind::Ident(_) | TokenKind::Num(_))
}

/// Vrai si un informat suit (mode formaté) : un `$`, un nombre porteur d'un
/// point décimal (`5.2`, lexé en `Num(5.2)`) ou suivi d'un `.` adjacent
/// (`8.`), ou un Ident adjacent à un morceau de format (`date9.`). Le cas du
/// nombre suivi d'un `-` (plage de colonnes) est déjà traité plus haut.
fn input_informat_follows(ts: &StatementStream) -> bool {
    let cur = ts.peek();
    match &cur.kind {
        // `$char10.` : le `$` ouvre un informat caractère.
        TokenKind::Dollar => true,
        TokenKind::Num(n) => {
            // `5.2` : le point décimal est DANS le token (partie fractionnaire).
            if n.fract() != 0.0 {
                return true;
            }
            // `8.` : un `.` adjacent suit le nombre entier.
            let next = ts.peek2();
            next.span.start == cur.span.end && next.kind == TokenKind::Dot
        }
        // `date9.` : un Ident dont le morceau suivant adjacent est un format.
        TokenKind::Ident(_) => ident_begins_format(ts),
        _ => false,
    }
}

/// `file <dest> ;` (M14.2). La destination est un littéral chemin
/// (`'sortie.txt'`) OU le mot-clé `log` / `print`. Toute autre forme →
/// erreur claire. (Les options FILE — `LRECL=`, `MOD`... — ne sont pas
/// supportées.)
pub(super) fn parse_file(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `file`
    let tok = ts.peek().clone();
    let dest = match &tok.kind {
        TokenKind::Str {
            value,
            suffix: StrSuffix::None | StrSuffix::Name,
        } => {
            let s = value.clone();
            ts.next();
            PutDest::Path(s)
        }
        TokenKind::Ident(name) if name.eq_ignore_ascii_case("log") => {
            ts.next();
            PutDest::Log
        }
        TokenKind::Ident(name) if name.eq_ignore_ascii_case("print") => {
            ts.next();
            PutDest::Print
        }
        _ => {
            return Err(SasError::parse(
                "expected a quoted file path, LOG or PRINT after FILE",
                tok.span,
            ));
        }
    };
    ts.expect_semi()?;
    Ok(DsStmt::File { dest })
}

/// `put <items> ;` (M14.2). Miroir de sortie d'`input`. Modes pris en
/// charge :
/// - liste : `put name age` (format d'affichage de chaque variable) ;
/// - nommé : `put name= age=` (`name=VALEUR`) ;
/// - littéral : `put 'Report for' name` ;
/// - formaté : `put x 8.2` / `put d date9.` ;
/// - pointeurs `@n`, `+n`, `/`, hold `@`/`@@`, et `put _all_;`.
///
/// On lit les tokens jusqu'au `;` final (consommé).
pub(super) fn parse_put(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `put`
    let mut items: Vec<PutItem> = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi => {
                ts.next();
                return Ok(DsStmt::Put(items));
            }
            // `@@` (double hold), `@n` (pointeur de colonne) ou `@` (hold).
            TokenKind::At => {
                ts.next(); // `@`
                if ts.peek().kind == TokenKind::At {
                    ts.next(); // second `@`
                    items.push(PutItem::HoldLineDouble);
                } else if let TokenKind::Num(n) = ts.peek().kind {
                    if n.fract() != 0.0 || n < 1.0 {
                        return Err(SasError::parse(
                            "the column pointer @n must be a positive integer",
                            ts.peek().span,
                        ));
                    }
                    ts.next();
                    items.push(PutItem::ColumnPointer(n as usize));
                } else {
                    // `@` final (hold simple) — doit être suivi du `;`.
                    items.push(PutItem::HoldLine);
                }
            }
            // `+n` : avance relative du curseur.
            TokenKind::Plus => {
                ts.next(); // `+`
                let n_tok = ts.peek().clone();
                let TokenKind::Num(n) = n_tok.kind else {
                    return Err(SasError::parse(
                        "expected a positive integer after '+' in the PUT statement",
                        n_tok.span,
                    ));
                };
                if n.fract() != 0.0 || n < 0.0 {
                    return Err(SasError::parse(
                        "the column skip +n must be a non-negative integer",
                        n_tok.span,
                    ));
                }
                ts.next();
                items.push(PutItem::SkipColumns(n as usize));
            }
            // `/` : passage à la ligne de sortie suivante.
            TokenKind::Slash => {
                ts.next();
                items.push(PutItem::NextLine);
            }
            // Un littéral chaîne : écrit verbatim.
            TokenKind::Str {
                value,
                suffix: StrSuffix::None | StrSuffix::Name,
            } => {
                let s = value.clone();
                ts.next();
                items.push(PutItem::Literal(s));
            }
            // Un nom de variable : forme nommée (`name=`), formatée
            // (`name 8.2`) ou liste (`name`). `_all_` est un cas spécial.
            TokenKind::Ident(name) => {
                let name = name.clone();
                if name.eq_ignore_ascii_case("_all_") {
                    ts.next();
                    items.push(PutItem::All);
                    continue;
                }
                validate_sas_name(&name, tok.span)?;
                ts.next();
                items.push(parse_put_var(ts, name)?);
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable, a literal, a column pointer or ';' in the PUT statement",
                    tok.span,
                ));
            }
        }
    }
}

/// Suffixe d'une variable PUT : `[= | format]`.
/// - `name=` : forme nommée (`name=VALEUR`). On distingue du début d'une
///   assignation : dans un PUT, `name=` n'est jamais suivi d'une expression
///   significative — l'item suivant est un autre item PUT ou le `;`.
/// - `name fmt.` : forme formatée (format adjacent comme dans FORMAT/INPUT).
/// - sinon : forme liste (format d'affichage par défaut de la variable).
fn parse_put_var(ts: &mut StatementStream, name: String) -> Result<PutItem> {
    // Forme nommée `name=`.
    if ts.peek().kind == TokenKind::Eq {
        ts.next(); // `=`
        return Ok(PutItem::NamedVar(name));
    }
    // Forme formatée : un format suit (token adjacent `$`, `8.2`, `date9.`).
    if put_format_follows(ts) {
        let token = super::expr::read_format_token(ts)?;
        return Ok(PutItem::Var {
            name,
            format: Some(token),
        });
    }
    // Forme liste pure.
    Ok(PutItem::Var { name, format: None })
}

/// Vrai si un format suit (forme formatée d'un item PUT). Identique à
/// `input_informat_follows` : un `$`, un nombre fractionnaire (`5.2`) ou
/// entier suivi d'un `.` adjacent (`8.`), ou un Ident adjacent à un morceau
/// de format (`date9.`).
fn put_format_follows(ts: &StatementStream) -> bool {
    let cur = ts.peek();
    match &cur.kind {
        TokenKind::Dollar => true,
        TokenKind::Num(n) => {
            if n.fract() != 0.0 {
                return true;
            }
            let next = ts.peek2();
            next.span.start == cur.span.end && next.kind == TokenKind::Dot
        }
        TokenKind::Ident(_) => ident_begins_format(ts),
        _ => false,
    }
}

/// `datalines;` / `cards;` (M14). Le mot-clé a été lu par `parse_statement` ;
/// ici on consomme le `;` puis le token `DataLines` (émis par le lexer juste
/// après ce `;`). Les variantes `4` (`datalines4`/`cards4`) sont équivalentes
/// au parsing près (le terminateur a déjà été géré par le lexer).
pub(super) fn parse_datalines(ts: &mut StatementStream) -> Result<DsStmt> {
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

/// `by [descending] v1 [descending] v2 ... ;` → `DsStmt::By` (M3). Le
/// mot-clé DESCENDING s'applique à la variable qui le SUIT. La validité
/// (présence d'un SET, variables sur les inputs) est tranchée à la
/// compilation.
pub(super) fn parse_by(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `by`
    let mut items: Vec<(String, bool)> = Vec::new();
    let mut descending = false;
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi => {
                if descending {
                    return Err(SasError::parse(
                        "expected a variable name after DESCENDING in the BY statement",
                        tok.span,
                    ));
                }
                if items.is_empty() {
                    return Err(SasError::parse(
                        "expected a variable name in the BY statement",
                        tok.span,
                    ));
                }
                ts.next();
                return Ok(DsStmt::By(items));
            }
            TokenKind::Ident(name) => {
                let name = name.clone();
                if name.eq_ignore_ascii_case("descending") {
                    if descending {
                        return Err(SasError::parse(
                            "expected a variable name after DESCENDING in the BY statement",
                            tok.span,
                        ));
                    }
                    descending = true;
                } else {
                    validate_sas_name(&name, tok.span)?;
                    items.push((name, descending));
                    descending = false;
                }
                ts.next();
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable name in the BY statement",
                    tok.span,
                ));
            }
        }
    }
}
