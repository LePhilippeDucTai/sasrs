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

use crate::ast::{DataStepAst, DatasetRef, GlobalStmt};
use crate::error::{Result, SasError};
use crate::lexer::Lexer;
use crate::procs::ProcAst;
use crate::source::SourceFile;
use crate::token::{Span, Token, TokenKind};

pub enum Block {
    Global(GlobalStmt),
    DataStep(DataStepAst),
    Proc { name: String, ast: ProcAst },
    /// `run;` isolé ou statement vide : écho dans le log, aucune action.
    Empty,
}

/// `title` → 1, `title3` → 3 ; `None` si ce n'est pas un mot-clé TITLEn.
pub(crate) fn title_level(name: &str) -> Option<u8> {
    let lower = name.to_ascii_lowercase();
    let rest = lower.strip_prefix("title")?;
    match rest {
        "" => Some(1),
        _ if rest.len() == 1 && rest.as_bytes()[0].is_ascii_digit() && rest != "0" => {
            rest.parse().ok()
        }
        _ => None,
    }
}

/// Mot-clé qui ouvre un bloc (frontière de step implicite). Les statements
/// globaux sont des frontières de step en SAS, au même titre que DATA/PROC.
fn is_block_head_kw(lower: &str) -> bool {
    matches!(lower, "data" | "proc" | "libname" | "options") || title_level(lower).is_some()
}

fn validate_sas_name(name: &str, span: Span) -> Result<()> {
    if name.len() > 32 {
        return Err(SasError::parse(
            format!("The name {} exceeds the SAS maximum of 32 characters.", name.to_uppercase()),
            span,
        ));
    }
    Ok(())
}

pub struct StatementStream<'a> {
    pub src: &'a SourceFile,
    toks: Vec<Token>,
    pos: usize,
}

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

    /// `ident [ . ident ]` → DatasetRef. Valide les noms SAS (≤32 chars).
    pub fn parse_dataset_ref(&mut self) -> Result<DatasetRef> {
        let tok = self.peek().clone();
        let Some(first) = tok.ident().map(str::to_string) else {
            return Err(SasError::parse("expected a dataset name", tok.span));
        };
        self.next();
        validate_sas_name(&first, tok.span)?;
        if self.peek().kind != TokenKind::Dot {
            return Ok(DatasetRef {
                libref: None,
                name: first,
            });
        }
        self.next(); // '.'
        let member_tok = self.peek().clone();
        let Some(member) = member_tok.ident().map(str::to_string) else {
            return Err(SasError::parse(
                "expected a member name after '.'",
                member_tok.span,
            ));
        };
        self.next();
        validate_sas_name(&member, member_tok.span)?;
        if first.len() > 8 {
            return Err(SasError::parse(
                format!("The libref {} exceeds 8 characters.", first.to_uppercase()),
                tok.span,
            ));
        }
        Ok(DatasetRef {
            libref: Some(first),
            name: member,
        })
    }

    /// Liste de noms de variables jusqu'au `;` (non consommé). Au moins un.
    pub fn parse_name_list(&mut self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        while let Some(name) = self.peek().ident().map(str::to_string) {
            validate_sas_name(&name, self.peek().span)?;
            self.next();
            names.push(name);
        }
        if names.is_empty() {
            return Err(SasError::parse(
                "expected a variable name",
                self.peek().span,
            ));
        }
        Ok(names)
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
mod tests {
    use super::*;

    fn stream(src: &SourceFile) -> StatementStream<'_> {
        StatementStream::new(src).unwrap()
    }

    #[test]
    fn peek_next_eof() {
        let src = SourceFile::new("x = 1;");
        let mut ts = stream(&src);
        assert!(ts.peek().is_kw("x"));
        assert!(ts.next().is_kw("x"));
        assert_eq!(ts.next().kind, TokenKind::Eq);
        assert_eq!(ts.next().kind, TokenKind::Num(1.0));
        assert_eq!(ts.next().kind, TokenKind::Semi);
        assert!(ts.at_eof());
        // next() reste sur Eof.
        assert_eq!(ts.next().kind, TokenKind::Eof);
        assert_eq!(ts.next().kind, TokenKind::Eof);
    }

    #[test]
    fn expect_semi_ok_and_err() {
        let src = SourceFile::new("; x");
        let mut ts = stream(&src);
        assert!(ts.expect_semi().is_ok());
        assert!(ts.expect_semi().is_err());
    }

    #[test]
    fn skip_to_semi_consumes_semi() {
        let src = SourceFile::new("a b c ; next");
        let mut ts = stream(&src);
        ts.skip_to_semi();
        assert!(ts.peek().is_kw("next"));
        // À EOF : no-op.
        ts.skip_to_semi();
        ts.next();
        ts.skip_to_semi();
        assert!(ts.at_eof());
    }

    #[test]
    fn dataset_ref_one_and_two_level() {
        let src = SourceFile::new("a mylib.b ;");
        let mut ts = stream(&src);
        let r1 = ts.parse_dataset_ref().unwrap();
        assert_eq!(r1, DatasetRef { libref: None, name: "a".into() });
        let r2 = ts.parse_dataset_ref().unwrap();
        assert_eq!(
            r2,
            DatasetRef { libref: Some("mylib".into()), name: "b".into() }
        );
        assert_eq!(ts.peek().kind, TokenKind::Semi);
    }

    #[test]
    fn dataset_ref_rejects_long_names() {
        let long = "x".repeat(33);
        let src1 = SourceFile::new(format!("{long};"));
        assert!(stream(&src1).parse_dataset_ref().is_err());
        let src2 = SourceFile::new("librefnine.a;");
        assert!(stream(&src2).parse_dataset_ref().is_err());
        let src3 = SourceFile::new("lib.;");
        assert!(stream(&src3).parse_dataset_ref().is_err());
    }

    #[test]
    fn name_list_until_semi() {
        let src = SourceFile::new("a b c;");
        let mut ts = stream(&src);
        assert_eq!(ts.parse_name_list().unwrap(), vec!["a", "b", "c"]);
        assert_eq!(ts.peek().kind, TokenKind::Semi);
        // Liste vide → erreur.
        assert!(ts.parse_name_list().is_err());
    }

    #[test]
    fn next_block_none_on_empty_and_inert() {
        let src = SourceFile::new("");
        assert!(stream(&src).next_block().is_none());
        let src = SourceFile::new(" ;;  ; ");
        assert!(stream(&src).next_block().is_none());
        let src = SourceFile::new("* just a comment statement ;");
        assert!(stream(&src).next_block().is_none());
    }

    #[test]
    fn lone_run_is_empty_block() {
        let src = SourceFile::new("run;");
        let mut ts = stream(&src);
        let (block, span) = ts.next_block().unwrap();
        assert!(matches!(block.unwrap(), Block::Empty));
        assert_eq!(span, Span::new(0, 4)); // couvre `run;`
        assert!(ts.next_block().is_none());
    }

    #[test]
    fn macro_call_errors_and_recovers() {
        let src = SourceFile::new("%let x = 1; run;");
        let mut ts = stream(&src);
        let (block, _) = ts.next_block().unwrap();
        let Err(err) = block else { panic!("expected an error") };
        assert!(err.to_string().contains("macro facility"));
        // Récupération : le bloc suivant est le run; isolé.
        let (block, _) = ts.next_block().unwrap();
        assert!(matches!(block.unwrap(), Block::Empty));
    }

    #[test]
    fn unknown_statement_errors_and_recovers() {
        let src = SourceFile::new("frobnicate a b; run;");
        let mut ts = stream(&src);
        let (block, _) = ts.next_block().unwrap();
        let Err(err) = block else { panic!("expected an error") };
        assert!(err.to_string().contains("FROBNICATE"));
        let (block, _) = ts.next_block().unwrap();
        assert!(matches!(block.unwrap(), Block::Empty));
    }

    #[test]
    fn non_ident_head_errors() {
        let src = SourceFile::new("= 1; run;");
        let mut ts = stream(&src);
        let (block, _) = ts.next_block().unwrap();
        assert!(block.is_err());
        let (block, _) = ts.next_block().unwrap();
        assert!(matches!(block.unwrap(), Block::Empty));
    }

    #[test]
    fn proc_without_name_errors() {
        let src = SourceFile::new("proc ; run;");
        let mut ts = stream(&src);
        let (block, _) = ts.next_block().unwrap();
        assert!(block.is_err());
        // skip_to_step_boundary a consommé le run; de récupération.
        assert!(ts.next_block().is_none());
    }

    #[test]
    fn step_boundary_stops_before_block_heads() {
        let src = SourceFile::new("x = 1; y = 2; data b;");
        let mut ts = stream(&src);
        ts.skip_to_step_boundary();
        assert!(ts.peek().is_kw("data"));

        let src = SourceFile::new("garbage tokens run; data b;");
        let mut ts = stream(&src);
        ts.skip_to_step_boundary();
        assert!(ts.peek().is_kw("data"));

        let src = SourceFile::new("x = 1; title 'boundary';");
        let mut ts = stream(&src);
        ts.skip_to_step_boundary();
        assert!(ts.peek().is_kw("title"));
    }

    #[test]
    fn title_levels() {
        assert_eq!(title_level("title"), Some(1));
        assert_eq!(title_level("TITLE3"), Some(3));
        assert_eq!(title_level("title9"), Some(9));
        assert_eq!(title_level("title0"), None);
        assert_eq!(title_level("title10"), None);
        assert_eq!(title_level("titles"), None);
        assert_eq!(title_level("data"), None);
    }
}
