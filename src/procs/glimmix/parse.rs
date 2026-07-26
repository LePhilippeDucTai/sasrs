use super::*;

// ───────────────────────── Parser helpers ─────────────────────────

pub(super) fn parse_cov_type(ts: &mut StatementStream) -> CovType {
    let v = ts.peek().ident().map(|s| s.to_ascii_lowercase());
    let t = match v.as_deref() {
        Some("cs") => CovType::Cs,
        Some("un") => CovType::Un,
        Some("ar") => CovType::Ar1,
        _ => CovType::Vc,
    };
    ts.next();
    if ts.peek().kind == TokenKind::LParen {
        ts.next();
        while ts.peek().kind != TokenKind::RParen
            && ts.peek().kind != TokenKind::Semi
            && ts.peek().kind != TokenKind::Eof
        {
            ts.next();
        }
        if ts.peek().kind == TokenKind::RParen {
            ts.next();
        }
    }
    t
}

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC GLIMMIX. Called AFTER `proc glimmix` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<GlimmixAst> {
    let mut data: Option<DatasetRef> = None;
    let mut method = Method::Rspl;

    // PROC GLIMMIX statement options until `;`.
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
        } else if tk.is_kw("method") {
            common::consume_option_eq(ts, "METHOD")?;
            let v = ts.peek().ident().map(|s| s.to_ascii_lowercase());
            method = match v.as_deref() {
                Some("laplace") => Method::Laplace,
                Some("quad") => Method::Quad,
                _ => Method::Rspl,
            };
            ts.next();
        } else {
            ts.next();
        }
    }

    let mut class_vars: Vec<String> = Vec::new();
    let mut model: Option<ModelSpec> = None;
    let mut random: Option<RandomSpec> = None;
    let mut freq_var: Option<String> = None;
    let mut weight_var: Option<String> = None;
    let mut estimate_labels: Vec<String> = Vec::new();
    let mut contrast_labels: Vec<String> = Vec::new();
    let mut lsmeans: Vec<String> = Vec::new();

    common::parse_proc_body(ts, |ts, kw| {
        if kw == "class" {
            ts.next();
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    class_vars.push(name);
                }
                ts.next();
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "model" {
            ts.next();
            model = Some(parse_model(ts)?);
            Ok(true)
        } else if kw == "random" {
            ts.next();
            random = Some(parse_random(ts)?);
            Ok(true)
        } else if kw == "freq" {
            ts.next();
            if let Some(name) = ts.peek().ident().map(str::to_string) {
                freq_var = Some(name);
                ts.next();
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "weight" {
            ts.next();
            if let Some(name) = ts.peek().ident().map(str::to_string) {
                weight_var = Some(name);
                ts.next();
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "estimate" {
            ts.next();
            if let TokenKind::Str { value, .. } = &ts.peek().kind {
                estimate_labels.push(value.clone());
            }
            ts.skip_to_semi();
            Ok(true)
        } else if kw == "contrast" {
            ts.next();
            if let TokenKind::Str { value, .. } = &ts.peek().kind {
                contrast_labels.push(value.clone());
            }
            ts.skip_to_semi();
            Ok(true)
        } else if kw == "lsmeans" {
            ts.next();
            if let Some(name) = ts.peek().ident().map(str::to_string) {
                lsmeans.push(name);
            }
            ts.skip_to_semi();
            Ok(true)
        } else {
            Ok(false)
        }
    })?;

    Ok(GlimmixAst {
        data,
        method,
        class_vars,
        model,
        random,
        freq_var,
        weight_var,
        estimate_labels,
        contrast_labels,
        lsmeans,
    })
}

/// Parse the MODEL statement body (after `model`).
pub(super) fn parse_model(ts: &mut StatementStream) -> Result<ModelSpec> {
    let response = common::parse_model_response(ts, "expected response variable in MODEL")?;

    // Optional response options: (event='val' | descending)
    let (event, descending) = common::parse_response_options(ts);

    common::expect_model_eq(ts, "expected '=' in MODEL statement")?;

    let fixed = common::parse_effect_list(ts);

    let mut dist_opt: Option<Distribution> = None;
    let mut link_opt: Option<LinkFunction> = None;
    let mut solution = false;
    let mut noint = false;

    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
            let tk = ts.peek();
            if tk.is_kw("dist") || tk.is_kw("distribution") || tk.is_kw("d") {
                common::consume_option_eq(ts, "DIST")?;
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    let span = ts.peek().span;
                    ts.next();
                    dist_opt = Some(match name.to_ascii_lowercase().as_str() {
                        "normal" | "gaussian" | "gauss" => Distribution::Normal,
                        "poisson" | "poi" => Distribution::Poisson,
                        "binary" | "bin" | "binomial" => Distribution::Binary,
                        "gamma" | "gam" => Distribution::Gamma,
                        "negbinomial" | "negbin" | "nb" => Distribution::NegBinomial,
                        // MQ9.2 — une distribution inconnue retombait
                        // SILENCIEUSEMENT sur NORMAL : l'utilisateur obtenait
                        // un modèle faux, sans le moindre diagnostic.
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
                }
            } else if tk.is_kw("link") {
                common::consume_option_eq(ts, "LINK")?;
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    let span = ts.peek().span;
                    ts.next();
                    link_opt = Some(match name.to_ascii_lowercase().as_str() {
                        "identity" | "id" => LinkFunction::Identity,
                        "log" => LinkFunction::Log,
                        "logit" => LinkFunction::Logit,
                        "probit" => LinkFunction::Probit,
                        "cloglog" | "cll" => LinkFunction::Cloglog,
                        // MQ9.2 — même piège que DIST= ci-dessus.
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
                }
            } else if tk.is_kw("solution") || tk.is_kw("s") {
                solution = true;
                ts.next();
            } else if tk.is_kw("noint") {
                noint = true;
                ts.next();
            } else {
                ts.next();
            }
        }
    }
    ts.expect_semi()?;

    let dist = dist_opt.unwrap_or(Distribution::Normal);
    let link = link_opt.unwrap_or_else(|| canonical_link(dist));

    Ok(ModelSpec {
        response,
        event,
        descending,
        fixed,
        dist,
        link,
        solution,
        noint,
    })
}

/// Parse the RANDOM statement body (after `random`).
pub(super) fn parse_random(ts: &mut StatementStream) -> Result<RandomSpec> {
    let effects = common::parse_effect_list(ts);

    let mut subject: Option<String> = None;
    let mut cov_type = CovType::Vc;
    let mut solution = false;

    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
            let tk = ts.peek();
            if tk.is_kw("subject") || tk.is_kw("subj") {
                common::consume_option_eq(ts, "SUBJECT")?;
                subject = ts.peek().ident().map(str::to_string);
                ts.next();
            } else if tk.is_kw("type") {
                common::consume_option_eq(ts, "TYPE")?;
                cov_type = parse_cov_type(ts);
            } else if tk.is_kw("solution") || tk.is_kw("s") {
                solution = true;
                ts.next();
            } else {
                ts.next();
            }
        }
    }
    ts.expect_semi()?;

    Ok(RandomSpec {
        effects,
        subject,
        cov_type,
        solution,
    })
}
