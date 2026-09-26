use super::*;

// ───────────────────────── Parser helpers ─────────────────────────

/// Parse a TYPE=... value, including `ar(1)`.
///
/// J02-P5 — unknown TYPE= values are an ERROR (SAS/STAT 9.4, The MIXED
/// Procedure, REPEATED/RANDOM `TYPE=` covariance structures) instead of the
/// former silent fallback to VC.
pub(super) fn parse_cov_type(ts: &mut StatementStream) -> Result<CovType> {
    let span = ts.peek().span;
    let v = ts.peek().ident().map(|s| s.to_ascii_lowercase());
    let t = match v.as_deref() {
        Some("vc") | None => CovType::Vc,
        Some("cs") => CovType::Cs,
        Some("un") => CovType::Un,
        Some("ar") => CovType::Ar1,
        Some(other) => {
            return Err(SasError::parse(
                format!(
                    "Unknown or unsupported TYPE= value '{}' in PROC MIXED; \
                     supported: VC, CS, UN, AR(1).",
                    other.to_uppercase()
                ),
                span,
            ));
        }
    };
    ts.next();
    // Consume an optional `(1)` after AR.
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
    Ok(t)
}

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC MIXED. Called AFTER `proc mixed` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<MixedAst> {
    let mut data: Option<DatasetRef> = None;
    let mut method = Method::Reml;
    let mut covtest = false;
    let mut nobound = false;
    let mut asycov = false;

    // PROC MIXED statement options, until `;`.
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
            let span = ts.peek().span;
            let v = ts.peek().ident().map(|s| s.to_ascii_lowercase());
            method = match v.as_deref() {
                Some("reml") | None => Method::Reml,
                Some("ml") => Method::Ml,
                Some(other) => {
                    return Err(SasError::parse(
                        format!(
                            "Unknown or unsupported METHOD= value '{}' in PROC MIXED; \
                             supported: REML, ML.",
                            other.to_uppercase()
                        ),
                        span,
                    ));
                }
            };
            ts.next();
        } else if tk.is_kw("covtest") {
            covtest = true;
            ts.next();
        } else if tk.is_kw("nobound") {
            nobound = true;
            ts.next();
        } else if tk.is_kw("asycov") {
            asycov = true;
            ts.next();
        } else {
            ts.next();
        }
    }

    let mut class_vars: Vec<String> = Vec::new();
    let mut model: Option<ModelSpec> = None;
    let mut random: Option<RandomSpec> = None;
    let mut repeated: Option<RepeatedSpec> = None;
    let lsmeans: Vec<LsmeansSpec> = Vec::new();
    let estimate_labels: Vec<String> = Vec::new();
    let contrast_labels: Vec<String> = Vec::new();

    common::parse_proc_body(ts, "MIXED", |ts, kw| {
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
        } else if kw == "repeated" {
            ts.next();
            repeated = Some(parse_repeated(ts)?);
            Ok(true)
        } else if matches!(kw, "estimate" | "contrast" | "lsmeans") {
            Err(common::unsupported_statement("MIXED", kw))
        } else {
            Ok(false)
        }
    })?;

    Ok(MixedAst {
        data,
        method,
        covtest,
        nobound,
        asycov,
        class_vars,
        model,
        random,
        repeated,
        lsmeans,
        estimate_labels,
        contrast_labels,
    })
}

/// Parse the MODEL statement body (after `model`): `response = <fixed> / opts;`.
pub(super) fn parse_model(ts: &mut StatementStream) -> Result<ModelSpec> {
    let response = common::parse_model_response(ts, "expected response variable in MODEL")?;
    common::expect_model_eq(ts, "expected '=' in MODEL statement")?;

    // Read fixed effects until `/` or `;`. J02-P5 — `a*b` / `a(b)` are NOT
    // silently flattened anymore (SAS/STAT 9.4 effect syntax): ERROR.
    let fixed = common::parse_effect_list_strict(ts, "MIXED")?;

    let mut solution = false;
    let mut noint = false;
    let mut ddfm: Option<String> = None;
    let mut nofit = false;

    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
            let tk = ts.peek();
            if tk.is_kw("solution") || tk.is_kw("s") {
                solution = true;
                ts.next();
            } else if tk.is_kw("noint") {
                noint = true;
                ts.next();
            } else if tk.is_kw("nofit") {
                nofit = true;
                ts.next();
            } else if tk.is_kw("ddfm") {
                common::consume_option_eq(ts, "DDFM")?;
                let span = ts.peek().span;
                let v = ts.peek().ident().map(|s| s.to_ascii_lowercase());
                match v.as_deref() {
                    Some("contain") => ddfm = v,
                    Some(other) => {
                        return Err(SasError::parse(
                            format!(
                                "DDFM={} is not supported in PROC MIXED; \
                                 only DDFM=CONTAIN is implemented.",
                                other.to_uppercase()
                            ),
                            span,
                        ));
                    }
                    None => {
                        return Err(SasError::parse("expected a value after DDFM=", span));
                    }
                }
                ts.next();
            } else {
                ts.next();
            }
        }
    }
    ts.expect_semi()?;

    Ok(ModelSpec {
        response,
        fixed,
        solution,
        noint,
        ddfm,
        nofit,
    })
}

/// Parse the RANDOM statement body (after `random`).
pub(super) fn parse_random(ts: &mut StatementStream) -> Result<RandomSpec> {
    let effects = common::parse_effect_list_strict(ts, "MIXED")?;

    let mut subject: Option<String> = None;
    let mut cov_type = CovType::Vc;

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
                cov_type = parse_cov_type(ts)?;
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
    })
}

/// Parse the REPEATED statement body (after `repeated`).
pub(super) fn parse_repeated(ts: &mut StatementStream) -> Result<RepeatedSpec> {
    let mut subject: Option<String> = None;
    let mut cov_type = CovType::Vc;

    // Skip any effect tokens before `/`.
    while ts.peek().kind != TokenKind::Semi
        && ts.peek().kind != TokenKind::Slash
        && ts.peek().kind != TokenKind::Eof
    {
        ts.next();
    }
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
                cov_type = parse_cov_type(ts)?;
            } else {
                ts.next();
            }
        }
    }
    ts.expect_semi()?;

    Ok(RepeatedSpec { subject, cov_type })
}
