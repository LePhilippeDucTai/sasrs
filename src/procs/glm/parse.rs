use super::*;

use crate::token::Span;

// ───────────────────────── Parser helpers ─────────────────────────

/// Parse a list of numeric coefficients from the token stream.
/// Reads numbers (and optional leading minus sign) until `;` or `/`.
pub(super) fn parse_coefficients(ts: &mut StatementStream) -> Vec<f64> {
    let mut coeffs = Vec::new();
    loop {
        let kind = ts.peek().kind.clone();
        match kind {
            TokenKind::Semi | TokenKind::Slash | TokenKind::Eof => break,
            TokenKind::Minus => {
                // Could be a negative number: consume `-` then number
                ts.next();
                let next_kind = ts.peek().kind.clone();
                if let TokenKind::Num(v) = next_kind {
                    coeffs.push(-v);
                    ts.next();
                } else {
                    // Not a number — stop
                    break;
                }
            }
            TokenKind::Num(v) => {
                coeffs.push(v);
                ts.next();
            }
            _ => break,
        }
    }
    coeffs
}

// ───────────────────── Contract diagnostics (J02-P8) ─────────────────────
//
// SAS/STAT 13.1 User's Guide (SAS 9.4), The GLM Procedure, Syntax
// (https://support.sas.com/documentation/cdl/en/statug/66859/HTML/default/statug_glm_syntax.htm),
// ESTIMATE statement (`ESTIMATE 'label' effect values </ options>;`, options
// DIVISOR=, E, SINGULAR=) and CONTRAST statement (options E, E=, ETYPE=,
// SINGULAR=). CONTRIBUTING §5: an option that can change a result is an
// ERROR, a display-only one (E) a WARNING. GLM is outside the functional
// scope of roadmap-avancee: no unit is named.

/// Valid SAS/STAT GLM statements that sasrs does not implement (BY, FREQ,
/// ID, WEIGHT already get the shared message of `unhandled_proc_statement`).
const UNSUPPORTED_STATEMENTS: &[&str] = &[
    "absorb", "code", "manova", "random", "repeated", "store", "test",
];

/// `'label' effect values </ options> ;` of an ESTIMATE or CONTRAST statement
/// (keyword consumed), through the `;`. `stmt` is `ESTIMATE` or `CONTRAST`.
fn parse_lincomb_statement(
    ts: &mut StatementStream,
    stmt: &str,
) -> Result<(String, String, Vec<f64>)> {
    // Read label (string literal)
    let label = if let TokenKind::Str { value, .. } = ts.peek().kind.clone() {
        ts.next();
        value
    } else {
        String::new()
    };
    // Read effect (ident)
    let Some(effect) = ts.peek().ident().map(str::to_string) else {
        return Err(SasError::parse(
            format!("expected an effect name in the {stmt} statement"),
            ts.peek().span,
        ));
    };
    ts.next();
    // Read coefficients
    let coefficients = parse_coefficients(ts);
    // J02-P8 — the options after `/` used to be skipped up to `;`.
    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        parse_lincomb_options(ts, stmt)?;
    }
    ts.expect_semi()?;
    Ok((label, effect, coefficients))
}

/// Options after `/` of an ESTIMATE or CONTRAST statement, up to `;` (not
/// consumed). E displays the L vector (display only → WARNING); DIVISOR=
/// (ESTIMATE), E=effect and ETYPE= (CONTRAST) and SINGULAR= change the
/// estimate, its standard error, the error term or the estimability check
/// (ERROR); anything else is an ERROR too.
fn parse_lincomb_options(ts: &mut StatementStream, stmt: &str) -> Result<()> {
    loop {
        if matches!(ts.peek().kind, TokenKind::Semi | TokenKind::Eof) {
            return Ok(());
        }
        let span = ts.peek().span;
        let opt = ts.peek().ident().unwrap_or("?").to_ascii_uppercase();
        let has_value = ts.peek2().kind == TokenKind::Eq;
        let result_option = match (opt.as_str(), has_value) {
            ("E", false) => {
                ts.next();
                ts.warn_ignored_display(format!(
                    "The E option of the {stmt} statement is ignored in PROC GLM; display \
                     customization is not supported."
                ));
                continue;
            }
            ("DIVISOR", true) => stmt == "ESTIMATE",
            ("SINGULAR", true) => true,
            ("E" | "ETYPE", true) => stmt == "CONTRAST",
            _ => false,
        };
        return Err(if result_option {
            SasError::parse(
                format!(
                    "The {opt}= option of the {stmt} statement is not supported in PROC GLM; it \
                     can affect results and cannot be ignored."
                ),
                span,
            )
        } else {
            SasError::parse(
                format!("Unknown or unsupported {stmt} option '{opt}' in PROC GLM."),
                span,
            )
        });
    }
}

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC GLM. Called AFTER `proc glm` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<GlmAst> {
    let mut input: Option<DatasetRef> = None;

    // PROC GLM statement options until `;`
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

    // Sub-statements until run;/quit;
    let mut class_vars: Vec<String> = Vec::new();
    let mut model: Option<GlmModel> = None;
    let mut lsmeans_vars: Vec<String> = Vec::new();
    let mut estimates: Vec<GlmEstimate> = Vec::new();
    let mut contrasts: Vec<GlmContrast> = Vec::new();
    let mut means_vars: Vec<String> = Vec::new();
    // J02-P2 (issue #17) — statement OUTPUT OUT= (p=/r=).
    let mut output: Option<GlmOutput> = None;
    // J02-P8 — ESTIMATE/CONTRAST statements in source order (span, keyword,
    // effect), checked against the MODEL once the body is read.
    let mut lincombs: Vec<(Span, &'static str, String)> = Vec::new();

    common::parse_proc_body(ts, "GLM", |ts, kw| {
        if kw == "class" {
            ts.next();
            class_vars = ts.parse_name_list()?;
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "model" {
            ts.next();
            // Read dependents: idents before `=` (the `=` itself is consumed).
            let dependents = common::parse_model_lhs(ts);
            // Read effects: idents (optionally joined by `*`) after `=` until `/` or `;`.
            // Build both the legacy flat `effects` list and the structured
            // `effect_terms` (Vec of CLASS-var-name lists) for the multiway engine.
            let (effects, effect_terms) = common::parse_effect_terms(ts);
            let mut solution = false;
            let mut noprint = false;
            if ts.peek().kind == TokenKind::Slash {
                ts.next();
                // Parse options until semi
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    if ts.peek().is_kw("solution") {
                        solution = true;
                    }
                    if ts.peek().is_kw("noprint") {
                        noprint = true;
                    }
                    ts.next();
                }
            }
            ts.expect_semi()?;
            model = Some(GlmModel {
                dependents,
                effects,
                effect_terms,
                solution,
                noprint,
            });
            Ok(true)
        } else if kw == "lsmeans" {
            ts.next();
            // Read lsmeans vars (idents before `/` or `;`)
            let mut vars: Vec<String> = Vec::new();
            loop {
                if ts.peek().kind == TokenKind::Semi
                    || ts.peek().kind == TokenKind::Eof
                    || ts.peek().kind == TokenKind::Slash
                {
                    break;
                }
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    vars.push(name);
                    ts.next();
                } else {
                    ts.next();
                }
            }
            // Skip options after `/`
            if ts.peek().kind == TokenKind::Slash {
                ts.next();
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            lsmeans_vars = vars;
            Ok(true)
        } else if kw == "estimate" || kw == "contrast" {
            let span = ts.peek().span;
            let stmt = if kw == "estimate" {
                "ESTIMATE"
            } else {
                "CONTRAST"
            };
            ts.next();
            let (label, effect, coefficients) = parse_lincomb_statement(ts, stmt)?;
            lincombs.push((span, stmt, effect.clone()));
            if stmt == "ESTIMATE" {
                estimates.push(GlmEstimate {
                    label,
                    effect,
                    coefficients,
                });
            } else {
                contrasts.push(GlmContrast {
                    label,
                    effect,
                    coefficients,
                });
            }
            Ok(true)
        } else if kw == "means" {
            ts.next();
            means_vars = ts.parse_name_list()?;
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "output" {
            // J02-P2 (issue #17) — statement OUTPUT OUT= : `p=`/`predicted=`
            // et `r=`/`residual=` honorés, tout autre mot-clé rejeté
            // explicitement (jamais d'honneur silencieux).
            ts.next();
            output = Some(parse_output_statement(ts)?);
            Ok(true)
        } else if UNSUPPORTED_STATEMENTS.contains(&kw) {
            // J02-P8 — valid SAS/STAT GLM statements that sasrs does not
            // implement used to be reported « 180-322 … not valid ».
            Err(common::unsupported_statement("GLM", kw))
        } else {
            Ok(false)
        }
    })?;

    // J02-P8 — an ESTIMATE/CONTRAST that the executed path cannot produce
    // used to be skipped without output nor NOTE: rejected before execution.
    if let Some(model) = &model {
        for (span, stmt, effect) in &lincombs {
            if let Some(msg) = lincomb_unsupported(stmt, effect, model, &class_vars) {
                return Err(SasError::parse(msg, *span));
            }
        }
    }

    Ok(GlmAst {
        data_options: GlmDataOptions { input },
        class_vars,
        model,
        lsmeans_vars,
        estimates,
        contrasts,
        means_vars,
        output,
    })
}

/// J02-P2 (issue #17) — parse one OUTPUT statement body (after `output`
/// consumed), through its terminating `;`. OUT= is required and at least one
/// statistic keyword (P= or R=) must be present.
pub(super) fn parse_output_statement(ts: &mut StatementStream) -> Result<GlmOutput> {
    let mut out: Option<DatasetRef> = None;
    let mut p: Option<String> = None;
    let mut r: Option<String> = None;
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
        } else if ts.peek().is_kw("p") || ts.peek().is_kw("predicted") {
            let kw = ts.peek().ident().unwrap_or("p").to_uppercase();
            common::consume_option_eq(ts, &kw)?;
            let tok = ts.peek().clone();
            let Some(name) = tok.ident().map(str::to_string) else {
                return Err(SasError::parse(
                    format!("expected an output variable name after {kw}="),
                    tok.span,
                ));
            };
            ts.next();
            p = Some(name);
        } else if ts.peek().is_kw("r") || ts.peek().is_kw("residual") {
            let kw = ts.peek().ident().unwrap_or("r").to_uppercase();
            common::consume_option_eq(ts, &kw)?;
            let tok = ts.peek().clone();
            let Some(name) = tok.ident().map(str::to_string) else {
                return Err(SasError::parse(
                    format!("expected an output variable name after {kw}="),
                    tok.span,
                ));
            };
            ts.next();
            r = Some(name);
        } else if ts.peek().is_kw("student")
            || ts.peek().is_kw("studentr")
            || ts.peek().is_kw("stdp")
            || ts.peek().is_kw("stdr")
            || ts.peek().is_kw("lower")
            || ts.peek().is_kw("upper")
            || ts.peek().is_kw("cookd")
            || ts.peek().is_kw("h")
            || ts.peek().is_kw("residuals")
        {
            let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
            return Err(SasError::parse(
                format!(
                    "Statistic option '{bad}' on the PROC GLM OUTPUT statement is not supported in sasrs."
                ),
                ts.peek().span,
            ));
        } else if ts.peek().ident().is_some() {
            let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
            return Err(SasError::parse(
                format!("Unexpected option '{bad}' on PROC GLM OUTPUT statement."),
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
            "The OUTPUT statement of PROC GLM requires the OUT= option.",
            ts.peek().span,
        ));
    };
    if p.is_none() && r.is_none() {
        return Err(SasError::parse(
            "The OUTPUT statement of PROC GLM requires at least one statistic keyword (P= and R= are the ones supported in sasrs).",
            ts.peek().span,
        ));
    }
    Ok(GlmOutput { out, p, r })
}
