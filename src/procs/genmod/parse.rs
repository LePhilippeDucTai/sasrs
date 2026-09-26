use super::*;

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC GENMOD. Called AFTER `proc genmod` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<GenmodAst> {
    let mut input: Option<DatasetRef> = None;

    // PROC GENMOD statement options until `;`
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
            ts.next();
        }
    }

    let mut class_vars: Vec<String> = Vec::new();
    let mut model: Option<GenmodModel> = None;
    let mut freq_var: Option<String> = None;

    common::parse_proc_body(ts, "GENMOD", |ts, kw| {
        if kw == "class" {
            ts.next();
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

            // Response variable
            let response = common::parse_model_response(ts, "expected response variable")?;

            // Optional response options: (event='val' descending ...)
            let (event, descending) = common::parse_response_options(ts);

            // Expect '='
            common::expect_model_eq(ts, "expected '=' after response variable in MODEL")?;

            // Predictors until '/' or ';'. J02-P5 — `a*b` / `a(b)` are NOT
            // silently flattened anymore (SAS/STAT 9.4, GENMOD « MODEL
            // Statement » effect syntax): they are an explicit ERROR.
            let predictors = common::parse_effect_list_strict(ts, "GENMOD")?;

            let mut dist_opt: Option<Distribution> = None;
            let mut link_opt: Option<LinkFunction> = None;
            let mut noprint = false;
            let mut scale_opt: Option<f64> = None;
            let mut noscale = false;

            if ts.peek().kind == TokenKind::Slash {
                ts.next(); // consume '/'
                // Parse options. J02-P5 — fin des replis silencieux
                // (SAS/STAT 9.4 User's Guide, The GENMOD Procedure) :
                // DIST=/LINK= inconnus, LINK=POWER(λ) avec λ≠-1 et toute
                // option MODEL inconnue (p.ex. OFFSET=) sont des ERROR.
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    if ts.peek().is_kw("dist") {
                        ts.next();
                        if ts.peek().kind == TokenKind::Eq {
                            ts.next();
                        }
                        let span = ts.peek().span;
                        if let Some(name) = ts.peek().ident().map(str::to_string) {
                            ts.next();
                            dist_opt = Some(match name.to_ascii_lowercase().as_str() {
                                "poisson" => Distribution::Poisson,
                                "binomial" | "bin" => Distribution::Binomial,
                                "normal" => Distribution::Normal,
                                "gamma" => Distribution::Gamma,
                                other => {
                                    return Err(SasError::parse(
                                        format!(
                                            "Unknown DIST= value '{}' on the MODEL statement.",
                                            other.to_uppercase()
                                        ),
                                        span,
                                    ));
                                }
                            });
                        } else {
                            return Err(SasError::parse(
                                "expected a distribution name after DIST=",
                                span,
                            ));
                        }
                    } else if ts.peek().is_kw("link") {
                        ts.next();
                        if ts.peek().kind == TokenKind::Eq {
                            ts.next();
                        }
                        let span = ts.peek().span;
                        if let Some(name) = ts.peek().ident().map(str::to_string) {
                            ts.next();
                            link_opt = Some(match name.to_ascii_lowercase().as_str() {
                                "log" => LinkFunction::Log,
                                "logit" => LinkFunction::Logit,
                                "identity" => LinkFunction::Identity,
                                "reciprocal" | "inverse" => LinkFunction::Reciprocal,
                                // LINK=POWER(λ) — seule λ = -1 (l'inverse) est
                                // rendue ; toute autre puissance serait un
                                // modèle différent ajusté en silence.
                                "power" => {
                                    if ts.peek().kind == TokenKind::LParen {
                                        ts.next();
                                        let neg = ts.peek().kind == TokenKind::Minus;
                                        if neg {
                                            ts.next();
                                        }
                                        if let TokenKind::Num(v) = ts.peek().kind {
                                            ts.next();
                                            let lambda = if neg { -v } else { v };
                                            if ts.peek().kind == TokenKind::RParen {
                                                ts.next();
                                            }
                                            if (lambda - (-1.0)).abs() < 1e-12 {
                                                LinkFunction::Reciprocal
                                            } else {
                                                return Err(SasError::parse(
                                                    format!(
                                                        "LINK=POWER({lambda}) is not supported; only \
                                                         POWER(-1) (the reciprocal) is implemented."
                                                    ),
                                                    span,
                                                ));
                                            }
                                        } else {
                                            return Err(SasError::parse(
                                                "expected a number in LINK=POWER(...)",
                                                ts.peek().span,
                                            ));
                                        }
                                    } else {
                                        return Err(SasError::parse(
                                            "LINK=POWER requires a parenthesized exponent, \
                                             e.g. LINK=POWER(-1).",
                                            span,
                                        ));
                                    }
                                }
                                other => {
                                    return Err(SasError::parse(
                                        format!(
                                            "Unknown LINK= value '{}' on the MODEL statement.",
                                            other.to_uppercase()
                                        ),
                                        span,
                                    ));
                                }
                            });
                        } else {
                            return Err(SasError::parse("expected a link name after LINK=", span));
                        }
                    } else if ts.peek().is_kw("noprint") {
                        noprint = true;
                        ts.next();
                    } else if ts.peek().is_kw("noscale") {
                        noscale = true;
                        ts.next();
                    } else if ts.peek().is_kw("scale") {
                        ts.next();
                        if ts.peek().kind == TokenKind::Eq {
                            ts.next();
                        }
                        // SCALE=<number>; accept numeric literal.
                        if let TokenKind::Num(v) = ts.peek().kind {
                            scale_opt = Some(v);
                            ts.next();
                        } else if let Some(s) = ts.peek().ident().map(str::to_string) {
                            if let Ok(v) = s.parse::<f64>() {
                                scale_opt = Some(v);
                            }
                            ts.next();
                        } else {
                            ts.next();
                        }
                    } else {
                        // J02-P5 — option MODEL inconnue (p.ex. OFFSET=) :
                        // ERROR au lieu de l'ignorer en silence.
                        let span = ts.peek().span;
                        let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
                        return Err(SasError::parse(
                            format!("Unknown or unsupported MODEL option '{bad}' in PROC GENMOD."),
                            span,
                        ));
                    }
                }
            }
            ts.expect_semi()?;

            // Determine distribution (default Poisson if only link given)
            let dist = dist_opt.unwrap_or(Distribution::Poisson);
            // If LINK not given, use canonical link for the distribution
            let link = link_opt.unwrap_or_else(|| canonical_link(&dist));

            model = Some(GenmodModel {
                response,
                event,
                descending,
                predictors,
                dist,
                link,
                noprint,
                scale: scale_opt,
                noscale,
            });
            Ok(true)
        } else if kw == "freq" {
            ts.next();
            if let Some(name) = ts.peek().ident().map(str::to_string) {
                freq_var = Some(name);
                ts.next();
            }
            ts.expect_semi()?;
            Ok(true)
        } else {
            Ok(false)
        }
    })?;

    Ok(GenmodAst {
        data_options: GenmodDataOptions { input },
        model,
        freq_var,
        class_vars,
    })
}
