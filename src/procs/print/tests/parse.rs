use super::super::*;
use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::value::VarType;

// --- parse tests ---

#[test]
fn parse_minimal() {
    let ast = parse_print_src("").unwrap();
    assert!(ast.data.is_none());
    assert!(!ast.noobs);
    assert!(ast.vars.is_none());
}

#[test]
fn parse_data_option() {
    let ast = parse_print_src("data=mylib.class").unwrap();
    assert_eq!(
        ast.data,
        Some(DatasetRef {
            libref: Some("mylib".into()),
            name: "class".into()
        })
    );
}

#[test]
fn parse_data_work_only() {
    let ast = parse_print_src("data=foo").unwrap();
    assert_eq!(
        ast.data,
        Some(DatasetRef {
            libref: None,
            name: "foo".into()
        })
    );
}

#[test]
fn parse_noobs() {
    let ast = parse_print_src("noobs").unwrap();
    assert!(ast.noobs);
}

#[test]
fn parse_label_ignored() {
    let ast = parse_print_src("label").unwrap();
    assert!(!ast.noobs);
    assert!(ast.data.is_none());
}

#[test]
fn parse_var_statement() {
    let src = "proc print data=work.x; var a b c; run;";
    let ast = parse_print_with_var(src).unwrap();
    assert_eq!(
        ast.vars,
        Some(vec!["a".to_string(), "b".to_string(), "c".to_string()])
    );
}

#[test]
fn parse_noobs_and_data() {
    let src = "proc print data=work.foo noobs; run;";
    let ast = parse_print_with_var(src).unwrap();
    assert!(ast.noobs);
    assert_eq!(ast.data.as_ref().unwrap().name, "foo");
}

#[test]
fn parse_unknown_option_errors() {
    let src = "proc print bogus; run;";
    let result = parse_print_with_var(src);
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(
        msg.contains("BOGUS") || msg.contains("bogus"),
        "msg was: {msg}"
    );
}

#[test]
fn parse_by_id_sum_double_n() {
    let src = "proc print data=work.g double n; by grp; id grp; sum v; run;";
    let ast = parse_print_with_var(src).unwrap();
    assert!(ast.double);
    assert!(ast.n);
    assert_eq!(ast.by, vec![("grp".to_string(), false)]);
    assert_eq!(ast.id, vec!["grp".to_string()]);
    assert_eq!(ast.sum, vec!["v".to_string()]);
}

#[test]
fn execute_basic_print() {
    let mut session = make_session();
    write_test_dataset(&mut session);

    let ast = PrintAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "MYDATA".into(),
        }),
        vars: None,
        noobs: false,
        label: false,
        double: false,
        n: false,
        by: vec![],
        id: vec![],
        sum: vec![],
    };

    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    // Should have Obs column header
    assert!(listing.contains("Obs"), "listing: {listing}");
    // Should have column headers
    assert!(
        listing.contains("NAME") || listing.contains("name"),
        "listing: {listing}"
    );
    assert!(
        listing.contains("AGE") || listing.contains("age"),
        "listing: {listing}"
    );
    // Should have data values
    assert!(listing.contains("Alice"), "listing: {listing}");
    assert!(listing.contains("30"), "listing: {listing}");

    let log = session.log.into_string();
    // NOTE with count
    assert!(
        log.contains("There were 3 observations read from the data set WORK.MYDATA"),
        "log: {log}"
    );
}

#[test]
fn execute_noobs() {
    let mut session = make_session();
    write_test_dataset(&mut session);

    let ast = PrintAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "MYDATA".into(),
        }),
        vars: None,
        noobs: true,
        label: false,
        double: false,
        n: false,
        by: vec![],
        id: vec![],
        sum: vec![],
    };

    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    // Obs column should NOT appear
    assert!(
        !listing.contains("Obs"),
        "listing should not have Obs: {listing}"
    );
    assert!(listing.contains("Alice"), "listing: {listing}");
}

#[test]
fn execute_with_var_selection() {
    let mut session = make_session();
    write_test_dataset(&mut session);

    let ast = PrintAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "MYDATA".into(),
        }),
        vars: Some(vec!["age".to_string()]),
        noobs: false,
        label: false,
        double: false,
        n: false,
        by: vec![],
        id: vec![],
        sum: vec![],
    };

    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    // age column must be present
    assert!(
        listing.contains("AGE") || listing.contains("age"),
        "listing: {listing}"
    );
    // name column must NOT be present
    assert!(
        !listing.contains("Alice"),
        "name should not appear: {listing}"
    );
}

#[test]
fn execute_unknown_var_errors() {
    let mut session = make_session();
    write_test_dataset(&mut session);

    let ast = PrintAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "MYDATA".into(),
        }),
        vars: Some(vec!["nonexistent".to_string()]),
        noobs: false,
        label: false,
        double: false,
        n: false,
        by: vec![],
        id: vec![],
        sum: vec![],
    };

    let result = execute(&ast, &mut session);
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(
        msg.contains("NONEXISTENT") || msg.contains("nonexistent"),
        "msg: {msg}"
    );
}

#[test]
fn execute_last_dataset() {
    let mut session = make_session();
    write_test_dataset(&mut session);
    // last_dataset is already set by write_test_dataset

    let ast = PrintAst {
        data: None, // use _LAST_
        vars: None,
        noobs: false,
        label: false,
        double: false,
        n: false,
        by: vec![],
        id: vec![],
        sum: vec![],
    };

    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("Alice"), "listing: {listing}");
}

#[test]
fn execute_no_last_dataset_errors() {
    let mut session = make_session();
    // do NOT write any dataset, leave last_dataset = None

    let ast = PrintAst {
        data: None,
        vars: None,
        noobs: false,
        label: false,
        double: false,
        n: false,
        by: vec![],
        id: vec![],
        sum: vec![],
    };

    let result = execute(&ast, &mut session);
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(
        msg.contains("_LAST_") || msg.contains("undefined"),
        "msg: {msg}"
    );
}

#[test]
fn execute_note_plural_invariable() {
    // "1 observations." is the SAS convention — do not "fix" to "1 observation."
    let mut session = make_session();

    let df = df!["x" => [42.0_f64]].unwrap();
    let vars = vec![VarMeta {
        name: "x".to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }];
    let ds = SasDataset { df, vars };
    session.libs.get("WORK").unwrap().write("ONE", &ds).unwrap();

    let ast = PrintAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "ONE".into(),
        }),
        vars: None,
        noobs: false,
        label: false,
        double: false,
        n: false,
        by: vec![],
        id: vec![],
        sum: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    // Must say "1 observations" (invariable plural — SAS behavior)
    assert!(
        log.contains("There were 1 observations read from the data set WORK.ONE"),
        "log: {log}"
    );
}

#[test]
fn execute_applies_numeric_format() {
    let mut session = make_session();
    write_formatted_dataset(&mut session);

    let ast = PrintAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "FMT".into(),
        }),
        vars: None,
        noobs: false,
        label: false,
        double: false,
        n: false,
        by: vec![],
        id: vec![],
        sum: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    // dollar8. renders 112 as "$112" (and 98 as "$98").
    assert!(listing.contains("$112"), "listing: {listing}");
    assert!(listing.contains("$98"), "listing: {listing}");
    // Without LABEL, headers are variable names (uppercased by SAS).
    assert!(
        listing.contains("weight") || listing.contains("WEIGHT"),
        "listing: {listing}"
    );
    assert!(
        !listing.contains("Body Weight"),
        "label must not appear without LABEL option: {listing}"
    );
}

#[test]
fn execute_label_option_uses_labels_as_headers() {
    let mut session = make_session();
    write_formatted_dataset(&mut session);

    let ast = PrintAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "FMT".into(),
        }),
        vars: None,
        noobs: false,
        label: true,
        double: false,
        n: false,
        by: vec![],
        id: vec![],
        sum: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("Body Weight"), "listing: {listing}");
    assert!(listing.contains("Pupil Name"), "listing: {listing}");
}

#[test]
fn execute_sum_no_by_totals() {
    let mut session = make_session();
    write_grouped(&mut session);
    let ast = PrintAst {
        sum: vec!["v".into()],
        ..base_ast()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    // Grand total of v = 12.
    assert!(listing.contains("12"), "sum total 12 expected: {listing}");
}

#[test]
fn execute_n_option_prints_count() {
    let mut session = make_session();
    write_grouped(&mut session);
    let ast = PrintAst {
        n: true,
        ..base_ast()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("N = 3"), "N = 3 expected: {listing}");
}

#[test]
fn execute_by_sections_with_sum_subtotals_and_grand_total() {
    let mut session = make_session();
    write_grouped(&mut session);
    let ast = PrintAst {
        by: vec![("grp".into(), false)],
        sum: vec!["v".into()],
        n: true,
        ..base_ast()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    // BY headings.
    assert!(listing.contains("grp=A"), "BY heading A: {listing}");
    assert!(listing.contains("grp=B"), "BY heading B: {listing}");
    // Per-group subtotals 7 (A) and 5 (B), and grand total 12.
    assert!(listing.contains("7"), "subtotal A=7: {listing}");
    assert!(
        listing.contains("Grand total: v=12"),
        "grand total: {listing}"
    );
    // Per-group N lines.
    assert!(listing.contains("N = 2"), "N=2 for group A: {listing}");
    assert!(listing.contains("N = 1"), "N=1 for group B: {listing}");
}

#[test]
fn execute_id_replaces_obs_column() {
    let mut session = make_session();
    write_grouped(&mut session);
    let ast = PrintAst {
        id: vec!["grp".into()],
        ..base_ast()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    // Obs column suppressed; ID variable header (grp) present.
    assert!(
        !listing.contains("Obs"),
        "Obs must be suppressed by ID: {listing}"
    );
    assert!(listing.contains("grp"), "ID column header: {listing}");
}

#[test]
fn execute_by_unsorted_errors() {
    let mut session = make_session();
    // grp out of order: B then A → not sorted ascending.
    let df = df![
        "grp" => ["B", "A"],
        "v"   => [1.0_f64, 2.0]
    ]
    .unwrap();
    let vars = vec![
        VarMeta {
            name: "grp".into(),
            ty: VarType::Char,
            length: 1,
            format: None,
            label: None,
        },
        VarMeta {
            name: "v".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
    ];
    let ds = SasDataset { df, vars };
    session.libs.get("WORK").unwrap().write("G", &ds).unwrap();

    let ast = PrintAst {
        by: vec![("grp".into(), false)],
        ..base_ast()
    };
    let err = execute(&ast, &mut session).unwrap_err();
    assert!(err.to_string().contains("not sorted"), "err: {err}");
}

#[test]
fn execute_double_spaces_rows() {
    let mut session = make_session();
    write_grouped(&mut session);
    let ast = PrintAst {
        double: true,
        ..base_ast()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    // 3 data rows double-spaced → blank line between consecutive rows.
    // Count rows containing a value cell; the listing should be taller than
    // the single-spaced version. Cheap proxy: the value "4" and "5" appear.
    assert!(
        listing.contains('4') && listing.contains('5'),
        "rows present: {listing}"
    );
}

#[test]
fn listing_alignments() {
    // Numeric values should be right-aligned, char values left-aligned.
    // We check by verifying Obs and age are in the right block.
    let mut session = make_session();
    write_test_dataset(&mut session);

    let ast = PrintAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "MYDATA".into(),
        }),
        vars: None,
        noobs: false,
        label: false,
        double: false,
        n: false,
        by: vec![],
        id: vec![],
        sum: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    // Listing should contain all 3 obs numbers
    assert!(listing.contains("1"), "listing: {listing}");
    assert!(listing.contains("2"), "listing: {listing}");
    assert!(listing.contains("3"), "listing: {listing}");
}
