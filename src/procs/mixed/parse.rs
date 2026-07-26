use super::*;

// ───────────────────────── Parser helpers ─────────────────────────

/// Parse a TYPE=... value, including `ar(1)`.
pub(super) fn parse_cov_type(ts: &mut StatementStream) -> CovType {
    let v = ts.peek().ident().map(|s| s.to_ascii_lowercase());
    let t = match v.as_deref() {
        Some("cs") => CovType::Cs,
        Some("un") => CovType::Un,
        Some("ar") => CovType::Ar1,
        _ => CovType::Vc,
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
    t
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
            let v = ts.peek().ident().map(|s| s.to_ascii_lowercase());
            method = match v.as_deref() {
                Some("ml") => Method::Ml,
                _ => Method::Reml,
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
    let mut lsmeans: Vec<LsmeansSpec> = Vec::new();
    let mut estimate_labels: Vec<String> = Vec::new();
    let mut contrast_labels: Vec<String> = Vec::new();

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
            random = Some(parse_random(ts));
            Ok(true)
        } else if kw == "repeated" {
            ts.next();
            repeated = Some(parse_repeated(ts));
            Ok(true)
        } else if kw == "lsmeans" {
            ts.next();
            if let Some(spec) = parse_lsmeans(ts) {
                lsmeans.push(spec);
            }
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

    // Read fixed effects until `/` or `;`.
    let fixed = common::parse_effect_list(ts);

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
                ddfm = ts.peek().ident().map(|s| s.to_ascii_lowercase());
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
pub(super) fn parse_random(ts: &mut StatementStream) -> RandomSpec {
    let effects = common::parse_effect_list(ts);

    let mut subject: Option<String> = None;
    let mut cov_type = CovType::Vc;

    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
            let tk = ts.peek();
            if tk.is_kw("subject") || tk.is_kw("subj") {
                let _ = common::consume_option_eq(ts, "SUBJECT");
                subject = ts.peek().ident().map(str::to_string);
                ts.next();
            } else if tk.is_kw("type") {
                let _ = common::consume_option_eq(ts, "TYPE");
                cov_type = parse_cov_type(ts);
            } else {
                ts.next();
            }
        }
    }
    let _ = ts.expect_semi();

    RandomSpec {
        effects,
        subject,
        cov_type,
    }
}

/// Parse the REPEATED statement body (after `repeated`).
pub(super) fn parse_repeated(ts: &mut StatementStream) -> RepeatedSpec {
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
                let _ = common::consume_option_eq(ts, "SUBJECT");
                subject = ts.peek().ident().map(str::to_string);
                ts.next();
            } else if tk.is_kw("type") {
                let _ = common::consume_option_eq(ts, "TYPE");
                cov_type = parse_cov_type(ts);
            } else {
                ts.next();
            }
        }
    }
    let _ = ts.expect_semi();

    RepeatedSpec { subject, cov_type }
}

/// Parse the LSMEANS statement body (after `lsmeans`).
pub(super) fn parse_lsmeans(ts: &mut StatementStream) -> Option<LsmeansSpec> {
    let effect = ts.peek().ident().map(str::to_string)?;
    ts.next();

    let mut diff = false;
    let mut pdiff = false;
    let mut cl = false;
    let mut alpha = 0.05;

    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
            let tk = ts.peek();
            if tk.is_kw("diff") {
                diff = true;
                ts.next();
            } else if tk.is_kw("pdiff") {
                pdiff = true;
                ts.next();
            } else if tk.is_kw("cl") {
                cl = true;
                ts.next();
            } else if tk.is_kw("alpha") {
                let _ = common::consume_option_eq(ts, "ALPHA");
                if let TokenKind::Num(v) = ts.peek().kind {
                    alpha = v;
                }
                ts.next();
            } else {
                ts.next();
            }
        }
    }
    let _ = ts.expect_semi();

    Some(LsmeansSpec {
        effect,
        diff,
        pdiff,
        cl,
        alpha,
    })
}
