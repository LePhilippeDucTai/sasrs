use super::*;

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC GENMOD. Called AFTER `proc genmod` has been consumed.
///
/// J02-P1 — fin des replis silencieux (SAS/STAT 9.4 User's Guide, The GENMOD
/// Procedure) : l'option PROC `DESCENDING` est honorée, toute autre option
/// PROC est une ERROR ; les options CLASS, `SCALE=PEARSON|DEVIANCE` et
/// `SCALE=<n>` sous POISSON/BINOMIAL sont des ERROR (levées par
/// roadmap-avancee J07-P2) ; sans `DIST=`, la loi est NORMAL (défaut SAS) ;
/// les instructions GENMOD valides non implémentées suivent le message
/// « not supported … cannot be ignored » du contrat.
pub fn parse(ts: &mut StatementStream) -> Result<GenmodAst> {
    let mut input: Option<DatasetRef> = None;
    let mut proc_descending = false;

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
        } else if ts.peek().is_kw("descending") || ts.peek().is_kw("desc") {
            // PROC GENMOD statement, DESCENDING: reverses the order of the
            // response levels (SAS/STAT 9.4, The GENMOD Procedure).
            proc_descending = true;
            ts.next();
        } else {
            // J02-P1 — every other PROC option was skipped token by token.
            return Err(common::unknown_option_error(ts, "GENMOD"));
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
                } else if matches!(ts.peek().kind, TokenKind::LParen | TokenKind::Slash) {
                    // J02-P1 — `(REF= PARAM= ORDER= DESC MISSING …)` and
                    // `/ options` used to be skipped (their words even became
                    // CLASS variables): the coding silently ignored them.
                    return Err(SasError::parse(
                        "CLASS statement options (REF=, PARAM=, ORDER=, DESCENDING, MISSING, \
                         ...) are not supported in PROC GENMOD; the coding of the CLASS \
                         effects would silently differ from the request (planned: \
                         roadmap-avancee J07-P2).",
                        ts.peek().span,
                    ));
                } else {
                    return Err(SasError::parse(
                        "expected a variable name in the CLASS statement",
                        ts.peek().span,
                    ));
                }
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "model" {
            ts.next(); // consume "model"

            // Response variable
            let response = common::parse_model_response(ts, "expected response variable")?;

            // Response options: (DESCENDING|DESC EVENT='v'|FIRST|LAST) are
            // honored; ORDER=/REF= and unknown tokens are ERRORs (J02-P1).
            let common::ResponseOptions { event, descending } =
                common::parse_response_options_checked(ts, "GENMOD")?;

            // Expect '='
            common::expect_model_eq(ts, "expected '=' after response variable in MODEL")?;

            // Predictors until '/' or ';'. J02-P5 — `a*b` / `a(b)` are NOT
            // silently flattened anymore (SAS/STAT 9.4, GENMOD « MODEL
            // Statement » effect syntax): they are an explicit ERROR.
            let predictors = common::parse_effect_list_strict(ts, "GENMOD")?;

            let mut dist_opt: Option<Distribution> = None;
            let mut link_opt: Option<LinkFunction> = None;
            let mut noprint = false;
            let mut scale_opt: Option<(f64, crate::token::Span)> = None;
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
                        let span = ts.peek().span;
                        ts.next();
                        if ts.peek().kind == TokenKind::Eq {
                            ts.next();
                        }
                        // SCALE=<number>. J02-P1 — SCALE=PEARSON|P|DEVIANCE|D
                        // (and any other value) used to be skipped silently.
                        if let TokenKind::Num(v) = ts.peek().kind {
                            scale_opt = Some((v, span));
                            ts.next();
                        } else {
                            let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
                            return Err(SasError::parse(
                                format!(
                                    "SCALE={bad} is not supported in PROC GENMOD; only a numeric \
                                     SCALE= value is implemented (planned: roadmap-avancee \
                                     J07-P2)."
                                ),
                                ts.peek().span,
                            ));
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

            // J02-P1 — DIST= defaults to NORMAL (SAS/STAT 9.4, The GENMOD
            // Procedure, MODEL statement, DIST=: « If you do not specify a
            // distribution, the normal distribution is assumed »); the code
            // used to fit a Poisson model.
            let dist = dist_opt.unwrap_or(Distribution::Normal);
            // If LINK not given, use canonical link for the distribution
            let link = link_opt.unwrap_or_else(|| canonical_link(&dist));
            // J02-P1 — SCALE=<n> under POISSON/BINOMIAL left the standard
            // errors at the φ=1 model without any diagnostic.
            if let Some((_, span)) = scale_opt
                && matches!(dist, Distribution::Poisson | Distribution::Binomial)
            {
                return Err(SasError::parse(
                    "SCALE=<n> is not supported for DIST=POISSON or DIST=BINOMIAL in PROC \
                     GENMOD; the dispersion would be silently ignored (planned: \
                     roadmap-avancee J07-P2).",
                    span,
                ));
            }

            model = Some(GenmodModel {
                response,
                event,
                // PROC-level DESCENDING applies to the MODEL response.
                descending: descending || proc_descending,
                predictors,
                dist,
                link,
                noprint,
                scale: scale_opt.map(|(v, _)| v),
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
        } else if UNSUPPORTED_STATEMENTS.contains(&kw) {
            // J02-P1 — valid PROC GENMOD statements that are not implemented:
            // contract message instead of « 180-322 not valid ».
            Err(common::unsupported_statement("GENMOD", kw))
        } else if kw == "effectplot" {
            // Graphics only: display customization (WARNING, CONTRIBUTING §5).
            ts.warn_ignored_display(common::ignored_display_statement("GENMOD", kw));
            ts.skip_to_semi();
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

/// Instructions valides de PROC GENMOD (SAS/STAT 9.4 User's Guide, The GENMOD
/// Procedure, Syntax) non implémentées par sasrs, hors de la liste partagée
/// de `common::unhandled_proc_statement` (BY, WEIGHT, OUTPUT, ID, ESTIMATE,
/// CONTRAST, LSMEANS…). Toutes peuvent changer les résultats ; EFFECTPLOT
/// (graphique seul) est traité à part avec un WARNING.
const UNSUPPORTED_STATEMENTS: &[&str] = &[
    "assess",
    "bayes",
    "code",
    "deviance",
    "effect",
    "exact",
    "exactoptions",
    "fwdlink",
    "invlink",
    "lsmestimate",
    "repeated",
    "slice",
    "store",
    "strata",
    "variance",
    "zeromodel",
];
