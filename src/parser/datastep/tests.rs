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

/// `DsStmt::Set` sans options de niveau statement (M16.4).
fn set_stmt(specs: Vec<DatasetSpec>) -> DsStmt {
    DsStmt::Set {
        specs,
        options: crate::ast::SetOptions::default(),
    }
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
            set_stmt(vec![dspec("inp")]),
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
            set_stmt(vec![dspec("i")]),
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
        vec![set_stmt(vec![
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
    // `proklamation` n'est pas un statement connu (ni assignation, ni sum) :
    // l'étape doit échouer MAIS le stream doit être positionné après le
    // `run;` pour le bloc suivant (test de resynchronisation du parser).
    let file =
        SourceFile::new("data o; proklamation target; set i; run; data b; run;");
    let mut ts = StatementStream::new(&file).unwrap();
    assert!(ts.next().is_kw("data"));
    let err = parse_data_step(&mut ts).unwrap_err();
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
    assert_eq!(ast1.stmts, vec![set_stmt(vec![dspec("x")])]);
    // Frontière implicite : `data` non consommé.
    assert!(ts.peek().is_kw("data"));
    ts.next();
    let ast2 = parse_data_step(&mut ts).unwrap();
    assert_eq!(ast2.outputs, vec![dspec("b")]);
    assert_eq!(ast2.stmts, vec![set_stmt(vec![dspec("y")])]);
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

/// Constructeur d'un `DsStmt::Array` simple pour les tests.
fn array_stmt(
    name: &str,
    dims: Option<Vec<usize>>,
    char_len: Option<usize>,
    vars: Vec<&str>,
) -> DsStmt {
    DsStmt::Array {
        name: name.to_string(),
        dims,
        char_len,
        vars: vars.into_iter().map(String::from).collect(),
        initial: vec![],
        temporary: false,
        special: None,
    }
}

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
    let DsStmt::Set { specs, .. } = &ast.stmts[0] else {
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
    let DsStmt::Set { specs, .. } = &ast.stmts[0] else {
        panic!("expected a SET statement");
    };
    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].options.keep, Some(vec!["x".to_string()]));
    assert_eq!(specs[1], dspec("b"));
}

// ── SET options END= / NOBS= / POINT= (M16.4) ─────────────────────────

#[test]
fn set_end_nobs_point_options_parse() {
    let ast = parse("data o; set a b end=eof nobs=n point=p; run;").unwrap();
    let DsStmt::Set { specs, options } = &ast.stmts[0] else {
        panic!("expected a SET statement");
    };
    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0], dspec("a"));
    assert_eq!(specs[1], dspec("b"));
    assert_eq!(options.end.as_deref(), Some("eof"));
    assert_eq!(options.nobs.as_deref(), Some("n"));
    assert_eq!(options.point.as_deref(), Some("p"));
}

#[test]
fn set_options_order_independent() {
    let ast = parse("data o; set a point=p end=eof; run;").unwrap();
    let DsStmt::Set { options, .. } = &ast.stmts[0] else {
        panic!("expected a SET statement");
    };
    assert_eq!(options.point.as_deref(), Some("p"));
    assert_eq!(options.end.as_deref(), Some("eof"));
    assert!(options.nobs.is_none());
}

#[test]
fn set_without_options_has_default() {
    let ast = parse("data o; set a; run;").unwrap();
    let DsStmt::Set { options, .. } = &ast.stmts[0] else {
        panic!("expected a SET statement");
    };
    assert_eq!(*options, crate::ast::SetOptions::default());
}

#[test]
fn set_unknown_option_errors() {
    let err = parse("data o; set a bogus=z; run;").unwrap_err();
    assert!(
        err.to_string().contains("unknown SET option"),
        "got: {err}"
    );
}

#[test]
fn set_duplicate_option_errors() {
    let err = parse("data o; set a end=e1 end=e2; run;").unwrap_err();
    assert!(
        err.to_string().contains("more than once"),
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

// ── INFILE / INPUT / DATALINES (M14) ─────────────────────────────────

fn var_item(name: &str, is_char: bool) -> InputItem {
    InputItem::Var {
        name: name.to_string(),
        is_char,
        cols: None,
        informat: None,
        list_modifier: false,
    }
}

#[test]
fn input_list_mode_dollar() {
    let ast = parse("data o; input name $ age height; datalines;\nx 1 2\n;\nrun;").unwrap();
    let DsStmt::Input(items) = &ast.stmts[0] else {
        panic!("expected an INPUT statement, got {:?}", ast.stmts[0]);
    };
    assert_eq!(
        items,
        &vec![
            var_item("name", true),
            var_item("age", false),
            var_item("height", false),
        ]
    );
    // Le bloc datalines est capturé.
    assert_eq!(
        ast.stmts[1],
        DsStmt::Datalines(vec!["x 1 2".to_string()])
    );
}

#[test]
fn input_column_mode() {
    let ast = parse("data o; input name $ 1-10 age 11-13; datalines;\n;\nrun;").unwrap();
    let DsStmt::Input(items) = &ast.stmts[0] else {
        panic!("expected an INPUT statement");
    };
    assert_eq!(
        items,
        &vec![
            InputItem::Var {
                name: "name".to_string(),
                is_char: true,
                cols: Some((1, 10)),
                informat: None,
                list_modifier: false,
            },
            InputItem::Var {
                name: "age".to_string(),
                is_char: false,
                cols: Some((11, 13)),
                informat: None,
                list_modifier: false,
            },
        ]
    );
}

#[test]
fn input_formatted_mode() {
    let ast =
        parse("data o; input name $char10. d date9. x 8.2; datalines;\n;\nrun;").unwrap();
    let DsStmt::Input(items) = &ast.stmts[0] else {
        panic!("expected an INPUT statement");
    };
    assert_eq!(
        items,
        &vec![
            InputItem::Var {
                name: "name".to_string(),
                is_char: false,
                cols: None,
                informat: Some("$char10.".to_string()),
                list_modifier: false,
            },
            InputItem::Var {
                name: "d".to_string(),
                is_char: false,
                cols: None,
                informat: Some("date9.".to_string()),
                list_modifier: false,
            },
            InputItem::Var {
                name: "x".to_string(),
                is_char: false,
                cols: None,
                informat: Some("8.2".to_string()),
                list_modifier: false,
            },
        ]
    );
}

#[test]
fn input_list_modifier_colon() {
    let ast = parse("data o; input x :date9.; datalines;\n;\nrun;").unwrap();
    let DsStmt::Input(items) = &ast.stmts[0] else {
        panic!("expected an INPUT statement");
    };
    assert_eq!(
        items,
        &vec![InputItem::Var {
            name: "x".to_string(),
            is_char: false,
            cols: None,
            informat: Some("date9.".to_string()),
            list_modifier: true,
        }]
    );
}

#[test]
fn input_pointers_and_holds() {
    let ast = parse("data o; input @5 x 8. +2 y / z @@; datalines;\n;\nrun;").unwrap();
    let DsStmt::Input(items) = &ast.stmts[0] else {
        panic!("expected an INPUT statement");
    };
    assert_eq!(items[0], InputItem::ColumnPointer(5));
    assert_eq!(
        items[1],
        InputItem::Var {
            name: "x".to_string(),
            is_char: false,
            cols: None,
            informat: Some("8.".to_string()),
            list_modifier: false,
        }
    );
    assert_eq!(items[2], InputItem::SkipColumns(2));
    assert_eq!(items[3], var_item("y", false));
    assert_eq!(items[4], InputItem::NextLine);
    assert_eq!(items[5], var_item("z", false));
    assert_eq!(items[6], InputItem::HoldLineDouble);
}

#[test]
fn input_trailing_hold_single() {
    let ast = parse("data o; input x @; run;").unwrap();
    let DsStmt::Input(items) = &ast.stmts[0] else {
        panic!("expected an INPUT statement");
    };
    assert_eq!(items[0], var_item("x", false));
    assert_eq!(items[1], InputItem::HoldLine);
}

#[test]
fn infile_datalines_with_options() {
    let ast =
        parse("data o; infile datalines dlm=',' dsd missover; input a b; datalines;\n;\nrun;")
            .unwrap();
    let DsStmt::Infile { source, options } = &ast.stmts[0] else {
        panic!("expected an INFILE statement, got {:?}", ast.stmts[0]);
    };
    assert_eq!(*source, InfileSource::Datalines);
    assert_eq!(options.delimiter.as_deref(), Some(","));
    assert!(options.dsd);
    assert!(options.missover);
}

#[test]
fn infile_path_with_numeric_options() {
    let ast =
        parse("data o; infile 'data.txt' firstobs=2 obs=10 lrecl=256 truncover; input x; run;")
            .unwrap();
    let DsStmt::Infile { source, options } = &ast.stmts[0] else {
        panic!("expected an INFILE statement");
    };
    assert_eq!(*source, InfileSource::Path("data.txt".to_string()));
    assert_eq!(options.firstobs, Some(2));
    assert_eq!(options.obs, Some(10));
    assert_eq!(options.lrecl, Some(256));
    assert!(options.truncover);
}

#[test]
fn infile_unknown_option_errors() {
    let err = parse("data o; infile datalines frobnicate; input x; run;").unwrap_err();
    assert!(
        err.to_string().contains("INFILE option FROBNICATE is not supported."),
        "got: {err}"
    );
}

#[test]
fn datalines_without_infile_parses() {
    // `datalines;` peut alimenter `input` sans `infile datalines;`.
    let ast = parse("data o; input x y; datalines;\n1 2\n3 4\n;\nrun;").unwrap();
    assert!(matches!(ast.stmts[0], DsStmt::Input(_)));
    assert_eq!(
        ast.stmts[1],
        DsStmt::Datalines(vec!["1 2".to_string(), "3 4".to_string()])
    );
}

#[test]
fn cards4_terminator_variant() {
    let ast = parse("data o; input x; cards4;\n1;2\n;;;;\nrun;").unwrap();
    assert_eq!(
        ast.stmts[1],
        DsStmt::Datalines(vec!["1;2".to_string()])
    );
}

// ── FILE / PUT (M14.2) ───────────────────────────────────────────────

#[test]
fn file_destinations() {
    let ast = parse("data _null_; file print; file log; file 'out.txt'; run;").unwrap();
    assert_eq!(ast.stmts[0], DsStmt::File { dest: PutDest::Print });
    assert_eq!(ast.stmts[1], DsStmt::File { dest: PutDest::Log });
    assert_eq!(
        ast.stmts[2],
        DsStmt::File {
            dest: PutDest::Path("out.txt".to_string())
        }
    );
}

#[test]
fn file_bad_destination_errors() {
    let err = parse("data _null_; file frobnicate; run;").unwrap_err();
    assert!(
        err.to_string()
            .contains("expected a quoted file path, LOG or PRINT after FILE"),
        "got: {err}"
    );
}

#[test]
fn put_list_named_literal() {
    let ast = parse("data _null_; put 'hi' name age=; run;").unwrap();
    let DsStmt::Put(items) = &ast.stmts[0] else {
        panic!("expected a PUT statement, got {:?}", ast.stmts[0]);
    };
    assert_eq!(
        items,
        &vec![
            PutItem::Literal("hi".to_string()),
            PutItem::Var {
                name: "name".to_string(),
                format: None,
            },
            PutItem::NamedVar("age".to_string()),
        ]
    );
}

#[test]
fn put_formatted_and_pointers() {
    let ast = parse("data _null_; put @5 x 8.2 +2 d date9. / y @@; run;").unwrap();
    let DsStmt::Put(items) = &ast.stmts[0] else {
        panic!("expected a PUT statement");
    };
    assert_eq!(items[0], PutItem::ColumnPointer(5));
    assert_eq!(
        items[1],
        PutItem::Var {
            name: "x".to_string(),
            format: Some("8.2".to_string()),
        }
    );
    assert_eq!(items[2], PutItem::SkipColumns(2));
    assert_eq!(
        items[3],
        PutItem::Var {
            name: "d".to_string(),
            format: Some("date9.".to_string()),
        }
    );
    assert_eq!(items[4], PutItem::NextLine);
    assert_eq!(
        items[5],
        PutItem::Var {
            name: "y".to_string(),
            format: None,
        }
    );
    assert_eq!(items[6], PutItem::HoldLineDouble);
}

#[test]
fn put_all_and_single_hold() {
    let ast = parse("data _null_; put _all_ @; run;").unwrap();
    let DsStmt::Put(items) = &ast.stmts[0] else {
        panic!("expected a PUT statement");
    };
    assert_eq!(items[0], PutItem::All);
    assert_eq!(items[1], PutItem::HoldLine);
}

#[test]
fn put_empty_is_blank_line() {
    let ast = parse("data _null_; put; run;").unwrap();
    assert_eq!(ast.stmts[0], DsStmt::Put(Vec::new()));
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

// ── SELECT / WHEN / OTHERWISE (M16.1) ────────────────────────────────

#[test]
fn select_selector_form_parses() {
    let ast = parse(
        "data o; select (x); when (1, 2) y = 1; when (3) y = 2; otherwise y = 0; end; run;",
    )
    .unwrap();
    let DsStmt::Select {
        selector,
        whens,
        otherwise,
    } = &ast.stmts[0]
    else {
        panic!("expected a SELECT statement");
    };
    assert_eq!(*selector, Some(var("x")));
    assert_eq!(whens.len(), 2);
    // Première clause : deux valeurs.
    assert_eq!(whens[0].values, vec![Expr::Num(1.0), Expr::Num(2.0)]);
    assert_eq!(
        *whens[0].body,
        DsStmt::Assign {
            var: "y".to_string(),
            expr: Expr::Num(1.0),
        }
    );
    assert_eq!(whens[1].values, vec![Expr::Num(3.0)]);
    assert!(otherwise.is_some());
}

#[test]
fn select_boolean_form_parses() {
    let ast = parse(
        "data o; select; when (x < 1) y = 1; otherwise y = 0; end; run;",
    )
    .unwrap();
    let DsStmt::Select {
        selector, whens, ..
    } = &ast.stmts[0]
    else {
        panic!("expected a SELECT statement");
    };
    assert_eq!(*selector, None);
    // Forme booléenne : une seule expression (la condition) par WHEN.
    assert_eq!(whens[0].values.len(), 1);
    assert_eq!(
        whens[0].values[0],
        Expr::Binary {
            op: BinaryOp::Lt,
            left: Box::new(var("x")),
            right: Box::new(Expr::Num(1.0)),
        }
    );
}

#[test]
fn select_do_block_body_parses() {
    let ast = parse(
        "data o; select (x); when (1) do; a = 1; b = 2; end; end; run;",
    )
    .unwrap();
    let DsStmt::Select { whens, .. } = &ast.stmts[0] else {
        panic!("expected a SELECT statement");
    };
    let DsStmt::Block(stmts) = &*whens[0].body else {
        panic!("expected a DO block body");
    };
    assert_eq!(stmts.len(), 2);
}

#[test]
fn select_boolean_when_rejects_value_list() {
    // En forme booléenne, une liste de valeurs `when (a, b)` est illégale.
    let err = parse("data o; select; when (1, 2) y = 1; end; run;").unwrap_err();
    assert!(
        err.to_string().contains("single expression"),
        "got: {err}"
    );
}

#[test]
fn select_missing_end_is_error() {
    let err = parse("data o; select (x); when (1) y = 1; run;").unwrap_err();
    assert!(err.to_string().contains("missing END"), "got: {err}");
}

#[test]
fn select_empty_when_list_is_error() {
    let err = parse("data o; select (x); when () y = 1; end; run;").unwrap_err();
    assert!(
        err.to_string().contains("at least one value"),
        "got: {err}"
    );
}

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
