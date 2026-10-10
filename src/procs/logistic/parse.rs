use super::*;

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC LOGISTIC. Called AFTER `proc logistic` has been consumed.
///
/// J02-P5 — fin des replis silencieux (SAS/STAT 9.4 User's Guide, The
/// LOGISTIC Procedure) : l'option PROC `DESCENDING` est honorée, `ORDER=`
/// et toute option PROC inconnue sont des ERROR ; dans le MODEL, un `LINK=`
/// inconnu et toute option inconnue sont des ERROR ; le statement CLASS
/// analyse `(PARAM= REF=)` — `PARAM=REF` avec `REF=FIRST|LAST` est honoré,
/// tout autre PARAM= est une ERROR (plus de jetons pris pour des variables).
///
/// J02-P1 — `CLASS … / PARAM= REF=` (options globales), options de réponse
/// `DESC`/`EVENT=FIRST|LAST`, mots-clés OUTPUT non implémentés, OUTPUT sans
/// OUT= et instructions LOGISTIC valides non implémentées : voir
/// `docs/support-contract.md`, « Replis silencieux supprimés ».
pub fn parse(ts: &mut StatementStream) -> Result<LogisticAst> {
    let mut input: Option<DatasetRef> = None;
    let mut proc_descending = false;

    // PROC LOGISTIC statement options, until `;`
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
        } else if ts.peek().is_kw("descending") || ts.peek().is_kw("desc") {
            // PROC-level DESCENDING — honored (applies to every MODEL).
            proc_descending = true;
            ts.next();
        } else if ts.peek().is_kw("order") {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "ORDER= is not supported in PROC LOGISTIC; response level order \
                 would silently differ from the request.",
                span,
            ));
        } else {
            // Unknown PROC-level option — ERROR instead of a silent skip.
            let span = ts.peek().span;
            let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
            return Err(SasError::parse(
                format!("Unknown or unsupported option '{bad}' on the PROC LOGISTIC statement."),
                span,
            ));
        }
    }

    // Sub-statements until run;/quit;
    let mut class_vars: Vec<ClassVar> = Vec::new();
    let mut model: Option<LogisticModel> = None;
    let mut freq_var: Option<String> = None;
    let mut outputs: Vec<LogisticOutput> = Vec::new();

    common::parse_proc_body(ts, "LOGISTIC", |ts, kw| {
        if kw == "class" {
            ts.next(); // consume "class"
            parse_class(ts, &mut class_vars)?;
            Ok(true)
        } else if kw == "model" {
            ts.next(); // consume "model"
            // Parse response variable name
            let response = common::parse_model_response(ts, "expected response variable")?;

            // Response options: (DESCENDING|DESC EVENT='v'|FIRST|LAST) are
            // honored; ORDER=/REF= and unknown tokens are ERRORs (J02-P1).
            let common::ResponseOptions { event, descending } =
                common::parse_response_options_checked(ts, "LOGISTIC")?;

            // Expect '='
            common::expect_model_eq(ts, "expected '=' after response variable in MODEL")?;

            // Parse predictors until '/' or ';'. J02-P5 — `a*b` / `a(b)` are
            // NOT silently flattened anymore: explicit ERROR.
            let predictors = common::parse_effect_list_strict(ts, "LOGISTIC")?;

            let mut noprint = false;
            let mut link = Link::Logit;

            if ts.peek().kind == TokenKind::Slash {
                ts.next(); // consume '/'
                // Parse options until ';'
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    if ts.peek().is_kw("noprint") {
                        noprint = true;
                        ts.next();
                    } else if ts.peek().is_kw("link") {
                        ts.next(); // consume "link"
                        if ts.peek().kind == TokenKind::Eq {
                            ts.next(); // consume '='
                            let span = ts.peek().span;
                            if let Some(name) = ts.peek().ident().map(str::to_string) {
                                link = match name.to_lowercase().as_str() {
                                    "logit" => Link::Logit,
                                    "cloglog" | "ccll" => Link::Cloglog,
                                    "probit" | "normit" => Link::Probit,
                                    other => {
                                        return Err(SasError::parse(
                                            format!(
                                                "Unknown LINK= value '{}' on the MODEL statement.",
                                                other.to_uppercase()
                                            ),
                                            span,
                                        ));
                                    }
                                };
                                ts.next();
                            } else {
                                return Err(SasError::parse(
                                    "expected a link name after LINK=",
                                    span,
                                ));
                            }
                        } else {
                            return Err(SasError::parse("expected '=' after LINK", ts.peek().span));
                        }
                    } else {
                        // Unknown MODEL option — ERROR instead of a silent skip.
                        let span = ts.peek().span;
                        let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
                        return Err(SasError::parse(
                            format!(
                                "Unknown or unsupported MODEL option '{bad}' in PROC LOGISTIC."
                            ),
                            span,
                        ));
                    }
                }
            }
            ts.expect_semi()?;
            model = Some(LogisticModel {
                response,
                event,
                // PROC-level DESCENDING applies unless the MODEL response
                // options already requested a specific ordering/event.
                descending: descending || proc_descending,
                predictors,
                noprint,
                link,
            });
            Ok(true)
        } else if kw == "freq" {
            ts.next(); // consume "freq"
            if let Some(name) = ts.peek().ident().map(str::to_string) {
                freq_var = Some(name);
                ts.next();
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "output" {
            let out_span = ts.peek().span;
            ts.next(); // consume "output"
            let mut out: Option<DatasetRef> = None;
            let mut predicted: Option<String> = None;
            let mut xbeta: Option<String> = None;
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                if ts.peek().is_kw("out") {
                    out = Some(common::parse_out_opt(ts)?);
                } else if ts.peek().is_kw("predicted")
                    || ts.peek().is_kw("pred")
                    || ts.peek().is_kw("prob")
                    || ts.peek().is_kw("p")
                {
                    common::consume_option_eq(ts, "PREDICTED")?;
                    predicted = Some(common::expect_ident(ts, "after PREDICTED=")?);
                } else if ts.peek().is_kw("xbeta") {
                    common::consume_option_eq(ts, "XBETA")?;
                    xbeta = Some(common::expect_ident(ts, "after XBETA=")?);
                } else {
                    // J02-P1 — LOWER=, UPPER=, STDXBETA=, RESCHI=, RESDEV=,
                    // H=, PREDPROBS=, … were skipped token by token: the
                    // OUT= dataset silently lacked the requested columns.
                    let span = ts.peek().span;
                    let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
                    return Err(SasError::parse(
                        format!(
                            "The OUTPUT option '{bad}' is not supported in PROC LOGISTIC; \
                             it can affect results and cannot be ignored (planned: \
                             roadmap-avancee J05-P4)."
                        ),
                        span,
                    ));
                }
            }
            ts.expect_semi()?;
            match out {
                Some(out_ref) => outputs.push(LogisticOutput {
                    out: out_ref,
                    predicted,
                    xbeta,
                }),
                // J02-P1 — the statement used to be dropped without a dataset.
                None => {
                    return Err(SasError::parse(
                        "OUTPUT without OUT= is not supported in PROC LOGISTIC; \
                         no output data set would be created.",
                        out_span,
                    ));
                }
            }
            Ok(true)
        } else if UNSUPPORTED_STATEMENTS.contains(&kw) {
            // J02-P1 — valid PROC LOGISTIC statements that are not
            // implemented: contract message instead of « 180-322 not valid ».
            Err(common::unsupported_statement("LOGISTIC", kw))
        } else if kw == "effectplot" {
            // Graphics only: display customization (WARNING, CONTRIBUTING §5).
            ts.warn_ignored_display(common::ignored_display_statement("LOGISTIC", kw));
            ts.skip_to_semi();
            Ok(true)
        } else {
            Ok(false)
        }
    })?;

    Ok(LogisticAst {
        data_options: LogisticDataOptions {
            input,
            descending: proc_descending,
        },
        class_vars,
        model,
        freq_var,
        outputs,
    })
}

/// Instructions valides de PROC LOGISTIC (SAS/STAT 9.4 User's Guide, The
/// LOGISTIC Procedure, Syntax) non implémentées par sasrs, hors de la liste
/// partagée de `common::unhandled_proc_statement` (BY, WEIGHT, ID, ESTIMATE,
/// CONTRAST, LSMEANS…). Toutes peuvent changer les résultats ; EFFECTPLOT
/// (graphique seul) est traité à part avec un WARNING.
const UNSUPPORTED_STATEMENTS: &[&str] = &[
    "code",
    "effect",
    "exact",
    "exactoptions",
    "lsmestimate",
    "nloptions",
    "oddsratio",
    "roc",
    "roccontrast",
    "score",
    "slice",
    "store",
    "strata",
    "test",
    "units",
];

/// Codage CLASS lu en forme parenthésée ou après `/`.
#[derive(Default, Clone, Copy)]
struct ClassCoding {
    /// `PARAM=REF|REFERENCE` écrit explicitement.
    param_ref: bool,
    /// `REF=FIRST` (`Some(true)`) / `REF=LAST` (`Some(false)`).
    ref_first: Option<bool>,
}

/// Lit une option CLASS (`PARAM=`, `REF=`) au token courant.
fn parse_class_option(ts: &mut StatementStream, coding: &mut ClassCoding) -> Result<()> {
    if ts.peek().is_kw("param") {
        ts.next();
        if ts.peek().kind == TokenKind::Eq {
            ts.next();
        }
        let span = ts.peek().span;
        let Some(v) = ts.peek().ident().map(str::to_string) else {
            return Err(SasError::parse("expected a value after PARAM=", span));
        };
        ts.next();
        if !(v.eq_ignore_ascii_case("ref") || v.eq_ignore_ascii_case("reference")) {
            return Err(SasError::parse(
                format!(
                    "CLASS PARAM={} is not supported in PROC LOGISTIC; \
                     only PARAM=REF is implemented (planned: roadmap-avancee J05-P2).",
                    v.to_uppercase()
                ),
                span,
            ));
        }
        coding.param_ref = true;
    } else if ts.peek().is_kw("ref") {
        ts.next();
        if ts.peek().kind == TokenKind::Eq {
            ts.next();
        }
        let span = ts.peek().span;
        if let TokenKind::Str { value, .. } = &ts.peek().kind {
            return Err(SasError::parse(
                format!(
                    "CLASS REF='{value}' is not supported in PROC LOGISTIC; use REF=FIRST \
                     or REF=LAST (planned: roadmap-avancee J05-P2)."
                ),
                span,
            ));
        }
        let Some(v) = ts.peek().ident().map(str::to_string) else {
            return Err(SasError::parse("expected a value after REF=", span));
        };
        ts.next();
        coding.ref_first = match v.to_ascii_lowercase().as_str() {
            "first" => Some(true),
            "last" => Some(false),
            other => {
                return Err(SasError::parse(
                    format!(
                        "CLASS REF={} is invalid; use REF=FIRST or REF=LAST.",
                        other.to_uppercase()
                    ),
                    span,
                ));
            }
        };
    } else {
        let span = ts.peek().span;
        let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
        return Err(SasError::parse(
            format!("Unknown or unsupported CLASS option '{bad}' in PROC LOGISTIC."),
            span,
        ));
    }
    Ok(())
}

/// `CLASS var [(options)] … [/ options];` — SAS/STAT 9.4, The LOGISTIC
/// Procedure, CLASS statement : les options globales après `/` s'appliquent
/// à toutes les variables ; une option parenthésée d'une variable prime.
///
/// J02-P1 — les options après `/` étaient lues comme des noms de variables
/// (`param`, `ref`, `first`…) et le codage demandé était perdu en silence.
fn parse_class(ts: &mut StatementStream, class_vars: &mut Vec<ClassVar>) -> Result<()> {
    let mut parsed: Vec<(String, ClassCoding)> = Vec::new();
    let mut global = ClassCoding::default();
    while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
        if ts.peek().kind == TokenKind::Slash {
            ts.next();
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                parse_class_option(ts, &mut global)?;
            }
            break;
        }
        let Some(name) = ts.peek().ident().map(str::to_string) else {
            return Err(SasError::parse(
                "expected a variable name in the CLASS statement",
                ts.peek().span,
            ));
        };
        ts.next();
        let mut coding = ClassCoding::default();
        if ts.peek().kind == TokenKind::LParen {
            ts.next();
            loop {
                match ts.peek().kind {
                    TokenKind::RParen => {
                        ts.next();
                        break;
                    }
                    TokenKind::Semi | TokenKind::Eof => break,
                    _ => parse_class_option(ts, &mut coding)?,
                }
            }
        }
        parsed.push((name, coding));
    }
    ts.expect_semi()?;
    for (name, coding) in parsed {
        class_vars.push(ClassVar {
            name,
            ref_first: coding.ref_first.or(global.ref_first).unwrap_or(false),
            param_explicit: coding.param_ref || global.param_ref,
        });
    }
    Ok(())
}
