//! Parser de l'étape DATA (sous-ensemble M1 ; M2+ étend).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! Appelé par `parser::next_block()` APRÈS consommation du mot-clé `data`.
//!
//! ## Statement DATA
//! `data ref [ref]* ;` — une ou plusieurs sorties (`DatasetRef`).
//! `data _null_;` → zéro sortie (reconnaître `_NULL_`, insensible casse).
//!
//! ## Statements du corps (boucle jusqu'à `run;` ou frontière implicite)
//! - `set ref ;`                  → `DsStmt::Set` (M1 : un seul dataset,
//!                                  pas d'options ; plusieurs → ERROR
//!                                  "not yet implemented")
//! - `ident = expr ;`             → `DsStmt::Assign`
//! - `if expr then stmt [else stmt]` → `DsStmt::If` ; les branches sont
//!   UN statement (récunsion sur le parseur de statement) ; `do; ...
//!   end;` permet les blocs.
//! - `if expr ;`                  → `DsStmt::SubsettingIf`
//! - `do ; stmts end ;`           → `DsStmt::Block` (M1 : non itératif
//!                                  seulement ; `do i = ...` → ERROR M2)
//! - `output ;`                   → `DsStmt::Output` (M1 : sans cible)
//! - `keep v1 v2... ;` / `drop ... ;`
//! - `stop ;`
//! - mot-clé inconnu (retain, merge, array, length, where, ...) → ERROR
//!   "Statement XXX is not yet implemented", l'étape entière est invalide
//!   (comme une erreur de compilation SAS : "step not executed") mais on
//!   CONTINUE de parser jusqu'à la frontière pour ne pas désynchroniser.
//!
//! Renvoie `DataStepAst { outputs, stmts, span }`. Si erreurs accumulées,
//! renvoyer la première (l'exécuteur loggue et saute le bloc).
//!
//! ## Choix d'implémentation
//!
//! ### Frontière implicite
//! En début de statement du corps, si le token de tête est un identifiant
//! qui ouvre un bloc (`data`/`proc`/`libname`/`options`/`title`n — la même
//! notion que `StatementStream::skip_to_step_boundary`, via `is_block_head_kw`),
//! ou si l'on atteint EOF, l'étape se termine SANS consommer ce token : le
//! `next_block()` suivant reprendra dessus. Un `run;` explicite, lui, est
//! consommé (`run` puis le `;`). On accepte aussi `quit;` comme terminateur
//! par robustesse, mais DATA emploie `run;`.
//!
//! ### Resynchronisation sur erreur
//! Une erreur dans le corps (statement non implémenté, syntaxe invalide,
//! `set` multi-dataset, `do` itératif...) n'interrompt PAS le parsing : on
//! mémorise la première erreur rencontrée puis on saute jusqu'au `;` du
//! statement fautif (`skip_to_semi`) et on poursuit la boucle. Ainsi, à la
//! fin, le stream est positionné APRÈS le `run;` (ou sur la frontière
//! implicite), prêt pour le bloc suivant, même quand l'étape est invalide.
//! Si au moins une erreur a été accumulée, `parse_data_step` la renvoie : le
//! `parse_block()` appelant attache alors le `skip_to_step_boundary` de
//! récupération, qui est ici un no-op puisqu'on est déjà à la frontière.
//!
//! ### Span
//! Le span couvre du premier token après `data` (déjà consommé par
//! l'appelant) jusqu'à la fin du dernier token consommé par l'étape (le `;`
//! du `run;`, ou la fin du dernier statement avant une frontière implicite).
//! Approximation raisonnable : on lit `start` sur le token de tête du
//! statement DATA et `end` via `prev_end()` à la sortie de la boucle.

#![allow(unused_variables, dead_code)]

use super::{is_block_head_kw, StatementStream};
use crate::ast::{DataStepAst, DsStmt};
use crate::error::{Result, SasError};
use crate::token::{Span, TokenKind};

pub fn parse_data_step(ts: &mut StatementStream) -> Result<DataStepAst> {
    let start = ts.peek().span.start;

    // --- Statement DATA : sorties ou _NULL_ ---
    let outputs = parse_data_outputs(ts)?;
    ts.expect_semi()?;

    // --- Corps : boucle jusqu'à `run;` / `quit;` ou frontière implicite ---
    let mut stmts = Vec::new();
    let mut first_err: Option<SasError> = None;

    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Eof => break,
            TokenKind::Semi => {
                // Statement vide.
                ts.next();
            }
            TokenKind::Star => {
                // Commentaire-statement `* texte ;` : sauter silencieusement.
                ts.skip_to_semi();
            }
            TokenKind::Ident(s) => {
                let lower = s.to_ascii_lowercase();
                if lower == "run" || lower == "quit" {
                    ts.next(); // run / quit
                    if ts.peek().kind == TokenKind::Semi {
                        ts.next();
                    }
                    break;
                }
                if is_block_head_kw(&lower) {
                    // Frontière implicite : NE PAS consommer le mot-clé.
                    break;
                }
                match parse_statement(ts) {
                    Ok(stmt) => stmts.push(stmt),
                    Err(e) => {
                        if first_err.is_none() {
                            first_err = Some(e);
                        }
                        // Resynchroniser jusqu'au `;` du statement fautif.
                        ts.skip_to_semi();
                    }
                }
            }
            _ => {
                // Tête de statement inattendue (ex. `=`, `(`...).
                let e = SasError::parse(
                    "expected a DATA step statement",
                    tok.span,
                );
                if first_err.is_none() {
                    first_err = Some(e);
                }
                ts.skip_to_semi();
            }
        }
    }

    if let Some(e) = first_err {
        return Err(e);
    }

    let end = ts.prev_end().max(start);
    Ok(DataStepAst {
        outputs,
        stmts,
        span: Span::new(start, end),
    })
}

/// Parse la liste de sorties du statement `data` (jusqu'au `;`, non
/// consommé). `_NULL_` (insensible casse) → zéro sortie.
fn parse_data_outputs(ts: &mut StatementStream) -> Result<Vec<crate::ast::DatasetRef>> {
    // Cas `data _null_;`.
    if ts.peek().is_kw("_null_") {
        ts.next();
        return Ok(Vec::new());
    }
    let mut outputs = Vec::new();
    while ts.peek().ident().is_some() {
        // `_null_` ne peut apparaître qu'en première position seule ; ici
        // tout ident est traité comme un nom de dataset de sortie.
        outputs.push(ts.parse_dataset_ref()?);
    }
    if outputs.is_empty() {
        return Err(SasError::parse(
            "expected a dataset name or _NULL_ after DATA",
            ts.peek().span,
        ));
    }
    Ok(outputs)
}

/// Un statement du corps (récursif pour IF/THEN/ELSE et DO/END).
///
/// À l'entrée, `ts.peek()` est un `Ident` non-frontière et différent de
/// `run`/`quit` (garanti par l'appelant pour le niveau supérieur ; pour les
/// récursions internes, vérifié localement). Au retour Ok, le `;` final du
/// statement a été consommé (sauf pour `if/then` dont le terminateur est
/// celui de la branche).
fn parse_statement(ts: &mut StatementStream) -> Result<DsStmt> {
    let tok = ts.peek().clone();
    let head = match tok.ident() {
        Some(s) => s.to_ascii_lowercase(),
        None => {
            return Err(SasError::parse(
                "expected a DATA step statement",
                tok.span,
            ));
        }
    };

    match head.as_str() {
        "set" => parse_set(ts),
        "if" => parse_if(ts),
        "do" => parse_do(ts),
        "output" => {
            ts.next();
            ts.expect_semi()?;
            Ok(DsStmt::Output)
        }
        "stop" => {
            ts.next();
            ts.expect_semi()?;
            Ok(DsStmt::Stop)
        }
        "keep" => {
            ts.next();
            let names = ts.parse_name_list()?;
            ts.expect_semi()?;
            Ok(DsStmt::Keep(names))
        }
        "drop" => {
            ts.next();
            let names = ts.parse_name_list()?;
            ts.expect_semi()?;
            Ok(DsStmt::Drop(names))
        }
        // `end` ne devrait pas apparaître en tête hors d'un bloc `do`.
        "end" => Err(SasError::parse(
            "no matching DO for END.",
            tok.span,
        )),
        _ => {
            // Mot-clé connu de SAS mais non implémenté en M1, OU assignation
            // `ident = expr;`. `StatementStream` n'expose pas de peek2, donc
            // on consomme l'ident de tête puis on inspecte le token suivant :
            // un `=` → assignation ; sinon → statement non implémenté. Le
            // span d'erreur est celui de l'ident de tête (déjà cloné).
            let var = tok
                .ident()
                .expect("matched an Ident head above")
                .to_string();
            ts.next(); // ident de tête
            if ts.peek().kind == TokenKind::Eq {
                ts.next(); // `=`
                let expr = super::expr::parse_expr(ts)?;
                ts.expect_semi()?;
                Ok(DsStmt::Assign { var, expr })
            } else {
                Err(SasError::parse(
                    format!(
                        "Statement {} is not yet implemented.",
                        head.to_uppercase()
                    ),
                    tok.span,
                ))
            }
        }
    }
}

/// `set ref ;` — M1 : exactement un dataset, sans options.
fn parse_set(ts: &mut StatementStream) -> Result<DsStmt> {
    let set_tok = ts.peek().clone();
    ts.next(); // `set`
    if ts.peek().kind == TokenKind::Semi {
        // `set;` sans dataset : non supporté en M1.
        return Err(SasError::parse(
            "Statement SET without a dataset is not yet implemented.",
            set_tok.span,
        ));
    }
    let ds = ts.parse_dataset_ref()?;
    // Un seul dataset, sans options : le token suivant doit être `;`.
    if ts.peek().kind != TokenKind::Semi {
        return Err(SasError::parse(
            "SET with multiple datasets or data set options is not yet implemented.",
            ts.peek().span,
        ));
    }
    ts.expect_semi()?;
    Ok(DsStmt::Set(ds))
}

/// `if expr then stmt [else stmt]` ou `if expr ;` (subsetting).
fn parse_if(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `if`
    let cond = super::expr::parse_expr(ts)?;
    if ts.peek().is_kw("then") {
        ts.next(); // `then`
        let then_branch = Box::new(parse_branch_statement(ts)?);
        let else_branch = if ts.peek().is_kw("else") {
            ts.next(); // `else`
            Some(Box::new(parse_branch_statement(ts)?))
        } else {
            None
        };
        Ok(DsStmt::If {
            cond,
            then_branch,
            else_branch,
        })
    } else if ts.peek().kind == TokenKind::Semi {
        ts.next(); // `;`
        Ok(DsStmt::SubsettingIf(cond))
    } else {
        Err(SasError::parse(
            "expected THEN or ';' after the IF condition",
            ts.peek().span,
        ))
    }
}

/// Une branche de IF/THEN ou IF/ELSE : UN statement (récursion). Les
/// frontières de bloc ou `run`/`quit` ne peuvent pas servir de branche.
fn parse_branch_statement(ts: &mut StatementStream) -> Result<DsStmt> {
    let tok = ts.peek().clone();
    if let Some(s) = tok.ident() {
        let lower = s.to_ascii_lowercase();
        if lower == "run" || lower == "quit" || is_block_head_kw(&lower) {
            return Err(SasError::parse(
                "expected a statement after THEN/ELSE",
                tok.span,
            ));
        }
    }
    parse_statement(ts)
}

/// `do ; stmts end ;` — M1 : bloc non itératif uniquement. `do i = ...`,
/// `do while (...)`, `do until (...)` → ERROR M2.
fn parse_do(ts: &mut StatementStream) -> Result<DsStmt> {
    let do_tok = ts.peek().clone();
    ts.next(); // `do`
    // Forme non itérative : `do` immédiatement suivi de `;`.
    if ts.peek().kind != TokenKind::Semi {
        // `do i = ...`, `do while`, `do until` : itératif/conditionnel.
        return Err(SasError::parse(
            "Iterative and conditional DO loops are not yet implemented.",
            do_tok.span,
        ));
    }
    ts.next(); // `;`
    let mut body = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Eof => {
                return Err(SasError::parse(
                    "missing END for DO block.",
                    tok.span,
                ));
            }
            TokenKind::Semi => {
                ts.next();
            }
            TokenKind::Star => {
                ts.skip_to_semi();
            }
            TokenKind::Ident(s) if s.eq_ignore_ascii_case("end") => {
                ts.next(); // `end`
                ts.expect_semi()?;
                return Ok(DsStmt::Block(body));
            }
            TokenKind::Ident(s) => {
                let lower = s.to_ascii_lowercase();
                if lower == "run" || lower == "quit" || is_block_head_kw(&lower) {
                    // Frontière atteinte sans END : DO non clos.
                    return Err(SasError::parse(
                        "missing END for DO block.",
                        tok.span,
                    ));
                }
                body.push(parse_statement(ts)?);
            }
            _ => {
                return Err(SasError::parse(
                    "expected a DATA step statement inside DO block",
                    tok.span,
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BinaryOp, DatasetRef, Expr};
    use crate::source::SourceFile;

    /// Parse une étape DATA en supposant le mot-clé `data` déjà consommé.
    fn parse(src: &str) -> Result<DataStepAst> {
        let file = SourceFile::new(src);
        let mut ts = StatementStream::new(&file).unwrap();
        // Consommer le `data` de tête comme le fait next_block().
        assert!(ts.peek().is_kw("data"), "test source must start with DATA");
        ts.next();
        parse_data_step(&mut ts)
    }

    fn dsref(name: &str) -> DatasetRef {
        DatasetRef {
            libref: None,
            name: name.to_string(),
        }
    }

    fn var(s: &str) -> Expr {
        Expr::Var(s.to_string())
    }

    #[test]
    fn simple_set_assign_run() {
        let ast = parse("data out; set inp; x = 1; run;").unwrap();
        assert_eq!(ast.outputs, vec![dsref("out")]);
        assert_eq!(
            ast.stmts,
            vec![
                DsStmt::Set(dsref("inp")),
                DsStmt::Assign {
                    var: "x".to_string(),
                    expr: Expr::Num(1.0),
                },
            ]
        );
        // Le span débute au token `out` (juste après `data `).
        assert_eq!(ast.span.start, "data ".len());
    }

    #[test]
    fn data_null_has_no_outputs() {
        let ast = parse("data _null_; stop; run;").unwrap();
        assert!(ast.outputs.is_empty());
        assert_eq!(ast.stmts, vec![DsStmt::Stop]);
    }

    #[test]
    fn data_null_case_insensitive() {
        let ast = parse("data _NULL_; run;").unwrap();
        assert!(ast.outputs.is_empty());
    }

    #[test]
    fn multiple_outputs() {
        let ast = parse("data a b lib.c; set d; run;").unwrap();
        assert_eq!(
            ast.outputs,
            vec![
                dsref("a"),
                dsref("b"),
                DatasetRef {
                    libref: Some("lib".to_string()),
                    name: "c".to_string(),
                },
            ]
        );
    }

    #[test]
    fn if_then_else_nested() {
        let ast = parse(
            "data o; set i; if x = 1 then y = 10; else if x = 2 then y = 20; else y = 0; run;",
        )
        .unwrap();
        // Structure : Set, puis un If avec else=If(else=Assign).
        assert_eq!(ast.stmts.len(), 2);
        let DsStmt::If {
            cond,
            then_branch,
            else_branch,
        } = &ast.stmts[1]
        else {
            panic!("expected an IF statement");
        };
        assert_eq!(
            *cond,
            Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(var("x")),
                right: Box::new(Expr::Num(1.0)),
            }
        );
        assert_eq!(
            **then_branch,
            DsStmt::Assign {
                var: "y".to_string(),
                expr: Expr::Num(10.0),
            }
        );
        // Le else est un IF imbriqué.
        let Some(else_b) = else_branch else {
            panic!("expected an else branch");
        };
        let DsStmt::If {
            else_branch: inner_else,
            ..
        } = &**else_b
        else {
            panic!("expected a nested IF in the else branch");
        };
        assert_eq!(
            **inner_else.as_ref().unwrap(),
            DsStmt::Assign {
                var: "y".to_string(),
                expr: Expr::Num(0.0),
            }
        );
    }

    #[test]
    fn subsetting_if() {
        let ast = parse("data o; set i; if x > 5; run;").unwrap();
        assert_eq!(ast.stmts.len(), 2);
        assert_eq!(
            ast.stmts[1],
            DsStmt::SubsettingIf(Expr::Binary {
                op: BinaryOp::Gt,
                left: Box::new(var("x")),
                right: Box::new(Expr::Num(5.0)),
            })
        );
    }

    #[test]
    fn non_iterative_do_block() {
        let ast = parse("data o; set i; if x then do; y = 1; output; end; run;").unwrap();
        assert_eq!(ast.stmts.len(), 2);
        let DsStmt::If { then_branch, .. } = &ast.stmts[1] else {
            panic!("expected an IF");
        };
        assert_eq!(
            **then_branch,
            DsStmt::Block(vec![
                DsStmt::Assign {
                    var: "y".to_string(),
                    expr: Expr::Num(1.0),
                },
                DsStmt::Output,
            ])
        );
    }

    #[test]
    fn output_keep_drop_stop() {
        let ast = parse("data o; set i; output; keep a b; drop c; stop; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![
                DsStmt::Set(dsref("i")),
                DsStmt::Output,
                DsStmt::Keep(vec!["a".to_string(), "b".to_string()]),
                DsStmt::Drop(vec!["c".to_string()]),
                DsStmt::Stop,
            ]
        );
    }

    #[test]
    fn set_two_datasets_errors() {
        let err = parse("data o; set a b; run;").unwrap_err();
        assert!(err.to_string().contains("not yet implemented"));
    }

    #[test]
    fn unimplemented_statement_errors_but_resyncs() {
        // `retain` n'est pas implémenté en M1. L'étape doit échouer MAIS le
        // stream doit être positionné après le `run;` pour le bloc suivant.
        let file = SourceFile::new("data o; retain x 0; set i; run; data b; run;");
        let mut ts = StatementStream::new(&file).unwrap();
        assert!(ts.next().is_kw("data"));
        let err = parse_data_step(&mut ts).unwrap_err();
        assert!(err.to_string().to_uppercase().contains("RETAIN"));
        assert!(err.to_string().contains("not yet implemented"));
        // Resynchronisation : on est sur le `data` de la deuxième étape.
        assert!(ts.peek().is_kw("data"));
        ts.next();
        let ast2 = parse_data_step(&mut ts).unwrap();
        assert_eq!(ast2.outputs, vec![dsref("b")]);
    }

    #[test]
    fn implicit_boundary_without_run() {
        // Pas de `run;` : un `data b;` qui suit clôt l'étape sans être
        // consommé.
        let file = SourceFile::new("data a; set x; data b; set y; run;");
        let mut ts = StatementStream::new(&file).unwrap();
        assert!(ts.next().is_kw("data"));
        let ast1 = parse_data_step(&mut ts).unwrap();
        assert_eq!(ast1.outputs, vec![dsref("a")]);
        assert_eq!(ast1.stmts, vec![DsStmt::Set(dsref("x"))]);
        // Frontière implicite : `data` non consommé.
        assert!(ts.peek().is_kw("data"));
        ts.next();
        let ast2 = parse_data_step(&mut ts).unwrap();
        assert_eq!(ast2.outputs, vec![dsref("b")]);
        assert_eq!(ast2.stmts, vec![DsStmt::Set(dsref("y"))]);
    }

    #[test]
    fn iterative_do_errors_m2() {
        let err = parse("data o; do i = 1 to 10; output; end; run;").unwrap_err();
        assert!(err.to_string().contains("not yet implemented"));
    }

    #[test]
    fn comment_statement_in_body_is_skipped() {
        let ast = parse("data o; set i; * this is a comment ; x = 1; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![
                DsStmt::Set(dsref("i")),
                DsStmt::Assign {
                    var: "x".to_string(),
                    expr: Expr::Num(1.0),
                },
            ]
        );
    }

    #[test]
    fn empty_statements_are_skipped() {
        let ast = parse("data o; set i;; ; x = 1; run;").unwrap();
        assert_eq!(ast.stmts.len(), 2);
    }

    #[test]
    fn data_without_output_name_errors() {
        let file = SourceFile::new("data ; run;");
        let mut ts = StatementStream::new(&file).unwrap();
        assert!(ts.next().is_kw("data"));
        assert!(parse_data_step(&mut ts).is_err());
    }
}
