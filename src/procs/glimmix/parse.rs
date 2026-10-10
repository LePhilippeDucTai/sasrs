use super::*;

use crate::token::Span;

// ───────────────────────── Contract diagnostics (J02-P3) ─────────────────────────
//
// SAS/STAT 9.4 User's Guide, The GLIMMIX Procedure, Syntax
// (https://support.sas.com/documentation/cdl/en/statug/68162/HTML/default/statug_glimmix_syntax.htm):
// every PROC, MODEL, RANDOM and CLASS option that sasrs does not implement
// used to be skipped token by token, WEIGHT was noted then ignored and only
// the last RANDOM statement was kept (audit d0b4d90). CONTRIBUTING §5: an
// option that can change a result is an ERROR (step rejected at parse time),
// a display-only option a WARNING. Same wording as the PROC MIXED contract
// (J02-P2).

/// Suffix naming the roadmap-avancee unit that will lift a provisional ERROR.
fn planned(unit: Option<&str>) -> String {
    match unit {
        Some(u) => format!(" (planned: roadmap-avancee {u})"),
        None => String::new(),
    }
}

/// « The OPT option » / « The OPT option of the STMT statement ».
fn option_subject(stmt: &str, opt: &str) -> String {
    if stmt == "PROC" {
        format!("The {opt} option")
    } else {
        format!("The {opt} option of the {stmt} statement")
    }
}

/// ERROR for a valid but unimplemented option of `stmt` (`PROC` for the PROC
/// GLIMMIX statement itself).
fn unsupported_option(stmt: &str, opt: &str, unit: Option<&str>, span: Span) -> SasError {
    SasError::parse(
        format!(
            "{} is not supported in PROC GLIMMIX; it can affect results and cannot be \
             ignored{}.",
            option_subject(stmt, opt),
            planned(unit)
        ),
        span,
    )
}

/// ERROR for a construction sasrs cannot represent faithfully.
fn unsupported_construct(msg: &str, unit: Option<&str>, span: Span) -> SasError {
    SasError::parse(
        format!(
            "{msg} is not supported in PROC GLIMMIX; it can affect results and cannot be \
             ignored{}.",
            planned(unit)
        ),
        span,
    )
}

/// WARNING text for a display-only option that is not implemented.
fn ignored_display_option(stmt: &str, opt: &str) -> String {
    format!(
        "{} is ignored in PROC GLIMMIX; display customization is not supported.",
        option_subject(stmt, opt)
    )
}

/// Upper-case name of the current option token (`?` for a non-identifier).
fn option_name(ts: &StatementStream) -> String {
    ts.peek().ident().unwrap_or("?").to_ascii_uppercase()
}

/// `NAME=` when the current option is followed by `=`, `NAME` otherwise.
fn option_label(ts: &StatementStream) -> String {
    if ts.peek_nth(1).kind == TokenKind::Eq {
        format!("{}=", option_name(ts))
    } else {
        option_name(ts)
    }
}

/// Consume the arguments of a display-only option: `(suboptions)`,
/// `= value[, value …]`, `= (list)` or `= name(suboptions)`.
fn skip_display_value(ts: &mut StatementStream) {
    ts.skip_balanced_parens();
    if ts.peek().kind != TokenKind::Eq {
        return;
    }
    ts.next();
    if ts.peek().kind == TokenKind::LParen {
        ts.skip_balanced_parens();
        return;
    }
    loop {
        if !matches!(ts.peek().kind, TokenKind::Num(_) | TokenKind::Str { .. })
            && ts.peek().ident().is_none()
        {
            break;
        }
        ts.next();
        ts.skip_balanced_parens();
        if ts.peek().kind != TokenKind::Comma {
            break;
        }
        ts.next();
    }
}

/// Queue the WARNING of the display-only option at the current token and
/// consume the option with its arguments.
fn warn_display_option(ts: &mut StatementStream, stmt: &str) {
    ts.warn_ignored_display(ignored_display_option(stmt, &option_name(ts)));
    ts.next();
    skip_display_value(ts);
}

/// PROC GLIMMIX statement options limited to the displayed output (SAS/STAT
/// 9.4, PROC GLIMMIX statement, « Displayed Output »).
const DISPLAY_PROC_OPTIONS: &[&str] = &[
    "asycorr",
    "asycov",
    "gradient",
    "h",
    "hess",
    "hessian",
    "itdetails",
    "list",
    "namelen",
    "nobsdetail",
    "noclprint",
    "oddsratio",
    "or",
    "plots",
];

/// Valid PROC GLIMMIX statement options that are not implemented and can
/// change a result or create a data set (basic, optimization, computational
/// options and singularity tolerances).
const UNSUPPORTED_PROC_OPTIONS: &[&str] = &[
    "abspconv",
    "chol",
    "cholesky",
    "empirical",
    "exphessian",
    "fdigits",
    "inititer",
    "maxlmmupdate",
    "maxopt",
    "nobound",
    "nofit",
    "noinitglm",
    "noprofile",
    "noreml",
    "order",
    "outdesign",
    "pconv",
    "profile",
    "scoremod",
    "scoring",
    "singchol",
    "singres",
    "singular",
    "subgrad",
    "subgradient",
];

/// MODEL statement options limited to the displayed output (SAS/STAT 9.4,
/// MODEL statement, « Statistical Output », plus ALPHA= and CHISQ which only
/// add confidence limits or test columns).
const DISPLAY_MODEL_OPTIONS: &[&str] = &[
    "alpha",
    "chisq",
    "cl",
    "corrb",
    "covb",
    "covbi",
    "e",
    "e1",
    "e2",
    "e3",
    "intercept",
    "oddsratio",
    "or",
    "stdcoef",
];

/// RANDOM statement options limited to the displayed output (SAS/STAT 9.4,
/// RANDOM statement, « Statistical Output », and the KNOTINFO display).
const DISPLAY_RANDOM_OPTIONS: &[&str] = &[
    "alpha", "cl", "g", "gc", "gci", "gcorr", "gi", "knotinfo", "s", "solution", "v", "vc", "vci",
    "vcorr", "vi",
];

/// Valid PROC GLIMMIX statements (SAS/STAT 9.4, The GLIMMIX Procedure,
/// Syntax) that are not implemented, beyond the shared list of
/// `common::unhandled_proc_statement` (BY, ID, OUTPUT, WHERE…). All can
/// change results or create output data.
const UNSUPPORTED_STATEMENTS: &[&str] = &[
    "code",
    "contrast",
    "covtest",
    "effect",
    "estimate",
    "lsmeans",
    "lsmestimate",
    "nloptions",
    "parms",
    "slice",
    "store",
];

/// R-side keyword of the RANDOM statement (SAS/STAT 9.4, RANDOM statement:
/// `_RESIDUAL_`, aliases `_RESID_`, `RESID`, `RESIDUAL`).
fn is_residual_keyword(effect: &str) -> bool {
    matches!(
        effect.to_ascii_lowercase().as_str(),
        "_residual_" | "_resid_" | "resid" | "residual"
    )
}

fn cov_type_keyword(t: CovType) -> &'static str {
    match t {
        CovType::Vc => "VC",
        CovType::Cs => "CS",
        CovType::Ar1 => "AR(1)",
        CovType::Un => "UN",
    }
}

fn dist_keyword(d: Distribution) -> &'static str {
    match d {
        Distribution::Normal => "NORMAL",
        Distribution::Poisson => "POISSON",
        Distribution::Binary => "BINARY",
        Distribution::Gamma => "GAMMA",
        Distribution::NegBinomial => "NEGBINOMIAL",
    }
}

fn link_keyword(l: LinkFunction) -> &'static str {
    match l {
        LinkFunction::Identity => "IDENTITY",
        LinkFunction::Log => "LOG",
        LinkFunction::Logit => "LOGIT",
        LinkFunction::Probit => "PROBIT",
        LinkFunction::Cloglog => "CLOGLOG",
    }
}

// ───────────────────────── Parser helpers ─────────────────────────

/// Parse a TYPE= value, including `ar(1)`.
///
/// J02-P5 — unknown TYPE= values are an ERROR (SAS/STAT 9.4, The GLIMMIX
/// Procedure, RANDOM/`TYPE=` covariance structures) instead of the former
/// silent fallback to VC. J02-P3 — a missing value also fell back to VC, and a
/// parenthesized argument other than AR(1) (e.g. the banded `UN(1)`) was
/// swallowed, fitting the full structure: ERROR.
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
                    "Unknown or unsupported TYPE= value '{}' in PROC GLIMMIX; \
                     supported: VC, CS, UN, AR(1).",
                    other.to_uppercase()
                ),
                span,
            ));
        }
        None => {
            return Err(SasError::parse(
                "expected a covariance structure after TYPE= in PROC GLIMMIX",
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

/// `SUBJECT=effect` (alias `SUB=`, SAS/STAT 9.4 RANDOM statement): only a
/// single variable is implemented. J02-P3 — `id(grp)` and `a*b` used to keep
/// the first identifier only (subjects merged across groups): ERROR.
fn parse_subject(ts: &mut StatementStream) -> Result<String> {
    common::consume_option_eq(ts, "SUBJECT")?;
    let span = ts.peek().span;
    let subject = ts
        .peek()
        .ident()
        .map(str::to_string)
        .ok_or_else(|| SasError::parse("expected a variable name after SUBJECT=", span))?;
    ts.next();
    if matches!(ts.peek().kind, TokenKind::LParen | TokenKind::Star) {
        return Err(unsupported_construct(
            "A nested or crossed SUBJECT= effect (id(group), a*b)",
            Some("J08-P4"),
            ts.peek().span,
        ));
    }
    Ok(subject)
}

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC GLIMMIX. Called AFTER `proc glimmix` has been consumed.
///
/// J02-P3 — end of the silent fallbacks (SAS/STAT 9.4 User's Guide, The
/// GLIMMIX Procedure): unimplemented PROC/MODEL/RANDOM/CLASS options, WEIGHT,
/// several RANDOM statements, a G-side TYPE=AR(1)|UN, NOINT with a CLASS
/// effect, the DIST/LINK pairs the random-effect solvers mishandle and FREQ
/// with a NORMAL random-effect REML fit are ERRORs; display-only options are
/// WARNINGs; valid but unimplemented statements use the contract « not
/// supported … cannot be ignored » message.
pub fn parse(ts: &mut StatementStream) -> Result<GlimmixAst> {
    let mut data: Option<DatasetRef> = None;
    let mut method = Method::Rspl;
    let mut ic_none: Option<String> = None;

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
        let span = tk.span;
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
                Some("rspl") => Method::Rspl,
                Some("laplace") => Method::Laplace,
                // QUAD parses but defers with an explicit execution error.
                Some("quad" | "quadrature") => Method::Quad,
                Some(other) => {
                    return Err(SasError::parse(
                        format!(
                            "Unknown or unsupported METHOD= value '{}' in PROC GLIMMIX; \
                             supported: RSPL, LAPLACE, QUAD.",
                            other.to_uppercase()
                        ),
                        span,
                    ));
                }
                // J02-P3 — `METHOD=` without a value fell back to RSPL.
                None => {
                    return Err(SasError::parse("expected a value after METHOD=", span));
                }
            };
            ts.next();
            if method == Method::Quad {
                // QUAD(QPOINTS=…): the method itself is rejected at execution.
                ts.skip_balanced_parens();
            }
        } else if matches!(kw.as_str(), "initglm" | "startglm") {
            // Honored: every GLMM fit of sasrs (PQL, LAPLACE, R-side RSPL)
            // starts from the estimates of the model without random effects.
            ts.next();
        } else if kw == "noitprint"
            || (kw == "plots"
                && ts.peek_nth(1).kind == TokenKind::Eq
                && ts.peek_nth(2).is_kw("none")
                && matches!(ts.peek_nth(3).kind, TokenKind::Semi | TokenKind::Ident(_)))
        {
            // Honored: no Iteration History table and no graphics are
            // produced by PROC GLIMMIX in sasrs.
            ts.next();
            skip_display_value(ts);
        } else if matches!(kw.as_str(), "ic" | "infocrit") {
            // IC=NONE only suppresses the information criteria (display,
            // checked once the model is known); IC=PQ|Q change their
            // definition.
            let name = option_name(ts);
            common::consume_option_eq(ts, &name)?;
            if ts.peek().is_kw("none") {
                ic_none = Some(format!("{name}=NONE"));
                ts.next();
            } else {
                return Err(unsupported_option("PROC", &format!("{name}="), None, span));
            }
        } else if DISPLAY_PROC_OPTIONS.contains(&kw.as_str()) {
            warn_display_option(ts, "PROC");
        } else if UNSUPPORTED_PROC_OPTIONS.contains(&kw.as_str()) {
            return Err(unsupported_option("PROC", &option_label(ts), None, span));
        } else {
            // J02-P3 — unknown options used to be skipped token by token.
            return Err(common::unknown_option_error(ts, "GLIMMIX"));
        }
    }

    let mut class_vars: Vec<String> = Vec::new();
    let mut model: Option<ModelSpec> = None;
    let mut noint_span: Option<Span> = None;
    let mut random: Option<RandomSpec> = None;
    let mut random_span: Option<Span> = None;
    let mut freq_var: Option<String> = None;
    let mut freq_span: Option<Span> = None;

    common::parse_proc_body(ts, "GLIMMIX", |ts, kw| {
        if kw == "class" {
            ts.next();
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    class_vars.push(name);
                    ts.next();
                } else if matches!(ts.peek().kind, TokenKind::LParen | TokenKind::Slash) {
                    // J02-P3 — `(REF= ORDER= DESC …)` and `/ options` used to
                    // be skipped (their words even became CLASS variables).
                    return Err(SasError::parse(
                        "CLASS statement options (REF=, ORDER=, DESCENDING, ...) are not \
                         supported in PROC GLIMMIX; the coding of the CLASS effects would \
                         silently differ from the request.",
                        ts.peek().span,
                    ));
                } else {
                    return Err(SasError::parse(
                        "expected a variable name in the CLASS statement",
                        ts.peek().span,
                    ));
                }
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
                // J02-P3 — the last RANDOM statement used to win silently.
                return Err(unsupported_construct(
                    "More than one RANDOM statement",
                    Some("J08-P4"),
                    span,
                ));
            }
            ts.next();
            random = Some(parse_random(ts)?);
            random_span = Some(span);
            Ok(true)
        } else if kw == "freq" {
            freq_span = Some(ts.peek().span);
            ts.next();
            if let Some(name) = ts.peek().ident().map(str::to_string) {
                freq_var = Some(name);
                ts.next();
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "weight" {
            // J02-P3 — WEIGHT used to emit a NOTE, then the fit ignored it.
            Err(SasError::parse(
                format!(
                    "The WEIGHT statement is not supported in PROC GLIMMIX; it can affect \
                     results and cannot be ignored{}.",
                    planned(Some("J08-P4"))
                ),
                ts.peek().span,
            ))
        } else if UNSUPPORTED_STATEMENTS.contains(&kw) {
            Err(common::unsupported_statement("GLIMMIX", kw))
        } else {
            Ok(false)
        }
    })?;

    // J02-P3 — NOINT with a CLASS effect: the reference coding still drops
    // the last level, so one fixed-effect column was missing.
    if let (Some(span), Some(m)) = (noint_span, &model)
        && m.fixed
            .iter()
            .any(|e| class_vars.iter().any(|c| c.eq_ignore_ascii_case(e)))
    {
        return Err(unsupported_construct(
            "NOINT with a CLASS fixed effect",
            Some("J08-P2"),
            span,
        ));
    }

    // J02-P3 — DIST/LINK pairs that the G-side random-effect solvers used to
    // mishandle without any diagnostic.
    if let (Some(span), Some(m), Some(r)) = (random_span, &model, &random)
        && !r.residual
    {
        // The NORMAL variance-components solver (fit_vc) fits the
        // identity-link model whatever LINK= says.
        if method == Method::Rspl
            && m.dist == Distribution::Normal
            && m.link != LinkFunction::Identity
        {
            return Err(unsupported_construct(
                &format!(
                    "DIST=NORMAL with LINK={} and a G-side RANDOM effect under METHOD=RSPL",
                    link_keyword(m.link)
                ),
                Some("J08-P2"),
                span,
            ));
        }
        // The Laplace log-density only knows NORMAL/IDENTITY, POISSON/LOG and
        // the binary links; any other pair fell into its Bernoulli branch.
        if method == Method::Laplace
            && matches!(m.dist, Distribution::Normal | Distribution::Poisson)
            && m.link != canonical_link(m.dist)
        {
            return Err(unsupported_construct(
                &format!(
                    "METHOD=LAPLACE with DIST={} and LINK={}",
                    dist_keyword(m.dist),
                    link_keyword(m.link)
                ),
                Some("J08-P3"),
                span,
            ));
        }
    }

    // J02-P3 — the NORMAL/IDENTITY random-effect REML solvers (variance
    // components, R-side structure) never received the frequencies: FREQ was
    // ignored while « Number of Observations Used » showed their sum.
    if let (Some(span), Some(m), Some(_)) = (freq_span, &model, &random)
        && method == Method::Rspl
        && m.dist == Distribution::Normal
        && m.link == LinkFunction::Identity
    {
        return Err(unsupported_construct(
            "A FREQ statement with DIST=NORMAL, LINK=IDENTITY and a RANDOM statement under \
             METHOD=RSPL",
            None,
            span,
        ));
    }

    // IC=NONE is the default display of the pseudo-likelihood and GLM-mode
    // fits (no information criteria); only the LAPLACE fit prints them.
    if let Some(name) = ic_none
        && method == Method::Laplace
        && random.is_some()
    {
        ts.warn_ignored_display(ignored_display_option("PROC", &name));
    }

    Ok(GlimmixAst {
        data,
        method,
        class_vars,
        model,
        random,
        freq_var,
    })
}

/// Parse the MODEL statement body (after `model`). Returns the spec and the
/// span of a NOINT option, if any.
pub(super) fn parse_model(ts: &mut StatementStream) -> Result<(ModelSpec, Option<Span>)> {
    let response = common::parse_model_response(ts, "expected response variable in MODEL")?;

    // Optional response options: (event='val' | descending). The checked
    // form of J02-P1 (EVENT=FIRST|LAST honored, ORDER=/REF= rejected) is not
    // used yet: switching to it leaves the legacy parser of
    // src/procs/common/model.rs without caller, outside the files of J02-P3
    // (see docs/support-contract.md).
    let (event, descending) = common::parse_response_options(ts);

    common::expect_model_eq(ts, "expected '=' in MODEL statement")?;

    // J02-P5 — `a*b` / `a(b)` are NOT silently flattened anymore:
    // explicit ERROR (SAS/STAT 9.4 effect syntax).
    let fixed = common::parse_effect_list_strict(ts, "GLIMMIX")?;

    let mut dist_opt: Option<Distribution> = None;
    let mut link_opt: Option<LinkFunction> = None;
    let mut solution = false;
    let mut noint: Option<Span> = None;

    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
            let span = ts.peek().span;
            let kw = ts
                .peek()
                .ident()
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default();
            match kw.as_str() {
                "dist" | "distribution" | "d" => {
                    common::consume_option_eq(ts, "DIST")?;
                    let span = ts.peek().span;
                    // J02-P3 — a missing value left the default NORMAL.
                    let name = ts.peek().ident().map(str::to_string).ok_or_else(|| {
                        SasError::parse("expected a distribution name after DIST=", span)
                    })?;
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
                "link" => {
                    common::consume_option_eq(ts, "LINK")?;
                    let span = ts.peek().span;
                    // J02-P3 — a missing value left the canonical link.
                    let name = ts.peek().ident().map(str::to_string).ok_or_else(|| {
                        SasError::parse("expected a link function after LINK=", span)
                    })?;
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
                "solution" | "s" => {
                    solution = true;
                    ts.next();
                }
                "noint" => {
                    noint = Some(span);
                    ts.next();
                }
                "ddfm" => {
                    // J02-P5 — DDFM was previously swallowed as an unknown
                    // option; only CONTAIN is implemented, anything else is an
                    // ERROR.
                    common::consume_option_eq(ts, "DDFM")?;
                    let span = ts.peek().span;
                    match ts.peek().ident().map(|s| s.to_ascii_lowercase()).as_deref() {
                        Some("contain") => {}
                        Some(other) => {
                            return Err(SasError::parse(
                                format!(
                                    "DDFM={} is not supported in PROC GLIMMIX; \
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
                }
                "htype" => {
                    // HTYPE=3 is the default and the only implemented test
                    // type; Type I/II tests are not computed.
                    common::consume_option_eq(ts, "HTYPE")?;
                    let mut types: Vec<String> = Vec::new();
                    while let TokenKind::Num(t) = ts.peek().kind {
                        types.push(format_best(t, 12));
                        ts.next();
                        if ts.peek().kind != TokenKind::Comma {
                            break;
                        }
                        ts.next();
                    }
                    if types.is_empty() || types.iter().any(|t| t != "3") {
                        return Err(unsupported_option(
                            "MODEL",
                            &format!("HTYPE={}", types.join(",")),
                            None,
                            span,
                        ));
                    }
                }
                k if DISPLAY_MODEL_OPTIONS.contains(&k) => warn_display_option(ts, "MODEL"),
                // J02-P3 — OFFSET=, OBSWEIGHT=, DDF=, NOCENTER and every other
                // MODEL option used to be skipped token by token.
                _ => return Err(unsupported_option("MODEL", &option_label(ts), None, span)),
            }
        }
    }
    ts.expect_semi()?;

    let dist = dist_opt.unwrap_or(Distribution::Normal);
    let link = link_opt.unwrap_or_else(|| canonical_link(dist));

    Ok((
        ModelSpec {
            response,
            event,
            descending,
            fixed,
            dist,
            link,
            solution,
            noint: noint.is_some(),
        },
        noint,
    ))
}

/// Parse the RANDOM statement body (after `random`).
///
/// J02-P3 — `RANDOM _RESIDUAL_ / SUBJECT= TYPE=AR(1)|UN` (SAS/STAT 9.4,
/// RANDOM statement) exposes the R-side structure that a G-side
/// `RANDOM INTERCEPT / TYPE=AR(1)|UN` used to be reinterpreted as, without
/// any random effect; that G-side request is now an ERROR. RANDOM options
/// other than SUBJECT=/TYPE= used to be skipped: display-only ones are
/// WARNINGs, the others ERRORs.
pub(super) fn parse_random(ts: &mut StatementStream) -> Result<RandomSpec> {
    let effects_span = ts.peek().span;
    let effects = common::parse_effect_list_strict(ts, "GLIMMIX")?;
    let residual = effects.iter().any(|e| is_residual_keyword(e));
    if residual && effects.len() > 1 {
        return Err(unsupported_construct(
            "RANDOM _RESIDUAL_ combined with other random effects",
            None,
            effects_span,
        ));
    }

    let mut subject: Option<String> = None;
    let mut cov_type = CovType::Vc;
    let mut type_span: Option<Span> = None;

    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
            let span = ts.peek().span;
            let kw = ts
                .peek()
                .ident()
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default();
            if matches!(kw.as_str(), "subject" | "subj" | "sub") {
                subject = Some(parse_subject(ts)?);
            } else if kw == "type" {
                type_span = Some(span);
                common::consume_option_eq(ts, "TYPE")?;
                cov_type = parse_cov_type(ts)?;
            } else if DISPLAY_RANDOM_OPTIONS.contains(&kw.as_str()) {
                warn_display_option(ts, "RANDOM");
            } else {
                // GROUP=, RESIDUAL, NOFULLZ, LDATA=, KNOTMETHOD=, WEIGHT=…
                return Err(unsupported_option("RANDOM", &option_label(ts), None, span));
            }
        }
    }
    ts.expect_semi()?;

    if residual {
        if !matches!(cov_type, CovType::Ar1 | CovType::Un) {
            return Err(unsupported_construct(
                &format!(
                    "RANDOM _RESIDUAL_ with TYPE={} (only TYPE=AR(1) and TYPE=UN are \
                     implemented on the R side)",
                    cov_type_keyword(cov_type)
                ),
                None,
                type_span.unwrap_or(effects_span),
            ));
        }
    } else if let (Some(span), CovType::Ar1 | CovType::Un) = (type_span, cov_type) {
        let ty = cov_type_keyword(cov_type);
        return Err(SasError::parse(
            format!(
                "TYPE={ty} for a G-side RANDOM effect is not supported in PROC GLIMMIX; it \
                 can affect results and cannot be ignored{}. An R-side structure is \
                 requested by RANDOM _RESIDUAL_ / SUBJECT= TYPE={ty}.",
                planned(Some("J08-P4"))
            ),
            span,
        ));
    }

    Ok(RandomSpec {
        effects,
        subject,
        cov_type,
        residual,
    })
}
