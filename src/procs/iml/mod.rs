//! PROC IML — Interactive Matrix Language (M28a.1 + M28a.2).
//!
//! PROC IML est un **sous-langage à part entière** : il possède son propre
//! lexer/parser/évaluateur, indépendants du parser SAS principal. Ce choix est
//! imposé par des collisions de surface avec la syntaxe SAS :
//!
//! - `'` en fin d'expression = **transposée** (pas un délimiteur de chaîne) ;
//! - `*` = **produit matriciel** ; `#` = **produit de Hadamard** ;
//! - `{1 2, 3 4}` = littéral matriciel (espace = élément, `,` = nouvelle ligne) ;
//! - `QUIT;` termine le bloc (pas `RUN;`).
//!
//! Le lexer SAS (`src/lexer.rs`) reconnaît `proc iml`, capture le texte brut du
//! corps jusqu'au `quit;`, et l'émet comme `TokenKind::ImlBody(String)`. Ce
//! module re-lexe cette chaîne avec sa propre grammaire.
//!
//! **Type de valeur IML** : toujours une matrice `Vec<Vec<f64>>` (les scalaires
//! sont des matrices 1×1).
//!
//! ## Périmètre
//! - M28a.1 : littéraux matriciels, indexation 1-basée (scalaire / ligne `*` /
//!   colonne `*`), opérateurs (`'` transposée, `-` unaire, `@` Kronecker, `*`
//!   produit, `#` Hadamard, `/` division scalaire, `+`/`-`, comparaisons),
//!   fonctions `NROW`/`NCOL`/`DIM`/`T`, statement d'assignation, `PRINT`.
//! - M28a.2 : contrôle de flux (`IF/THEN/ELSE`, `DO i=a TO b [BY c]`,
//!   `DO WHILE`, `DO UNTIL`) et fonctions statistiques élémentaires (`MEAN`,
//!   `SUM`, `STD`, `MIN`, `MAX`, `ABS`, `SQRT`, `EXP`, `LOG`).
//! - M28a.3 : algèbre linéaire — `INV`, `SOLVE`, `EIGVAL` (symétrique),
//!   `CHOL` (upper, convention SAS), `CALL QR(Q, R, A)`,
//!   `CALL SVDCD(U, D, V, A)` (méthode ATA-Jacobi).
//! - M28a.4 : I/O datasets — `CREATE ds FROM mat[COLNAME=cn]`, `APPEND FROM`,
//!   `CLOSE`, `USE`, `READ ALL VAR {..} INTO mat`. Différés : `READ NEXT`,
//!   `WHERE`, `LOAD`/`STORE`/`SHOW`.
//! - M34.10 : `SHAPE(x, nrow [, ncol])` (reshape row-major avec recyclage),
//!   sous-matrices à intervalle `a[1:2, 1:3]`, `a[2:3, *]`, `DET(A)`,
//!   `EIGVEC(A)` et `CALL EIGEN(values, vectors, A)` (symétrique).
//!
//! ## Contrat (J02-P5, CONTRIBUTING §5)
//! Rien n'est ignoré en silence : option du statement PROC IML autre que
//! SYMSIZE=/WORKSIZE= → ERROR ; options PRINT `[…]` → WARNING d'affichage ;
//! instruction SAS/IML valide non implémentée (START, STORE, READ NEXT…) →
//! ERROR « not supported … cannot be ignored » au parsing ; instruction
//! inconnue → ERROR 180-322 (repli partagé `common::unhandled_proc_statement`).
//! MEAN/STD sont des statistiques par colonne, SUM/MIN/MAX portent sur tous
//! leurs arguments ; division par zéro → WARNING et valeur manquante ;
//! LOG/SQRT hors domaine → ERROR ; une valeur manquante s'imprime `.`.
//! Détail : `docs/support-contract.md`.

use crate::error::{Result, SasError};
use crate::session::Session;
use std::collections::HashMap;

mod ast;
mod env;
mod eval;
mod exec;
mod lexer;
mod matrix;
mod parser;
mod render;

pub use ast::ImlExpr;
pub use ast::ImlIndex;
pub use ast::ImlOp;
pub use ast::ImlPrintItem;
pub use ast::ImlProgram;
pub use ast::ImlStmt;
pub use ast::UnaryOp;
pub use parser::parse;
pub use parser::parse_body;

use env::*;
use eval::*;
use exec::*;
use lexer::*;
use matrix::*;
use render::*;

// ───────────────────────── Évaluateur ─────────────────────────

type Matrix = Vec<Vec<f64>>;

/// Tampon d'un dataset ouvert en écriture par CREATE/APPEND/CLOSE.
struct OpenWrite {
    colnames: Vec<String>,
    rows: Vec<Vec<f64>>,
}

/// Exécute un programme IML.
///
/// J02-P5 — la sortie PRINT des instructions exécutées avant une erreur
/// d'exécution est rendue (SAS l'imprime au fil de l'exécution ; elle était
/// perdue). L'étape s'arrête à la première erreur (SAS reprendrait à
/// l'instruction suivante) : les tables encore ouvertes ne sont alors pas
/// écrites (WARNING). Sans erreur, QUIT ferme et écrit toutes les tables
/// ouvertes (SAS/IML 9.4, CLOSE statement).
pub fn execute(prog: &ImlProgram, session: &mut Session) -> Result<()> {
    let mut env = Env::new();
    let mut ops: Vec<PrintOp> = Vec::new();
    let result = exec_stmts(&prog.stmts, &mut env, &mut ops, session);
    render_output(&ops, session);
    match result {
        Ok(()) => close_open_writes(&mut env, session),
        Err(e) => {
            for (name, _) in std::mem::take(&mut env.open_writes) {
                session.log.warning(&format!(
                    "The data set {name} was not written because PROC IML stopped at an \
                     execution error."
                ));
            }
            Err(e)
        }
    }
}

/// Rend la sortie PRINT capturée ; aucune page quand rien n'a été imprimé.
fn render_output(ops: &[PrintOp], session: &mut Session) {
    if ops.is_empty() {
        return;
    }
    session.listing.page_header();
    let ls = session.listing.ls();
    let pad = ls.saturating_sub("The IML Procedure".len()) / 2;
    session
        .listing
        .write_line(&format!("{}{}", " ".repeat(pad), "The IML Procedure"));
    session.listing.blank();

    for op in ops {
        match op {
            PrintOp::Text(t) => {
                session.listing.write_line(t);
                session.listing.blank();
            }
            PrintOp::Matrix { name, m } => render_matrix(name, m, session),
        }
    }
}

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod tests;
