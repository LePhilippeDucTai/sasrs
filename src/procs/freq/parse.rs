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
            common::consume_option_eq(ts, "DATA")?;
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
    let mut output: Option<FreqOutput> = None;

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, "FREQ", |ts, kw| {
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
            // J02-P1 — statement OUTPUT (issue #16) : `output out=<ds> chisq;`
            // matérialise les statistiques CHISQ de la DERNIÈRE requête
            // TABLES en dataset (colonnes _PCHI_, _PCHI_DF_, P_PCHI —
            // décision a43c8c14).
            "output" => {
                ts.next();
                output = Some(parse_output_statement(ts)?);
                true
            }
            _ => false,
        })
    })?;

    // SAS requires a TABLES statement when OUTPUT is present (the statistics
    // come from the last TABLES request).
    if output.is_some() && tables.is_empty() {
        return Err(SasError::runtime(
            "The OUTPUT statement requires a preceding TABLES statement in PROC FREQ.",
        ));
    }

    Ok(FreqAst {
        data,
        tables,
        weight,
        by,
        output,
    })
}

/// J02-P1/J02-P2 — parse one OUTPUT statement body (after `output`
/// consumed), through its terminating `;`. Only OUT= and the CHISQ/FISHER
/// statistic keywords are honored (J02-P2, issue #17 : FISHER →
/// XP2_FISH/LXP2_FISH) ; EXACT et les autres mots-clés statistiques
/// lèvent une erreur explicite au lieu d'être ignorés silencieusement (une
/// statistique demandée mais absente ne doit jamais paraître honorée).
pub(super) fn parse_output_statement(ts: &mut StatementStream) -> Result<FreqOutput> {
    let mut out: Option<DatasetRef> = None;
    let mut chisq = false;
    let mut fisher = false;
    loop {
        match &ts.peek().kind {
            TokenKind::Semi => {
                ts.next();
                break;
            }
            TokenKind::Eof => break,
            _ => {}
        }
        if ts.peek().is_kw("out") {
            common::consume_option_eq(ts, "OUT")?;
            out = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("chisq") {
            ts.next();
            chisq = true;
        } else if ts.peek().is_kw("fisher") {
            // J02-P2 (issue #17) — OUTPUT FISHER : p exacte bilatérale
            // (XP2_FISH) et son logarithme népérien (LXP2_FISH), doc FREQ
            // « OUTPUT Statement » citée par le corpus.
            ts.next();
            fisher = true;
        } else if ts.peek().is_kw("exact") {
            return Err(SasError::parse(
                "EXACT is not an OUTPUT statement statistic keyword in sasrs; use FISHER.",
                ts.peek().span,
            ));
        } else if ts.peek().is_kw("agree")
            || ts.peek().is_kw("measures")
            || ts.peek().is_kw("relrisk")
            || ts.peek().is_kw("trend")
            || ts.peek().is_kw("expected")
            || ts.peek().is_kw("n")
        {
            let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
            return Err(SasError::parse(
                format!(
                    "Statistic option '{bad}' on the PROC FREQ OUTPUT statement is not supported in sasrs."
                ),
                ts.peek().span,
            ));
        } else if ts.peek().ident().is_some() {
            let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
            return Err(SasError::parse(
                format!("Unexpected option '{bad}' on PROC FREQ OUTPUT statement."),
                ts.peek().span,
            ));
        } else {
            // Unexpected token: stop (let expect_semi catch it).
            ts.expect_semi()?;
            break;
        }
    }

    let Some(out) = out else {
        return Err(SasError::parse(
            "The OUTPUT statement of PROC FREQ requires the OUT= option.",
            ts.peek().span,
        ));
    };
    if !chisq && !fisher {
        return Err(SasError::parse(
            "The OUTPUT statement of PROC FREQ requires at least one statistic keyword (CHISQ and FISHER are the ones supported in sasrs).",
            ts.peek().span,
        ));
    }
    Ok(FreqOutput { out, chisq, fisher })
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
                common::consume_option_eq(ts, "OUT")?;
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
            } else if ts.peek().ident().is_some() {
                // J02-P4 — unknown TABLES option: no silent skip (a typo or an
                // unimplemented request must not look honored).
                let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
                return Err(SasError::parse(
                    format!("Unexpected option '{bad}' on PROC FREQ TABLES statement."),
                    ts.peek().span,
                ));
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
