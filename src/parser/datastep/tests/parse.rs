use super::*;
use crate::ast::Expr;

// ── M16.3 : DO liste de valeurs, DO OVER, RETAIN littéraux date ───────

#[test]
fn parse_do_list_numeric() {
    let ast = parse("data o; do i = 1, 3, 5; end; run;").unwrap();
    let DsStmt::DoList { index, items, body } = &ast.stmts[0] else {
        panic!("expected DoList, got {:?}", ast.stmts[0]);
    };
    assert_eq!(index, "i");
    assert_eq!(
        *items,
        vec![
            DoListItem::Value(Expr::Num(1.0)),
            DoListItem::Value(Expr::Num(3.0)),
            DoListItem::Value(Expr::Num(5.0)),
        ]
    );
    assert!(body.is_empty());
}

#[test]
fn parse_do_list_single_value() {
    // Une valeur unique sans clause TO/BY est une liste à un élément.
    let ast = parse("data o; do i = 42; end; run;").unwrap();
    let DsStmt::DoList { items, .. } = &ast.stmts[0] else {
        panic!("expected DoList");
    };
    assert_eq!(*items, vec![DoListItem::Value(Expr::Num(42.0))]);
}

#[test]
fn parse_do_list_character() {
    let ast = parse("data o; do c = 'red', 'blue'; end; run;").unwrap();
    let DsStmt::DoList { items, .. } = &ast.stmts[0] else {
        panic!("expected DoList");
    };
    assert_eq!(
        *items,
        vec![
            DoListItem::Value(Expr::Str("red".to_string())),
            DoListItem::Value(Expr::Str("blue".to_string())),
        ]
    );
}

#[test]
fn parse_do_list_mixed_range_and_values() {
    let ast = parse("data o; do i = 1 to 5 by 2, 10, 20 to 30; end; run;").unwrap();
    let DsStmt::DoList { items, .. } = &ast.stmts[0] else {
        panic!("expected DoList");
    };
    assert_eq!(
        *items,
        vec![
            DoListItem::Range {
                from: Expr::Num(1.0),
                to: Expr::Num(5.0),
                by: Some(Expr::Num(2.0)),
            },
            DoListItem::Value(Expr::Num(10.0)),
            DoListItem::Range {
                from: Expr::Num(20.0),
                to: Expr::Num(30.0),
                by: None,
            },
        ]
    );
}

#[test]
fn parse_classic_do_still_doloop() {
    // `do i = 1 to 10 by 2;` SANS virgule reste un DoLoop classique.
    let ast = parse("data o; do i = 1 to 10 by 2; end; run;").unwrap();
    assert!(matches!(ast.stmts[0], DsStmt::DoLoop { .. }));
}

#[test]
fn parse_do_over() {
    let ast = parse("data o; array a{3} x y z; do over a; a = a + 1; end; run;").unwrap();
    let DsStmt::DoOver { array, body } = &ast.stmts[1] else {
        panic!("expected DoOver, got {:?}", ast.stmts[1]);
    };
    assert_eq!(array, "a");
    assert_eq!(body.len(), 1);
}

#[test]
fn parse_do_list_rejects_while() {
    let err = parse("data o; do i = 1, 3 while(x); end; run;").unwrap_err();
    assert!(
        err.to_string().contains("WHILE/UNTIL are not allowed"),
        "got: {err}"
    );
}

#[test]
fn parse_retain_date_literal_bare() {
    // `21710d` (numérique + suffixe d) → valeur SAS date 21710.
    let ast = parse("data o; retain d 21710d; run;").unwrap();
    assert_eq!(
        ast.stmts,
        vec![DsStmt::Retain(vec![("d".to_string(), Some(Expr::Num(21710.0)))])]
    );
}

#[test]
fn parse_retain_date_literal_quoted() {
    // `'02JAN1960'd` = 1.
    let ast = parse("data o; retain d '02JAN1960'd; run;").unwrap();
    assert_eq!(
        ast.stmts,
        vec![DsStmt::Retain(vec![("d".to_string(), Some(Expr::Num(1.0)))])]
    );
}

#[test]
fn parse_retain_datetime_literal() {
    // `'01JAN1960 00:01:00'dt` = 60 secondes.
    let ast = parse("data o; retain dt '01JAN1960 00:01:00'dt; run;").unwrap();
    assert_eq!(
        ast.stmts,
        vec![DsStmt::Retain(vec![("dt".to_string(), Some(Expr::Num(60.0)))])]
    );
}

// ── M17.1 : DECLARE HASH + méthodes ──────────────────────────────────

#[test]
fn parse_declare_hash_no_options() {
    let ast = parse("data _null_; declare hash h(); run;").unwrap();
    assert_eq!(
        ast.stmts,
        vec![DsStmt::DeclareHash {
            name: "h".to_string(),
            options: vec![],
        }]
    );
}

#[test]
fn parse_declare_hash_no_parens() {
    // Sans parenthèses du tout.
    let ast = parse("data _null_; declare hash h; run;").unwrap();
    assert_eq!(
        ast.stmts,
        vec![DsStmt::DeclareHash {
            name: "h".to_string(),
            options: vec![],
        }]
    );
}

#[test]
fn parse_dcl_alias_with_options() {
    // Alias `dcl` + options key:value séparées par virgules.
    let ast =
        parse("data _null_; dcl hash h(ordered:'yes', duplicate:'replace', multidata:'yes'); run;")
            .unwrap();
    assert_eq!(
        ast.stmts,
        vec![DsStmt::DeclareHash {
            name: "h".to_string(),
            options: vec![
                ("ordered".to_string(), "yes".to_string()),
                ("duplicate".to_string(), "replace".to_string()),
                ("multidata".to_string(), "yes".to_string()),
            ],
        }]
    );
}

#[test]
fn parse_declare_hash_dataset_option() {
    let ast = parse("data _null_; declare hash h(dataset:'work.lookup'); run;").unwrap();
    assert_eq!(
        ast.stmts,
        vec![DsStmt::DeclareHash {
            name: "h".to_string(),
            options: vec![("dataset".to_string(), "work.lookup".to_string())],
        }]
    );
}

#[test]
fn parse_hash_define_key_data_done() {
    let ast = parse(
        "data _null_; declare hash h(); h.defineKey('k1', 'k2'); h.defineData('v'); h.defineDone(); run;",
    )
    .unwrap();
    assert_eq!(
        ast.stmts,
        vec![
            DsStmt::DeclareHash {
                name: "h".to_string(),
                options: vec![],
            },
            DsStmt::HashMethod(Box::new(crate::ast::HashMethodCall {
                object: "h".to_string(),
                method: "defineKey".to_string(),
                args: vec![
                    crate::ast::HashArg::Positional(Expr::Str("k1".to_string())),
                    crate::ast::HashArg::Positional(Expr::Str("k2".to_string())),
                ],
            })),
            DsStmt::HashMethod(Box::new(crate::ast::HashMethodCall {
                object: "h".to_string(),
                method: "defineData".to_string(),
                args: vec![crate::ast::HashArg::Positional(Expr::Str("v".to_string()))],
            })),
            DsStmt::HashMethod(Box::new(crate::ast::HashMethodCall {
                object: "h".to_string(),
                method: "defineDone".to_string(),
                args: vec![],
            })),
        ]
    );
}

#[test]
fn parse_declare_non_hash_object_errors() {
    // Type d'objet ni HASH ni HITER → erreur de parsing.
    let err = parse("data _null_; declare bogus it('h'); run;").unwrap_err();
    assert!(err.to_string().to_uppercase().contains("BOGUS"));
}

#[test]
fn parse_declare_hiter() {
    let ast = parse("data _null_; declare hiter hi('h'); run;").unwrap();
    assert_eq!(
        ast.stmts,
        vec![DsStmt::DeclareHiter {
            name: "hi".to_string(),
            hash_name: "h".to_string(),
        }]
    );
}

#[test]
fn parse_hash_method_as_expression() {
    let ast = parse("data _null_; rc = h.find(); run;").unwrap();
    assert_eq!(
        ast.stmts,
        vec![DsStmt::Assign {
            var: "rc".to_string(),
            expr: Expr::HashMethod(Box::new(crate::ast::HashMethodCall {
                object: "h".to_string(),
                method: "find".to_string(),
                args: vec![],
            })),
        }]
    );
}

#[test]
fn parse_hash_method_named_args() {
    let ast = parse("data _null_; h.add(key: 1, data: 'x'); run;").unwrap();
    assert_eq!(
        ast.stmts,
        vec![DsStmt::HashMethod(Box::new(crate::ast::HashMethodCall {
            object: "h".to_string(),
            method: "add".to_string(),
            args: vec![
                crate::ast::HashArg::Named("key".to_string(), Expr::Num(1.0)),
                crate::ast::HashArg::Named("data".to_string(), Expr::Str("x".to_string())),
            ],
        }))]
    );
}
