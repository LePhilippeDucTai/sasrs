use super::super::*;
use super::*;
use crate::dataset::SasDataset;
use crate::value::VarType;
use polars::df;

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
    assert!(
        !listing.contains("36"),
        "36 should be filtered out: {listing}"
    );
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
        where_: Some(parse_test_expr("sex = 'M';")),
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
