use super::*;

use crate::token::Span;

// ───────────────────────── Contract diagnostics (J02-P4) ─────────────────────────
//
// SAS/STAT 9.4 User's Guide, The DISCRIM Procedure, PROC DISCRIM Statement:
// every option sasrs does not implement used to be skipped token by token,
// data set options of DATA= included (audit d0b4d90). CONTRIBUTING §5: an
// option that can change a result or a data set is an ERROR (step rejected
// at parse time), a display-only option a WARNING; an unknown option is the
// shared « Unexpected option » ERROR.

/// Suffix naming the roadmap-avancee unit that will lift a provisional ERROR.
fn planned(unit: Option<&str>) -> String {
    match unit {
        Some(u) => format!(" (planned: roadmap-avancee {u})"),
        None => String::new(),
    }
}

/// ERROR for a valid but unimplemented PROC DISCRIM option.
fn unsupported_option(opt: &str, unit: Option<&str>, span: Span) -> SasError {
    SasError::parse(
        format!(
            "The {opt} option is not supported in PROC DISCRIM; it can affect results and \
             cannot be ignored{}.",
            planned(unit)
        ),
        span,
    )
}

/// WARNING text for a display-only option that is not implemented.
fn ignored_display_option(opt: &str) -> String {
    format!("The {opt} option is ignored in PROC DISCRIM; display customization is not supported.")
}

/// Upper-case name of the current option token, with `=` when a value follows.
fn option_name(ts: &StatementStream) -> String {
    let name = ts.peek().ident().unwrap_or("?").to_ascii_uppercase();
    if ts.peek_nth(1).kind == TokenKind::Eq {
        format!("{name}=")
    } else {
        name
    }
}

/// Valid PROC DISCRIM options that roadmap-avancee J09-P2 implements
/// (cross validation, OUTSTAT=, test data set).
const J09_P2_OPTIONS: &[&str] = &[
    "crosslist",
    "crossvalidate",
    "outstat",
    "testdata",
    "testout",
];

/// Other valid PROC DISCRIM options that can change a result or create a
/// data set (canonical analysis, nonparametric methods, classification
/// criteria, other output data sets).
const UNSUPPORTED_OPTIONS: &[&str] = &[
    "can",
    "canonical",
    "canprefix",
    "crosslisterr",
    "k",
    "kernel",
    "kprop",
    "metric",
    "ncan",
    "outcross",
    "outd",
    "r",
    "scores",
    "singular",
    "slpool",
    "testlist",
    "testlisterr",
    "testoutd",
    "threshold",
];

/// Display-only PROC DISCRIM options (« Control Displayed Output », LISTERR
/// and NOCLASSIFY, which only withhold or add printed results). PCOV and
/// WCOV are not listed: the pooled and within-class covariance matrices they
/// request are always printed (honored); DISTANCE has its own WARNING.
const DISPLAY_OPTIONS: &[&str] = &[
    "all",
    "anova",
    "bcorr",
    "bcov",
    "bsscp",
    "formula",
    "listerr",
    "manova",
    "noclassify",
    "noprint",
    "pcorr",
    "posterr",
    "psscp",
    "short",
    "simple",
    "stdmean",
    "tcorr",
    "tcov",
    "tsscp",
    "wcorr",
    "wsscp",
];

/// Valid PROC DISCRIM statements (SAS/STAT 9.4, The DISCRIM Procedure,
/// Syntax) that are not implemented, beyond the shared list of
/// `common::unhandled_proc_statement` (BY, FREQ, WEIGHT). They used to be
/// 180-322 ERRORs.
const UNSUPPORTED_STATEMENTS: &[&str] = &["testclass", "testfreq", "testid"];

/// `DATA=`/`OUT=` followed by `(`: the data set options used to be skipped
/// token by token with the unknown PROC options.
fn reject_dataset_options(ts: &StatementStream, opt: &str) -> Result<()> {
    if ts.peek().kind == TokenKind::LParen {
        return Err(SasError::parse(
            format!(
                "Data set options on {opt}= are not supported in PROC DISCRIM; they can \
                 affect results and cannot be ignored."
            ),
            ts.peek().span,
        ));
    }
    Ok(())
}

/// `CLASS variable;` / `ID variable;` — one variable each (SAS 9.4 syntax).
/// J02-P4 — the variables after the first used to be dropped silently.
fn parse_single_variable(ts: &mut StatementStream, stmt: &str) -> Result<String> {
    let span = ts.peek().span;
    let mut names = ts.parse_name_list()?;
    if names.len() > 1 {
        return Err(SasError::parse(
            format!(
                "The {stmt} statement of PROC DISCRIM takes a single variable; found {}.",
                names.join(" ").to_uppercase()
            ),
            span,
        ));
    }
    ts.expect_semi()?;
    Ok(names.swap_remove(0))
}

/// `PRIORS EQUAL | PROPORTIONAL | PROP | level=p … ;` (SAS 9.4 PRIORS
/// statement). J02-P4 — explicit probabilities (`'A'=.3 B=.7 …`) and any
/// other form used to be replaced by EQUAL in silence.
fn parse_priors(ts: &mut StatementStream) -> Result<Priors> {
    let span = ts.peek().span;
    if ts.peek_nth(1).kind == TokenKind::Semi {
        let keyword = ts.peek().ident().map(str::to_ascii_lowercase);
        let priors = match keyword.as_deref() {
            Some("equal") => Some(Priors::Equal),
            Some("proportional") | Some("prop") => Some(Priors::Proportional),
            _ => None,
        };
        if let Some(priors) = priors {
            ts.next();
            ts.next();
            return Ok(priors);
        }
    }
    let mut n = 0;
    let mut explicit = false;
    while !matches!(ts.peek_nth(n).kind, TokenKind::Semi | TokenKind::Eof) {
        explicit |= ts.peek_nth(n).kind == TokenKind::Eq;
        n += 1;
    }
    if explicit {
        return Err(SasError::parse(
            "PRIORS with explicit probabilities is not supported in PROC DISCRIM; it can \
             affect results and cannot be ignored (planned: roadmap-avancee J09-P2).",
            span,
        ));
    }
    Err(SasError::parse(
        "The PRIORS statement of PROC DISCRIM expects EQUAL, PROPORTIONAL or \
         level=probability pairs.",
        span,
    ))
}

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC DISCRIM. Called AFTER `proc discrim` has been consumed.
///
/// J02-P4 — end of the silent fallbacks: PROC options go through the shared
/// option loop (implemented options honored, LIST honored, valid but
/// unimplemented options ERROR or display WARNING, others « Unexpected
/// option »); explicit PRIORS, several CLASS or ID variables and the TEST*
/// statements are ERRORs.
pub fn parse(ts: &mut StatementStream) -> Result<DiscrimAst> {
    let mut data: Option<DatasetRef> = None;
    let mut out: Option<DatasetRef> = None;
    let mut method: Option<String> = None;
    let mut pool = Pool::Yes;
    let mut list = false;

    // PROC DISCRIM statement options, until `;`.
    common::parse_proc_options(ts, "DISCRIM", |ts, kw| {
        let span = ts.peek().span;
        match kw {
            "data" => {
                data = Some(common::parse_dataset_opt(ts, "DATA")?);
                reject_dataset_options(ts, "DATA")?;
            }
            "out" => {
                out = Some(common::parse_out_opt(ts)?);
                reject_dataset_options(ts, "OUT")?;
            }
            "method" => {
                // J02-P5 — plus de repli silencieux vers LDA : seul METHOD=NORMAL
                // est rendu (SAS/STAT 9.4, The DISCRIM Procedure).
                common::consume_option_eq(ts, "METHOD")?;
                let span = ts.peek().span;
                let v = ts.peek().ident().map(|s| s.to_ascii_uppercase());
                match v.as_deref() {
                    Some("NORMAL") => method = v,
                    Some(other) => {
                        return Err(SasError::parse(
                            format!(
                                "METHOD={} is not supported in PROC DISCRIM; \
                                 only METHOD=NORMAL is implemented.",
                                other
                            ),
                            span,
                        ));
                    }
                    None => {
                        return Err(SasError::parse("expected a value after METHOD=", span));
                    }
                }
                ts.next();
            }
            "pool" => {
                // J02-P5 — POOL=NO (QDA) et POOL=TEST ne retombent plus sur la
                // covariance pooled : ERROR explicite ; POOL= inconnu : ERROR.
                // J02-P4 — POOL= sans valeur (ou suivi d'un nombre) valait
                // POOL=YES en silence.
                common::consume_option_eq(ts, "POOL")?;
                let span = ts.peek().span;
                let v = ts.peek().ident().map(|s| s.to_ascii_lowercase());
                pool = match v.as_deref() {
                    Some("yes") => Pool::Yes,
                    Some("no") => {
                        return Err(SasError::parse(
                            "POOL=NO (quadratic discriminant analysis) is not supported \
                             in PROC DISCRIM; only POOL=YES is implemented.",
                            span,
                        ));
                    }
                    Some("test") => {
                        return Err(SasError::parse(
                            "POOL=TEST is not supported in PROC DISCRIM; \
                             only POOL=YES is implemented.",
                            span,
                        ));
                    }
                    Some(other) => {
                        return Err(SasError::parse(
                            format!(
                                "Unknown POOL= value '{}' in PROC DISCRIM; \
                                 use POOL=YES.",
                                other.to_uppercase()
                            ),
                            span,
                        ));
                    }
                    None => {
                        return Err(SasError::parse(
                            "expected YES, NO or TEST after POOL=",
                            span,
                        ));
                    }
                };
                ts.next();
            }
            "list" => {
                list = true;
                ts.next();
            }
            // Pooled / within-class covariance matrices: always printed.
            "pcov" | "wcov" => {
                ts.next();
            }
            "distance" => {
                // The squared distances between class means are always
                // printed; the F statistics and probabilities are not.
                ts.warn_ignored_display(
                    "The F statistics and probabilities of the DISTANCE option are not \
                     displayed in PROC DISCRIM; display customization is not supported."
                        .to_string(),
                );
                ts.next();
            }
            _ if J09_P2_OPTIONS.contains(&kw) => {
                return Err(unsupported_option(&option_name(ts), Some("J09-P2"), span));
            }
            _ if UNSUPPORTED_OPTIONS.contains(&kw) => {
                return Err(unsupported_option(&option_name(ts), None, span));
            }
            _ if DISPLAY_OPTIONS.contains(&kw) => {
                ts.warn_ignored_display(ignored_display_option(&kw.to_ascii_uppercase()));
                ts.next();
            }
            _ => return Ok(false),
        }
        Ok(true)
    })?;

    // Sub-statements until run;/quit;
    let mut class_var: Option<String> = None;
    let mut var_vars: Vec<String> = Vec::new();
    let mut id_var: Option<String> = None;
    let mut priors = Priors::Equal;

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, "DISCRIM", |ts, kw| {
        Ok(match kw {
            "class" => {
                ts.next();
                class_var = Some(parse_single_variable(ts, "CLASS")?);
                true
            }
            "var" => {
                // J02-P4 — the former loop skipped non-name tokens: `x1-x3`
                // silently became `x1 x3`.
                ts.next();
                var_vars = ts.parse_name_list()?;
                ts.expect_semi()?;
                true
            }
            "id" => {
                ts.next();
                id_var = Some(parse_single_variable(ts, "ID")?);
                true
            }
            "priors" => {
                ts.next();
                priors = parse_priors(ts)?;
                true
            }
            _ if UNSUPPORTED_STATEMENTS.contains(&kw) => {
                return Err(common::unsupported_statement("DISCRIM", kw));
            }
            _ => false,
        })
    })?;

    Ok(DiscrimAst {
        data,
        out,
        method,
        pool,
        priors,
        list,
        class_var,
        var_vars,
        id_var,
    })
}

/// Guards (CLASS/VAR required). Returns the CLASS variable name.
pub(super) fn check_options(ast: &DiscrimAst) -> Result<&String> {
    let class_name = ast
        .class_var
        .as_ref()
        .ok_or_else(|| SasError::runtime("CLASS statement required in PROC DISCRIM"))?;

    if ast.var_vars.is_empty() {
        return Err(SasError::runtime(
            "VAR statement with at least one numeric variable required in PROC DISCRIM",
        ));
    }
    Ok(class_name)
}
