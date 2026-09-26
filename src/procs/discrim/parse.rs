use super::*;

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC DISCRIM. Called AFTER `proc discrim` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<DiscrimAst> {
    let mut data: Option<DatasetRef> = None;
    let mut out: Option<DatasetRef> = None;
    let mut outstat: Option<DatasetRef> = None;
    let mut method: Option<String> = None;
    let mut pool = Pool::Yes;
    let mut priors = Priors::Equal;
    let mut noclassify = false;
    let mut crossvalidate = false;
    let mut short = false;

    // PROC DISCRIM statement options, until `;`
    loop {
        let tk = ts.peek();
        if tk.kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if tk.kind == TokenKind::Eof {
            break;
        }
        if tk.is_kw("data") {
            data = Some(common::parse_dataset_opt(ts, "DATA")?);
        } else if tk.is_kw("out") {
            out = Some(common::parse_out_opt(ts)?);
        } else if tk.is_kw("outstat") {
            outstat = Some(common::parse_dataset_opt(ts, "OUTSTAT")?);
        } else if tk.is_kw("method") {
            // J02-P5 — plus de repli silencieux vers LDA : seul METHOD=NORMAL
            // est rendu (SAS/STAT 9.4, The DISCRIM Procedure).
            common::consume_option_eq(ts, "METHOD")?;
            let span = ts.peek().span;
            let v = ts.peek().ident().map(|s| s.to_ascii_uppercase());
            match v.as_deref() {
                Some("NORMAL") => method = v,
                Some(other) => {
                    return Err(SasError::parse(
                        format!(
                            "METHOD={} is not supported in PROC DISCRIM; \
                             only METHOD=NORMAL is implemented.",
                            other
                        ),
                        span,
                    ));
                }
                None => {
                    return Err(SasError::parse("expected a value after METHOD=", span));
                }
            }
            ts.next();
        } else if tk.is_kw("pool") {
            // J02-P5 — POOL=NO (QDA) et POOL=TEST ne retombent plus sur la
            // covariance pooled : ERROR explicite ; POOL= inconnu : ERROR.
            common::consume_option_eq(ts, "POOL")?;
            let span = ts.peek().span;
            let v = ts.peek().ident().map(|s| s.to_ascii_lowercase());
            pool = match v.as_deref() {
                Some("yes") | None => Pool::Yes,
                Some("no") => {
                    return Err(SasError::parse(
                        "POOL=NO (quadratic discriminant analysis) is not supported \
                         in PROC DISCRIM; only POOL=YES is implemented.",
                        span,
                    ));
                }
                Some("test") => {
                    return Err(SasError::parse(
                        "POOL=TEST is not supported in PROC DISCRIM; \
                         only POOL=YES is implemented.",
                        span,
                    ));
                }
                Some(other) => {
                    return Err(SasError::parse(
                        format!(
                            "Unknown POOL= value '{}' in PROC DISCRIM; \
                             use POOL=YES.",
                            other.to_uppercase()
                        ),
                        span,
                    ));
                }
            };
            ts.next();
        } else if tk.is_kw("noclassify") {
            noclassify = true;
            ts.next();
        } else if tk.is_kw("crossvalidate") {
            crossvalidate = true;
            ts.next();
        } else if tk.is_kw("short") {
            short = true;
            ts.next();
        } else {
            // Skip unknown proc-level options.
            ts.next();
        }
    }

    // Sub-statements until run;/quit;
    let mut class_var: Option<String> = None;
    let mut var_vars: Vec<String> = Vec::new();
    let mut id_var: Option<String> = None;

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, "DISCRIM", |ts, kw| {
        Ok(match kw {
            "class" => {
                ts.next();
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    class_var = Some(name);
                    ts.next();
                }
                ts.skip_to_semi();
                true
            }
            "var" => {
                ts.next();
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    if let Some(name) = ts.peek().ident().map(str::to_string) {
                        var_vars.push(name);
                        ts.next();
                    } else {
                        ts.next();
                    }
                }
                ts.expect_semi()?;
                true
            }
            "id" => {
                ts.next();
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    id_var = Some(name);
                    ts.next();
                }
                ts.skip_to_semi();
                true
            }
            "priors" => {
                ts.next();
                let v = ts.peek().ident().map(|s| s.to_ascii_lowercase());
                priors = match v.as_deref() {
                    Some("proportional") | Some("prop") => Priors::Proportional,
                    _ => Priors::Equal,
                };
                ts.skip_to_semi();
                true
            }
            _ => false,
        })
    })?;

    Ok(DiscrimAst {
        data,
        out,
        outstat,
        method,
        pool,
        priors,
        noclassify,
        crossvalidate,
        short,
        class_var,
        var_vars,
        id_var,
    })
}

/// Guards (CLASS/VAR required) + NOTEs for parse-accepted, unimplemented
/// options. Returns the CLASS variable name.
pub(super) fn check_options<'a>(ast: &'a DiscrimAst, session: &mut Session) -> Result<&'a String> {
    let class_name = ast
        .class_var
        .as_ref()
        .ok_or_else(|| SasError::runtime("CLASS statement required in PROC DISCRIM"))?;

    if ast.var_vars.is_empty() {
        return Err(SasError::runtime(
            "VAR statement with at least one numeric variable required in PROC DISCRIM",
        ));
    }

    // Parse-accepted options that are not implemented → NOTE.
    // (METHOD≠NORMAL et POOL=NO|TEST sont des ERROR au parsing depuis J02-P5.)
    if ast.outstat.is_some() {
        session
            .log
            .note("OUTSTAT= is parse-accepted but not implemented in PROC DISCRIM.");
    }
    if ast.noclassify {
        session
            .log
            .note("NOCLASSIFY is parse-accepted but not implemented in PROC DISCRIM.");
    }
    if ast.crossvalidate {
        session
            .log
            .note("CROSSVALIDATE is parse-accepted but not implemented in PROC DISCRIM.");
    }
    if ast.short {
        session
            .log
            .note("SHORT is parse-accepted but not implemented in PROC DISCRIM.");
    }
    Ok(class_name)
}
