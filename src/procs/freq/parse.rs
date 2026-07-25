use super::*;

/// Parse `proc freq [data=a] ; [tables ...;]... run;`. Called AFTER
/// "proc freq" has been consumed. Consumes through `run;`/`quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<FreqAst> {
    let mut data: Option<DatasetRef> = None;

    // --- PROC FREQ statement options, until `;` ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            common::expect_eq(ts, "DATA")?;
            data = Some(ts.parse_dataset_ref()?);
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!(
                    "Unexpected option '{}' on PROC FREQ statement.",
                    name.to_uppercase()
                ),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC FREQ statement.",
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; ---
    let mut tables: Vec<TableRequest> = Vec::new();
    let mut weight: Option<String> = None;
    let mut by: Vec<(String, bool)> = Vec::new();

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "tables" | "table" => {
                ts.next();
                let reqs = parse_tables(ts)?;
                tables.extend(reqs);
                true
            }
            "weight" => {
                ts.next();
                weight = Some(common::parse_weight(ts)?);
                true
            }
            "by" => {
                ts.next();
                by = common::parse_by(ts)?;
                true
            }
            _ => false,
        })
    })?;

    Ok(FreqAst {
        data,
        tables,
        weight,
        by,
    })
}

/// Parse one TABLES statement body (after "tables" consumed), through its
/// terminating `;`. Returns one TableRequest per spec.
pub(super) fn parse_tables(ts: &mut StatementStream) -> Result<Vec<TableRequest>> {
    let mut specs: Vec<Vec<String>> = Vec::new();

    // Specs until `/` (options) or `;`.
    loop {
        match &ts.peek().kind {
            TokenKind::Semi | TokenKind::Slash | TokenKind::Eof => break,
            _ => {}
        }
        // One spec: v or v1*v2.
        let first_tok = ts.peek().clone();
        let Some(first) = first_tok.ident().map(str::to_string) else {
            return Err(SasError::parse(
                "expected a variable name in the TABLES statement",
                first_tok.span,
            ));
        };
        ts.next();
        let mut vars = vec![first];
        // Allow an arbitrary chain v1*v2*v3*… (n-way crosstab).
        while ts.peek().kind == TokenKind::Star {
            ts.next();
            let snd_tok = ts.peek().clone();
            let Some(snd) = snd_tok.ident().map(str::to_string) else {
                return Err(SasError::parse(
                    "expected a variable name after '*' in the TABLES statement",
                    snd_tok.span,
                ));
            };
            ts.next();
            vars.push(snd);
        }
        specs.push(vars);
    }

    // Options after `/`.
    let mut missing = false;
    let mut out: Option<DatasetRef> = None;
    let mut nofreq = false;
    let mut nopercent = false;
    let mut norow = false;
    let mut nocol = false;
    let mut nocum = false;
    let mut chisq = false;
    let mut fisher = false;
    let mut agree = false;
    let mut measures = false;
    let mut trend = false;
    let mut list = false;
    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        loop {
            match &ts.peek().kind {
                TokenKind::Semi | TokenKind::Eof => break,
                _ => {}
            }
            if ts.peek().is_kw("missing") {
                ts.next();
                missing = true;
            } else if ts.peek().is_kw("out") {
                common::expect_eq(ts, "OUT")?;
                out = Some(ts.parse_dataset_ref()?);
            } else if ts.peek().is_kw("nopercent") {
                ts.next();
                nopercent = true;
            } else if ts.peek().is_kw("norow") {
                ts.next();
                norow = true;
            } else if ts.peek().is_kw("nocol") {
                ts.next();
                nocol = true;
            } else if ts.peek().is_kw("nofreq") {
                ts.next();
                nofreq = true;
            } else if ts.peek().is_kw("nocum") {
                ts.next();
                nocum = true;
            } else if ts.peek().is_kw("chisq") {
                ts.next();
                chisq = true;
            } else if ts.peek().is_kw("fisher") || ts.peek().is_kw("exact") {
                ts.next();
                fisher = true;
            } else if ts.peek().is_kw("agree") {
                ts.next();
                agree = true;
            } else if ts.peek().is_kw("measures") || ts.peek().is_kw("relrisk") {
                ts.next();
                measures = true;
            } else if ts.peek().is_kw("trend") {
                ts.next();
                trend = true;
            } else if ts.peek().is_kw("list") {
                ts.next();
                list = true;
            } else if let Some(name) = ts.peek().ident().map(str::to_string) {
                // Unknown option: ignore leniently (skip the token, and any
                // `=value` that follows).
                ts.next();
                if ts.peek().kind == TokenKind::Eq {
                    ts.next();
                    // skip a single value token (ident/num)
                    if !matches!(ts.peek().kind, TokenKind::Semi | TokenKind::Eof) {
                        ts.next();
                    }
                }
            } else {
                // Unexpected token among options: stop (let expect_semi catch
                // the terminator).
                break;
            }
        }
    }

    ts.expect_semi()?;

    // OUT= requires exactly one table spec on the TABLES statement (SAS rule).
    if out.is_some() && specs.len() != 1 {
        return Err(SasError::runtime(
            "The OUT= option in PROC FREQ requires a single table request on the TABLES statement.",
        ));
    }

    let n = specs.len();
    Ok(specs
        .into_iter()
        .enumerate()
        .map(|(i, vars)| TableRequest {
            vars,
            missing,
            // OUT= only applies (and is only valid) for a single spec.
            out: if i == 0 && n == 1 { out.clone() } else { None },
            nofreq,
            nopercent,
            norow,
            nocol,
            nocum,
            chisq,
            fisher,
            agree,
            measures,
            trend,
            list,
        })
        .collect())
}
