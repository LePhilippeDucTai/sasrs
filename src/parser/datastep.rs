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

use super::{is_block_head_kw, validate_sas_name, StatementStream};
use crate::ast::{DataStepAst, DsStmt, Expr, LengthSpec};
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
        "retain" => parse_retain(ts),
        "length" => parse_length(ts),
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
            if ts.peek().kind == TokenKind::Eq {
                ts.next(); // `=`
                let expr = super::expr::parse_expr(ts)?;
                ts.expect_semi()?;
                Ok(DsStmt::Assign { var, expr })
            } else if ts.peek().kind == TokenKind::Plus {
                ts.next(); // `+`
                let expr = super::expr::parse_expr(ts)?;
                ts.expect_semi()?;
                Ok(DsStmt::Sum { var, expr })
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
        // `merge` n'est pas implémenté en M2. L'étape doit échouer MAIS le
        // stream doit être positionné après le `run;` pour le bloc suivant.
        let file = SourceFile::new("data o; merge x y; set i; run; data b; run;");
        let mut ts = StatementStream::new(&file).unwrap();
        assert!(ts.next().is_kw("data"));
        let err = parse_data_step(&mut ts).unwrap_err();
        assert!(err.to_string().to_uppercase().contains("MERGE"));
        assert!(err.to_string().contains("not yet implemented"));
        // Resynchronisation : on est sur le `data` de la deuxième étape.
        assert!(ts.peek().is_kw("data"));
        ts.next();
        let ast2 = parse_data_step(&mut ts).unwrap();
        assert_eq!(ast2.outputs, vec![dsref("b")]);
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
