use super::*;

use crate::parser::StatementStream;
use crate::procs::common;
use crate::token::TokenKind;

// ───────────────────────── Parser IML ─────────────────────────

pub(super) struct Parser {
    pub(super) toks: Vec<Tok>,
    pub(super) pos: usize,
    /// J02-P5 — WARNINGs d'affichage (options PRINT, instructions d'affichage
    /// ignorées), émis avant l'exécution de l'étape, même si le parsing échoue
    /// ensuite.
    pub(super) warnings: Vec<String>,
}

mod control;
mod expr;
mod stmt;

/// Suffix naming the roadmap-avancee unit that will lift a provisional ERROR.
fn planned(unit: Option<&str>) -> String {
    match unit {
        Some(u) => format!(" (planned: roadmap-avancee {u})"),
        None => String::new(),
    }
}

/// ERROR for a valid SAS/IML construction that sasrs does not implement, in
/// the wording of the contract catalogue (`common::unsupported_statement`),
/// naming the plan unit that will implement it when one is planned.
fn unsupported(what: &str, unit: Option<&str>) -> SasError {
    SasError::runtime(format!(
        "{what} is not supported in PROC IML; it can affect results and cannot be ignored{}.",
        planned(unit)
    ))
}

impl Parser {
    pub(super) fn peek(&self) -> &Tok {
        &self.toks[self.pos]
    }

    pub(super) fn next(&mut self) -> Tok {
        let t = self.toks[self.pos].clone();
        if t != Tok::Eof {
            self.pos += 1;
        }
        t
    }

    pub(super) fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == t {
            self.next();
            true
        } else {
            false
        }
    }

    pub(super) fn expect(&mut self, t: &Tok, what: &str) -> Result<()> {
        if self.eat(t) {
            Ok(())
        } else {
            Err(SasError::runtime(format!(
                "IML: expected {what}, found {}.",
                self.peek().describe()
            )))
        }
    }

    pub(super) fn expect_ident(&mut self, what: &str) -> Result<String> {
        match self.next() {
            Tok::Ident(s) => Ok(s),
            other => Err(SasError::runtime(format!(
                "IML: expected {what}, found {}.",
                other.describe()
            ))),
        }
    }

    pub(super) fn expect_number(&mut self) -> Result<f64> {
        match self.next() {
            Tok::Num(v) => Ok(v),
            other => Err(SasError::runtime(format!(
                "IML: expected a number, found {}.",
                other.describe()
            ))),
        }
    }
}

/// Parse le corps brut d'un bloc IML ; rend aussi les WARNINGs d'affichage
/// accumulés jusqu'à la fin du parsing ou jusqu'à l'erreur.
fn parse_source(src: &str) -> (Result<ImlProgram>, Vec<String>) {
    let toks = match lex(src) {
        Ok(toks) => toks,
        Err(e) => return (Err(e), Vec::new()),
    };
    let mut p = Parser {
        toks,
        pos: 0,
        warnings: Vec::new(),
    };
    let prog = p.parse_program();
    (prog, p.warnings)
}

/// Parse le corps brut d'un bloc IML.
pub fn parse_body(src: &str) -> Result<ImlProgram> {
    parse_source(src).0
}

/// Entrée appelée par `parse_proc` : `ts` est positionné après `proc iml`,
/// sur les options du statement ; le corps suit sous forme de `ImlBody`.
pub fn parse(ts: &mut StatementStream) -> Result<ImlProgram> {
    if let Err(e) = parse_proc_options(ts) {
        skip_statement_and_body(ts);
        return Err(e);
    }
    if ts.peek().kind == TokenKind::Semi {
        ts.next();
    }
    let tok = ts.peek().clone();
    if let TokenKind::ImlBody(body) = tok.kind {
        ts.next();
        let (prog, warnings) = parse_source(&body);
        for warning in warnings {
            ts.warn_ignored_display(warning);
        }
        prog
    } else {
        // PROC IML sans corps capturé (pas de quit;) : corps vide.
        Err(SasError::parse(
            "PROC IML requires a QUIT; to terminate the block.",
            tok.span,
        ))
    }
}

/// Options of the PROC IML statement (SAS/IML 9.4, PROC IML Statement:
/// `PROC IML <SYMSIZE=n1> <WORKSIZE=n2>;`). Both size the initial symbol
/// space and workspace, in kilobytes; SAS acquires more memory automatically
/// when they are exhausted (SAS/IML 9.4, Further Notes, « Memory and
/// Workspace »), so they have no observable effect and sasrs, which
/// allocates on demand, accepts them. Any other token used to be skipped
/// (audit d0b4d90, J02-P5): it is now the « Unexpected option » ERROR.
fn parse_proc_options(ts: &mut StatementStream) -> Result<()> {
    loop {
        if matches!(
            ts.peek().kind,
            TokenKind::Semi | TokenKind::ImlBody(_) | TokenKind::Eof
        ) {
            return Ok(());
        }
        let name = match ts.peek().ident().map(str::to_ascii_uppercase) {
            Some(name) if name == "SYMSIZE" || name == "WORKSIZE" => name,
            _ => return Err(common::unknown_option_error(ts, "IML")),
        };
        common::consume_option_eq(ts, &name)?;
        match ts.peek().kind {
            TokenKind::Num(v) if v >= 0.0 => {
                ts.next();
            }
            _ => {
                return Err(SasError::parse(
                    format!("expected a number of kilobytes after {name}="),
                    ts.peek().span,
                ));
            }
        }
    }
}

/// After an error on the PROC IML statement, consume the rest of the
/// statement and the captured body: the caller then resumes at the next
/// step instead of skipping it with the body.
fn skip_statement_and_body(ts: &mut StatementStream) {
    while !matches!(
        ts.peek().kind,
        TokenKind::Semi | TokenKind::ImlBody(_) | TokenKind::Eof
    ) {
        ts.next();
    }
    if ts.peek().kind == TokenKind::Semi {
        ts.next();
    }
    if matches!(ts.peek().kind, TokenKind::ImlBody(_)) {
        ts.next();
    }
}
