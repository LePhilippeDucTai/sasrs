//! Parser de l'étape DATA (sous-ensemble M1 ; M2+ étend).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! Appelé par `parser::next_block()` APRÈS consommation du mot-clé `data`.
//!
//! ## Statement DATA
//! `data spec [spec]* ;` — une ou plusieurs sorties (`DatasetSpec`,
//! chacune avec ses options de dataset `(keep=/drop=/rename=/where=)`).
//! `data _null_;` → zéro sortie (reconnaître `_NULL_`, insensible casse).
//!
//! ## Statements du corps (boucle jusqu'à `run;` ou frontière implicite)
//! - `set spec [spec]* ;`         → `DsStmt::Set` (M3 : un ou plusieurs
//!                                  datasets, options de dataset
//!                                  acceptées sur chacun)
//! - `by [descending] v ... ;`    → `DsStmt::By` (M3)
//! - `ident = expr ;`             → `DsStmt::Assign`
//! - `if expr then stmt [else stmt]` → `DsStmt::If` ; les branches sont
//!   UN statement (récunsion sur le parseur de statement) ; `do; ...
//!   end;` permet les blocs.
//! - `if expr ;`                  → `DsStmt::SubsettingIf`
//! - `do ; stmts end ;`           → `DsStmt::Block` (non itératif)
//! - `do i = e1 [to e2] [by e3] [while(c)] [until(c)]; ... end;`,
//!   `do while(c); ... end;`, `do until(c); ... end;` → `DsStmt::DoLoop`
//!   (M2) ; liste de valeurs `do i = 1, 5, 9;` → ERROR "not yet
//!   implemented"
//! - `output [ref...] ;`          → `DsStmt::Output(Vec<DatasetRef>)`
//!   (liste vide = toutes les sorties ; `output a b;` écrit dans a ET b)
//! - `delete ;`                   → `DsStmt::Delete` (M2)
//! - `keep v1 v2... ;` / `drop ... ;`
//! - `stop ;`
//! - `retain [v [init]]... ;`     → `DsStmt::Retain` (M2) ; init = littéral
//!   Num (avec `-` unaire replié), Str ou missing (`.`/`.a`...)
//! - `length v... [$] n ... ;`    → `DsStmt::Length` (M2)
//! - `var + expr ;`               → `DsStmt::Sum` (M2 ; PAS de forme `-`)
//! - mot-clé inconnu (merge, array, where, ...) → ERROR
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
//! Une erreur dans le corps (statement non implémenté, syntaxe
//! invalide...) n'interrompt PAS le parsing : on
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

use super::{is_block_head_kw, validate_sas_name, StatementStream};
use crate::ast::{AttribItem, DataStepAst, DsStmt, Expr, LengthSpec};
use crate::error::{Result, SasError};
use crate::token::{Span, StrSuffix, TokenKind};
use crate::value::MissingKind;

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
/// consommé). Chaque sortie est un `DatasetSpec` (options de dataset
/// acceptées). `_NULL_` (insensible casse) → zéro sortie.
fn parse_data_outputs(ts: &mut StatementStream) -> Result<Vec<crate::ast::DatasetSpec>> {
    // Cas `data _null_;`.
    if ts.peek().is_kw("_null_") {
        ts.next();
        return Ok(Vec::new());
    }
    let mut outputs = Vec::new();
    while ts.peek().ident().is_some() {
        // `_null_` ne peut apparaître qu'en première position seule ; ici
        // tout ident est traité comme un nom de dataset de sortie.
        outputs.push(ts.parse_dataset_spec()?);
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
        "merge" => parse_merge(ts),
        "by" => parse_by(ts),
        "if" => parse_if(ts),
        "do" => parse_do(ts),
        "output" => {
            // `output;` → toutes les sorties (liste vide) ;
            // `output a [b...];` → sorties ciblées (noms seuls, sans
            // options — la validation contre la liste du DATA est faite à
            // la compilation).
            ts.next();
            let mut targets = Vec::new();
            while ts.peek().ident().is_some() {
                targets.push(ts.parse_dataset_ref()?);
            }
            ts.expect_semi()?;
            Ok(DsStmt::Output(targets))
        }
        "delete" => {
            ts.next();
            ts.expect_semi()?;
            Ok(DsStmt::Delete)
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
        "retain" => parse_retain(ts),
        "length" => parse_length(ts),
        "format" => parse_format(ts),
        "label" => parse_label(ts),
        "attrib" => parse_attrib(ts),
        "array" => parse_array(ts),
        "call" => parse_call_routine(ts),
        // `end` ne devrait pas apparaître en tête hors d'un bloc `do`.
        "end" => Err(SasError::parse(
            "no matching DO for END.",
            tok.span,
        )),
        _ => {
            // Mot-clé connu de SAS mais non implémenté, assignation
            // `ident = expr;` OU sum statement `ident + expr;`.
            // `StatementStream` n'expose pas de peek2, donc on consomme
            // l'ident de tête puis on inspecte le token suivant : un `=` →
            // assignation, un `+` → sum statement ; sinon → statement non
            // implémenté. (La forme `var - expr;` N'EXISTE PAS en SAS — un
            // `-` après l'ident tombe dans l'erreur.) Le span d'erreur est
            // celui de l'ident de tête (déjà cloné).
            let var = tok
                .ident()
                .expect("matched an Ident head above")
                .to_string();
            ts.next(); // ident de tête
            match ts.peek().kind {
                TokenKind::Eq => {
                    ts.next(); // `=`
                    let expr = super::expr::parse_expr(ts)?;
                    ts.expect_semi()?;
                    Ok(DsStmt::Assign { var, expr })
                }
                TokenKind::Plus => {
                    ts.next(); // `+`
                    let expr = super::expr::parse_expr(ts)?;
                    ts.expect_semi()?;
                    Ok(DsStmt::Sum { var, expr })
                }
                // `arr{i} = e;` / `arr[i] = e;` : assignation indexée.
                TokenKind::LBrace | TokenKind::LBracket => {
                    let Expr::Index { name, index } = super::expr::parse_index(ts, var)?
                    else {
                        unreachable!("parse_index always returns Expr::Index");
                    };
                    parse_assign_indexed_tail(ts, name, *index)
                }
                // `arr(i) = e;` : forme à parenthèses — le nom sera validé
                // array à la COMPILATION (ici on parse l'indice).
                TokenKind::LParen => {
                    ts.next(); // `(`
                    let index = super::expr::parse_expr(ts)?;
                    if ts.peek().kind == TokenKind::Comma {
                        return Err(SasError::parse(
                            "Multi-dimensional arrays are not yet implemented.",
                            ts.peek().span,
                        ));
                    }
                    if ts.peek().kind != TokenKind::RParen {
                        return Err(SasError::parse(
                            format!(
                                "expected ')' to close the array subscript of {}",
                                var.to_uppercase()
                            ),
                            ts.peek().span,
                        ));
                    }
                    ts.next(); // `)`
                    parse_assign_indexed_tail(ts, var, index)
                }
                _ => Err(SasError::parse(
                    format!(
                        "Statement {} is not yet implemented.",
                        head.to_uppercase()
                    ),
                    tok.span,
                )),
            }
        }
    }
}

/// `set spec [spec]* ;` — un ou plusieurs datasets (M3), chacun avec ses
/// options de dataset.
fn parse_set(ts: &mut StatementStream) -> Result<DsStmt> {
    let set_tok = ts.peek().clone();
    ts.next(); // `set`
    if ts.peek().kind == TokenKind::Semi {
        // `set;` sans dataset : non supporté.
        return Err(SasError::parse(
            "Statement SET without a dataset is not yet implemented.",
            set_tok.span,
        ));
    }
    let mut specs = Vec::new();
    while ts.peek().ident().is_some() {
        specs.push(ts.parse_dataset_spec()?);
    }
    ts.expect_semi()?;
    Ok(DsStmt::Set(specs))
}

/// `merge spec [spec]* ;` — un ou plusieurs datasets (M3), chacun avec ses
/// options de dataset (dont `in=`). Match-merge SAS par BY. La validité (un
/// seul SET/MERGE par étape, présence d'un BY, tri) est tranchée à la
/// compilation/exécution.
fn parse_merge(ts: &mut StatementStream) -> Result<DsStmt> {
    let merge_tok = ts.peek().clone();
    ts.next(); // `merge`
    if ts.peek().kind == TokenKind::Semi {
        return Err(SasError::parse(
            "Statement MERGE without a dataset is not yet implemented.",
            merge_tok.span,
        ));
    }
    let mut specs = Vec::new();
    while ts.peek().ident().is_some() {
        specs.push(ts.parse_dataset_spec()?);
    }
    ts.expect_semi()?;
    Ok(DsStmt::Merge(specs))
}

/// `call <name>(arg [, arg]*);` (M11.5) — appel d'une CALL routine. On
/// parse le nom de la routine, puis une liste d'arguments entre parenthèses
/// (expressions séparées par des virgules ; liste vide autorisée), puis le
/// `;`. La validation de la routine (seule SYMPUT est exécutée en v1) est
/// faite à l'EXÉCUTION : une routine inconnue produit une erreur runtime
/// « not yet implemented », pas une erreur de parsing.
fn parse_call_routine(ts: &mut StatementStream) -> Result<DsStmt> {
    let call_tok = ts.peek().clone();
    ts.next(); // `call`
    let name = match ts.peek().ident() {
        Some(s) => s.to_string(),
        None => {
            return Err(SasError::parse(
                "expected a CALL routine name",
                ts.peek().span,
            ));
        }
    };
    ts.next(); // nom de la routine
    let mut args = Vec::new();
    if ts.peek().kind == TokenKind::LParen {
        ts.next(); // `(`
        if ts.peek().kind != TokenKind::RParen {
            loop {
                args.push(super::expr::parse_expr(ts)?);
                match ts.peek().kind {
                    TokenKind::Comma => {
                        ts.next();
                    }
                    _ => break,
                }
            }
        }
        if ts.peek().kind != TokenKind::RParen {
            return Err(SasError::parse(
                format!(
                    "expected ')' to close the arguments of CALL {}",
                    name.to_uppercase()
                ),
                ts.peek().span,
            ));
        }
        ts.next(); // `)`
    }
    ts.expect_semi()?;
    Ok(DsStmt::CallRoutine { name, args })
}

/// `by [descending] v1 [descending] v2 ... ;` → `DsStmt::By` (M3). Le
/// mot-clé DESCENDING s'applique à la variable qui le SUIT. La validité
/// (présence d'un SET, variables sur les inputs) est tranchée à la
/// compilation.
fn parse_by(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `by`
    let mut items: Vec<(String, bool)> = Vec::new();
    let mut descending = false;
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi => {
                if descending {
                    return Err(SasError::parse(
                        "expected a variable name after DESCENDING in the BY statement",
                        tok.span,
                    ));
                }
                if items.is_empty() {
                    return Err(SasError::parse(
                        "expected a variable name in the BY statement",
                        tok.span,
                    ));
                }
                ts.next();
                return Ok(DsStmt::By(items));
            }
            TokenKind::Ident(name) => {
                let name = name.clone();
                if name.eq_ignore_ascii_case("descending") {
                    if descending {
                        return Err(SasError::parse(
                            "expected a variable name after DESCENDING in the BY statement",
                            tok.span,
                        ));
                    }
                    descending = true;
                } else {
                    validate_sas_name(&name, tok.span)?;
                    items.push((name, descending));
                    descending = false;
                }
                ts.next();
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable name in the BY statement",
                    tok.span,
                ));
            }
        }
    }
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

/// `do ...; stmts end ;` — quatre formes :
/// - `do;` : bloc non itératif → `DsStmt::Block` (chemin M1 conservé) ;
/// - `do i = e1 [to e2] [by e3] [while(c)] [until(c)];` : itératif ;
/// - `do while(c);` / `do until(c);` : conditionnel pur.
///
/// `do i = 1, 5, 9;` (liste de valeurs, y compris à UNE valeur sans
/// clause TO/BY/WHILE/UNTIL) → ERROR "not yet implemented" propre.
/// `while` et `until` ne sont pas réservés : `do while = 1 to 2;` reste
/// un DO itératif d'index `while` (le `=` est inspecté avant le `(`).
fn parse_do(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `do`
    let head = ts.peek().clone();
    match &head.kind {
        // Forme non itérative : `do` immédiatement suivi de `;`.
        TokenKind::Semi => {
            ts.next(); // `;`
            Ok(DsStmt::Block(parse_do_body(ts)?))
        }
        TokenKind::Ident(name) => {
            let name = name.clone();
            let lower = name.to_ascii_lowercase();
            ts.next(); // l'ident (index potentiel, ou while/until)
            if ts.peek().kind == TokenKind::Eq {
                // `do i = ...` : itératif.
                validate_sas_name(&name, head.span)?;
                ts.next(); // `=`
                parse_iterative_do(ts, name, head.span)
            } else if (lower == "while" || lower == "until")
                && ts.peek().kind == TokenKind::LParen
            {
                // `do while(c);` / `do until(c);` : conditionnel pur.
                let cond = parse_paren_cond(ts)?;
                ts.expect_semi()?;
                let body = parse_do_body(ts)?;
                let (while_, until) = if lower == "while" {
                    (Some(cond), None)
                } else {
                    (None, Some(cond))
                };
                Ok(DsStmt::DoLoop {
                    index: None,
                    to: None,
                    by: None,
                    while_,
                    until,
                    body,
                })
            } else {
                Err(SasError::parse(
                    "expected '=', WHILE(...) or UNTIL(...) after DO",
                    head.span,
                ))
            }
        }
        _ => Err(SasError::parse(
            "expected ';', an index variable, WHILE(...) or UNTIL(...) after DO",
            head.span,
        )),
    }
}

/// Clauses d'un DO itératif après `do index =` : `from [to e] [by e]`
/// (TO/BY acceptés dans les deux ordres, comme SAS) puis WHILE/UNTIL en
/// ordre quelconque, UN seul de chaque. Termine sur le `;` puis parse le
/// corps jusqu'à `end;`.
fn parse_iterative_do(
    ts: &mut StatementStream,
    index_name: String,
    index_span: Span,
) -> Result<DsStmt> {
    let from = super::expr::parse_expr(ts)?;
    if ts.peek().kind == TokenKind::Comma {
        return Err(SasError::parse(
            "DO loops over a list of values are not yet implemented.",
            ts.peek().span,
        ));
    }
    let mut to: Option<Expr> = None;
    let mut by: Option<Expr> = None;
    let mut while_: Option<Expr> = None;
    let mut until: Option<Expr> = None;
    loop {
        let tok = ts.peek().clone();
        let Some(kw) = tok.ident().map(str::to_ascii_lowercase) else {
            break;
        };
        match kw.as_str() {
            "to" if to.is_none() => {
                ts.next();
                to = Some(super::expr::parse_expr(ts)?);
            }
            "by" if by.is_none() => {
                ts.next();
                by = Some(super::expr::parse_expr(ts)?);
            }
            "while" if while_.is_none() => {
                ts.next();
                while_ = Some(parse_paren_cond(ts)?);
            }
            "until" if until.is_none() => {
                ts.next();
                until = Some(parse_paren_cond(ts)?);
            }
            "to" | "by" | "while" | "until" => {
                return Err(SasError::parse(
                    format!("duplicate {} clause in the DO statement", kw.to_uppercase()),
                    tok.span,
                ));
            }
            _ => break,
        }
    }
    // Pas de clause du tout : `do i = 1;` est une liste de valeurs à un
    // élément → même erreur "not yet implemented" que la forme à virgules.
    if to.is_none() && by.is_none() && while_.is_none() && until.is_none() {
        return Err(SasError::parse(
            "DO loops over a list of values are not yet implemented.",
            index_span,
        ));
    }
    ts.expect_semi()?;
    let body = parse_do_body(ts)?;
    Ok(DsStmt::DoLoop {
        index: Some((index_name, from)),
        to,
        by,
        while_,
        until,
        body,
    })
}

/// `( expr )` après WHILE/UNTIL.
fn parse_paren_cond(ts: &mut StatementStream) -> Result<Expr> {
    let tok = ts.peek().clone();
    if tok.kind != TokenKind::LParen {
        return Err(SasError::parse(
            "expected '(' after WHILE/UNTIL in the DO statement",
            tok.span,
        ));
    }
    ts.next(); // `(`
    let cond = super::expr::parse_expr(ts)?;
    let tok = ts.peek().clone();
    if tok.kind != TokenKind::RParen {
        return Err(SasError::parse(
            "expected ')' after the WHILE/UNTIL condition",
            tok.span,
        ));
    }
    ts.next(); // `)`
    Ok(cond)
}

/// Corps d'un DO (toutes formes) : statements jusqu'au `end ;` (consommé).
fn parse_do_body(ts: &mut StatementStream) -> Result<Vec<DsStmt>> {
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
                return Ok(body);
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

/// Fin commune d'une assignation indexée : l'indice est parsé, il reste
/// `= expr ;`.
fn parse_assign_indexed_tail(
    ts: &mut StatementStream,
    array: String,
    index: Expr,
) -> Result<DsStmt> {
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            format!(
                "expected '=' after the array reference {}",
                array.to_uppercase()
            ),
            ts.peek().span,
        ));
    }
    ts.next(); // `=`
    let expr = super::expr::parse_expr(ts)?;
    ts.expect_semi()?;
    Ok(DsStmt::AssignIndexed { array, index, expr })
}

/// `array arr{3} x y z;` — déclaration d'array 1-D (M2). Délimiteurs de
/// dimension interchangeables (`{}`, `[]`, `()` — fermant assorti).
/// Formes : `{n}` taille explicite, `{*}` taille déduite de la liste ;
/// `$ [len]` array caractère (longueur défaut 8) ; liste de variables
/// optionnelle (vide → éléments auto-nommés à la compilation), plages
/// numérotées `x1-x3` expansées ICI. Hors périmètre M2 → erreurs propres :
/// multi-dimensions, valeurs initiales `(...)`, `_temporary_` et listes
/// spéciales `_numeric_`/`_character_`/`_all_`.
fn parse_array(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `array`
    let name_tok = ts.peek().clone();
    let Some(name) = name_tok.ident().map(str::to_string) else {
        return Err(SasError::parse(
            "expected an array name in the ARRAY statement",
            name_tok.span,
        ));
    };
    validate_sas_name(&name, name_tok.span)?;
    ts.next();

    // ── Dimension : `{n}`, `[n]`, `(n)` ou `{*}`... ─────────────────────
    let open = ts.peek().clone();
    let closer = match open.kind {
        TokenKind::LBrace => TokenKind::RBrace,
        TokenKind::LBracket => TokenKind::RBracket,
        TokenKind::LParen => TokenKind::RParen,
        _ => {
            return Err(SasError::parse(
                "expected '{', '[' or '(' after the array name",
                open.span,
            ));
        }
    };
    ts.next(); // ouvrant
    let dim_tok = ts.peek().clone();
    let size = match dim_tok.kind {
        TokenKind::Star => {
            ts.next();
            None
        }
        TokenKind::Num(n) => {
            if n.fract() != 0.0 || n < 1.0 {
                return Err(SasError::parse(
                    "the array dimension must be a positive integer",
                    dim_tok.span,
                ));
            }
            ts.next();
            Some(n as usize)
        }
        _ => {
            return Err(SasError::parse(
                "expected a dimension or '*' in the ARRAY statement",
                dim_tok.span,
            ));
        }
    };
    if ts.peek().kind == TokenKind::Comma {
        return Err(SasError::parse(
            "Multi-dimensional arrays are not yet implemented.",
            ts.peek().span,
        ));
    }
    if ts.peek().kind != closer {
        return Err(SasError::parse(
            "expected the matching closing delimiter of the array dimension",
            ts.peek().span,
        ));
    }
    ts.next(); // fermant

    // ── `$ [len]` : array caractère, longueur défaut 8 ──────────────────
    let mut char_len: Option<usize> = None;
    if ts.peek().kind == TokenKind::Dollar {
        ts.next(); // `$`
        char_len = Some(8);
        if let TokenKind::Num(n) = ts.peek().kind {
            let num_span = ts.peek().span;
            if n.fract() != 0.0 || n < 1.0 {
                return Err(SasError::parse(
                    "the length in an ARRAY statement must be a positive integer",
                    num_span,
                ));
            }
            ts.next();
            char_len = Some(n as usize);
        }
    }

    // ── Liste de variables (plages x1-x3 expansées ici) ──────────────────
    let mut vars: Vec<String> = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi => {
                ts.next();
                return Ok(DsStmt::Array {
                    name,
                    size,
                    char_len,
                    vars,
                });
            }
            TokenKind::Ident(v) => {
                let v = v.clone();
                let lower = v.to_ascii_lowercase();
                if matches!(
                    lower.as_str(),
                    "_temporary_" | "_numeric_" | "_character_" | "_all_"
                ) {
                    return Err(SasError::parse(
                        format!(
                            "{} in the ARRAY statement is not yet implemented.",
                            v.to_uppercase()
                        ),
                        tok.span,
                    ));
                }
                validate_sas_name(&v, tok.span)?;
                ts.next();
                if ts.peek().kind == TokenKind::Minus {
                    // Plage numérotée `x1-x3`.
                    ts.next(); // `-`
                    let end_tok = ts.peek().clone();
                    let Some(end_name) = end_tok.ident().map(str::to_string) else {
                        return Err(SasError::parse(
                            "expected a variable name after '-' in the ARRAY statement",
                            end_tok.span,
                        ));
                    };
                    validate_sas_name(&end_name, end_tok.span)?;
                    ts.next();
                    expand_numbered_range(&v, &end_name, tok.span.merge(end_tok.span), &mut vars)?;
                } else {
                    vars.push(v);
                }
            }
            // `(1 2 3)` : valeurs initiales — hors périmètre M2.
            TokenKind::LParen => {
                return Err(SasError::parse(
                    "Array initial values are not yet implemented.",
                    tok.span,
                ));
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable name in the ARRAY statement",
                    tok.span,
                ));
            }
        }
    }
}

/// Découpe `x12` en (`x`, `12`) : préfixe + suffixe numérique FINAL.
/// `None` si le nom ne se termine pas par un chiffre (ou n'a pas de
/// préfixe).
fn split_numbered(name: &str) -> Option<(&str, &str)> {
    let bytes = name.as_bytes();
    let mut i = bytes.len();
    while i > 0 && bytes[i - 1].is_ascii_digit() {
        i -= 1;
    }
    if i == bytes.len() || i == 0 {
        return None;
    }
    Some((&name[..i], &name[i..]))
}

/// Expanse la plage numérotée `x1-x3` en x1 x2 x3 : même préfixe (insensible
/// à la casse — la casse du premier nom est conservée), suffixes numériques,
/// bornes croissantes ; la largeur du suffixe de départ est conservée
/// (`x01-x03` → x01 x02 x03). Sinon, erreur claire. Partagée entre ARRAY et
/// les listes des options de dataset KEEP=/DROP= (`pub(super)`).
pub(super) fn expand_numbered_range(
    start: &str,
    end: &str,
    span: Span,
    out: &mut Vec<String>,
) -> Result<()> {
    let err = || {
        SasError::parse(
            format!(
                "invalid variable range {start}-{end} \
                 (expected the same prefix with increasing numeric suffixes, e.g. x1-x3)"
            ),
            span,
        )
    };
    let (Some((p1, s1)), Some((p2, s2))) = (split_numbered(start), split_numbered(end)) else {
        return Err(err());
    };
    if !p1.eq_ignore_ascii_case(p2) {
        return Err(err());
    }
    let (Ok(a), Ok(b)) = (s1.parse::<u64>(), s2.parse::<u64>()) else {
        return Err(err());
    };
    if a > b {
        return Err(err());
    }
    let width = s1.len();
    for n in a..=b {
        out.push(format!("{p1}{n:0width$}"));
    }
    Ok(())
}

/// `retain [v [init]]... ;` — la liste peut être vide (`retain;` = toutes
/// les variables du PDV). Chaque nom peut être suivi d'une valeur initiale
/// LITTÉRALE : nombre (avec `-` unaire, replié en `Expr::Num` négatif),
/// chaîne, ou missing (`.` / `.a`.. / `._`, adjacence vérifiée par spans
/// comme dans le parser d'expressions).
fn parse_retain(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `retain`
    let mut items: Vec<(String, Option<Expr>)> = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi => {
                ts.next();
                return Ok(DsStmt::Retain(items));
            }
            TokenKind::Ident(name) => {
                let name = name.clone();
                validate_sas_name(&name, tok.span)?;
                ts.next();
                let init = parse_retain_init(ts)?;
                items.push((name, init));
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable name in the RETAIN statement",
                    tok.span,
                ));
            }
        }
    }
}

/// Valeur initiale optionnelle d'un élément de RETAIN : un littéral, ou
/// rien (le token suivant est alors un autre nom ou le `;`).
fn parse_retain_init(ts: &mut StatementStream) -> Result<Option<Expr>> {
    let tok = ts.peek().clone();
    match &tok.kind {
        TokenKind::Num(n) => {
            ts.next();
            Ok(Some(Expr::Num(*n)))
        }
        TokenKind::Minus => {
            // `-5` : moins unaire sur littéral numérique, replié.
            ts.next(); // `-`
            let num_tok = ts.peek().clone();
            let TokenKind::Num(n) = num_tok.kind else {
                return Err(SasError::parse(
                    "expected a numeric literal after '-' in the RETAIN statement",
                    num_tok.span,
                ));
            };
            ts.next();
            Ok(Some(Expr::Num(-n)))
        }
        TokenKind::Str { value, suffix } => {
            // M2 : seuls les littéraux simples sont acceptés comme valeur
            // initiale (pas de '...'d/'...'t — viendront avec les formats).
            match suffix {
                StrSuffix::None | StrSuffix::Name => {
                    let s = value.clone();
                    ts.next();
                    Ok(Some(Expr::Str(s)))
                }
                _ => Err(SasError::parse(
                    "date/time literals are not yet implemented as RETAIN initial values",
                    tok.span,
                )),
            }
        }
        TokenKind::Dot => {
            // `.` seul, ou missing spécial `.a`.. / `._` si l'ident d'UNE
            // lettre/`_` est ADJACENT (spans jointifs, comme expr.rs).
            let dot_end = tok.span.end;
            ts.next(); // `.`
            if let TokenKind::Ident(s) = &ts.peek().kind {
                if ts.peek().span.start == dot_end && s.chars().count() == 1 {
                    if let Some(kind) = MissingKind::from_letter(s.chars().next().unwrap()) {
                        ts.next();
                        return Ok(Some(Expr::Missing(kind)));
                    }
                }
            }
            Ok(Some(Expr::Missing(MissingKind::Dot)))
        }
        // Pas de littéral : l'élément n'a pas de valeur initiale.
        _ => Ok(None),
    }
}

/// `length v1 v2 $ 20 v3 5;` — suites répétables de « noms... [$] n » ; le
/// `$` s'applique au groupe de noms qui précède le nombre. La validation
/// des PLAGES de longueur (char 1..=32767, num 3..=8) est faite à la
/// compilation ; ici on exige seulement un entier positif.
fn parse_length(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `length`
    let mut items: Vec<(String, LengthSpec)> = Vec::new();
    let mut group: Vec<String> = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Ident(name) => {
                let name = name.clone();
                validate_sas_name(&name, tok.span)?;
                ts.next();
                group.push(name);
            }
            TokenKind::Dollar | TokenKind::Num(_) => {
                let is_char = tok.kind == TokenKind::Dollar;
                if is_char {
                    ts.next(); // `$`
                }
                let num_tok = ts.peek().clone();
                let TokenKind::Num(n) = num_tok.kind else {
                    return Err(SasError::parse(
                        "expected a length after '$' in the LENGTH statement",
                        num_tok.span,
                    ));
                };
                if group.is_empty() {
                    return Err(SasError::parse(
                        "expected a variable name before the length in the LENGTH statement",
                        tok.span,
                    ));
                }
                if n.fract() != 0.0 || n < 1.0 {
                    return Err(SasError::parse(
                        "the length in a LENGTH statement must be a positive integer",
                        num_tok.span,
                    ));
                }
                ts.next(); // le nombre
                let spec = LengthSpec {
                    char: is_char,
                    len: n as usize,
                };
                for name in group.drain(..) {
                    items.push((name, spec));
                }
            }
            TokenKind::Semi => {
                // Noms restés sans longueur → erreur AVANT de consommer le
                // `;` (la resynchronisation de l'appelant le consommera).
                if !group.is_empty() {
                    return Err(SasError::parse(
                        "expected a length in the LENGTH statement",
                        tok.span,
                    ));
                }
                if items.is_empty() {
                    return Err(SasError::parse(
                        "expected a variable name in the LENGTH statement",
                        tok.span,
                    ));
                }
                ts.next();
                return Ok(DsStmt::Length(items));
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable name or a length in the LENGTH statement",
                    tok.span,
                ));
            }
        }
    }
}

/// `format weight height 8.2 name $char10.;` (M4) — suites répétables de
/// « noms... token-de-format ». Chaque groupe associe une liste d'un-ou-
/// plusieurs noms de variables au token de format qui le suit (lu via
/// `read_format_token`, robuste au découpage du lexer). L'application aux
/// variables (et la validation du token) est faite à la compilation.
fn parse_format(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `format`
    let mut groups: Vec<(Vec<String>, String)> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi => {
                if !names.is_empty() {
                    return Err(SasError::parse(
                        "expected a format after the variable name(s) in the FORMAT statement",
                        tok.span,
                    ));
                }
                if groups.is_empty() {
                    return Err(SasError::parse(
                        "expected a variable name in the FORMAT statement",
                        tok.span,
                    ));
                }
                ts.next();
                return Ok(DsStmt::Format(groups));
            }
            // Un Ident est SOIT un nom de variable, SOIT le début d'un token
            // de format (ex. `date9.` se lexe `date9` + `.`). On tranche par
            // l'adjacence : si le token suivant touche cet Ident et est un
            // morceau de format (`$`/nombre/`.`), l'Ident ouvre le format.
            TokenKind::Ident(name) if !ident_begins_format(ts) => {
                let name = name.clone();
                validate_sas_name(&name, tok.span)?;
                ts.next();
                names.push(name);
            }
            // Token de format : `$`/nombre/`.` en tête, ou un Ident adjacent
            // à un tel morceau. Clôt le groupe de noms courant.
            _ => {
                if names.is_empty() {
                    return Err(SasError::parse(
                        "expected a variable name in the FORMAT statement",
                        tok.span,
                    ));
                }
                let token = super::expr::read_format_token(ts)?;
                groups.push((std::mem::take(&mut names), token));
            }
        }
    }
}

/// Vrai si le token courant (un Ident) ouvre un token de format plutôt qu'un
/// nom de variable : le token SUIVANT est ADJACENT (aucun espace) et est un
/// morceau de format (`$`, un nombre ou `.`). Ainsi `date9.` (Ident `date9`
/// collé à `.`) est un format, alors que `weight 8.2` (espace) garde
/// `weight` comme nom.
fn ident_begins_format(ts: &StatementStream) -> bool {
    let cur = ts.peek();
    let next = ts.peek2();
    next.span.start == cur.span.end
        && matches!(
            next.kind,
            TokenKind::Dollar | TokenKind::Num(_) | TokenKind::Dot
        )
}

/// `label weight='Body Weight' name='Pupil';` (M4) — paires
/// `ident = 'libellé'` jusqu'au `;`. La valeur est un littéral chaîne.
fn parse_label(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `label`
    let mut pairs: Vec<(String, String)> = Vec::new();
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi => {
                if pairs.is_empty() {
                    return Err(SasError::parse(
                        "expected a variable name in the LABEL statement",
                        tok.span,
                    ));
                }
                ts.next();
                return Ok(DsStmt::Label(pairs));
            }
            TokenKind::Ident(name) => {
                let name = name.clone();
                validate_sas_name(&name, tok.span)?;
                ts.next();
                if ts.peek().kind != TokenKind::Eq {
                    return Err(SasError::parse(
                        "expected '=' after the variable name in the LABEL statement",
                        ts.peek().span,
                    ));
                }
                ts.next(); // `=`
                let label = expect_string_literal(ts, "LABEL")?;
                pairs.push((name, label));
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable name in the LABEL statement",
                    tok.span,
                ));
            }
        }
    }
}

/// `attrib weight format=8.2 label='Body Weight';` (M4) — un item par
/// groupe de variables (un ou plusieurs noms) suivi d'options
/// `format=<token>`, `label='...'`, `length=[$]n`. `length=` est parsé mais
/// NON appliqué en M4 (simplification documentée). Un nouveau nom de
/// variable (sans `=`) après des options clôt l'item courant.
fn parse_attrib(ts: &mut StatementStream) -> Result<DsStmt> {
    ts.next(); // `attrib`
    let mut items: Vec<AttribItem> = Vec::new();
    let mut vars: Vec<String> = Vec::new();
    let mut format: Option<String> = None;
    let mut label: Option<String> = None;
    let mut length: Option<LengthSpec> = None;
    let flush = |vars: &mut Vec<String>,
                 format: &mut Option<String>,
                 label: &mut Option<String>,
                 length: &mut Option<LengthSpec>,
                 items: &mut Vec<AttribItem>| {
        if !vars.is_empty() {
            items.push(AttribItem {
                vars: std::mem::take(vars),
                format: format.take(),
                label: label.take(),
                length: length.take(),
            });
        }
    };
    loop {
        let tok = ts.peek().clone();
        match &tok.kind {
            TokenKind::Semi => {
                flush(&mut vars, &mut format, &mut label, &mut length, &mut items);
                if items.is_empty() {
                    return Err(SasError::parse(
                        "expected a variable name in the ATTRIB statement",
                        tok.span,
                    ));
                }
                ts.next();
                return Ok(DsStmt::Attrib(items));
            }
            TokenKind::Ident(name) => {
                let name = name.clone();
                let lower = name.to_ascii_lowercase();
                // Une option `format=/label=/length=` : le mot-clé est suivi
                // d'un `=`. On consomme l'ident puis on inspecte le `=`.
                if matches!(lower.as_str(), "format" | "label" | "length") {
                    // Sauvegarde du span pour les messages d'erreur.
                    let kw_span = tok.span;
                    ts.next(); // mot-clé d'option
                    if ts.peek().kind != TokenKind::Eq {
                        return Err(SasError::parse(
                            format!(
                                "expected '=' after {} in the ATTRIB statement",
                                lower.to_uppercase()
                            ),
                            ts.peek().span,
                        ));
                    }
                    ts.next(); // `=`
                    if vars.is_empty() {
                        return Err(SasError::parse(
                            "expected a variable name before the attributes in the ATTRIB statement",
                            kw_span,
                        ));
                    }
                    match lower.as_str() {
                        "format" => format = Some(super::expr::read_format_token(ts)?),
                        "label" => label = Some(expect_string_literal(ts, "ATTRIB")?),
                        "length" => length = Some(parse_attrib_length(ts)?),
                        _ => unreachable!(),
                    }
                } else {
                    // Un nom de variable : s'il commence un nouvel item (des
                    // attributs ont déjà été lus), on flush l'item précédent.
                    validate_sas_name(&name, tok.span)?;
                    if format.is_some() || label.is_some() || length.is_some() {
                        flush(&mut vars, &mut format, &mut label, &mut length, &mut items);
                    }
                    ts.next();
                    vars.push(name);
                }
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable name or an attribute in the ATTRIB statement",
                    tok.span,
                ));
            }
        }
    }
}

/// `length=[$]n` pour ATTRIB : `$` optionnel (caractère), puis un entier
/// positif.
fn parse_attrib_length(ts: &mut StatementStream) -> Result<LengthSpec> {
    let is_char = ts.peek().kind == TokenKind::Dollar;
    if is_char {
        ts.next(); // `$`
    }
    let num_tok = ts.peek().clone();
    let TokenKind::Num(n) = num_tok.kind else {
        return Err(SasError::parse(
            "expected a length after LENGTH= in the ATTRIB statement",
            num_tok.span,
        ));
    };
    if n.fract() != 0.0 || n < 1.0 {
        return Err(SasError::parse(
            "the length in an ATTRIB statement must be a positive integer",
            num_tok.span,
        ));
    }
    ts.next();
    Ok(LengthSpec {
        char: is_char,
        len: n as usize,
    })
}

/// Lit un littéral chaîne simple (`'...'` / `"..."`) et renvoie sa valeur.
/// Les suffixes datés ne sont pas acceptés comme libellés.
fn expect_string_literal(ts: &mut StatementStream, stmt: &str) -> Result<String> {
    let tok = ts.peek().clone();
    match &tok.kind {
        TokenKind::Str {
            value,
            suffix: StrSuffix::None | StrSuffix::Name,
        } => {
            let s = value.clone();
            ts.next();
            Ok(s)
        }
        _ => Err(SasError::parse(
            format!("expected a quoted string in the {stmt} statement"),
            tok.span,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BinaryOp, DatasetRef, DatasetSpec, Expr};
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

    /// Spec sans options.
    fn dspec(name: &str) -> DatasetSpec {
        DatasetSpec::plain(dsref(name))
    }

    fn var(s: &str) -> Expr {
        Expr::Var(s.to_string())
    }

    #[test]
    fn simple_set_assign_run() {
        let ast = parse("data out; set inp; x = 1; run;").unwrap();
        assert_eq!(ast.outputs, vec![dspec("out")]);
        assert_eq!(
            ast.stmts,
            vec![
                DsStmt::Set(vec![dspec("inp")]),
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
                dspec("a"),
                dspec("b"),
                DatasetSpec::plain(DatasetRef {
                    libref: Some("lib".to_string()),
                    name: "c".to_string(),
                }),
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
                DsStmt::Output(vec![]),
            ])
        );
    }

    #[test]
    fn output_keep_drop_stop() {
        let ast = parse("data o; set i; output; keep a b; drop c; stop; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![
                DsStmt::Set(vec![dspec("i")]),
                DsStmt::Output(vec![]),
                DsStmt::Keep(vec!["a".to_string(), "b".to_string()]),
                DsStmt::Drop(vec!["c".to_string()]),
                DsStmt::Stop,
            ]
        );
    }

    #[test]
    fn set_two_datasets_parses() {
        let ast = parse("data o; set a lib.b; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![DsStmt::Set(vec![
                dspec("a"),
                DatasetSpec::plain(DatasetRef {
                    libref: Some("lib".to_string()),
                    name: "b".to_string(),
                }),
            ])]
        );
    }

    // ── BY + FIRST./LAST. (M3) ───────────────────────────────────────────

    #[test]
    fn by_single_variable() {
        let ast = parse("data o; set a b; by x; run;").unwrap();
        assert_eq!(ast.stmts.len(), 2);
        assert_eq!(ast.stmts[1], DsStmt::By(vec![("x".to_string(), false)]));
    }

    #[test]
    fn by_two_variables_with_descending() {
        let ast = parse("data o; set a; by grp descending age; run;").unwrap();
        assert_eq!(
            ast.stmts[1],
            DsStmt::By(vec![
                ("grp".to_string(), false),
                ("age".to_string(), true),
            ])
        );
        // DESCENDING ne porte que sur la variable qui le SUIT.
        let ast = parse("data o; set a; by descending grp age; run;").unwrap();
        assert_eq!(
            ast.stmts[1],
            DsStmt::By(vec![
                ("grp".to_string(), true),
                ("age".to_string(), false),
            ])
        );
    }

    #[test]
    fn by_without_set_parses() {
        // Accepté au parse : c'est la compilation qui tranche.
        let ast = parse("data o; by x; run;").unwrap();
        assert_eq!(ast.stmts, vec![DsStmt::By(vec![("x".to_string(), false)])]);
    }

    #[test]
    fn by_trailing_descending_or_empty_errors() {
        let err = parse("data o; set a; by x descending; run;").unwrap_err();
        assert!(err.to_string().contains("DESCENDING"), "got: {err}");
        let err = parse("data o; set a; by; run;").unwrap_err();
        assert!(err.to_string().contains("variable name"), "got: {err}");
    }

    #[test]
    fn first_last_in_expressions() {
        let ast = parse("data o; set a; by grp; if first.grp then n = 0; run;").unwrap();
        let DsStmt::If { cond, .. } = &ast.stmts[2] else {
            panic!("expected an IF statement");
        };
        // Nom canonique MAJUSCULE "FIRST.<VAR>".
        assert_eq!(*cond, var("FIRST.GRP"));
        let ast = parse("data o; set a; by grp; l = Last.Grp; run;").unwrap();
        assert_eq!(
            ast.stmts[2],
            DsStmt::Assign {
                var: "l".to_string(),
                expr: var("LAST.GRP"),
            }
        );
    }

    #[test]
    fn lib_member_in_expression_is_an_error() {
        // Un ident autre que first/last suivi de `.ident` n'est pas une
        // référence valide en expression : le `.` orphelin fait échouer le
        // statement.
        let err = parse("data o; x = a.b; run;").unwrap_err();
        assert!(err.to_string().contains("';'"), "got: {err}");
    }

    #[test]
    fn unimplemented_statement_errors_but_resyncs() {
        // `update` n'est pas implémenté (M3 ajoute MERGE, pas UPDATE).
        // L'étape doit échouer MAIS le stream doit être positionné après le
        // `run;` pour le bloc suivant.
        let file = SourceFile::new("data o; update x y; set i; run; data b; run;");
        let mut ts = StatementStream::new(&file).unwrap();
        assert!(ts.next().is_kw("data"));
        let err = parse_data_step(&mut ts).unwrap_err();
        assert!(err.to_string().to_uppercase().contains("UPDATE"));
        assert!(err.to_string().contains("not yet implemented"));
        // Resynchronisation : on est sur le `data` de la deuxième étape.
        assert!(ts.peek().is_kw("data"));
        ts.next();
        let ast2 = parse_data_step(&mut ts).unwrap();
        assert_eq!(ast2.outputs, vec![dspec("b")]);
    }

    // ── MERGE (M3) ───────────────────────────────────────────────────────

    #[test]
    fn merge_two_datasets_parses() {
        let ast = parse("data o; merge a b; by id; run;").unwrap();
        assert_eq!(
            ast.stmts[0],
            DsStmt::Merge(vec![dspec("a"), dspec("b")])
        );
        assert_eq!(ast.stmts[1], DsStmt::By(vec![("id".to_string(), false)]));
    }

    #[test]
    fn merge_with_in_option_parses() {
        let ast = parse("data o; merge a(in=ina) b(in=inb); by id; run;").unwrap();
        let DsStmt::Merge(specs) = &ast.stmts[0] else {
            panic!("expected a MERGE statement");
        };
        assert_eq!(specs[0].options.in_.as_deref(), Some("ina"));
        assert_eq!(specs[1].options.in_.as_deref(), Some("inb"));
    }

    #[test]
    fn merge_without_dataset_errors() {
        let err = parse("data o; merge; by id; run;").unwrap_err();
        assert!(err.to_string().to_uppercase().contains("MERGE"));
    }

    // ── RETAIN (M2) ──────────────────────────────────────────────────────

    #[test]
    fn retain_empty_list() {
        let ast = parse("data o; retain; run;").unwrap();
        assert_eq!(ast.stmts, vec![DsStmt::Retain(vec![])]);
    }

    #[test]
    fn retain_mixed_inits() {
        let ast = parse("data o; retain x 0 y 'ab' z; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![DsStmt::Retain(vec![
                ("x".to_string(), Some(Expr::Num(0.0))),
                ("y".to_string(), Some(Expr::Str("ab".to_string()))),
                ("z".to_string(), None),
            ])]
        );
    }

    #[test]
    fn retain_negative_and_missing_inits() {
        use crate::value::MissingKind;
        let ast = parse("data o; retain a -5 b . c .z; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![DsStmt::Retain(vec![
                ("a".to_string(), Some(Expr::Num(-5.0))),
                ("b".to_string(), Some(Expr::Missing(MissingKind::Dot))),
                ("c".to_string(), Some(Expr::Missing(MissingKind::Letter(25)))),
            ])]
        );
    }

    #[test]
    fn retain_dot_then_separate_name_is_plain_missing() {
        use crate::value::MissingKind;
        // `. a` (espace) : missing ordinaire pour x, puis variable a.
        let ast = parse("data o; retain x . a 5; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![DsStmt::Retain(vec![
                ("x".to_string(), Some(Expr::Missing(MissingKind::Dot))),
                ("a".to_string(), Some(Expr::Num(5.0))),
            ])]
        );
    }

    #[test]
    fn retain_minus_without_number_errors() {
        let err = parse("data o; retain x -; run;").unwrap_err();
        assert!(err.to_string().contains("numeric literal"));
    }

    // ── Sum statement (M2) ───────────────────────────────────────────────

    #[test]
    fn sum_statement_parses() {
        let ast = parse("data o; n + 1; total + x * 2; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![
                DsStmt::Sum {
                    var: "n".to_string(),
                    expr: Expr::Num(1.0),
                },
                DsStmt::Sum {
                    var: "total".to_string(),
                    expr: Expr::Binary {
                        op: BinaryOp::Mul,
                        left: Box::new(var("x")),
                        right: Box::new(Expr::Num(2.0)),
                    },
                },
            ]
        );
    }

    #[test]
    fn sum_statement_is_not_confused_with_assignment() {
        let ast = parse("data o; n = 1; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![DsStmt::Assign {
                var: "n".to_string(),
                expr: Expr::Num(1.0),
            }]
        );
    }

    #[test]
    fn sum_statement_minus_form_is_rejected() {
        // `var - expr;` n'existe pas en SAS.
        let err = parse("data o; total - x; run;").unwrap_err();
        assert!(err.to_string().contains("not yet implemented"));
    }

    // ── LENGTH (M2) ──────────────────────────────────────────────────────

    #[test]
    fn length_groups_char_and_num() {
        let ast = parse("data o; length a b $ 12 c 5; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![DsStmt::Length(vec![
                ("a".to_string(), LengthSpec { char: true, len: 12 }),
                ("b".to_string(), LengthSpec { char: true, len: 12 }),
                ("c".to_string(), LengthSpec { char: false, len: 5 }),
            ])]
        );
    }

    #[test]
    fn length_dollar_glued_to_number() {
        let ast = parse("data o; length nm $20; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![DsStmt::Length(vec![(
                "nm".to_string(),
                LengthSpec { char: true, len: 20 },
            )])]
        );
    }

    #[test]
    fn length_without_trailing_number_errors() {
        let err = parse("data o; length a b; run;").unwrap_err();
        assert!(err.to_string().contains("expected a length"));
    }

    #[test]
    fn length_without_names_errors() {
        let err = parse("data o; length $ 4; run;").unwrap_err();
        assert!(err.to_string().contains("variable name"));
        let err = parse("data o; length; run;").unwrap_err();
        assert!(err.to_string().contains("variable name"));
    }

    #[test]
    fn length_non_integer_errors() {
        let err = parse("data o; length a $ 2.5; run;").unwrap_err();
        assert!(err.to_string().contains("positive integer"));
    }

    #[test]
    fn implicit_boundary_without_run() {
        // Pas de `run;` : un `data b;` qui suit clôt l'étape sans être
        // consommé.
        let file = SourceFile::new("data a; set x; data b; set y; run;");
        let mut ts = StatementStream::new(&file).unwrap();
        assert!(ts.next().is_kw("data"));
        let ast1 = parse_data_step(&mut ts).unwrap();
        assert_eq!(ast1.outputs, vec![dspec("a")]);
        assert_eq!(ast1.stmts, vec![DsStmt::Set(vec![dspec("x")])]);
        // Frontière implicite : `data` non consommé.
        assert!(ts.peek().is_kw("data"));
        ts.next();
        let ast2 = parse_data_step(&mut ts).unwrap();
        assert_eq!(ast2.outputs, vec![dspec("b")]);
        assert_eq!(ast2.stmts, vec![DsStmt::Set(vec![dspec("y")])]);
    }

    // ── DO itératif / conditionnel (M2) ──────────────────────────────────

    /// Déstructure un DoLoop ou panique.
    fn as_do_loop(
        stmt: &DsStmt,
    ) -> (
        &Option<(String, Expr)>,
        &Option<Expr>,
        &Option<Expr>,
        &Option<Expr>,
        &Option<Expr>,
        &Vec<DsStmt>,
    ) {
        let DsStmt::DoLoop {
            index,
            to,
            by,
            while_,
            until,
            body,
        } = stmt
        else {
            panic!("expected a DoLoop, got {stmt:?}");
        };
        (index, to, by, while_, until, body)
    }

    #[test]
    fn iterative_do_to_by() {
        let ast = parse("data o; do i = 1 to 10 by 2; x = i; end; run;").unwrap();
        let (index, to, by, while_, until, body) = as_do_loop(&ast.stmts[0]);
        assert_eq!(*index, Some(("i".to_string(), Expr::Num(1.0))));
        assert_eq!(*to, Some(Expr::Num(10.0)));
        assert_eq!(*by, Some(Expr::Num(2.0)));
        assert!(while_.is_none() && until.is_none());
        assert_eq!(
            *body,
            vec![DsStmt::Assign {
                var: "x".to_string(),
                expr: var("i"),
            }]
        );
    }

    #[test]
    fn iterative_do_to_without_by() {
        let ast = parse("data o; do i = 1 to n; end; run;").unwrap();
        let (index, to, by, ..) = as_do_loop(&ast.stmts[0]);
        assert_eq!(*index, Some(("i".to_string(), Expr::Num(1.0))));
        assert_eq!(*to, Some(var("n")));
        assert!(by.is_none());
    }

    #[test]
    fn iterative_do_with_while() {
        let ast = parse("data o; do i = 1 to 10 while(x < 5); end; run;").unwrap();
        let (_, to, _, while_, until, _) = as_do_loop(&ast.stmts[0]);
        assert_eq!(*to, Some(Expr::Num(10.0)));
        assert_eq!(
            *while_,
            Some(Expr::Binary {
                op: BinaryOp::Lt,
                left: Box::new(var("x")),
                right: Box::new(Expr::Num(5.0)),
            })
        );
        assert!(until.is_none());
    }

    #[test]
    fn iterative_do_with_until() {
        let ast = parse("data o; do i = 1 to 10 until(x); end; run;").unwrap();
        let (_, _, _, while_, until, _) = as_do_loop(&ast.stmts[0]);
        assert!(while_.is_none());
        assert_eq!(*until, Some(var("x")));
    }

    #[test]
    fn pure_do_while() {
        let ast = parse("data o; do while(x < 3); x + 1; end; run;").unwrap();
        let (index, to, by, while_, until, body) = as_do_loop(&ast.stmts[0]);
        assert!(index.is_none() && to.is_none() && by.is_none() && until.is_none());
        assert!(while_.is_some());
        assert_eq!(body.len(), 1);
    }

    #[test]
    fn pure_do_until() {
        let ast = parse("data o; do until(x >= 3); x + 1; end; run;").unwrap();
        let (index, to, by, while_, until, _) = as_do_loop(&ast.stmts[0]);
        assert!(index.is_none() && to.is_none() && by.is_none() && while_.is_none());
        assert!(until.is_some());
    }

    #[test]
    fn iterative_do_to_by_while_until_combined() {
        let ast =
            parse("data o; do i = 0 to 8 by 2 while(a) until(b); end; run;").unwrap();
        let (index, to, by, while_, until, body) = as_do_loop(&ast.stmts[0]);
        assert_eq!(*index, Some(("i".to_string(), Expr::Num(0.0))));
        assert_eq!(*to, Some(Expr::Num(8.0)));
        assert_eq!(*by, Some(Expr::Num(2.0)));
        assert_eq!(*while_, Some(var("a")));
        assert_eq!(*until, Some(var("b")));
        assert!(body.is_empty());
    }

    #[test]
    fn do_value_list_errors() {
        let err = parse("data o; do i = 1, 5; end; run;").unwrap_err();
        assert!(err.to_string().contains("not yet implemented"), "got: {err}");
        // Une seule valeur sans clause = liste à un élément : même erreur.
        let err = parse("data o; do i = 1; end; run;").unwrap_err();
        assert!(err.to_string().contains("not yet implemented"), "got: {err}");
    }

    #[test]
    fn do_duplicate_clause_errors() {
        let err = parse("data o; do i = 1 to 2 to 3; end; run;").unwrap_err();
        assert!(err.to_string().contains("duplicate TO"), "got: {err}");
    }

    #[test]
    fn do_missing_end_errors() {
        let err = parse("data o; do i = 1 to 3; x = i; run;").unwrap_err();
        assert!(err.to_string().contains("missing END"), "got: {err}");
        let err = parse("data o; do while(1); x = 1;").unwrap_err();
        assert!(err.to_string().contains("missing END"), "got: {err}");
    }

    #[test]
    fn do_while_without_paren_errors() {
        // `do while ;` sans parenthèse : ni `=` ni `(`.
        let err = parse("data o; do while; end; run;").unwrap_err();
        assert!(err.to_string().contains("WHILE"), "got: {err}");
    }

    #[test]
    fn do_index_named_while_is_iterative() {
        // `while` n'est pas réservé : `do while = 1 to 2;` est un DO
        // itératif d'index `while`.
        let ast = parse("data o; do while = 1 to 2; end; run;").unwrap();
        let (index, to, ..) = as_do_loop(&ast.stmts[0]);
        assert_eq!(*index, Some(("while".to_string(), Expr::Num(1.0))));
        assert_eq!(*to, Some(Expr::Num(2.0)));
    }

    #[test]
    fn nested_do_loops_parse() {
        let ast = parse("data o; do i = 1 to 2; do j = 1 to 3; n + 1; end; end; run;")
            .unwrap();
        let (.., body) = as_do_loop(&ast.stmts[0]);
        let (index, .., inner_body) = as_do_loop(&body[0]);
        assert_eq!(index.as_ref().unwrap().0, "j");
        assert_eq!(inner_body.len(), 1);
    }

    // ── DELETE (M2) ──────────────────────────────────────────────────────

    #[test]
    fn delete_parses_alone_and_in_if() {
        let ast = parse("data o; set i; if age = . then delete; delete; run;").unwrap();
        let DsStmt::If { then_branch, .. } = &ast.stmts[1] else {
            panic!("expected an IF");
        };
        assert_eq!(**then_branch, DsStmt::Delete);
        assert_eq!(ast.stmts[2], DsStmt::Delete);
    }

    // ── ARRAY (M2, lot 3) ────────────────────────────────────────────────

    #[test]
    fn array_declaration_three_delimiter_forms() {
        let expected = vec![DsStmt::Array {
            name: "a".to_string(),
            size: Some(3),
            char_len: None,
            vars: vec!["x".to_string(), "y".to_string(), "z".to_string()],
        }];
        for src in [
            "data o; array a{3} x y z; run;",
            "data o; array a[3] x y z; run;",
            "data o; array a(3) x y z; run;",
        ] {
            let ast = parse(src).unwrap();
            assert_eq!(ast.stmts, expected, "source: {src}");
        }
    }

    #[test]
    fn array_star_size_is_none() {
        let ast = parse("data o; array a{*} x y z; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![DsStmt::Array {
                name: "a".to_string(),
                size: None,
                char_len: None,
                vars: vec!["x".to_string(), "y".to_string(), "z".to_string()],
            }]
        );
    }

    #[test]
    fn array_auto_named_elements_empty_var_list() {
        // `array a{3};` : la liste reste vide (auto-noms a1 a2 a3 à la
        // compilation).
        let ast = parse("data o; array a{3}; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![DsStmt::Array {
                name: "a".to_string(),
                size: Some(3),
                char_len: None,
                vars: vec![],
            }]
        );
    }

    #[test]
    fn array_char_with_and_without_length() {
        let ast = parse("data o; array c{3} $ 8 c1 c2 c3; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![DsStmt::Array {
                name: "c".to_string(),
                size: Some(3),
                char_len: Some(8),
                vars: vec!["c1".to_string(), "c2".to_string(), "c3".to_string()],
            }]
        );
        // `$` sans longueur : défaut 8.
        let ast = parse("data o; array c{2} $ u v; run;").unwrap();
        let DsStmt::Array { char_len, .. } = &ast.stmts[0] else {
            panic!("expected an ARRAY statement");
        };
        assert_eq!(*char_len, Some(8));
    }

    #[test]
    fn array_numbered_range_is_expanded() {
        let ast = parse("data o; array a{3} x1-x3; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![DsStmt::Array {
                name: "a".to_string(),
                size: Some(3),
                char_len: None,
                vars: vec!["x1".to_string(), "x2".to_string(), "x3".to_string()],
            }]
        );
        // Largeur de suffixe conservée et plage mêlée à d'autres noms.
        let ast = parse("data o; array a{*} w q01-q03 z; run;").unwrap();
        let DsStmt::Array { vars, .. } = &ast.stmts[0] else {
            panic!("expected an ARRAY statement");
        };
        assert_eq!(*vars, vec!["w", "q01", "q02", "q03", "z"]);
    }

    #[test]
    fn array_invalid_range_errors() {
        // Préfixes différents.
        let err = parse("data o; array a{3} x1-y3; run;").unwrap_err();
        assert!(err.to_string().contains("invalid variable range"), "got: {err}");
        // Bornes décroissantes.
        let err = parse("data o; array a{3} x3-x1; run;").unwrap_err();
        assert!(err.to_string().contains("invalid variable range"), "got: {err}");
        // Pas de suffixe numérique.
        let err = parse("data o; array a{3} x-y; run;").unwrap_err();
        assert!(err.to_string().contains("invalid variable range"), "got: {err}");
    }

    #[test]
    fn array_multi_dimension_errors() {
        let err = parse("data o; array a{2,3} x1-x6; run;").unwrap_err();
        assert!(
            err.to_string().contains("Multi-dimensional arrays are not yet implemented."),
            "got: {err}"
        );
    }

    #[test]
    fn array_initial_values_errors() {
        let err = parse("data o; array a{3} x y z (1 2 3); run;").unwrap_err();
        assert!(
            err.to_string().contains("initial values are not yet implemented"),
            "got: {err}"
        );
    }

    #[test]
    fn array_special_lists_error() {
        for src in [
            "data o; array a{3} _temporary_; run;",
            "data o; array a{*} _numeric_; run;",
            "data o; array a{*} _character_; run;",
            "data o; array a{*} _all_; run;",
        ] {
            let err = parse(src).unwrap_err();
            assert!(err.to_string().contains("not yet implemented"), "source: {src}, got: {err}");
        }
    }

    #[test]
    fn array_indexed_rvalue_in_assignment() {
        let ast = parse("data o; x = a{i + 1}; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![DsStmt::Assign {
                var: "x".to_string(),
                expr: Expr::Index {
                    name: "a".to_string(),
                    index: Box::new(Expr::Binary {
                        op: BinaryOp::Add,
                        left: Box::new(var("i")),
                        right: Box::new(Expr::Num(1.0)),
                    }),
                },
            }]
        );
    }

    #[test]
    fn array_indexed_lvalue_braces_and_brackets() {
        let expected = vec![DsStmt::AssignIndexed {
            array: "a".to_string(),
            index: var("i"),
            expr: Expr::Num(0.0),
        }];
        let ast = parse("data o; a{i} = 0; run;").unwrap();
        assert_eq!(ast.stmts, expected);
        let ast = parse("data o; a[i] = 0; run;").unwrap();
        assert_eq!(ast.stmts, expected);
    }

    #[test]
    fn array_indexed_lvalue_paren_form() {
        // `a(i) = e;` : la forme à parenthèses est dispatchée en
        // AssignIndexed (le nom sera validé array à la compilation).
        let ast = parse("data o; a(i) = i * 10; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![DsStmt::AssignIndexed {
                array: "a".to_string(),
                index: var("i"),
                expr: Expr::Binary {
                    op: BinaryOp::Mul,
                    left: Box::new(var("i")),
                    right: Box::new(Expr::Num(10.0)),
                },
            }]
        );
    }

    #[test]
    fn comment_statement_in_body_is_skipped() {
        let ast = parse("data o; set i; * this is a comment ; x = 1; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![
                DsStmt::Set(vec![dspec("i")]),
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

    // ── Options de dataset + OUTPUT ciblé (M2, lot 4) ────────────────────

    #[test]
    fn data_with_keep_drop_rename_where_options() {
        let ast = parse(
            "data out(keep=a b drop=c rename=(a=aa) where=(a > 1)); run;",
        )
        .unwrap();
        assert_eq!(ast.outputs.len(), 1);
        let spec = &ast.outputs[0];
        assert_eq!(spec.dref, dsref("out"));
        assert_eq!(
            spec.options.keep,
            Some(vec!["a".to_string(), "b".to_string()])
        );
        assert_eq!(spec.options.drop, Some(vec!["c".to_string()]));
        assert_eq!(
            spec.options.rename,
            vec![("a".to_string(), "aa".to_string())]
        );
        assert_eq!(
            spec.options.where_,
            Some(Expr::Binary {
                op: BinaryOp::Gt,
                left: Box::new(var("a")),
                right: Box::new(Expr::Num(1.0)),
            })
        );
    }

    #[test]
    fn set_with_options_parses() {
        let ast = parse(
            "data o; set inp(keep=name age where=(age > 13) rename=(age=years)); run;",
        )
        .unwrap();
        let DsStmt::Set(specs) = &ast.stmts[0] else {
            panic!("expected a SET statement");
        };
        let spec = &specs[0];
        assert_eq!(spec.dref, dsref("inp"));
        assert_eq!(
            spec.options.keep,
            Some(vec!["name".to_string(), "age".to_string()])
        );
        assert!(spec.options.drop.is_none());
        assert_eq!(
            spec.options.rename,
            vec![("age".to_string(), "years".to_string())]
        );
        assert!(spec.options.where_.is_some());
    }

    #[test]
    fn keep_accepts_parenthesized_list_and_ranges() {
        // Forme parenthésée.
        let ast = parse("data o(keep=(x y)); run;").unwrap();
        assert_eq!(
            ast.outputs[0].options.keep,
            Some(vec!["x".to_string(), "y".to_string()])
        );
        // Plage numérotée, forme nue.
        let ast = parse("data o; set i(drop=v1-v3 keep=w); run;").unwrap();
        let DsStmt::Set(specs) = &ast.stmts[0] else {
            panic!("expected a SET statement");
        };
        let spec = &specs[0];
        assert_eq!(
            spec.options.drop,
            Some(vec!["v1".to_string(), "v2".to_string(), "v3".to_string()])
        );
        assert_eq!(spec.options.keep, Some(vec!["w".to_string()]));
    }

    #[test]
    fn output_targeted_one_and_two_names() {
        let ast = parse("data a b; output a; output a b; output; run;").unwrap();
        assert_eq!(ast.outputs, vec![dspec("a"), dspec("b")]);
        assert_eq!(ast.stmts[0], DsStmt::Output(vec![dsref("a")]));
        assert_eq!(
            ast.stmts[1],
            DsStmt::Output(vec![dsref("a"), dsref("b")])
        );
        assert_eq!(ast.stmts[2], DsStmt::Output(vec![]));
    }

    #[test]
    fn unknown_dataset_option_errors() {
        let err = parse("data o(obs=5); run;").unwrap_err();
        assert!(
            err.to_string().contains("Dataset option OBS is not supported."),
            "got: {err}"
        );
        let err = parse("data o; set i(firstobs=2); run;").unwrap_err();
        assert!(
            err.to_string()
                .contains("Dataset option FIRSTOBS is not supported."),
            "got: {err}"
        );
    }

    #[test]
    fn rename_without_parens_errors() {
        let err = parse("data o(rename=a=b); run;").unwrap_err();
        assert!(
            err.to_string().contains("RENAME= dataset option requires"),
            "got: {err}"
        );
    }

    #[test]
    fn where_without_parens_errors() {
        let err = parse("data o; set i(where=age > 1); run;").unwrap_err();
        assert!(
            err.to_string().contains("WHERE= dataset option requires"),
            "got: {err}"
        );
    }

    #[test]
    fn empty_keep_list_errors() {
        let err = parse("data o(keep=); run;").unwrap_err();
        assert!(
            err.to_string()
                .contains("expected a variable name in the KEEP= dataset option"),
            "got: {err}"
        );
    }

    #[test]
    fn set_options_then_second_dataset_parses() {
        let ast = parse("data o; set a(keep=x) b; run;").unwrap();
        let DsStmt::Set(specs) = &ast.stmts[0] else {
            panic!("expected a SET statement");
        };
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].options.keep, Some(vec!["x".to_string()]));
        assert_eq!(specs[1], dspec("b"));
    }

    // ── FORMAT / LABEL / ATTRIB (M4) ──────────────────────────────────────

    #[test]
    fn format_groups_vars_and_tokens() {
        let ast = parse("data o; format weight height 8.2 name $char10.; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![DsStmt::Format(vec![
                (
                    vec!["weight".to_string(), "height".to_string()],
                    "8.2".to_string()
                ),
                (vec!["name".to_string()], "$char10.".to_string()),
            ])]
        );
    }

    #[test]
    fn format_single_var_date9() {
        let ast = parse("data o; format dob date9.; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![DsStmt::Format(vec![(
                vec!["dob".to_string()],
                "date9.".to_string()
            )])]
        );
    }

    #[test]
    fn format_missing_token_errors() {
        let err = parse("data o; format weight; run;").unwrap_err();
        assert!(err.to_string().contains("format"), "got: {err}");
    }

    #[test]
    fn label_pairs() {
        let ast = parse("data o; label weight='Body Weight' name='Pupil'; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![DsStmt::Label(vec![
                ("weight".to_string(), "Body Weight".to_string()),
                ("name".to_string(), "Pupil".to_string()),
            ])]
        );
    }

    #[test]
    fn label_missing_equals_errors() {
        let err = parse("data o; label weight 'x'; run;").unwrap_err();
        assert!(err.to_string().contains("'='"), "got: {err}");
    }

    #[test]
    fn attrib_format_and_label() {
        let ast = parse("data o; attrib weight format=8.2 label='Body Weight'; run;").unwrap();
        assert_eq!(
            ast.stmts,
            vec![DsStmt::Attrib(vec![AttribItem {
                vars: vec!["weight".to_string()],
                format: Some("8.2".to_string()),
                label: Some("Body Weight".to_string()),
                length: None,
            }])]
        );
    }

    #[test]
    fn attrib_multiple_items() {
        let ast = parse(
            "data o; attrib a b format=dollar8. c label='C var' length=$ 10; run;",
        )
        .unwrap();
        assert_eq!(
            ast.stmts,
            vec![DsStmt::Attrib(vec![
                AttribItem {
                    vars: vec!["a".to_string(), "b".to_string()],
                    format: Some("dollar8.".to_string()),
                    label: None,
                    length: None,
                },
                AttribItem {
                    vars: vec!["c".to_string()],
                    format: None,
                    label: Some("C var".to_string()),
                    length: Some(LengthSpec { char: true, len: 10 }),
                },
            ])]
        );
    }
}
