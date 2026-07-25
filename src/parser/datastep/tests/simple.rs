use super::super::*;
use super::*;
use crate::ast::{BinaryOp, DatasetRef, DatasetSpec, Expr};
use crate::source::SourceFile;

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
