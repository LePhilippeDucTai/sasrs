use super::super::*;
use super::*;
use crate::dataset::SasDataset;
use polars::df;

#[test]
fn rbreak_grand_total_line() {
    let mut session = make_session();
    class_like(&mut session);
    let ast = ReportAst {
        data: Some(work_ref("C")),
        columns: Some(vec!["sex".into(), "age".into()]),
        defines: vec![
            Define {
                var: "sex".into(),
                usage: Usage::Group,
                order: OrderDir::Ascending,
                label: None,
                format: None,
                width: None,
                spacing: None,
            },
            Define {
                var: "age".into(),
                usage: Usage::Analysis("sum".into()),
                order: OrderDir::Ascending,
                label: None,
                format: None,
                width: None,
                spacing: None,
            },
        ],
        rbreak: Some(Break {
            var: None,
            summarize: true,
        }),
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    // Grand total of ages = 11+12+13+14+15 = 65.
    assert!(listing.contains("65"), "grand total 65: {listing}");
}

#[test]
fn rbreak_excluded_from_out_dataset() {
    let mut session = make_session();
    class_like(&mut session);
    let ast = ReportAst {
        data: Some(work_ref("C")),
        columns: Some(vec!["sex".into(), "age".into()]),
        defines: vec![
            Define {
                var: "sex".into(),
                usage: Usage::Group,
                order: OrderDir::Ascending,
                label: None,
                format: None,
                width: None,
                spacing: None,
            },
            Define {
                var: "age".into(),
                usage: Usage::Analysis("sum".into()),
                order: OrderDir::Ascending,
                label: None,
                format: None,
                width: None,
                spacing: None,
            },
        ],
        rbreak: Some(Break {
            var: None,
            summarize: true,
        }),
        out: Some(work_ref("RB")),
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let (out, _) = session.libs.get("WORK").unwrap().read("RB").unwrap();
    // Only the 2 group rows; the RBREAK grand total is not written.
    assert_eq!(out.n_obs(), 2);
}

#[test]
fn compute_simple_assignment() {
    let mut session = make_session();
    class_like(&mut session);
    // computed column `dbl` = age * 2 in a detail report.
    let ast = ReportAst {
        data: Some(work_ref("C")),
        columns: Some(vec!["sex".into(), "age".into(), "dbl".into()]),
        defines: vec![
            Define {
                var: "age".into(),
                usage: Usage::Display,
                order: OrderDir::Ascending,
                label: None,
                format: None,
                width: None,
                spacing: None,
            },
            Define {
                var: "dbl".into(),
                usage: Usage::Computed,
                order: OrderDir::Ascending,
                label: None,
                format: None,
                width: None,
                spacing: None,
            },
        ],
        computes: vec![Compute {
            target: "dbl".into(),
            stmts: vec![ComputeStmt::Assign {
                col: "dbl".into(),
                expr: parse_test_expr("age * 2;"),
            }],
        }],
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    // First row age 11 → dbl 22; age 15 → dbl 30.
    assert!(listing.contains("22"), "11*2=22: {listing}");
    assert!(listing.contains("30"), "15*2=30: {listing}");
}

#[test]
fn compute_after_line_text() {
    let mut session = make_session();
    class_like(&mut session);
    let ast = ReportAst {
        data: Some(work_ref("C")),
        columns: Some(vec!["sex".into(), "age".into()]),
        defines: vec![
            Define {
                var: "sex".into(),
                usage: Usage::Group,
                order: OrderDir::Ascending,
                label: None,
                format: None,
                width: None,
                spacing: None,
            },
            Define {
                var: "age".into(),
                usage: Usage::Analysis("sum".into()),
                order: OrderDir::Ascending,
                label: None,
                format: None,
                width: None,
                spacing: None,
            },
        ],
        computes: vec![Compute {
            target: "after".into(),
            stmts: vec![ComputeStmt::Line(vec![LineItem::Literal(
                "End of report".into(),
            )])],
        }],
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("End of report"), "line text: {listing}");
}

#[test]
fn compute_reads_cn_positional_reference() {
    // _C2_ is the 2nd COLUMN (age); ratio column = _C2_ / 10. Detail report.
    let mut session = make_session();
    let df = df![
        "sex" => ["F", "M"],
        "age" => [20.0_f64, 30.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("sex"), num_meta("age")],
    };
    write_dataset(&mut session, "T", ds);
    let ast = ReportAst {
        data: Some(work_ref("T")),
        columns: Some(vec!["sex".into(), "age".into(), "ratio".into()]),
        defines: vec![
            def("age", Usage::Display, None, None, None, None),
            def("ratio", Usage::Computed, None, None, None, None),
        ],
        computes: vec![Compute {
            target: "ratio".into(),
            stmts: vec![ComputeStmt::Assign {
                col: "ratio".into(),
                expr: parse_test_expr("_c2_ / 10;"),
            }],
        }],
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    // age 20 → ratio 2; age 30 → ratio 3.
    assert!(listing.contains('2'), "_c2_/10 = 2: {listing}");
    assert!(listing.contains('3'), "_c2_/10 = 3: {listing}");
}

#[test]
fn where_missing_semantics_dot_equals_dot() {
    let mut session = make_session();
    let df = df![
        "g" => ["a", "b", "c"],
        "x" => [Some(1.0_f64), None, Some(3.0)]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("g"), num_meta("x")],
    };
    write_dataset(&mut session, "M", ds);
    // where x = . → only the row with missing x survives (g='b').
    let ast = ReportAst {
        data: Some(work_ref("M")),
        columns: Some(vec!["g".into(), "x".into()]),
        defines: vec![Define {
            var: "x".into(),
            usage: Usage::Display,
            order: OrderDir::Ascending,
            label: None,
            format: None,
            width: None,
            spacing: None,
        }],
        where_: Some(parse_test_expr("x = .;")),
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains('b'), "missing row kept: {listing}");
    assert!(!listing.contains('a'), "non-missing filtered: {listing}");
}

#[test]
fn break_without_summarize_emits_no_subtotal() {
    let mut session = make_session();
    class_like(&mut session);
    let ast = ReportAst {
        data: Some(work_ref("C")),
        columns: Some(vec!["sex".into(), "age".into()]),
        defines: vec![
            Define {
                var: "sex".into(),
                usage: Usage::Group,
                order: OrderDir::Ascending,
                label: None,
                format: None,
                width: None,
                spacing: None,
            },
            Define {
                var: "age".into(),
                usage: Usage::Analysis("n".into()),
                order: OrderDir::Ascending,
                label: None,
                format: None,
                width: None,
                spacing: None,
            },
        ],
        breaks: vec![Break {
            var: Some("sex".into()),
            summarize: false,
        }],
        ..report_defaults()
    };
    // Should not panic; n for F=3, M=2.
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains('3'), "F n=3: {listing}");
    assert!(listing.contains('2'), "M n=2: {listing}");
}

#[test]
fn format_applies_to_displayed_numeric() {
    // DEFINE / FORMAT=5.1 on a detail numeric column. Oracle: 11 → "11.0".
    let mut session = make_session();
    let df = df!["age" => [11.0_f64, 12.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("age")],
    };
    write_dataset(&mut session, "T", ds);
    let ast = ReportAst {
        data: Some(work_ref("T")),
        columns: Some(vec!["age".into()]),
        defines: vec![def("age", Usage::Display, None, Some("5.1"), None, None)],
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("11.0"), "formatted 11.0: {listing}");
    assert!(listing.contains("12.0"), "formatted 12.0: {listing}");
}

#[test]
fn width_truncates_and_pads_column() {
    // WIDTH=3 on a char column truncates long values to 3 chars.
    let mut session = make_session();
    let df = df!["name" => ["Alfred", "Bo"]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("name")],
    };
    write_dataset(&mut session, "T", ds);
    let ast = ReportAst {
        data: Some(work_ref("T")),
        columns: Some(vec!["name".into()]),
        defines: vec![def("name", Usage::Display, None, None, Some(3), None)],
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    // "Alfred" truncated to "Alf"; full name must NOT appear.
    assert!(listing.contains("Alf"), "truncated to Alf: {listing}");
    assert!(
        !listing.contains("Alfred"),
        "full name truncated away: {listing}"
    );
}

#[test]
fn spacing_changes_intercolumn_gap() {
    // SPACING=6 before the second column → at least 6 spaces precede it.
    let mut session = make_session();
    let df = df![
        "a" => ["x", "y"],
        "b" => ["p", "q"]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("a"), char_meta("b")],
    };
    write_dataset(&mut session, "T", ds);
    let ast = ReportAst {
        data: Some(work_ref("T")),
        noheader: true,
        columns: Some(vec!["a".into(), "b".into()]),
        defines: vec![
            def("a", Usage::Display, None, None, None, None),
            def("b", Usage::Display, None, None, None, Some(6)),
        ],
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    // Row "x" then 6 spaces (spacing) then "p". Default leading spacing is 2
    // on column a. So a data line should contain "x      p" (1+6 = the gap).
    assert!(listing.contains("x      p"), "6-space gap: {listing:?}");
}

#[test]
fn line_with_format_renders_via_format_engine() {
    // compute after; line 'Total: ' age best8.; → grand total formatted.
    let mut session = make_session();
    class_like(&mut session);
    let ast = ReportAst {
        data: Some(work_ref("C")),
        columns: Some(vec!["sex".into(), "age".into()]),
        defines: vec![
            def("sex", Usage::Group, None, None, None, None),
            def("age", Usage::Analysis("sum".into()), None, None, None, None),
        ],
        rbreak: Some(Break {
            var: None,
            summarize: true,
        }),
        computes: vec![Compute {
            target: "after".into(),
            stmts: vec![ComputeStmt::Line(vec![
                LineItem::Literal("Total age: ".into()),
                LineItem::Expr(parse_test_expr("age;"), Some("best8.".into())),
            ])],
        }],
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    // Grand total of ages = 65, rendered by best8. as "65".
    assert!(
        listing.contains("Total age: 65"),
        "line with format: {listing}"
    );
}
