use super::*;

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC LOGISTIC. Called AFTER `proc logistic` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<LogisticAst> {
    let mut input: Option<DatasetRef> = None;

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
        } else {
            // Skip unknown proc-level options (DESCENDING as proc option: ignored)
            ts.next();
        }
    }

    // Sub-statements until run;/quit;
    let mut class_vars: Vec<String> = Vec::new();
    let mut model: Option<LogisticModel> = None;
    let mut freq_var: Option<String> = None;
    let mut outputs: Vec<LogisticOutput> = Vec::new();

    common::parse_proc_body(ts, |ts, kw| {
        if kw == "class" {
            ts.next(); // consume "class"
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    class_vars.push(name);
                    ts.next();
                } else {
                    ts.next();
                }
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

            // Parse predictors until '/' or ';'
            let predictors = common::parse_effect_list(ts);

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
                            if let Some(name) = ts.peek().ident().map(str::to_string) {
                                link = match name.to_lowercase().as_str() {
                                    "cloglog" | "ccll" => Link::Cloglog,
                                    "probit" | "normit" => Link::Probit,
                                    _ => Link::Logit,
                                };
                                ts.next();
                            }
                        }
                    } else {
                        ts.next(); // skip unknown options
                    }
                }
            }
            ts.expect_semi()?;
            model = Some(LogisticModel {
                response,
                event,
                descending,
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
                    common::expect_eq(ts, "PREDICTED")?;
                    predicted = ts.peek().ident().map(str::to_string);
                    if predicted.is_some() {
                        ts.next();
                    }
                } else if ts.peek().is_kw("xbeta") {
                    common::expect_eq(ts, "XBETA")?;
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
        data_options: LogisticDataOptions { input },
        class_vars,
        model,
        freq_var,
        outputs,
    })
}
