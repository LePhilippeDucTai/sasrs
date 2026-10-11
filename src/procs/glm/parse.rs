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

// ───────────────────── Contract diagnostics (J02-P8, J02-P12) ─────────────
//
// SAS/STAT 13.1 User's Guide (SAS 9.4), The GLM Procedure, Syntax
// (https://support.sas.com/documentation/cdl/en/statug/66859/HTML/default/statug_glm_syntax.htm),
// PROC GLM statement (options ALPHA=, DATA=, MANOVA, MULTIPASS, NAMELEN=,
// NOPRINT, ORDER=, OUTSTAT=, PLOTS)
// https://support.sas.com/documentation/cdl/en/statug/66859/HTML/default/statug_glm_syntax01.htm
// MODEL statement (options EFFECTSIZE … ZETA=)
// https://support.sas.com/documentation/cdl/en/statug/66859/HTML/default/statug_glm_syntax16.htm
// LSMEANS statement (options ADJUST= … TDIFF)
// https://support.sas.com/documentation/cdl/en/statug/66859/HTML/default/statug_glm_syntax10.htm
// ESTIMATE statement (`ESTIMATE 'label' effect values </ options>;`, options
// DIVISOR=, E, SINGULAR=) and CONTRAST statement (options E, E=, ETYPE=,
// SINGULAR=). CONTRIBUTING §5: an option that can change a result is an
// ERROR, a display-only one a WARNING. GLM is outside the functional
// scope of roadmap-avancee: no unit is named.

/// Contract ERROR suffix shared by the rejected requests.
const CANNOT: &str = "is not supported in PROC GLM; it can affect results and cannot be ignored";

/// Valid SAS/STAT GLM statements that sasrs does not implement (BY, FREQ,
/// ID, WEIGHT already get the shared message of `unhandled_proc_statement`).
const UNSUPPORTED_STATEMENTS: &[&str] = &[
    "absorb", "code", "manova", "random", "repeated", "store", "test",
];

/// Queue the shared display-only WARNING (J02-P11/J02-P8 wording).
fn warn_display(ts: &mut StatementStream, what: &str) {
    ts.warn_ignored_display(format!(
        "The {what} is ignored in PROC GLM; display customization is not supported."
    ));
}

/// Consume and discard the value of a bare-or-valued option keyword that was
/// just accepted (keyword already consumed): an optional parenthesized
/// suboption list (`plots(unpack)=…`) then `= value` or `= (… list …)`.
/// Keeps the loop synchronised for the accepted display options.
fn consume_display_option_value(ts: &mut StatementStream) {
    if ts.peek().kind == TokenKind::LParen {
        ts.skip_balanced_parens();
    }
    common::skip_option_value(ts);
}

/// J02-P12 — options of the PROC GLM statement. Base : every option other
/// than DATA= was skipped token by token. SAS/STAT 9.4 classification:
/// - DATA= is honored;
/// - ORDER= (« specifies the order in which to sort classification
///   variables … determines which parameters in the model correspond to each
///   level »), OUTSTAT= (output data set), ALPHA= (confidence level) and
///   MANOVA (mode of eliminating missing values) can change results or
///   create a table → ERROR;
/// - NOPRINT (« suppresses the normal display of results »), PLOTS (ODS
///   Graphics) and NAMELEN= (length of effect names in tables) only change
///   the display → WARNING (not applied);
/// - MULTIPASS only trades disk space for execution time → no observable
///   effect, accepted without a diagnostic (same policy as IML SYMSIZE=).
fn parse_proc_options(ts: &mut StatementStream, input: &mut Option<DatasetRef>) -> Result<()> {
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            return Ok(());
        }
        if ts.peek().kind == TokenKind::Eof {
            return Ok(());
        }
        if ts.peek().is_kw("data") {
            *input = Some(common::parse_dataset_opt(ts, "DATA")?);
            continue;
        }
        let span = ts.peek().span;
        let opt = ts.peek().ident().unwrap_or("?").to_ascii_uppercase();
        match opt.as_str() {
            "NOPRINT" => {
                ts.next();
                warn_display(ts, "NOPRINT option of the PROC GLM statement");
            }
            "PLOTS" | "PLOT" => {
                ts.next();
                consume_display_option_value(ts);
                warn_display(ts, "PLOTS= option of the PROC GLM statement");
            }
            "NAMELEN" => {
                ts.next();
                common::skip_option_value(ts);
                warn_display(ts, "NAMELEN= option of the PROC GLM statement");
            }
            "MULTIPASS" => {
                ts.next();
            }
            "ORDER" | "OUTSTAT" | "ALPHA" | "MANOVA" => {
                let eq = if ts.peek2().kind == TokenKind::Eq {
                    "="
                } else {
                    ""
                };
                return Err(SasError::parse(
                    format!("The {opt}{eq} option of the PROC GLM statement {CANNOT}."),
                    span,
                ));
            }
            _ => {
                return Err(SasError::parse(
                    format!("Unexpected option '{opt}' on PROC GLM statement."),
                    span,
                ));
            }
        }
    }
}

/// J02-P12 — options of the MODEL statement after `/`. Base : every option
/// other than SOLUTION was skipped token by token (`/ noint` kept a model
/// fitted with an intercept, `/ noprint` was read but not applied).
/// SAS/STAT 9.4 classification:
/// - SOLUTION is honored; SS1 and SS3 ask for tables that PROC GLM « also
///   displays by default » and sasrs always prints → accepted;
/// - NOPRINT and NOUNI suppress displayed results, E, E1 and E3 display
///   estimable functions whose sums of squares are the ones already
///   printed → WARNING (not applied);
/// - every other option changes the fitted model (NOINT), adds computed
///   results (SS2/SS4, E2/E4, ALPHA=, CLI, CLM, CLPARM, P, INVERSE, XPX,
///   EFFECTSIZE, INTERCEPT, ALIASING, TOLERANCE) or tunes the numerics
///   (SINGULAR=, ZETA=) → ERROR.
fn parse_model_options(ts: &mut StatementStream, solution: &mut bool) -> Result<()> {
    ts.next(); // consume `/`
    while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
        let span = ts.peek().span;
        let opt = ts.peek().ident().unwrap_or("?").to_ascii_uppercase();
        match opt.as_str() {
            "SOLUTION" => {
                ts.next();
                *solution = true;
            }
            "SS1" | "SS3" => {
                ts.next();
            }
            "NOPRINT" => {
                ts.next();
                warn_display(ts, "NOPRINT option of the MODEL statement");
            }
            "NOUNI" => {
                ts.next();
                warn_display(ts, "NOUNI option of the MODEL statement");
            }
            "E" | "E1" | "E3" => {
                ts.next();
                warn_display(ts, &format!("{opt} option of the MODEL statement"));
            }
            "NOINT" | "SS2" | "SS4" | "E2" | "E4" | "EFFECTSIZE" | "INTERCEPT" | "INT"
            | "ALIASING" | "TOLERANCE" | "ALPHA" | "CLI" | "CLM" | "CLPARM" | "P" | "INVERSE"
            | "I" | "XPX" | "SINGULAR" | "ZETA" => {
                let eq = if ts.peek2().kind == TokenKind::Eq {
                    "="
                } else {
                    ""
                };
                return Err(SasError::parse(
                    format!("The {opt}{eq} option of the MODEL statement {CANNOT}."),
                    span,
                ));
            }
            _ => {
                return Err(SasError::parse(
                    format!("Unexpected option '{opt}' on the PROC GLM MODEL statement."),
                    span,
                ));
            }
        }
    }
    Ok(())
}

/// J02-P12 — options of the LSMEANS statement after `/`. Base : they were
/// skipped token by token. SAS/STAT 9.4 classification:
/// - STDERR (abbreviation SE, as in `lsmeans a / se;`) « produces the
///   standard error of the LS-mean and the probability level » — both
///   already printed → accepted;
/// - E (« displays the coefficients of the linear functions »), NOPRINT
///   (« suppresses the normal display of results from the LSMEANS
///   statement ») and PLOT= (ODS Graphics) only change the display →
///   WARNING (not applied);
/// - PDIFF/DIFF, ADJUST=, OUT=, CL, ALPHA=, AT, OM/OBSMARGINS, BYLEVEL,
///   COV, E=, ETYPE=, LINES, SLICE=, SINGULAR= and TDIFF add tests,
///   adjusted p-values, confidence limits, a different weighting or an
///   output data set → ERROR.
fn parse_lsmeans_options(ts: &mut StatementStream) -> Result<()> {
    ts.next(); // consume `/`
    while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
        let span = ts.peek().span;
        let opt = ts.peek().ident().unwrap_or("?").to_ascii_uppercase();
        let has_value = ts.peek2().kind == TokenKind::Eq;
        match (opt.as_str(), has_value) {
            ("STDERR" | "SE" | "STD", _) => {
                ts.next();
            }
            ("E", false) => {
                ts.next();
                warn_display(ts, "E option of the LSMEANS statement");
            }
            ("NOPRINT", _) => {
                ts.next();
                warn_display(ts, "NOPRINT option of the LSMEANS statement");
            }
            ("PLOT" | "PLOTS", _) => {
                ts.next();
                consume_display_option_value(ts);
                warn_display(ts, "PLOT= option of the LSMEANS statement");
            }
            (
                "PDIFF" | "DIFF" | "ADJUST" | "OUT" | "CL" | "ALPHA" | "AT" | "OM" | "OBSMARGINS"
                | "BYLEVEL" | "COV" | "E" | "ETYPE" | "LINES" | "SLICE" | "SINGULAR" | "TDIFF",
                _,
            ) => {
                return Err(SasError::parse(
                    format!(
                        "The {opt}{} option of the LSMEANS statement {CANNOT}.",
                        if has_value { "=" } else { "" }
                    ),
                    span,
                ));
            }
            _ => {
                return Err(SasError::parse(
                    format!("Unexpected option '{opt}' on the PROC GLM LSMEANS statement."),
                    span,
                ));
            }
        }
    }
    Ok(())
}

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
    parse_proc_options(ts, &mut input)?;

    // Sub-statements until run;/quit;
    let mut class_vars: Vec<String> = Vec::new();
    let mut model: Option<GlmModel> = None;
    // J02-P12 — one entry per LSMEANS/MEANS statement, in source order
    // (SAS/STAT 9.4, LSMEANS statement: « Multiple effects can be specified
    // in one LSMEANS statement, or multiple LSMEANS statements can be
    // used »). Base : only the last statement was kept.
    let mut lsmeans_stmts: Vec<Vec<String>> = Vec::new();
    let mut estimates: Vec<GlmEstimate> = Vec::new();
    let mut contrasts: Vec<GlmContrast> = Vec::new();
    let mut means_stmts: Vec<Vec<String>> = Vec::new();
    // J02-P2 (issue #17) — statement OUTPUT OUT= (p=/r=).
    let mut output: Option<GlmOutput> = None;
    // J02-P8 — ESTIMATE/CONTRAST statements in source order (span, keyword,
    // effect, coefficients), checked against the MODEL once the body is read.
    let mut lincombs: Vec<(Span, &'static str, String, Vec<f64>)> = Vec::new();

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
            if ts.peek().kind == TokenKind::Slash {
                parse_model_options(ts, &mut solution)?;
            }
            ts.expect_semi()?;
            model = Some(GlmModel {
                dependents,
                effects,
                effect_terms,
                solution,
            });
            Ok(true)
        } else if kw == "lsmeans" {
            ts.next();
            // Read lsmeans effects (idents before `/` or `;`). J02-P12 —
            // a non-name token is no longer skipped: an interaction changes
            // which LS-means are computed.
            let mut vars: Vec<String> = Vec::new();
            loop {
                if ts.peek().kind == TokenKind::Semi
                    || ts.peek().kind == TokenKind::Eof
                    || ts.peek().kind == TokenKind::Slash
                {
                    break;
                }
                let tok = ts.peek().clone();
                match tok.ident() {
                    Some(name) => {
                        vars.push(name.to_string());
                        ts.next();
                    }
                    None if tok.kind == TokenKind::Star => {
                        return Err(SasError::parse(
                            format!("An interaction effect in the LSMEANS statement {CANNOT}."),
                            tok.span,
                        ));
                    }
                    None => {
                        return Err(SasError::parse(
                            "expected an effect name in the LSMEANS statement",
                            tok.span,
                        ));
                    }
                }
            }
            if vars.is_empty() {
                return Err(SasError::parse(
                    "expected an effect name in the LSMEANS statement",
                    ts.peek().span,
                ));
            }
            // Options after `/`
            if ts.peek().kind == TokenKind::Slash {
                parse_lsmeans_options(ts)?;
            }
            ts.expect_semi()?;
            lsmeans_stmts.push(vars);
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
            lincombs.push((span, stmt, effect.clone(), coefficients.clone()));
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
            means_stmts.push(ts.parse_name_list()?);
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
    // J02-P12 — an ESTIMATE the executed path cannot compute honestly (a
    // one-way function that is not estimable) used to be printed as
    // Σ cᵢ ȳᵢ : rejected too.
    if let Some(model) = &model {
        for (span, stmt, effect, coefficients) in &lincombs {
            if let Some(msg) = lincomb_unsupported(stmt, effect, model, &class_vars) {
                return Err(SasError::parse(msg, *span));
            }
            if *stmt == "ESTIMATE"
                && let Some(msg) = estimate_nonestimable(effect, coefficients, model, &class_vars)
            {
                return Err(SasError::parse(msg, *span));
            }
        }
    }

    Ok(GlmAst {
        data_options: GlmDataOptions { input },
        class_vars,
        model,
        lsmeans_stmts,
        estimates,
        contrasts,
        means_stmts,
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
