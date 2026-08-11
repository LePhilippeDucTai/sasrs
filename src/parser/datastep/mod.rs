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
//!   datasets, options de dataset
//!   acceptées sur chacun)
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

use super::{StatementStream, is_block_head_kw, validate_sas_name};
use crate::ast::{
    AttribItem, DataStepAst, DoListItem, DsStmt, Expr, InfileOptions, InfileSource, InputItem,
    LengthSpec, PutDest, PutItem, WhenClause,
};
use crate::error::{Result, SasError};
use crate::token::{Span, StrSuffix, TokenKind};
use crate::value::MissingKind;

use super::expr;

mod array;
mod attrs;
mod control;
mod hash;
mod io;
#[cfg(test)]
mod tests;

pub(super) use self::array::expand_numbered_range;
use self::array::{parse_array, parse_assign_indexed_tail};
use self::attrs::{
    parse_attrib, parse_format, parse_informat, parse_label, parse_length, parse_retain,
};
use self::control::{parse_do, parse_goto, parse_if, parse_link, parse_select};
pub(crate) use self::hash::parse_hash_args;
use self::hash::{parse_declare, parse_hash_method};
use self::io::{
    parse_by, parse_datalines, parse_file, parse_infile, parse_input, parse_merge, parse_modify,
    parse_put, parse_set, parse_update,
};

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
            TokenKind::DataLines(_) => {
                // Bloc verbatim orphelin (déjà consommé par `parse_datalines`
                // dans le cas normal) : ignoré par robustesse.
                ts.next();
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
                let e = SasError::parse("expected a DATA step statement", tok.span);
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
            return Err(SasError::parse("expected a DATA step statement", tok.span));
        }
    };

    // Étiquette de statement (M16.6) : `label_name: <statement>`. Un identifiant
    // suivi d'un `:` introduit une étiquette. On consomme `ident :`, puis on
    // parse récursivement le statement étiqueté (un seul). Détecté AVANT le
    // dispatch par mot-clé : n'importe quel identifiant peut être une étiquette.
    if ts.peek2().kind == TokenKind::Colon {
        let name = head; // déjà en minuscules ; conservé tel quel (résolu en MAJ)
        ts.next(); // ident d'étiquette
        ts.next(); // `:`
        // Étiquette suivie d'un `;` : statement étiqueté VIDE (no-op), licite en
        // SAS (`fin: ;`). Le corps est un bloc vide.
        if ts.peek().kind == TokenKind::Semi {
            ts.next(); // `;`
            return Ok(DsStmt::Labeled {
                name,
                stmt: Box::new(DsStmt::Block(Vec::new())),
            });
        }
        let stmt = parse_statement(ts)?;
        return Ok(DsStmt::Labeled {
            name,
            stmt: Box::new(stmt),
        });
    }

    // Appel de méthode d'objet hash (M17.1) : `h.method(args);`. Détecté
    // AVANT le dispatch par mot-clé — `ident . ident (` en début de statement
    // est un appel de méthode (la forme `lib.table` n'apparaît jamais seule en
    // tête d'un statement exécutable). On vérifie le motif complet pour ne pas
    // intercepter par erreur une autre construction.
    if ts.peek2().kind == TokenKind::Dot
        && ts.peek_nth(2).ident().is_some()
        && ts.peek_nth(3).kind == TokenKind::LParen
    {
        return parse_hash_method(ts);
    }

    match head.as_str() {
        "declare" | "dcl" => parse_declare(ts),
        "set" => parse_set(ts),
        "merge" => parse_merge(ts),
        "update" => parse_update(ts),
        "modify" => parse_modify(ts),
        "by" => parse_by(ts),
        "if" => parse_if(ts),
        "do" => parse_do(ts),
        "select" => parse_select(ts),
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
        // GOTO (M16.6) : `goto label;` ou `go to label;` (deux tokens).
        "goto" | "go" => parse_goto(ts, &head),
        "link" => parse_link(ts),
        "return" => {
            ts.next();
            ts.expect_semi()?;
            Ok(DsStmt::Return)
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
        "informat" => parse_informat(ts),
        "label" => parse_label(ts),
        "attrib" => parse_attrib(ts),
        // WHERE standalone (M40.3) : `where expr;` — filtre des datasets
        // d'entrée (SET/MERGE), résolu à la compilation. Comme les autres
        // mots-clés de statement, WHERE en tête l'emporte sur une
        // hypothétique variable du même nom.
        "where" => {
            ts.next(); // `where`
            let expr = super::expr::parse_expr(ts)?;
            ts.expect_semi()?;
            Ok(DsStmt::Where(expr))
        }
        "array" => parse_array(ts),
        "call" => parse_call_routine(ts),
        "infile" => parse_infile(ts),
        "input" => parse_input(ts),
        "file" => parse_file(ts),
        "put" => parse_put(ts),
        "datalines" | "cards" | "datalines4" | "cards4" => parse_datalines(ts),
        // `end` ne devrait pas apparaître en tête hors d'un bloc `do`.
        "end" => Err(SasError::parse("no matching DO for END.", tok.span)),
        _ => parse_assign_or_sum_tail(ts, &tok, &head),
    }
}

/// Queue du bras `_` de `parse_statement` : assignation `ident = expr;`,
/// sum statement `ident + expr;`, assignation indexée (`arr{i}`/`arr[i]`/
/// `arr(i)`), sinon « statement non implémenté ». `tok` est le token de
/// tête (non consommé), `head` son identifiant en minuscules.
fn parse_assign_or_sum_tail(
    ts: &mut StatementStream,
    tok: &crate::token::Token,
    head: &str,
) -> Result<DsStmt> {
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
        // `arr{i} = e;` / `arr[i] = e;` / `arr{i,j} = e;` :
        // assignation indexée (mono- ou multi-dimensionnelle).
        TokenKind::LBrace | TokenKind::LBracket => {
            let Expr::Index { name, indices } = super::expr::parse_index(ts, var)? else {
                unreachable!("parse_index always returns Expr::Index");
            };
            parse_assign_indexed_tail(ts, name, indices)
        }
        // `arr(i) = e;` / `arr(i,j) = e;` : forme à parenthèses — le
        // nom sera validé array à la COMPILATION (ici on parse les
        // indices, séparés par des virgules).
        TokenKind::LParen => {
            ts.next(); // `(`
            let mut indices = Vec::new();
            loop {
                indices.push(super::expr::parse_expr(ts)?);
                if ts.peek().kind == TokenKind::Comma {
                    ts.next();
                    continue;
                }
                break;
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
            parse_assign_indexed_tail(ts, var, indices)
        }
        _ => Err(SasError::parse(
            format!("Statement {} is not yet implemented.", head.to_uppercase()),
            tok.span,
        )),
    }
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
