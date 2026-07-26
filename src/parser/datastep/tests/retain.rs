use super::super::*;
use super::*;
use crate::ast::{BinaryOp, Expr};
use crate::source::SourceFile;

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
            (
                "c".to_string(),
                Some(Expr::Missing(MissingKind::Letter(25)))
            ),
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
            (
                "a".to_string(),
                LengthSpec {
                    char: true,
                    len: 12
                }
            ),
            (
                "b".to_string(),
                LengthSpec {
                    char: true,
                    len: 12
                }
            ),
            (
                "c".to_string(),
                LengthSpec {
                    char: false,
                    len: 5
                }
            ),
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
            LengthSpec {
                char: true,
                len: 20
            },
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
    assert_eq!(ast1.stmts, vec![set_stmt(vec![dspec("x")])]);
    // Frontière implicite : `data` non consommé.
    assert!(ts.peek().is_kw("data"));
    ts.next();
    let ast2 = parse_data_step(&mut ts).unwrap();
    assert_eq!(ast2.outputs, vec![dspec("b")]);
    assert_eq!(ast2.stmts, vec![set_stmt(vec![dspec("y")])]);
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
fn iterative_do_to_by_while_until_combined() {
    let ast = parse("data o; do i = 0 to 8 by 2 while(a) until(b); end; run;").unwrap();
    let (index, to, by, while_, until, body) = as_do_loop(&ast.stmts[0]);
    assert_eq!(*index, Some(("i".to_string(), Expr::Num(0.0))));
    assert_eq!(*to, Some(Expr::Num(8.0)));
    assert_eq!(*by, Some(Expr::Num(2.0)));
    assert_eq!(*while_, Some(var("a")));
    assert_eq!(*until, Some(var("b")));
    assert!(body.is_empty());
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
fn do_value_list_now_parses() {
    // M16.3 : les listes de valeurs sont désormais supportées (DoList).
    let ast = parse("data o; do i = 1, 5; end; run;").unwrap();
    assert!(matches!(ast.stmts[0], DsStmt::DoList { .. }));
    // Une seule valeur sans clause = liste à un élément.
    let ast = parse("data o; do i = 1; end; run;").unwrap();
    assert!(matches!(ast.stmts[0], DsStmt::DoList { .. }));
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
    let ast = parse("data o; do i = 1 to 2; do j = 1 to 3; n + 1; end; end; run;").unwrap();
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
