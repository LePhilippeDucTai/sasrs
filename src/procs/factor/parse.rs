use super::*;

// ───────────────────────── Parser ─────────────────────────

/// Parse `proc factor [options]; [var ...;] run;`.
/// Called AFTER "proc factor" has been consumed. Consumes through run;/quit;.
pub fn parse(ts: &mut StatementStream) -> Result<FactorAst> {
    let mut data: Option<DatasetRef> = None;
    let mut cov = false;
    let mut nfactors: Option<usize> = None;
    let mut method = "principal".to_string();
    let mut rotate = "none".to_string();
    let mut out: Option<DatasetRef> = None;

    // --- PROC FACTOR statement options, until `;` ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            data = Some(common::parse_dataset_opt(ts, "DATA")?);
        } else if ts.peek().is_kw("cov") || ts.peek().is_kw("covariance") {
            ts.next();
            cov = true;
        } else if ts.peek().is_kw("nfactors") {
            common::expect_eq(ts, "NFACTORS")?;
            let span = ts.peek().span;
            let k = match ts.peek().kind {
                TokenKind::Num(v) => v,
                _ => return Err(SasError::parse("expected a number after NFACTORS=", span)),
            };
            ts.next();
            nfactors = Some(k as usize);
        } else if ts.peek().is_kw("method") {
            common::expect_eq(ts, "METHOD")?;
            let span = ts.peek().span;
            match ts.peek().ident() {
                Some(m) => {
                    method = m.to_lowercase();
                    ts.next();
                }
                None => return Err(SasError::parse("expected a method name after METHOD=", span)),
            }
        } else if ts.peek().is_kw("rotate") {
            common::expect_eq(ts, "ROTATE")?;
            let span = ts.peek().span;
            match ts.peek().ident() {
                Some(r) => {
                    rotate = r.to_lowercase();
                    ts.next();
                }
                None => return Err(SasError::parse("expected a rotation name after ROTATE=", span)),
            }
        } else if ts.peek().is_kw("out") {
            out = Some(common::parse_out_opt(ts)?);
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!(
                    "Unexpected option '{}' on PROC FACTOR statement.",
                    name.to_uppercase()
                ),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC FACTOR statement.",
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; (combinateur partagé M31) ---
    let mut var: Vec<String> = Vec::new();
    common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "var" => {
                ts.next();
                var = ts.parse_name_list()?;
                ts.expect_semi()?;
                true
            }
            _ => false,
        })
    })?;

    Ok(FactorAst {
        data,
        cov,
        nfactors,
        method,
        rotate,
        out,
        var,
    })
}

/// Validate METHOD=, ROTATE= and the VAR list arity before any data access.
pub(super) fn validate_options(ast: &FactorAst) -> Result<()> {
    // Validate method.
    if ast.method != "principal" {
        return Err(SasError::runtime(format!(
            "PROC FACTOR METHOD={} is not supported. Only METHOD=PRINCIPAL is implemented.",
            ast.method.to_uppercase()
        )));
    }

    // Validate rotate.
    if ast.rotate == "oblimin" {
        return Err(SasError::runtime(
            "PROC FACTOR ROTATE=OBLIMIN is not yet implemented. Use ROTATE=PROMAX, VARIMAX or NONE.",
        ));
    }
    if ast.rotate != "none" && ast.rotate != "varimax" && ast.rotate != "promax" {
        return Err(SasError::runtime(format!(
            "PROC FACTOR ROTATE={} is not supported. Use ROTATE=PROMAX, VARIMAX or NONE.",
            ast.rotate.to_uppercase()
        )));
    }

    // At least 2 variables required.
    if ast.var.len() < 2 {
        return Err(SasError::runtime(
            "PROC FACTOR requires at least 2 variables.",
        ));
    }
    Ok(())
}
