use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::session::Session;
use crate::source::SourceFile;
use crate::value::VarType;
use polars::df;
use std::path::PathBuf;

fn make_session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

fn parse_report(src: &str) -> Result<ReportAst> {
    let source = SourceFile::new(src);
    let mut ts = crate::parser::StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "report"
    parse(&mut ts)
}

fn num_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}

fn char_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Char,
        length: 8,
        format: None,
        label: None,
    }
}

fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
    session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
    session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
}

/// Defaults for the advanced (M21.4) ReportAst fields, used with Rust's
/// struct-update syntax (`..report_defaults()`) in the execute tests.
fn report_defaults() -> ReportAst {
    ReportAst {
        data: None,
        noheader: false,
        columns: None,
        defines: vec![],
        where_: None,
        out: None,
        breaks: vec![],
        rbreak: None,
        computes: vec![],
    }
}

fn work_ref(name: &str) -> DatasetRef {
    DatasetRef {
        libref: Some("WORK".into()),
        name: name.into(),
    }
}

/// Parse a standalone expression (e.g. a WHERE condition) for tests. The
/// SourceFile is owned within this scope; the returned Expr is owned.
fn parse_test_expr(src: &str) -> Expr {
    let source = crate::source::SourceFile::new(src);
    let mut ts = crate::parser::StatementStream::new(&source).unwrap();
    crate::parser::expr::parse_expr(&mut ts).unwrap()
}

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
    let ast =
        parse_report("proc report data=a; define x / order order=descending; run;").unwrap();
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
    let ast =
        parse_report("proc report data=a; compute after; line 'hi'; endcomp; run;").unwrap();
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
    let ast =
        parse_report("proc report data=a; break after region / summarize; run;").unwrap();
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

#[test]
fn default_usages_numeric_analysis_char_display() {
    // No defines: numeric → ANALYSIS SUM, char → DISPLAY. Because there's
    // no group/order, this is a DETAIL report (raw per-row values).
    let mut session = make_session();
    let df = df![
        "name" => ["A", "B"],
        "n" => [3.0_f64, 4.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("name"), num_meta("n")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = ReportAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        noheader: false,
        columns: None,
        defines: vec![],
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    // Detail report → both raw rows present.
    assert!(listing.contains("A"), "listing: {listing}");
    assert!(listing.contains("B"), "listing: {listing}");
    assert!(listing.contains('3'), "listing: {listing}");
    assert!(listing.contains('4'), "listing: {listing}");
}

#[test]
fn missing_values_excluded_from_group_mean() {
    let mut session = make_session();
    let df = df![
        "g" => ["a", "a", "a"],
        "x" => [Some(2.0_f64), None, Some(4.0)]
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
    // mean over non-missing [2,4] = 3 (NOT (2+4)/3).
    assert!(listing.contains('3'), "mean excludes missing: {listing}");
}

#[test]
fn no_last_dataset_errors() {
    let mut session = make_session();
    let ast = ReportAst {
        data: None,
        noheader: false,
        columns: None,
        defines: vec![],
        ..report_defaults()
    };
    let r = execute(&ast, &mut session);
    assert!(r.is_err());
    assert!(r.err().unwrap().to_string().contains("_LAST_"));
}

#[test]
fn unknown_column_errors() {
    let mut session = make_session();
    let df = df!["x" => [1.0_f64]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);
    let ast = ReportAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        noheader: false,
        columns: Some(vec!["nope".into()]),
        defines: vec![],
        ..report_defaults()
    };
    let r = execute(&ast, &mut session);
    assert!(r.is_err());
    assert!(r.err().unwrap().to_string().contains("NOPE"));
}

#[test]
fn report_does_not_set_last_dataset() {
    let mut session = make_session();
    let df = df!["x" => [1.0_f64]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);
    // last_dataset is WORK.T after write.
    let before = session.last_dataset.clone();
    let ast = ReportAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        noheader: false,
        columns: None,
        defines: vec![],
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    // REPORT must not change last_dataset.
    assert_eq!(session.last_dataset, before);
}

// ─────────────────── M21.4 advanced feature tests ───────────────────

/// sashelp.class-like sex/age fixture: M/F with weights.
fn class_like(session: &mut Session) {
    // 3 F (ages 11,12,13 → sum 36) and 2 M (ages 14,15 → sum 29).
    let df = df![
        "sex" => ["F", "F", "F", "M", "M"],
        "age" => [11.0_f64, 12.0, 13.0, 14.0, 15.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("sex"), num_meta("age")],
    };
    write_dataset(session, "C", ds);
}

#[test]
fn where_filters_observations() {
    let mut session = make_session();
    class_like(&mut session);
    // where age > 12 → keep ages 13,14,15. Group by sex: F sum=13, M sum=29.
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
        where_: Some(parse_test_expr("age > 12;")),
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    // F filtered sum = 13, M = 29.
    assert!(listing.contains("13"), "F sum 13: {listing}");
    assert!(listing.contains("29"), "M sum 29: {listing}");
    // The filtered-out values 11/12 must not appear as an F total of 36.
    assert!(!listing.contains("36"), "36 should be filtered out: {listing}");
}

#[test]
fn where_char_equality_sas_cmp() {
    let mut session = make_session();
    class_like(&mut session);
    // where sex = 'M' → only M rows; detail report shows 14 and 15, not 11.
    let ast = ReportAst {
        data: Some(work_ref("C")),
        columns: Some(vec!["sex".into(), "age".into()]),
        defines: vec![Define {
            var: "age".into(),
            usage: Usage::Display,
            order: OrderDir::Ascending,
            label: None,
            format: None,
            width: None,
            spacing: None,
        }],
        where_: Some(
            parse_test_expr("sex = 'M';"),
        ),
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("14"), "{listing}");
    assert!(listing.contains("15"), "{listing}");
    assert!(!listing.contains("11"), "11 filtered: {listing}");
}

#[test]
fn out_dataset_written_and_typed() {
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
        out: Some(work_ref("R")),
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    // OUT= sets last_dataset and writes 2 rows (F, M).
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.R"));
    let (out, _) = session.libs.get("WORK").unwrap().read("R").unwrap();
    assert_eq!(out.n_obs(), 2);
    // sex stays char, age stays numeric.
    assert_eq!(out.vars[0].name.to_lowercase(), "sex");
    assert_eq!(out.vars[0].ty, VarType::Char);
    assert_eq!(out.vars[1].ty, VarType::Num);
    let age = decode_column(&out, 1).unwrap();
    // F sum 36, M sum 29.
    assert_eq!(age[0], Value::Num(36.0));
    assert_eq!(age[1], Value::Num(29.0));
}

#[test]
fn across_makes_columns_from_distinct_values() {
    let mut session = make_session();
    // region × sex crosstab of sales sum.
    let df = df![
        "region" => ["E", "E", "W", "W"],
        "sex" => ["F", "M", "F", "M"],
        "sales" => [10.0_f64, 20.0, 30.0, 40.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("region"), char_meta("sex"), num_meta("sales")],
    };
    write_dataset(&mut session, "X", ds);
    let ast = ReportAst {
        data: Some(work_ref("X")),
        columns: Some(vec!["region".into(), "sex".into(), "sales".into()]),
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
                var: "sex".into(),
                usage: Usage::Across,
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
    // Two across columns "F Sum" and "M Sum"; E row → F 10, M 20; W → 30,40.
    assert!(listing.contains("F SUM"), "across header F: {listing}");
    assert!(listing.contains("M SUM"), "across header M: {listing}");
    assert!(listing.contains("10"), "{listing}");
    assert!(listing.contains("20"), "{listing}");
    assert!(listing.contains("30"), "{listing}");
    assert!(listing.contains("40"), "{listing}");
}

#[test]
fn break_after_group_summary_line() {
    let mut session = make_session();
    // Two-level group region/sub; break after region summarizes.
    let df = df![
        "region" => ["E", "E", "W"],
        "sub" => ["a", "b", "c"],
        "sales" => [10.0_f64, 30.0, 100.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("region"), char_meta("sub"), num_meta("sales")],
    };
    write_dataset(&mut session, "B", ds);
    let ast = ReportAst {
        data: Some(work_ref("B")),
        columns: Some(vec!["region".into(), "sub".into(), "sales".into()]),
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
                var: "sub".into(),
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
        breaks: vec![Break {
            var: Some("region".into()),
            summarize: true,
        }],
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    // E subtotal = 10+30 = 40 appears after the E rows.
    assert!(listing.contains("40"), "E subtotal 40: {listing}");
    // W subtotal = 100.
    assert!(listing.contains("100"), "W subtotal 100: {listing}");
}

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
        where_: Some(
            parse_test_expr("x = .;"),
        ),
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains('b'), "missing row kept: {listing}");
    assert!(!listing.contains('a'), "non-missing filtered: {listing}");
}

#[test]
fn across_with_descending_direction() {
    let mut session = make_session();
    let df = df![
        "g" => ["x", "x"],
        "k" => [1.0_f64, 2.0],
        "v" => [5.0_f64, 7.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("g"), num_meta("k"), num_meta("v")],
    };
    write_dataset(&mut session, "AD", ds);
    let ast = ReportAst {
        data: Some(work_ref("AD")),
        columns: Some(vec!["g".into(), "k".into(), "v".into()]),
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
                var: "k".into(),
                usage: Usage::Across,
                order: OrderDir::Descending,
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
    // Descending across order → header for k=2 appears before k=1.
    let i2 = listing.find("2 SUM").unwrap();
    let i1 = listing.find("1 SUM").unwrap();
    assert!(i2 < i1, "descending across: {listing}");
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

// ─────────────────── M33.5 deferred-option tests ───────────────────

/// Build a DEFINE with optional format/width/spacing (M33.5 test helper).
fn def(
    var: &str,
    usage: Usage,
    label: Option<&str>,
    format: Option<&str>,
    width: Option<usize>,
    spacing: Option<usize>,
) -> Define {
    Define {
        var: var.into(),
        usage,
        order: OrderDir::Ascending,
        label: label.map(|s| s.to_string()),
        format: format.map(|s| s.to_string()),
        width,
        spacing,
    }
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
fn format_applies_to_displayed_numeric() {
    // DEFINE / FORMAT=5.1 on a detail numeric column. Oracle: 11 → "11.0".
    let mut session = make_session();
    let df = df!["age" => [11.0_f64, 12.0]].unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("age")] };
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
    let ds = SasDataset { df, vars: vec![char_meta("name")] };
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
    assert!(!listing.contains("Alfred"), "full name truncated away: {listing}");
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
    let ds = SasDataset { df, vars: vec![char_meta("a"), char_meta("b")] };
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
fn compute_reads_cn_positional_reference() {
    // _C2_ is the 2nd COLUMN (age); ratio column = _C2_ / 10. Detail report.
    let mut session = make_session();
    let df = df![
        "sex" => ["F", "M"],
        "age" => [20.0_f64, 30.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![char_meta("sex"), num_meta("age")] };
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
        rbreak: Some(Break { var: None, summarize: true }),
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
    assert!(listing.contains("Total age: 65"), "line with format: {listing}");
}

#[test]
fn parse_line_with_pointer_and_format() {
    // `line @5 total best8.;` parses to Pointer + Expr-with-format.
    let ast = parse_report(
        "proc report data=a; compute after; line @5 age best8.; endcomp; run;",
    )
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
