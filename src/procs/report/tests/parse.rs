use super::super::*;
use super::*;
use crate::dataset::SasDataset;
use polars::df;

// ───────────────────────── parse tests ─────────────────────────

#[test]
fn parse_minimal() {
    let ast = parse_report("proc report data=a nowd; run;").unwrap();
    assert_eq!(ast.data.as_ref().unwrap().name, "a");
    assert!(!ast.noheader);
    assert!(ast.columns.is_none());
    assert!(ast.defines.is_empty());
}

#[test]
fn parse_column_and_defines() {
    let ast = parse_report(
        "proc report data=a nowd; column region sales; \
         define region / group 'Region'; \
         define sales / analysis sum 'Total Sales'; run;",
    )
    .unwrap();
    assert_eq!(
        ast.columns,
        Some(vec!["region".to_string(), "sales".to_string()])
    );
    assert_eq!(ast.defines.len(), 2);
    assert_eq!(ast.defines[0].usage, Usage::Group);
    assert_eq!(ast.defines[0].label.as_deref(), Some("Region"));
    assert_eq!(ast.defines[1].usage, Usage::Analysis("sum".to_string()));
    assert_eq!(ast.defines[1].label.as_deref(), Some("Total Sales"));
}

#[test]
fn parse_order_descending() {
    let ast = parse_report("proc report data=a; define x / order order=descending; run;").unwrap();
    assert_eq!(ast.defines[0].usage, Usage::Order);
    assert_eq!(ast.defines[0].order, OrderDir::Descending);
}

#[test]
fn parse_analysis_default_stat_is_sum() {
    let ast = parse_report("proc report data=a; define x / analysis; run;").unwrap();
    assert_eq!(ast.defines[0].usage, Usage::Analysis("sum".to_string()));
}

#[test]
fn parse_noheader_option() {
    let ast = parse_report("proc report data=a noheader; run;").unwrap();
    assert!(ast.noheader);
}

#[test]
fn parse_columns_keyword_alias() {
    let ast = parse_report("proc report data=a; columns x y; run;").unwrap();
    assert_eq!(ast.columns, Some(vec!["x".to_string(), "y".to_string()]));
}

#[test]
fn parse_bad_proc_option_errors() {
    let r = parse_report("proc report data=a frobnicate; run;");
    assert!(r.is_err());
    assert!(r.err().unwrap().to_string().contains("FROBNICATE"));
}

#[test]
fn parse_across_usage_now_parses() {
    let ast = parse_report("proc report data=a; define x / across; run;").unwrap();
    assert_eq!(ast.defines[0].usage, Usage::Across);
}

#[test]
fn parse_compute_block_now_parses() {
    let ast = parse_report("proc report data=a; compute after; line 'hi'; endcomp; run;").unwrap();
    assert_eq!(ast.computes.len(), 1);
    assert_eq!(ast.computes[0].target, "after");
    assert!(matches!(ast.computes[0].stmts[0], ComputeStmt::Line(_)));
}

#[test]
fn parse_compute_assignment() {
    let ast =
        parse_report("proc report data=a; compute pct; pct = sales * 2; endcomp; run;").unwrap();
    match &ast.computes[0].stmts[0] {
        ComputeStmt::Assign { col, .. } => assert_eq!(col, "pct"),
        _ => panic!("expected assignment"),
    }
}

#[test]
fn parse_break_now_parses() {
    let ast = parse_report("proc report data=a; break after region / summarize; run;").unwrap();
    assert_eq!(ast.breaks.len(), 1);
    assert_eq!(ast.breaks[0].var.as_deref(), Some("region"));
    assert!(ast.breaks[0].summarize);
}

#[test]
fn parse_rbreak_now_parses() {
    let ast = parse_report("proc report data=a; rbreak after / summarize; run;").unwrap();
    assert!(ast.rbreak.is_some());
    assert!(ast.rbreak.as_ref().unwrap().var.is_none());
    assert!(ast.rbreak.as_ref().unwrap().summarize);
}

#[test]
fn parse_where_statement() {
    let ast = parse_report("proc report data=a; where age > 12; run;").unwrap();
    assert!(ast.where_.is_some());
}

#[test]
fn parse_out_option() {
    let ast = parse_report("proc report data=a out=work.b nowd; run;").unwrap();
    assert_eq!(ast.out.as_ref().unwrap().name, "b");
}

#[test]
fn parse_computed_define() {
    let ast = parse_report("proc report data=a; define c / computed; run;").unwrap();
    assert_eq!(ast.defines[0].usage, Usage::Computed);
}

#[test]
fn parse_unknown_define_option_errors() {
    let r = parse_report("proc report data=a; define x / display flow; run;");
    let msg = r.err().unwrap().to_string();
    assert!(msg.contains("FLOW"), "msg: {msg}");
}

#[test]
fn parse_define_format_width_spacing() {
    let ast = parse_report(
        "proc report data=a; \
         define x / analysis sum format=dollar8.2 width=10 spacing=4; run;",
    )
    .unwrap();
    let d = &ast.defines[0];
    assert_eq!(d.format.as_deref(), Some("dollar8.2"));
    assert_eq!(d.width, Some(10));
    assert_eq!(d.spacing, Some(4));
}

#[test]
fn parse_define_flow_still_errors() {
    // FLOW is genuinely deferred → clean error at parse.
    let r = parse_report("proc report data=a; define x / display flow; run;");
    assert!(r.err().unwrap().to_string().contains("FLOW"));
}

#[test]
fn parse_line_with_pointer_and_format() {
    // `line @5 total best8.;` parses to Pointer + Expr-with-format.
    let ast = parse_report("proc report data=a; compute after; line @5 age best8.; endcomp; run;")
        .unwrap();
    match &ast.computes[0].stmts[0] {
        ComputeStmt::Line(items) => {
            assert!(matches!(items[0], LineItem::Pointer(5)));
            match &items[1] {
                LineItem::Expr(_, fmt) => assert_eq!(fmt.as_deref(), Some("best8.")),
                _ => panic!("expected Expr item"),
            }
        }
        _ => panic!("expected LINE"),
    }
}

// ───────────────────────── execute tests ─────────────────────────

#[test]
fn detail_report_explicit_column_order() {
    let mut session = make_session();
    let df = df![
        "name" => ["Alice", "Bob", "Carol"],
        "age" => [30.0_f64, 25.0, 40.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("name"), num_meta("age")],
    };
    write_dataset(&mut session, "T", ds);

    // Reverse order: age then name. All DISPLAY (force via define for age
    // so it does NOT trigger summary; numeric default would be ANALYSIS
    // but with no group it's still a detail report → raw per-row value).
    let ast = ReportAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        noheader: false,
        columns: Some(vec!["age".into(), "name".into()]),
        defines: vec![],
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    // All three names and ages present (raw per-row values).
    assert!(listing.contains("Alice"), "listing: {listing}");
    assert!(listing.contains("Bob"), "listing: {listing}");
    assert!(listing.contains("Carol"), "listing: {listing}");
    assert!(listing.contains("30"), "listing: {listing}");
    assert!(listing.contains("25"), "listing: {listing}");
    assert!(listing.contains("40"), "listing: {listing}");
    // age column header before name (column order honored).
    let i_age = listing.find("age").unwrap();
    let i_name = listing.find("name").unwrap();
    assert!(i_age < i_name, "age header should precede name: {listing}");
}

#[test]
fn summary_report_group_sum_and_mean() {
    let mut session = make_session();
    let df = df![
        "region" => ["East", "East", "West"],
        "sales" => [10.0_f64, 30.0, 100.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("region"), num_meta("sales")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = ReportAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        noheader: false,
        columns: Some(vec!["region".into(), "sales".into()]),
        defines: vec![
            Define {
                var: "region".into(),
                usage: Usage::Group,
                order: OrderDir::Ascending,
                label: None,
                format: None,
                width: None,
                spacing: None,
            },
            Define {
                var: "sales".into(),
                usage: Usage::Analysis("sum".into()),
                order: OrderDir::Ascending,
                label: None,
                format: None,
                width: None,
                spacing: None,
            },
        ],
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    // East sum = 40, West sum = 100. Two group rows.
    assert!(listing.contains("East"), "listing: {listing}");
    assert!(listing.contains("West"), "listing: {listing}");
    assert!(listing.contains("40"), "East total 40: {listing}");
    assert!(listing.contains("100"), "West total 100: {listing}");
}

#[test]
fn summary_report_mean_stat() {
    let mut session = make_session();
    let df = df![
        "g" => ["a", "a", "b"],
        "x" => [2.0_f64, 4.0, 9.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("g"), num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = ReportAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        noheader: false,
        columns: Some(vec!["g".into(), "x".into()]),
        defines: vec![
            Define {
                var: "g".into(),
                usage: Usage::Group,
                order: OrderDir::Ascending,
                label: None,
                format: None,
                width: None,
                spacing: None,
            },
            Define {
                var: "x".into(),
                usage: Usage::Analysis("mean".into()),
                order: OrderDir::Ascending,
                label: None,
                format: None,
                width: None,
                spacing: None,
            },
        ],
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    // group a mean = 3, group b mean = 9.
    assert!(listing.contains("3"), "a mean 3: {listing}");
    assert!(listing.contains("9"), "b mean 9: {listing}");
}

#[test]
fn order_keeps_distinct_rows_group_collapses() {
    // ORDER variable with one analysis column: each distinct value of the
    // order var produces one row, identical to GROUP for a key tuple.
    let mut session = make_session();
    let df = df![
        "k" => [1.0_f64, 1.0, 2.0],
        "v" => [5.0_f64, 7.0, 11.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("k"), num_meta("v")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = ReportAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        noheader: false,
        columns: Some(vec!["k".into(), "v".into()]),
        defines: vec![
            Define {
                var: "k".into(),
                usage: Usage::Order,
                order: OrderDir::Ascending,
                label: None,
                format: None,
                width: None,
                spacing: None,
            },
            Define {
                var: "v".into(),
                usage: Usage::Analysis("sum".into()),
                order: OrderDir::Ascending,
                label: None,
                format: None,
                width: None,
                spacing: None,
            },
        ],
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    // k=1 → sum 12, k=2 → sum 11. Two rows.
    assert!(listing.contains("12"), "k=1 sum 12: {listing}");
    assert!(listing.contains("11"), "k=2 sum 11: {listing}");
}

#[test]
fn define_label_appears_and_noheader_suppresses() {
    let mut session = make_session();
    let df = df!["x" => [1.0_f64, 2.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    // With label, no group → detail report. Header shows label.
    let ast = ReportAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        noheader: false,
        columns: Some(vec!["x".into()]),
        defines: vec![Define {
            var: "x".into(),
            usage: Usage::Display,
            order: OrderDir::Ascending,
            label: Some("My X Label".into()),
            format: None,
            width: None,
            spacing: None,
        }],
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("My X Label"), "label header: {listing}");

    // Now noheader: label must NOT appear.
    let mut session2 = make_session();
    let df2 = df!["x" => [1.0_f64, 2.0]].unwrap();
    let ds2 = SasDataset {
        df: df2,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session2, "T", ds2);
    let ast2 = ReportAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        noheader: true,
        columns: Some(vec!["x".into()]),
        defines: vec![Define {
            var: "x".into(),
            usage: Usage::Display,
            order: OrderDir::Ascending,
            label: Some("My X Label".into()),
            format: None,
            width: None,
            spacing: None,
        }],
        ..report_defaults()
    };
    execute(&ast2, &mut session2).unwrap();
    let listing2 = session2.listing.into_string();
    assert!(
        !listing2.contains("My X Label"),
        "noheader must suppress label: {listing2}"
    );
    // Data still present.
    assert!(listing2.contains('1'), "data present: {listing2}");
}
