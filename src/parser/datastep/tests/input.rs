use super::*;
use crate::ast::{BinaryOp, Expr};

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
    assert_eq!(ast.stmts[1], DsStmt::Datalines(vec!["x 1 2".to_string()]));
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
    let ast = parse("data o; input name $char10. d date9. x 8.2; datalines;\n;\nrun;").unwrap();
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
        err.to_string()
            .contains("INFILE option FROBNICATE is not supported."),
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
    assert_eq!(ast.stmts[1], DsStmt::Datalines(vec!["1;2".to_string()]));
}

// ── FILE / PUT (M14.2) ───────────────────────────────────────────────

#[test]
fn file_destinations() {
    let ast = parse("data _null_; file print; file log; file 'out.txt'; run;").unwrap();
    assert_eq!(
        ast.stmts[0],
        DsStmt::File {
            dest: PutDest::Print
        }
    );
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

// ── SELECT / WHEN / OTHERWISE (M16.1) ────────────────────────────────

#[test]
fn select_selector_form_parses() {
    let ast =
        parse("data o; select (x); when (1, 2) y = 1; when (3) y = 2; otherwise y = 0; end; run;")
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
    let ast = parse("data o; select; when (x < 1) y = 1; otherwise y = 0; end; run;").unwrap();
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
    let ast = parse("data o; select (x); when (1) do; a = 1; b = 2; end; end; run;").unwrap();
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
    assert!(err.to_string().contains("single expression"), "got: {err}");
}

#[test]
fn select_missing_end_is_error() {
    let err = parse("data o; select (x); when (1) y = 1; run;").unwrap_err();
    assert!(err.to_string().contains("missing END"), "got: {err}");
}

#[test]
fn select_empty_when_list_is_error() {
    let err = parse("data o; select (x); when () y = 1; end; run;").unwrap_err();
    assert!(err.to_string().contains("at least one value"), "got: {err}");
}
