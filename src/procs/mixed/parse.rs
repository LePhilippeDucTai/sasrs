use super::*;

use crate::token::Span;

// ───────────────────────── Contract diagnostics (J02-P2) ─────────────────────────
//
// SAS/STAT 9.4 User's Guide, The MIXED Procedure, Syntax: every PROC, MODEL,
// RANDOM and REPEATED option that sasrs does not implement used to be skipped
// token by token (audit d0b4d90). CONTRIBUTING §5: an option that can change
// a result is an ERROR (step rejected at parse time), a display-only option a
// WARNING.

/// Suffix naming the roadmap-avancee unit that will lift a provisional ERROR.
fn planned(unit: Option<&str>) -> String {
    match unit {
        Some(u) => format!(" (planned: roadmap-avancee {u})"),
        None => String::new(),
    }
}

/// ERROR for a valid but unimplemented option of `stmt` (`PROC` for the PROC
/// MIXED statement itself).
fn unsupported_option(stmt: &str, opt: &str, unit: Option<&str>, span: Span) -> SasError {
    let what = if stmt == "PROC" {
        format!("The {opt} option")
    } else {
        format!("The {opt} option of the {stmt} statement")
    };
    SasError::parse(
        format!(
            "{what} is not supported in PROC MIXED; it can affect results and cannot be \
             ignored{}.",
            planned(unit)
        ),
        span,
    )
}

/// ERROR for a construction sasrs cannot represent faithfully.
fn unsupported_construct(msg: &str, unit: Option<&str>, span: Span) -> SasError {
    SasError::parse(
        format!(
            "{msg} is not supported in PROC MIXED; it can affect results and cannot be \
             ignored{}.",
            planned(unit)
        ),
        span,
    )
}

/// WARNING text for a display-only option that is not implemented.
fn ignored_display_option(stmt: &str, opt: &str) -> String {
    let what = if stmt == "PROC" {
        format!("The {opt} option")
    } else {
        format!("The {opt} option of the {stmt} statement")
    };
    format!("{what} is ignored in PROC MIXED; display customization is not supported.")
}

/// Upper-case name of the current option token (`?` for a non-identifier).
fn option_name(ts: &StatementStream) -> String {
    ts.peek().ident().unwrap_or("?").to_ascii_uppercase()
}

/// Consume `= value[, value …]` after a display-only option, if present.
fn skip_display_value(ts: &mut StatementStream) {
    if ts.peek().kind != TokenKind::Eq {
        return;
    }
    ts.next();
    loop {
        if !matches!(ts.peek().kind, TokenKind::Num(_)) && ts.peek().ident().is_none() {
            break;
        }
        ts.next();
        if ts.peek().kind != TokenKind::Comma {
            break;
        }
        ts.next();
    }
}

/// Valid PROC MIXED statement options (SAS/STAT 9.4, PROC MIXED statement)
/// that are not implemented and can change a result.
const UNSUPPORTED_PROC_OPTIONS: &[&str] = &[
    "absolute",
    "alpha",
    "asycorr",
    "convf",
    "convg",
    "convh",
    "dfbw",
    "empirical",
    "ic",
    "info",
    "itdetails",
    "lognote",
    "maxfunc",
    "maxiter",
    "mmeq",
    "mmeqsol",
    "namelen",
    "noclprint",
    "noinfo",
    "noitprint",
    "noprofile",
    "ord",
    "order",
    "plots",
    "ratio",
    "scoring",
    "sigiter",
    "update",
];

/// Valid PROC MIXED statements (SAS/STAT 9.4, The MIXED Procedure, Syntax)
/// that are not implemented, beyond the shared list of
/// `common::unhandled_proc_statement` (BY, WEIGHT, ID, ESTIMATE, CONTRAST,
/// LSMEANS…). All can change results or create output data.
const UNSUPPORTED_STATEMENTS: &[&str] =
    &["code", "lsmestimate", "parms", "prior", "slice", "store"];

/// Display-only RANDOM options (G/V matrices, EBLUP table).
const RANDOM_DISPLAY_OPTIONS: &[&str] = &[
    "g", "gc", "gci", "gcorr", "gi", "v", "vc", "vci", "vcorr", "vi", "solution", "s", "cl",
    "alpha",
];

/// Display-only REPEATED options (R matrix printing).
const REPEATED_DISPLAY_OPTIONS: &[&str] = &["r", "rc", "rci", "rcorr", "ri"];

// ───────────────────────── Parser helpers ─────────────────────────

/// Parse a TYPE=... value, including `ar(1)`.
///
/// J02-P5 — unknown TYPE= values are an ERROR (SAS/STAT 9.4, The MIXED
/// Procedure, REPEATED/RANDOM `TYPE=` covariance structures) instead of the
/// former silent fallback to VC. J02-P2 — a parenthesized argument other than
/// AR(1) (e.g. the banded `UN(1)`) used to be swallowed, fitting the full
/// structure: ERROR.
pub(super) fn parse_cov_type(ts: &mut StatementStream) -> Result<CovType> {
    let span = ts.peek().span;
    let v = ts.peek().ident().map(|s| s.to_ascii_lowercase());
    let t = match v.as_deref() {
        Some("vc") => CovType::Vc,
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
        None => {
            return Err(SasError::parse(
                "expected a covariance structure after TYPE= in PROC MIXED",
                span,
            ));
        }
    };
    ts.next();
    if ts.peek().kind == TokenKind::LParen {
        let is_ar1 = t == CovType::Ar1
            && matches!(ts.peek_nth(1).kind, TokenKind::Num(n) if n == 1.0)
            && ts.peek_nth(2).kind == TokenKind::RParen;
        if !is_ar1 {
            let mut arg = String::new();
            let mut n = 1;
            while !matches!(
                ts.peek_nth(n).kind,
                TokenKind::RParen | TokenKind::Semi | TokenKind::Eof
            ) {
                match &ts.peek_nth(n).kind {
                    TokenKind::Num(x) => arg.push_str(&x.to_string()),
                    _ => arg.push_str(ts.peek_nth(n).ident().unwrap_or("?")),
                }
                n += 1;
            }
            return Err(unsupported_construct(
                &format!(
                    "TYPE={}({}) (parameterized covariance structure)",
                    v.unwrap_or_default().to_uppercase(),
                    arg.to_uppercase()
                ),
                None,
                span,
            ));
        }
        ts.next();
        ts.next();
        ts.next();
    }
    Ok(t)
}

/// `SUBJECT=effect` (alias `SUB=`, SAS/STAT 9.4 RANDOM/REPEATED statements):
/// only a single variable is implemented. J02-P2 — `id(grp)` and `a*b` used
/// to keep the first identifier only: ERROR until J06-P2.
fn parse_subject(ts: &mut StatementStream) -> Result<Option<String>> {
    common::consume_option_eq(ts, "SUBJECT")?;
    let subject = ts.peek().ident().map(str::to_string);
    ts.next();
    if matches!(ts.peek().kind, TokenKind::LParen | TokenKind::Star) {
        return Err(unsupported_construct(
            "A nested or crossed SUBJECT= effect (id(group), a*b)",
            Some("J06-P2"),
            ts.peek().span,
        ));
    }
    Ok(subject)
}

/// Options after `/` of a RANDOM or REPEATED statement.
fn parse_cov_statement_options(
    ts: &mut StatementStream,
    stmt: &str,
    display: &[&str],
) -> Result<(Option<String>, CovType)> {
    let mut subject: Option<String> = None;
    let mut cov_type = CovType::Vc;
    while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
        let tk = ts.peek();
        let kw = tk
            .ident()
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        if matches!(kw.as_str(), "subject" | "subj" | "sub") {
            subject = parse_subject(ts)?;
        } else if kw == "type" {
            common::consume_option_eq(ts, "TYPE")?;
            cov_type = parse_cov_type(ts)?;
        } else if display.contains(&kw.as_str()) {
            ts.warn_ignored_display(ignored_display_option(stmt, &option_name(ts)));
            ts.next();
            skip_display_value(ts);
        } else {
            let span = tk.span;
            let name = option_name(ts);
            let name = if ts.peek_nth(1).kind == TokenKind::Eq {
                format!("{name}=")
            } else {
                name
            };
            return Err(unsupported_option(stmt, &name, None, span));
        }
    }
    Ok((subject, cov_type))
}

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC MIXED. Called AFTER `proc mixed` has been consumed.
///
/// J02-P2 — end of the silent fallbacks (SAS/STAT 9.4 User's Guide, The MIXED
/// Procedure): unimplemented PROC/MODEL/RANDOM/REPEATED options, several
/// RANDOM statements, RANDOM with REPEATED, nested SUBJECT=, NOINT with a
/// CLASS effect and NOBOUND outside the closed-form path are ERRORs;
/// display-only options are WARNINGs; valid but unimplemented statements use
/// the contract « not supported … cannot be ignored » message.
pub fn parse(ts: &mut StatementStream) -> Result<MixedAst> {
    let mut data: Option<DatasetRef> = None;
    let mut method = Method::Reml;
    let mut nobound: Option<Span> = None;

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
        let kw = tk
            .ident()
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        if kw == "data" {
            data = Some(common::parse_dataset_opt(ts, "DATA")?);
        } else if kw == "method" {
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
        } else if kw == "nobound" {
            nobound = Some(tk.span);
            ts.next();
        } else if matches!(kw.as_str(), "covtest" | "asycov") {
            // J02-P2 — used to emit a NOTE and be ignored.
            return Err(unsupported_option(
                "PROC",
                &kw.to_ascii_uppercase(),
                Some("J06-P3"),
                tk.span,
            ));
        } else if kw == "cl" {
            // Confidence limits of the covariance parameters: display only.
            ts.warn_ignored_display(ignored_display_option("PROC", "CL"));
            ts.next();
            skip_display_value(ts);
        } else if UNSUPPORTED_PROC_OPTIONS.contains(&kw.as_str()) {
            let span = tk.span;
            let name = if ts.peek_nth(1).kind == TokenKind::Eq {
                format!("{}=", option_name(ts))
            } else {
                option_name(ts)
            };
            return Err(unsupported_option("PROC", &name, None, span));
        } else {
            return Err(common::unknown_option_error(ts, "MIXED"));
        }
    }

    let mut class_vars: Vec<String> = Vec::new();
    let mut model: Option<ModelSpec> = None;
    let mut noint_span: Option<Span> = None;
    let mut random: Option<RandomSpec> = None;
    let mut repeated: Option<RepeatedSpec> = None;
    let mut random_span: Option<Span> = None;

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
            let (m, noint) = parse_model(ts)?;
            model = Some(m);
            noint_span = noint;
            Ok(true)
        } else if kw == "random" {
            let span = ts.peek().span;
            if random.is_some() {
                // J02-P2 — the last RANDOM statement used to win silently.
                return Err(unsupported_construct(
                    "More than one RANDOM statement",
                    Some("J06-P2"),
                    span,
                ));
            }
            ts.next();
            random = Some(parse_random(ts)?);
            random_span = Some(span);
            Ok(true)
        } else if kw == "repeated" {
            if repeated.is_some() {
                return Err(SasError::parse(
                    "Only one REPEATED statement is allowed in PROC MIXED.",
                    ts.peek().span,
                ));
            }
            ts.next();
            repeated = Some(parse_repeated(ts)?);
            Ok(true)
        } else if UNSUPPORTED_STATEMENTS.contains(&kw) {
            Err(common::unsupported_statement("MIXED", kw))
        } else {
            Ok(false)
        }
    })?;

    // J02-P2 — RANDOM with REPEATED: the REPEATED structure was fitted and
    // the RANDOM effects dropped silently.
    if let (Some(span), Some(_)) = (random_span, &repeated) {
        return Err(unsupported_construct(
            "Combining a RANDOM and a REPEATED statement",
            Some("J06-P2"),
            span,
        ));
    }

    // J02-P2 — NOINT with a CLASS effect: the reference coding still dropped
    // the last level, so one fixed-effect column was missing.
    if let (Some(span), Some(m)) = (noint_span, &model)
        && m.fixed
            .iter()
            .any(|e| class_vars.iter().any(|c| c.eq_ignore_ascii_case(e)))
    {
        return Err(unsupported_construct(
            "NOINT with a CLASS fixed effect",
            Some("J06-P2"),
            span,
        ));
    }

    let ast = MixedAst {
        data,
        method,
        nobound: nobound.is_some(),
        class_vars,
        model,
        random,
        repeated,
    };

    // J02-P2 — NOBOUND is honored by the closed-form legacy path only; the
    // general path kept every variance bounded without any diagnostic.
    if let Some(span) = nobound
        && !is_legacy_case(&ast)
    {
        return Err(unsupported_construct(
            "NOBOUND outside the single random-intercept, intercept-only model",
            Some("J06-P2"),
            span,
        ));
    }

    Ok(ast)
}

/// Parse the MODEL statement body (after `model`): `response = <fixed> / opts;`.
/// Returns the spec and the span of a NOINT option, if any.
pub(super) fn parse_model(ts: &mut StatementStream) -> Result<(ModelSpec, Option<Span>)> {
    let response = common::parse_model_response(ts, "expected response variable in MODEL")?;
    common::expect_model_eq(ts, "expected '=' in MODEL statement")?;

    // Read fixed effects until `/` or `;`. J02-P5 — `a*b` / `a(b)` are NOT
    // silently flattened anymore (SAS/STAT 9.4 effect syntax): ERROR.
    let fixed = common::parse_effect_list_strict(ts, "MIXED")?;

    let mut solution = false;
    let mut noint: Option<Span> = None;
    let mut ddfm: Option<String> = None;

    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
            let tk = ts.peek();
            if tk.is_kw("solution") || tk.is_kw("s") {
                solution = true;
                ts.next();
            } else if tk.is_kw("noint") {
                noint = Some(tk.span);
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
                // J02-P2 — OUTP=, OUTPM=, NOFIT and every other MODEL option
                // used to be skipped token by token.
                let span = tk.span;
                let name = if ts.peek_nth(1).kind == TokenKind::Eq {
                    format!("{}=", option_name(ts))
                } else {
                    option_name(ts)
                };
                return Err(unsupported_option("MODEL", &name, None, span));
            }
        }
    }
    ts.expect_semi()?;

    Ok((
        ModelSpec {
            response,
            fixed,
            solution,
            noint: noint.is_some(),
            ddfm,
        },
        noint,
    ))
}

/// Parse the RANDOM statement body (after `random`).
pub(super) fn parse_random(ts: &mut StatementStream) -> Result<RandomSpec> {
    let effects = common::parse_effect_list_strict(ts, "MIXED")?;

    let mut subject: Option<String> = None;
    let mut cov_type = CovType::Vc;

    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        (subject, cov_type) = parse_cov_statement_options(ts, "RANDOM", RANDOM_DISPLAY_OPTIONS)?;
    }
    ts.expect_semi()?;

    Ok(RandomSpec {
        effects,
        subject,
        cov_type,
    })
}

/// Parse the REPEATED statement body (after `repeated`).
///
/// J02-P2 — the repeated effect used to be skipped (R indexed by order of
/// appearance in every case); it is now kept so the execution can check that
/// order of appearance and effect levels agree.
pub(super) fn parse_repeated(ts: &mut StatementStream) -> Result<RepeatedSpec> {
    let mut effect: Option<String> = None;
    if let Some(name) = ts.peek().ident().map(str::to_string) {
        effect = Some(name);
        ts.next();
    }
    if !matches!(
        ts.peek().kind,
        TokenKind::Semi | TokenKind::Slash | TokenKind::Eof
    ) {
        return Err(unsupported_construct(
            "A REPEATED effect other than a single variable",
            Some("J06-P2"),
            ts.peek().span,
        ));
    }

    let mut subject: Option<String> = None;
    let mut cov_type = CovType::Vc;
    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        (subject, cov_type) =
            parse_cov_statement_options(ts, "REPEATED", REPEATED_DISPLAY_OPTIONS)?;
    }
    ts.expect_semi()?;

    Ok(RepeatedSpec {
        effect,
        subject,
        cov_type,
    })
}
