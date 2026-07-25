use super::*;

// ───────────────────────── Parser IML ─────────────────────────

pub(super) struct Parser {
    pub(super) toks: Vec<Tok>,
    pub(super) pos: usize,
}

mod stmt;
mod control;
mod expr;

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
                "IML: expected {what}, found {:?}",
                self.peek()
            )))
        }
    }

    pub(super) fn expect_ident(&mut self, what: &str) -> Result<String> {
        match self.next() {
            Tok::Ident(s) => Ok(s),
            other => Err(SasError::runtime(format!("IML: expected {what}, found {other:?}"))),
        }
    }

    pub(super) fn expect_number(&mut self) -> Result<f64> {
        match self.next() {
            Tok::Num(v) => Ok(v),
            other => Err(SasError::runtime(format!("IML: expected a number, found {other:?}"))),
        }
    }
}

/// Parse le corps brut d'un bloc IML.
pub fn parse_body(src: &str) -> Result<ImlProgram> {
    let toks = lex(src)?;
    let mut p = Parser { toks, pos: 0 };
    p.parse_program()
}

/// Entrée appelée par `parse_proc` : `ts` est positionné sur le token
/// `ImlBody`. On le consomme et on parse son contenu.
pub fn parse(ts: &mut crate::parser::StatementStream) -> Result<ImlProgram> {
    use crate::token::TokenKind;
    // Le statement `proc iml;` se termine par `;` : le consommer. PROC IML
    // n'accepte pas d'options dans notre périmètre, donc tout token avant le
    // `;` est ignoré (best-effort).
    while !matches!(ts.peek().kind, TokenKind::Semi | TokenKind::ImlBody(_) | TokenKind::Eof) {
        ts.next();
    }
    if ts.peek().kind == TokenKind::Semi {
        ts.next();
    }
    let tok = ts.peek().clone();
    if let TokenKind::ImlBody(body) = tok.kind {
        ts.next();
        parse_body(&body)
    } else {
        // PROC IML sans corps capturé (pas de quit;) : corps vide.
        Err(SasError::parse(
            "PROC IML requires a QUIT; to terminate the block.",
            tok.span,
        ))
    }
}

