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
    matches!(lower, "data" | "proc" | "libname" | "filename" | "options" | "ods") || title_level(lower).is_some()
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
        let first_upper = first.to_uppercase();
        // Allow special system librefs (DICTIONARY, SASHELP) that exceed 8 characters.
        if first.len() > 8 && !matches!(first_upper.as_str(), "DICTIONARY" | "SASHELP") {
            return Err(SasError::parse(
                format!("The libref {} exceeds 8 characters.", first_upper),
                tok.span,
            ));
        }
        Ok(DatasetRef {
            libref: Some(first),
            name: member,
        })
    }

    /// `ref [( options )]` → `DatasetSpec`. Options de dataset (M2,
    /// insensibles à la casse, séparées par des espaces) :
    /// - `keep = liste` / `drop = liste` : liste de noms ou de plages
    ///   numérotées `x1-x3` (même expansion que pour ARRAY). Deux formes :
    ///   parenthésée `keep=(x y)` (lue jusqu'au `)` interne) ou nue
    ///   `keep=x y` — la liste nue s'arrête au prochain Ident SUIVI de `=`
    ///   (c'est l'option suivante) ou au `)` fermant (simplification
    ///   documentée : SAS lui-même n'accepte pas les parenthèses internes,
    ///   on accepte les deux).
    /// - `rename = (old=new [old2=new2...])` : TOUJOURS parenthésé.
    /// - `where = (expr)` : parenthésé, une expression.
    /// - option inconnue → erreur "Dataset option X is not supported.".
    pub fn parse_dataset_spec(&mut self) -> Result<DatasetSpec> {
        let dref = self.parse_dataset_ref()?;
        let mut options = DatasetOptions::default();
        if self.peek().kind != TokenKind::LParen {
            return Ok(DatasetSpec { dref, options });
        }
        self.next(); // `(`
        loop {
            let tok = self.peek().clone();
            // Le nom d'option : un Ident, ou le mot-clé `in` (lexé en
            // `TokenKind::In`, l'opérateur — réutilisé ici comme option IN=).
            let opt_name: Option<String> = match &tok.kind {
                TokenKind::RParen => {
                    self.next();
                    break;
                }
                TokenKind::Ident(name) => Some(name.to_ascii_lowercase()),
                TokenKind::In => Some("in".to_string()),
                _ => None,
            };
            let Some(lower) = opt_name else {
                return Err(SasError::parse(
                    "expected a dataset option or ')'",
                    tok.span,
                ));
            };
            if !matches!(lower.as_str(), "keep" | "drop" | "rename" | "where" | "in") {
                return Err(SasError::parse(
                    format!("Dataset option {} is not supported.", lower.to_uppercase()),
                    tok.span,
                ));
            }
            self.next(); // le nom d'option
            if self.peek().kind != TokenKind::Eq {
                return Err(SasError::parse(
                    format!(
                        "expected '=' after the dataset option {}",
                        lower.to_uppercase()
                    ),
                    self.peek().span,
                ));
            }
            self.next(); // `=`
            match lower.as_str() {
                "keep" => {
                    let list = self.parse_option_name_list("KEEP")?;
                    options.keep.get_or_insert_with(Vec::new).extend(list);
                }
                "drop" => {
                    let list = self.parse_option_name_list("DROP")?;
                    options.drop.get_or_insert_with(Vec::new).extend(list);
                }
                "rename" => options.rename.extend(self.parse_rename_pairs()?),
                "where" => options.where_ = Some(self.parse_where_option()?),
                // `in=nom` (M3) : nom de variable automatique 0/1. Sa
                // validité (MERGE input seulement) est tranchée à la
                // compilation.
                "in" => {
                    let nm_tok = self.peek().clone();
                    let Some(nm) = nm_tok.ident().map(str::to_string) else {
                        return Err(SasError::parse(
                            "expected a variable name after the IN= dataset option",
                            nm_tok.span,
                        ));
                    };
                    validate_sas_name(&nm, nm_tok.span)?;
                    self.next();
                    options.in_ = Some(nm);
                }
                _ => unreachable!("filtered above"),
            }
        }
        Ok(DatasetSpec { dref, options })
    }

    /// Valeur d'un `keep=` / `drop=` : forme parenthésée `( noms )` ou nue
    /// `noms` (arrêt sur Ident-suivi-de-`=` ou `)`). Plages `x1-x3`
    /// expansées. Au moins un nom.
    fn parse_option_name_list(&mut self, opt: &str) -> Result<Vec<String>> {
        let mut names = Vec::new();
        let parenthesized = self.peek().kind == TokenKind::LParen;
        if parenthesized {
            self.next(); // `(`
        }
        loop {
            let tok = self.peek().clone();
            match &tok.kind {
                TokenKind::RParen if parenthesized => {
                    self.next();
                    break;
                }
                // Forme nue : le `)` fermant des options termine la liste
                // SANS être consommé.
                TokenKind::RParen => break,
                TokenKind::Ident(_) => {
                    // Forme nue : un Ident suivi de `=` est l'option
                    // SUIVANTE, pas un nom de la liste.
                    if !parenthesized && self.peek2().kind == TokenKind::Eq {
                        break;
                    }
                    self.parse_option_list_item(&mut names)?;
                }
                _ => {
                    return Err(SasError::parse(
                        format!("expected a variable name in the {opt}= dataset option"),
                        tok.span,
                    ));
                }
            }
        }
        if names.is_empty() {
            return Err(SasError::parse(
                format!("expected a variable name in the {opt}= dataset option"),
                self.peek().span,
            ));
        }
        Ok(names)
    }

    /// Un élément de liste keep=/drop= : nom simple ou plage `x1-x3`
    /// (expansée via la même routine que pour ARRAY).
    fn parse_option_list_item(&mut self, out: &mut Vec<String>) -> Result<()> {
        let tok = self.peek().clone();
        let name = tok.ident().expect("caller matched an Ident").to_string();
        validate_sas_name(&name, tok.span)?;
        self.next();
        if self.peek().kind == TokenKind::Minus {
            self.next(); // `-`
            let end_tok = self.peek().clone();
            let Some(end_name) = end_tok.ident().map(str::to_string) else {
                return Err(SasError::parse(
                    "expected a variable name after '-' in the dataset option",
                    end_tok.span,
                ));
            };
            validate_sas_name(&end_name, end_tok.span)?;
            self.next();
            datastep::expand_numbered_range(&name, &end_name, tok.span.merge(end_tok.span), out)?;
        } else {
            out.push(name);
        }
        Ok(())
    }

    /// `rename = (old=new [old2=new2...])` — TOUJOURS parenthésé, au moins
    /// une paire.
    fn parse_rename_pairs(&mut self) -> Result<Vec<(String, String)>> {
        if self.peek().kind != TokenKind::LParen {
            return Err(SasError::parse(
                "The RENAME= dataset option requires a parenthesized list of old=new pairs.",
                self.peek().span,
            ));
        }
        self.next(); // `(`
        let mut pairs = Vec::new();
        loop {
            let tok = self.peek().clone();
            match &tok.kind {
                TokenKind::RParen => {
                    self.next();
                    break;
                }
                TokenKind::Ident(old) => {
                    let old = old.clone();
                    validate_sas_name(&old, tok.span)?;
                    self.next();
                    if self.peek().kind != TokenKind::Eq {
                        return Err(SasError::parse(
                            "expected '=' between the old and new names in RENAME=",
                            self.peek().span,
                        ));
                    }
                    self.next(); // `=`
                    let new_tok = self.peek().clone();
                    let Some(new) = new_tok.ident().map(str::to_string) else {
                        return Err(SasError::parse(
                            "expected a new variable name after '=' in RENAME=",
                            new_tok.span,
                        ));
                    };
                    validate_sas_name(&new, new_tok.span)?;
                    self.next();
                    pairs.push((old, new));
                }
                _ => {
                    return Err(SasError::parse(
                        "expected an old=new pair or ')' in RENAME=",
                        tok.span,
                    ));
                }
            }
        }
        if pairs.is_empty() {
            return Err(SasError::parse(
                "expected at least one old=new pair in RENAME=",
                self.peek().span,
            ));
        }
        Ok(pairs)
    }

    /// `where = (expr)` — parenthésé, une expression.
    fn parse_where_option(&mut self) -> Result<crate::ast::Expr> {
        if self.peek().kind != TokenKind::LParen {
            return Err(SasError::parse(
                "The WHERE= dataset option requires a parenthesized expression.",
                self.peek().span,
            ));
        }
        self.next(); // `(`
        let cond = expr::parse_expr(self)?;
        if self.peek().kind != TokenKind::RParen {
            return Err(SasError::parse(
                "expected ')' after the WHERE= expression",
                self.peek().span,
            ));
        }
        self.next(); // `)`
        Ok(cond)
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
