use super::*;

/// `set spec [spec]* ;` — un ou plusieurs datasets (M3), chacun avec ses
/// options de dataset.
pub(crate) fn parse_set(ts: &mut StatementStream) -> Result<DsStmt> {
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
pub(crate) fn parse_set_options(ts: &mut StatementStream) -> Result<crate::ast::SetOptions> {
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
pub(crate) fn parse_merge(ts: &mut StatementStream) -> Result<DsStmt> {
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
pub(crate) fn parse_update(ts: &mut StatementStream) -> Result<DsStmt> {
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
    if opts.keep.is_some() || opts.drop.is_some() || !opts.rename.is_empty() || opts.in_.is_some() {
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
pub(crate) fn parse_modify(ts: &mut StatementStream) -> Result<DsStmt> {
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
pub(crate) fn parse_key_option(ts: &mut StatementStream) -> Result<Vec<String>> {
    if !ts.peek().is_kw("key") {
        return Ok(Vec::new());
    }
    ts.next(); // `key`
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse("expected '=' after KEY", ts.peek().span));
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

/// `by [descending] v1 [descending] v2 ... ;` → `DsStmt::By` (M3). Le
/// mot-clé DESCENDING s'applique à la variable qui le SUIT. La validité
/// (présence d'un SET, variables sur les inputs) est tranchée à la
/// compilation.
pub(crate) fn parse_by(ts: &mut StatementStream) -> Result<DsStmt> {
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
