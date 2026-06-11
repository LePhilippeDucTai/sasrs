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

#![allow(unused_variables, dead_code)]

pub mod datastep;
pub mod expr;
pub mod global;

use crate::ast::{DataStepAst, DatasetRef, GlobalStmt};
use crate::error::Result;
use crate::procs::ProcAst;
use crate::source::SourceFile;
use crate::token::{Span, Token};

pub enum Block {
    Global(GlobalStmt),
    DataStep(DataStepAst),
    Proc { name: String, ast: ProcAst },
    /// `run;` isolé ou statement vide : écho dans le log, aucune action.
    Empty,
}

pub struct StatementStream<'a> {
    pub src: &'a SourceFile,
    toks: Vec<Token>,
    pos: usize,
}

impl<'a> StatementStream<'a> {
    pub fn new(src: &'a SourceFile) -> Result<Self> {
        todo!("lexer la source et initialiser pos=0")
    }

    pub fn peek(&self) -> &Token {
        todo!("token courant sans avancer (Eof garanti en fin de vec)")
    }

    pub fn next(&mut self) -> Token {
        todo!("token courant puis avancer (rester sur Eof)")
    }

    pub fn at_eof(&self) -> bool {
        todo!()
    }

    /// Consomme un `;` ou signale une erreur de syntaxe.
    pub fn expect_semi(&mut self) -> Result<()> {
        todo!()
    }

    /// Saute jusqu'au prochain `;` inclus (récupération d'erreur).
    pub fn skip_to_semi(&mut self) {
        todo!()
    }

    /// Saute jusqu'après `run;`/`quit;`, ou s'arrête juste avant un
    /// `data`/`proc` en début de statement (frontière implicite).
    pub fn skip_to_step_boundary(&mut self) {
        todo!()
    }

    /// `ident [ . ident ]` → DatasetRef. Valide les noms SAS (≤32 chars).
    pub fn parse_dataset_ref(&mut self) -> Result<DatasetRef> {
        todo!()
    }

    /// Liste de noms de variables jusqu'au `;` (non consommé).
    pub fn parse_name_list(&mut self) -> Result<Vec<String>> {
        todo!()
    }

    /// Bloc suivant + span couvert (pour l'écho du log). `None` à EOF.
    pub fn next_block(&mut self) -> Option<(Result<Block>, Span)> {
        todo!("cf. algorithme en tête de fichier")
    }
}
