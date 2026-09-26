use super::*;

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC LOGISTIC. Called AFTER `proc logistic` has been consumed.
///
/// J02-P5 — fin des replis silencieux (SAS/STAT 9.4 User's Guide, The
/// LOGISTIC Procedure) : l'option PROC `DESCENDING` est honorée, `ORDER=`
/// et toute option PROC inconnue sont des ERROR ; dans le MODEL, un `LINK=`
/// inconnu et toute option inconnue sont des ERROR ; le statement CLASS
/// analyse `(PARAM= REF=)` — `PARAM=REF` avec `REF=FIRST|LAST` est honoré,
/// tout autre PARAM= est une ERROR (plus de jetons pris pour des variables).
pub fn parse(ts: &mut StatementStream) -> Result<LogisticAst> {
    let mut input: Option<DatasetRef> = None;
    let mut proc_descending = false;

    // PROC LOGISTIC statement options, until `;`
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            input = Some(common::parse_dataset_opt(ts, "DATA")?);
        } else if ts.peek().is_kw("descending") || ts.peek().is_kw("desc") {
            // PROC-level DESCENDING — honored (applies to every MODEL).
            proc_descending = true;
            ts.next();
        } else if ts.peek().is_kw("order") {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "ORDER= is not supported in PROC LOGISTIC; response level order \
                 would silently differ from the request.",
                span,
            ));
        } else {
            // Unknown PROC-level option — ERROR instead of a silent skip.
            let span = ts.peek().span;
            let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
            return Err(SasError::parse(
                format!("Unknown or unsupported option '{bad}' on the PROC LOGISTIC statement."),
                span,
            ));
        }
    }

    // Sub-statements until run;/quit;
    let mut class_vars: Vec<ClassVar> = Vec::new();
    let mut model: Option<LogisticModel> = None;
    let mut freq_var: Option<String> = None;
    let mut outputs: Vec<LogisticOutput> = Vec::new();

    common::parse_proc_body(ts, "LOGISTIC", |ts, kw| {
        if kw == "class" {
            ts.next(); // consume "class"
            // `var [(options)] var [(options)] … ;` — SAS/STAT 9.4, CLASS
            // statement. Only PARAM=REF with REF=FIRST|LAST is supported.
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                let name = match ts.peek().ident().map(str::to_string) {
                    Some(n) => {
                        ts.next();
                        n
                    }
                    None => {
                        ts.next();
                        continue;
                    }
                };
                let mut ref_first = false;
                if ts.peek().kind == TokenKind::LParen {
                    ts.next();
                    loop {
                        if ts.peek().kind == TokenKind::RParen {
                            ts.next();
                            break;
                        }
                        if ts.peek().kind == TokenKind::Semi || ts.peek().kind == TokenKind::Eof {
                            break;
                        }
                        if ts.peek().is_kw("param") {
                            ts.next();
                            if ts.peek().kind == TokenKind::Eq {
                                ts.next();
                            }
                            let span = ts.peek().span;
                            if let Some(v) = ts.peek().ident().map(str::to_string) {
                                ts.next();
                                if !v.eq_ignore_ascii_case("ref") {
                                    return Err(SasError::parse(
                                        format!(
                                            "CLASS PARAM={} is not supported in PROC LOGISTIC; \
                                             only PARAM=REF is implemented.",
                                            v.to_uppercase()
                                        ),
                                        span,
                                    ));
                                }
                            } else {
                                return Err(SasError::parse("expected a value after PARAM=", span));
                            }
                        } else if ts.peek().is_kw("ref") {
                            ts.next();
                            if ts.peek().kind == TokenKind::Eq {
                                ts.next();
                            }
                            let span = ts.peek().span;
                            if let Some(v) = ts.peek().ident().map(str::to_string) {
                                ts.next();
                                match v.to_ascii_lowercase().as_str() {
                                    "first" => ref_first = true,
                                    "last" => ref_first = false,
                                    other => {
                                        return Err(SasError::parse(
                                            format!(
                                                "CLASS REF={} is invalid; use REF=FIRST or REF=LAST.",
                                                other.to_uppercase()
                                            ),
                                            span,
                                        ));
                                    }
                                }
                            } else {
                                return Err(SasError::parse("expected a value after REF=", span));
                            }
                        } else {
                            let span = ts.peek().span;
                            let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
                            return Err(SasError::parse(
                                format!(
                                    "Unknown or unsupported CLASS option '{bad}' in PROC LOGISTIC."
                                ),
                                span,
                            ));
                        }
                    }
                }
                class_vars.push(ClassVar { name, ref_first });
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "model" {
            ts.next(); // consume "model"
            // Parse response variable name
            let response = common::parse_model_response(ts, "expected response variable")?;

            // Parse optional response options: (event='val' descending ...)
            let (event, descending) = common::parse_response_options(ts);

            // Expect '='
            common::expect_model_eq(ts, "expected '=' after response variable in MODEL")?;

            // Parse predictors until '/' or ';'. J02-P5 — `a*b` / `a(b)` are
            // NOT silently flattened anymore: explicit ERROR.
            let predictors = common::parse_effect_list_strict(ts, "LOGISTIC")?;

            let mut noprint = false;
            let mut link = Link::Logit;

            if ts.peek().kind == TokenKind::Slash {
                ts.next(); // consume '/'
                // Parse options until ';'
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    if ts.peek().is_kw("noprint") {
                        noprint = true;
                        ts.next();
                    } else if ts.peek().is_kw("link") {
                        ts.next(); // consume "link"
                        if ts.peek().kind == TokenKind::Eq {
                            ts.next(); // consume '='
                            let span = ts.peek().span;
                            if let Some(name) = ts.peek().ident().map(str::to_string) {
                                link = match name.to_lowercase().as_str() {
                                    "logit" => Link::Logit,
                                    "cloglog" | "ccll" => Link::Cloglog,
                                    "probit" | "normit" => Link::Probit,
                                    other => {
                                        return Err(SasError::parse(
                                            format!(
                                                "Unknown LINK= value '{}' on the MODEL statement.",
                                                other.to_uppercase()
                                            ),
                                            span,
                                        ));
                                    }
                                };
                                ts.next();
                            } else {
                                return Err(SasError::parse(
                                    "expected a link name after LINK=",
                                    span,
                                ));
                            }
                        } else {
                            return Err(SasError::parse("expected '=' after LINK", ts.peek().span));
                        }
                    } else {
                        // Unknown MODEL option — ERROR instead of a silent skip.
                        let span = ts.peek().span;
                        let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
                        return Err(SasError::parse(
                            format!(
                                "Unknown or unsupported MODEL option '{bad}' in PROC LOGISTIC."
                            ),
                            span,
                        ));
                    }
                }
            }
            ts.expect_semi()?;
            model = Some(LogisticModel {
                response,
                event,
                // PROC-level DESCENDING applies unless the MODEL response
                // options already requested a specific ordering/event.
                descending: descending || proc_descending,
                predictors,
                noprint,
                link,
            });
            Ok(true)
        } else if kw == "freq" {
            ts.next(); // consume "freq"
            if let Some(name) = ts.peek().ident().map(str::to_string) {
                freq_var = Some(name);
                ts.next();
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "output" {
            ts.next(); // consume "output"
            let mut out: Option<DatasetRef> = None;
            let mut predicted: Option<String> = None;
            let mut xbeta: Option<String> = None;
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                if ts.peek().is_kw("out") {
                    out = Some(common::parse_out_opt(ts)?);
                } else if ts.peek().is_kw("predicted")
                    || ts.peek().is_kw("pred")
                    || ts.peek().is_kw("prob")
                    || ts.peek().is_kw("p")
                {
                    common::consume_option_eq(ts, "PREDICTED")?;
                    predicted = ts.peek().ident().map(str::to_string);
                    if predicted.is_some() {
                        ts.next();
                    }
                } else if ts.peek().is_kw("xbeta") {
                    common::consume_option_eq(ts, "XBETA")?;
                    xbeta = ts.peek().ident().map(str::to_string);
                    if xbeta.is_some() {
                        ts.next();
                    }
                } else {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            if let Some(out_ref) = out {
                outputs.push(LogisticOutput {
                    out: out_ref,
                    predicted,
                    xbeta,
                });
            }
            Ok(true)
        } else {
            Ok(false)
        }
    })?;

    Ok(LogisticAst {
        data_options: LogisticDataOptions {
            input,
            descending: proc_descending,
        },
        class_vars,
        model,
        freq_var,
        outputs,
    })
}
