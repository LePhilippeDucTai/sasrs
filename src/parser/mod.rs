//! Block-by-block parsing of a SAS program.
//!
//! # Plan du fichier (M1) — voir PLAN.md
//!
//! SAS a une grammaire contextuelle : chaque PROC possède sa propre
//! syntaxe. La stratégie est donc de découper le programme en *blocs*
//! (statement global | étape DATA | étape PROC) et de déléguer chaque bloc
//! à un sous-parser. On ne parse JAMAIS tout le fichier d'avance :
//! l'exécuteur appelle `next_block()` puis exécute, car plus tard le
//! processeur macro (`%let`, CALL SYMPUT) peut modifier la source en aval.
//!
//! ## `StatementStream::new`
//! Lexe la source entière (`Lexer::tokenize`) et garde `pos`.
//!
//! ## `next_block()` — algorithme
//! 1. Sauter les statements vides (`;`) et les commentaires-statements
//!    (`* texte... ;`) : si le token au DÉBUT d'un statement est `Star`,
//!    consommer jusqu'au `Semi` inclus.
//! 2. `Eof` → `None`.
//! 3. Mot-clé de tête (insensible à la casse) :
//!    - `data`    → `datastep::parse_data_step(self)` ; consomme jusqu'à
//!      `run;` inclus (ou frontière implicite : prochain `data`/`proc`
//!      en début de statement, comme SAS).
//!    - `proc`    → lire le nom, déléguer à `procs::parse_proc(name, self)`.
//!      PROC inconnue → ERROR "Procedure XXX not found", récupération :
//!      `skip_to_step_boundary()`.
//!    - `libname` / `options` / `title`..`title9` → `global::parse_global`.
//!    - `run` seul → bloc vide (no-op, écho seulement).
//!    - `TokenKind::MacroCall` → ERROR "The macro facility is not yet
//!      implemented", skip jusqu'au `;`.
//!    - autre → ERROR syntaxe, skip jusqu'au `;`.
//! 4. Retourner `(Result<Block>, Span)` où `Span` couvre du premier token
//!    consommé au `;` final inclus — l'exécuteur s'en sert pour échoer les
//!    lignes source dans le log AVANT d'exécuter.
//!
//! ## Récupération d'erreur
//! Une erreur de parsing n'arrête pas la session : l'exécuteur logge
//! ERROR, le stream saute à la frontière suivante et on continue.
//!
//! ## Helpers pour les sous-parsers
//! `peek/next/is_kw/expect_kw/expect_semi/parse_dataset_ref/parse_name_list`.
//! `parse_dataset_ref` : `ident [ . ident ]` → `DatasetRef` (libref None =
//! WORK). Les noms SAS font ≤ 32 caractères — valider, sinon ERROR.

// Certains helpers ne sont consommés que par les sous-parsers livrés plus
// tard dans M1 (datastep, global, procs).
#![allow(dead_code)]

pub mod datastep;
pub mod expr;
pub mod global;

use crate::ast::{DataStepAst, DatasetOptions, DatasetRef, DatasetSpec, GlobalStmt};
use crate::error::{Result, SasError};
use crate::lexer::Lexer;
use crate::procs::ProcAst;
use crate::source::SourceFile;
use crate::token::{Span, Token, TokenKind};

mod block;

pub use block::Block;

pub(crate) use block::*;

pub struct StatementStream<'a> {
    pub src: &'a SourceFile,
    toks: Vec<Token>,
    pos: usize,
}

mod dataset;

impl<'a> StatementStream<'a> {
    pub fn new(src: &'a SourceFile) -> Result<Self> {
        let toks = Lexer::new(&src.text).tokenize()?;
        Ok(StatementStream { src, toks, pos: 0 })
    }

    pub fn peek(&self) -> &Token {
        // tokenize() garantit un Eof terminal et next() ne le dépasse pas.
        &self.toks[self.pos]
    }

    pub fn next(&mut self) -> Token {
        let tok = self.toks[self.pos].clone();
        if tok.kind != TokenKind::Eof {
            self.pos += 1;
        }
        tok
    }

    /// Token APRÈS le token courant (lookahead de 2 ; Eof terminal au-delà).
    pub fn peek2(&self) -> &Token {
        let i = (self.pos + 1).min(self.toks.len() - 1);
        &self.toks[i]
    }

    /// Token à `n` positions du token courant (`peek_nth(0)` == `peek()` ;
    /// borné sur l'Eof terminal au-delà de la fin).
    pub fn peek_nth(&self, n: usize) -> &Token {
        let i = (self.pos + n).min(self.toks.len() - 1);
        &self.toks[i]
    }

    pub fn at_eof(&self) -> bool {
        self.peek().kind == TokenKind::Eof
    }

    /// Fin (offset) du dernier token consommé — pour borner le span d'un bloc.
    fn prev_end(&self) -> usize {
        if self.pos == 0 {
            0
        } else {
            self.toks[self.pos - 1].span.end
        }
    }

    /// Consomme un `;` ou signale une erreur de syntaxe.
    pub fn expect_semi(&mut self) -> Result<()> {
        if self.peek().kind == TokenKind::Semi {
            self.next();
            Ok(())
        } else {
            Err(SasError::parse("expected a ';'", self.peek().span))
        }
    }

    /// Saute jusqu'au prochain `;` inclus (récupération d'erreur).
    pub fn skip_to_semi(&mut self) {
        loop {
            match self.peek().kind {
                TokenKind::Eof => return,
                TokenKind::Semi => {
                    self.next();
                    return;
                }
                _ => {
                    self.next();
                }
            }
        }
    }

    /// Saute jusqu'après `run;`/`quit;`, ou s'arrête juste avant un
    /// `data`/`proc`/statement global en début de statement (frontière
    /// implicite). Best-effort : le test de frontière à l'entrée suppose
    /// qu'on est en début de statement ; appelée en plein milieu d'un
    /// statement erroné, elle peut au pire avaler ce statement-là, ce qui
    /// est le comportement de récupération voulu.
    pub fn skip_to_step_boundary(&mut self) {
        loop {
            match &self.peek().kind {
                TokenKind::Eof => return,
                TokenKind::Semi => {
                    self.next();
                }
                TokenKind::Ident(s) => {
                    let lower = s.to_ascii_lowercase();
                    if is_block_head_kw(&lower) {
                        return;
                    }
                    if lower == "run" || lower == "quit" {
                        self.next();
                        if self.peek().kind == TokenKind::Semi {
                            self.next();
                        }
                        return;
                    }
                    self.skip_to_semi();
                }
                _ => self.skip_to_semi(),
            }
        }
    }

    /// Saute les statements vides (`;`) et les commentaires-statements
    /// (`* texte ;`) qui précèdent un bloc.
    fn skip_inert(&mut self) {
        loop {
            match self.peek().kind {
                TokenKind::Semi => {
                    self.next();
                }
                TokenKind::Star => {
                    // Commentaire-statement : tout jusqu'au `;` inclus.
                    self.skip_to_semi();
                }
                _ => return,
            }
        }
    }

    /// Bloc suivant + span couvert (pour l'écho du log). `None` à EOF.
    pub fn next_block(&mut self) -> Option<(Result<Block>, Span)> {
        self.skip_inert();
        if self.at_eof() {
            return None;
        }
        let start = self.peek().span.start;
        let result = self.parse_block();
        let span = Span::new(start, self.prev_end().max(start));
        Some((result, span))
    }

    fn parse_block(&mut self) -> Result<Block> {
        let tok = self.peek().clone();
        match &tok.kind {
            TokenKind::Ident(head) => {
                let lower = head.to_ascii_lowercase();
                match lower.as_str() {
                    "data" => {
                        self.next();
                        match datastep::parse_data_step(self) {
                            Ok(ast) => Ok(Block::DataStep(ast)),
                            Err(e) => {
                                self.skip_to_step_boundary();
                                Err(e)
                            }
                        }
                    }
                    "proc" => {
                        self.next();
                        let name_tok = self.peek().clone();
                        let Some(name) = name_tok.ident().map(str::to_string) else {
                            self.skip_to_step_boundary();
                            return Err(SasError::parse(
                                "expected a procedure name after PROC",
                                name_tok.span,
                            ));
                        };
                        self.next();
                        match crate::procs::parse_proc(&name, self) {
                            Ok(ast) => Ok(Block::Proc { name, ast }),
                            Err(e) => {
                                self.skip_to_step_boundary();
                                Err(e)
                            }
                        }
                    }
                    "run" | "quit" => {
                        self.next();
                        if self.peek().kind == TokenKind::Semi {
                            self.next();
                        }
                        Ok(Block::Empty)
                    }
                    _ if is_block_head_kw(&lower) => match global::parse_global(self) {
                        Ok(stmt) => Ok(Block::Global(stmt)),
                        Err(e) => {
                            self.skip_to_semi();
                            Err(e)
                        }
                    },
                    _ => {
                        self.skip_to_semi();
                        Err(SasError::parse(
                            format!(
                                "Statement '{}' is not valid or it is used out of proper order.",
                                head.to_uppercase()
                            ),
                            tok.span,
                        ))
                    }
                }
            }
            TokenKind::MacroCall(_) => {
                self.skip_to_semi();
                Err(SasError::parse(
                    "The macro facility is not yet implemented.",
                    tok.span,
                ))
            }
            _ => {
                self.skip_to_semi();
                Err(SasError::parse(
                    "Statement is not valid or it is used out of proper order.",
                    tok.span,
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests;
