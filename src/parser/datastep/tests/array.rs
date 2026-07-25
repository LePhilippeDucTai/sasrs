use super::*;
use crate::ast::{BinaryOp, Expr};

#[test]
fn array_declaration_three_delimiter_forms() {
    let expected = vec![array_stmt("a", Some(vec![3]), None, vec!["x", "y", "z"])];
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
        vec![array_stmt("a", None, None, vec!["x", "y", "z"])]
    );
}

#[test]
fn array_auto_named_elements_empty_var_list() {
    // `array a{3};` : la liste reste vide (auto-noms a1 a2 a3 à la
    // compilation).
    let ast = parse("data o; array a{3}; run;").unwrap();
    assert_eq!(ast.stmts, vec![array_stmt("a", Some(vec![3]), None, vec![])]);
}

#[test]
fn array_char_with_and_without_length() {
    let ast = parse("data o; array c{3} $ 8 c1 c2 c3; run;").unwrap();
    assert_eq!(
        ast.stmts,
        vec![array_stmt("c", Some(vec![3]), Some(8), vec!["c1", "c2", "c3"])]
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
        vec![array_stmt("a", Some(vec![3]), None, vec!["x1", "x2", "x3"])]
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
fn array_multi_dimension_parses() {
    // M16.2 : `{2,3}` → dims [2, 3].
    let ast = parse("data o; array a{2,3} x1-x6; run;").unwrap();
    let DsStmt::Array { dims, vars, .. } = &ast.stmts[0] else {
        panic!("expected an ARRAY statement");
    };
    assert_eq!(*dims, Some(vec![2, 3]));
    assert_eq!(vars.len(), 6);
    // 3-D.
    let ast = parse("data o; array b{2,3,2} v1-v12; run;").unwrap();
    let DsStmt::Array { dims, .. } = &ast.stmts[0] else {
        panic!("expected an ARRAY statement");
    };
    assert_eq!(*dims, Some(vec![2, 3, 2]));
}

#[test]
fn array_initial_values_parse() {
    // M16.2 : valeurs initiales `(1, 2, 3)` (virgules) et `(1 2 3)`
    // (espaces) acceptées.
    for src in [
        "data o; array a{3} x y z (1, 2, 3); run;",
        "data o; array a{3} x y z (1 2 3); run;",
    ] {
        let ast = parse(src).unwrap();
        let DsStmt::Array { initial, .. } = &ast.stmts[0] else {
            panic!("expected an ARRAY statement");
        };
        assert_eq!(
            *initial,
            vec![Expr::Num(1.0), Expr::Num(2.0), Expr::Num(3.0)]
        );
    }
}

#[test]
fn array_special_lists_parse() {
    // M16.2 : _TEMPORARY_ et listes spéciales parsées.
    let ast = parse("data o; array a{3} _temporary_; run;").unwrap();
    let DsStmt::Array { temporary, .. } = &ast.stmts[0] else {
        panic!("expected an ARRAY statement");
    };
    assert!(*temporary);
    for (src, want) in [
        ("data o; array a{*} _numeric_; run;", crate::ast::ArraySpecial::Numeric),
        ("data o; array a{*} _character_; run;", crate::ast::ArraySpecial::Character),
        ("data o; array a{*} _all_; run;", crate::ast::ArraySpecial::All),
    ] {
        let ast = parse(src).unwrap();
        let DsStmt::Array { special, .. } = &ast.stmts[0] else {
            panic!("expected an ARRAY statement");
        };
        assert_eq!(*special, Some(want), "source: {src}");
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
                indices: vec![Expr::Binary {
                    op: BinaryOp::Add,
                    left: Box::new(var("i")),
                    right: Box::new(Expr::Num(1.0)),
                }],
            },
        }]
    );
}

#[test]
fn array_multi_index_rvalue_parses() {
    let ast = parse("data o; x = a{i, j}; run;").unwrap();
    assert_eq!(
        ast.stmts,
        vec![DsStmt::Assign {
            var: "x".to_string(),
            expr: Expr::Index {
                name: "a".to_string(),
                indices: vec![var("i"), var("j")],
            },
        }]
    );
}

#[test]
fn array_indexed_lvalue_braces_and_brackets() {
    let expected = vec![DsStmt::AssignIndexed {
        array: "a".to_string(),
        indices: vec![var("i")],
        expr: Expr::Num(0.0),
    }];
    let ast = parse("data o; a{i} = 0; run;").unwrap();
    assert_eq!(ast.stmts, expected);
    let ast = parse("data o; a[i] = 0; run;").unwrap();
    assert_eq!(ast.stmts, expected);
}

#[test]
fn array_multi_index_lvalue_parses() {
    let ast = parse("data o; a{i, j} = 0; run;").unwrap();
    assert_eq!(
        ast.stmts,
        vec![DsStmt::AssignIndexed {
            array: "a".to_string(),
            indices: vec![var("i"), var("j")],
            expr: Expr::Num(0.0),
        }]
    );
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
            indices: vec![var("i")],
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
            set_stmt(vec![dspec("i")]),
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
fn empty_keep_list_errors() {
    let err = parse("data o(keep=); run;").unwrap_err();
    assert!(
        err.to_string()
            .contains("expected a variable name in the KEEP= dataset option"),
        "got: {err}"
    );
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
    let DsStmt::Set { specs, .. } = &ast.stmts[0] else {
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
