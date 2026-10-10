//! J02-P6 — diagnostics de contrat communs aux procédures graphiques
//! (SGPLOT, GPLOT, GCHART, PLOT).
//!
//! Ces aides sont compilées dans LES DEUX builds : les WARNING d'affichage,
//! les ERROR de contrat et la validation de `DATA=` sont donc identiques avec
//! ou sans `--features graphics` ; le moteur de rendu (gated) n'en est que le
//! consommateur. Règle de sévérité : CONTRIBUTING §5 et
//! `docs/support-contract.md` — une option graphique non rendue est une
//! personnalisation d'affichage (WARNING, code 1), une demande qui change les
//! objets produits (images par groupe BY, table ou fichier de sortie) est une
//! ERROR (étape rejetée, code 2).

use crate::ast::DatasetRef;
use crate::dataset::SasDataset;
use crate::error::{Result, SasError};
use crate::parser::StatementStream;
use crate::session::Session;
use crate::token::{Span, TokenKind};

/// Suffix naming the roadmap-avancee unit that will lift a provisional
/// diagnostic.
pub(crate) fn planned(unit: &str) -> String {
    format!(" (planned: roadmap-avancee {unit})")
}

/// WARNING text for a display option that the image engine does not render.
/// `stmt` is `None` for an option of the PROC statement itself.
pub(crate) fn ignored_option(proc_name: &str, stmt: Option<&str>, option: &str) -> String {
    match stmt {
        Some(stmt) => format!(
            "The {option} option of the {stmt} statement is ignored in PROC {proc_name}; \
             display customization is not supported."
        ),
        None => format!(
            "The {option} option is ignored in PROC {proc_name}; display customization is \
             not supported."
        ),
    }
}

/// ERROR for a valid option that is not implemented and would change the
/// objects produced by the step (output catalog, data set or file).
pub(crate) fn unsupported_option(proc_name: &str, option: &str, span: Span) -> SasError {
    SasError::parse(
        format!(
            "The {option} option is not supported in PROC {proc_name}; it can affect results \
             and cannot be ignored."
        ),
        span,
    )
}

/// ERROR for a valid statement that is not implemented: the catalogue message
/// of `common::unsupported_statement`, with the unit that will lift it.
pub(crate) fn unsupported_statement(
    proc_name: &str,
    stmt: &str,
    unit: Option<&str>,
    span: Span,
) -> SasError {
    SasError::parse(
        format!(
            "The {} statement is not supported in PROC {proc_name}; it can affect results and \
             cannot be ignored{}.",
            stmt.to_ascii_uppercase(),
            unit.map(planned).unwrap_or_default()
        ),
        span,
    )
}

/// BY statement: no image is produced per BY group yet (J13-P4). Same
/// catalogue message for the four graphics procedures; the step is rejected
/// before execution.
pub(crate) fn by_not_supported(proc_name: &str, span: Span) -> SasError {
    unsupported_statement(proc_name, "BY", Some("J13-P4"), span)
}

/// Upper-case label of the option at the current token: `NAME=` when an `=`
/// follows, `NAME` for a flag (`?` for a non-identifier).
pub(crate) fn option_label(ts: &StatementStream) -> String {
    let name = ts.peek().ident().unwrap_or("?").to_ascii_uppercase();
    if ts.peek2().kind == TokenKind::Eq {
        format!("{name}=")
    } else {
        name
    }
}

/// True for `TO` / `BY`, the keywords of a value list (`0 to 100 by 10`).
fn is_list_keyword(ts: &StatementStream) -> bool {
    ts.peek()
        .ident()
        .is_some_and(|s| s.eq_ignore_ascii_case("to") || s.eq_ignore_ascii_case("by"))
}

/// Consume one item of an option value: a number (signed), a string, a
/// parenthesized group, or a name with its optional `.suffix` (`lib.table`,
/// format `best12.`) and `(sub-options)`. Returns false, consuming nothing, on
/// any other token.
fn skip_value_item(ts: &mut StatementStream) -> bool {
    match ts.peek().kind {
        TokenKind::LParen => {
            ts.skip_balanced_parens();
            true
        }
        TokenKind::Minus | TokenKind::Plus if matches!(ts.peek2().kind, TokenKind::Num(_)) => {
            ts.next();
            ts.next();
            true
        }
        TokenKind::Num(_) | TokenKind::Str { .. } => {
            ts.next();
            true
        }
        TokenKind::Ident(_) => {
            let name = ts.next();
            if ts.peek().kind == TokenKind::Dot && ts.peek().span.start == name.span.end {
                let dot = ts.next();
                if ts.peek().ident().is_some() && ts.peek().span.start == dot.span.end {
                    ts.next();
                }
            }
            ts.skip_balanced_parens();
            true
        }
        _ => false,
    }
}

/// Consume the arguments of the option whose name was just consumed:
/// `(sub-options)`, `= value`, `= (…)`, `= name(…)`, or a value list such as
/// `= 10 20 30` / `= 0 to 100 by 10`. Stops on the next option name, `/` or
/// `;` so that the statement stays synchronized.
pub(crate) fn skip_option_args(ts: &mut StatementStream) {
    ts.skip_balanced_parens();
    if ts.peek().kind != TokenKind::Eq {
        return;
    }
    ts.next();
    if !skip_value_item(ts) {
        return;
    }
    loop {
        if is_list_keyword(ts) || ts.peek().kind == TokenKind::Comma {
            ts.next();
            if !skip_value_item(ts) {
                return;
            }
        } else if matches!(
            ts.peek().kind,
            TokenKind::Num(_) | TokenKind::Str { .. } | TokenKind::Minus | TokenKind::Plus
        ) {
            if !skip_value_item(ts) {
                return;
            }
        } else {
            return;
        }
    }
}

/// Queue the display WARNING of the option at the current token, then consume
/// the option with its arguments.
pub(crate) fn warn_option(ts: &mut StatementStream, proc_name: &str, stmt: Option<&str>) {
    ts.warn_ignored_display(ignored_option(proc_name, stmt, &option_label(ts)));
    ts.next();
    skip_option_args(ts);
}

/// Error for a token that cannot start an option in an option list.
pub(crate) fn expected_option(ts: &StatementStream, proc_name: &str, stmt: &str) -> SasError {
    SasError::parse(
        format!("Expected an option name in the {stmt} statement of PROC {proc_name}."),
        ts.peek().span,
    )
}

/// Numbers of a parenthesized value list `( … )` (SGPLOT `VALUES=`, AXIS
/// `ORDER=`), the `(` being current: signed numbers in order, other items
/// (`TO`, `BY`, strings) skipped. Consumes the closing `)`; stops on `;`
/// without consuming it. J02-P6 — the sign of `-10` used to be dropped
/// (`VALUES=(-10 to 10)` gave the range 10..10).
pub(crate) fn value_list_numbers(ts: &mut StatementStream) -> Vec<f64> {
    let mut nums: Vec<f64> = Vec::new();
    ts.next(); // (
    let mut depth = 1usize;
    loop {
        match ts.peek().kind {
            TokenKind::Semi | TokenKind::Eof => break,
            TokenKind::LParen => {
                depth += 1;
                ts.next();
            }
            TokenKind::RParen => {
                ts.next();
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            TokenKind::Num(f) => {
                nums.push(f);
                ts.next();
            }
            TokenKind::Minus | TokenKind::Plus if matches!(ts.peek2().kind, TokenKind::Num(_)) => {
                let negative = ts.next().kind == TokenKind::Minus;
                if let TokenKind::Num(f) = ts.next().kind {
                    nums.push(if negative { -f } else { f });
                }
            }
            _ => {
                ts.next();
            }
        }
    }
    nums
}

/// A number as shown in a diagnostic: integers without `.0`.
fn fmt_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// WARNING for an axis value list (`VALUES=`, `ORDER=`) of which the engine
/// only draws the first two numbers, as the axis range; the display WARNING
/// of the whole option when fewer than two numbers are given.
pub(crate) fn partial_value_list(
    proc_name: &str,
    stmt: &str,
    option: &str,
    nums: &[f64],
) -> String {
    match nums {
        [lo, hi, ..] => format!(
            "The {option} option of the {stmt} statement is only partly honored in PROC \
             {proc_name}: its first two numbers ({} and {}) set the axis range; the other values \
             and the tick marks are ignored.",
            fmt_number(*lo),
            fmt_number(*hi)
        ),
        _ => ignored_option(proc_name, Some(stmt), option),
    }
}

/// Opens `DATA=` (or `_LAST_`) and checks that every variable named by the
/// step exists, in both builds. J02-P6 — the default build never opened the
/// table of SGPLOT, GPLOT and GCHART: a missing table or variable went
/// unnoticed (« image deferred », code 0). Each missing variable gets its own
/// ERROR line, as in SAS; the last one is returned to stop the step.
pub(crate) fn open_checked(
    data: &Option<DatasetRef>,
    session: &mut Session,
    vars: &[&str],
) -> Result<SasDataset> {
    let (ds, _, _) = crate::procs::common::open_input(data, session)?;
    let mut missing: Vec<String> = Vec::new();
    for var in vars {
        let upper = var.to_ascii_uppercase();
        if !ds.vars.iter().any(|m| m.name.eq_ignore_ascii_case(var)) && !missing.contains(&upper) {
            missing.push(upper);
        }
    }
    match missing.split_last() {
        None => Ok(ds),
        Some((last, others)) => {
            for name in others {
                session.log.error(&format!("Variable {name} not found."));
            }
            Err(SasError::runtime(format!("Variable {last} not found.")))
        }
    }
}
