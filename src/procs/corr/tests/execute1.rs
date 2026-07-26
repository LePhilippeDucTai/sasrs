use super::super::*;
use super::*;
use crate::dataset::SasDataset;
use polars::df;

// ───────────── execute tests ─────────────

#[test]
fn execute_perfect_correlation_listing() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0],
        "y" => [2.0_f64, 4.0, 6.0, 8.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = CorrAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        nosimple: false,
        noprob: false,
        nocorr: false,
        var: vec![],
        with: vec![],
        partial: vec![],
        pearson: false,
        spearman: false,
        kendall: false,
        hoeffding: false,
        weight: None,
        outp: None,
        outs: None,
        outk: None,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(listing.contains("The CORR Procedure"), "{listing}");
    assert!(listing.contains("Simple Statistics"), "{listing}");
    assert!(
        listing.contains("Pearson Correlation Coefficients"),
        "{listing}"
    );
    // Diagonal 1.00000 and off-diagonal 1.00000 (perfectly correlated).
    assert!(listing.contains("1.00000"), "{listing}");
    // Variable summary line.
    assert!(listing.contains("2 Variables:"), "{listing}");
}

#[test]
fn execute_nosimple_noprob_toggles() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0],
        "y" => [1.0_f64, 3.0, 2.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = CorrAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        nosimple: true,
        noprob: true,
        nocorr: false,
        var: vec!["x".into(), "y".into()],
        with: vec![],
        partial: vec![],
        pearson: false,
        spearman: false,
        kendall: false,
        hoeffding: false,
        weight: None,
        outp: None,
        outs: None,
        outk: None,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(
        !listing.contains("Simple Statistics"),
        "nosimple: {listing}"
    );
    assert!(!listing.contains("Prob > |r|"), "noprob: {listing}");
    assert!(
        listing.contains("Pearson Correlation Coefficients"),
        "{listing}"
    );
}

#[test]
fn execute_missing_pairwise_n_line() {
    let mut session = make_session();
    // x and y share 4 complete rows; x and z share only 3 (one missing),
    // so pairwise N differs and the N line should appear.
    let df = df![
        "x" => [Some(1.0_f64), Some(2.0), Some(3.0), Some(4.0)],
        "y" => [Some(2.0_f64), Some(1.0), Some(4.0), Some(3.0)],
        "z" => [Some(1.0_f64), None, Some(2.0), Some(5.0)]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y"), num_meta("z")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = CorrAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        nosimple: true,
        noprob: true,
        nocorr: false,
        var: vec!["x".into(), "y".into(), "z".into()],
        with: vec![],
        partial: vec![],
        pearson: false,
        spearman: false,
        kendall: false,
        hoeffding: false,
        weight: None,
        outp: None,
        outs: None,
        outk: None,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    // N line should show a "3" somewhere in the matrix region.
    assert!(listing.contains(" 3"), "expected N line with 3: {listing}");
    assert!(listing.contains(" 4"), "expected N line with 4: {listing}");
}

#[test]
fn execute_constant_variable_missing_r() {
    let mut session = make_session();
    let df = df![
        "x" => [5.0_f64, 5.0, 5.0, 5.0],
        "y" => [1.0_f64, 2.0, 3.0, 4.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = CorrAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        nosimple: true,
        noprob: true,
        nocorr: false,
        var: vec!["x".into(), "y".into()],
        with: vec![],
        partial: vec![],
        pearson: false,
        spearman: false,
        kendall: false,
        hoeffding: false,
        weight: None,
        outp: None,
        outs: None,
        outk: None,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    // Off-diagonal r between constant x and y is missing → ".".
    assert!(listing.contains(" ."), "expected missing r '.': {listing}");
}

#[test]
fn execute_with_statement_shapes_matrix() {
    let mut session = make_session();
    let df = df![
        "a" => [1.0_f64, 2.0, 3.0, 4.0],
        "b" => [4.0_f64, 3.0, 2.0, 1.0],
        "w" => [1.0_f64, 2.0, 3.0, 4.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("a"), num_meta("b"), num_meta("w")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = CorrAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        nosimple: true,
        noprob: true,
        nocorr: false,
        var: vec!["a".into(), "b".into()],
        with: vec!["w".into()],
        partial: vec![],
        pearson: false,
        spearman: false,
        kendall: false,
        hoeffding: false,
        weight: None,
        outp: None,
        outs: None,
        outk: None,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(listing.contains("1 With Variables:"), "{listing}");
    assert!(listing.contains("2 Variables:"), "{listing}");
    // w perfectly correlates with a (1.00000) and anti with b (-1.00000).
    assert!(listing.contains("1.00000"), "{listing}");
    assert!(listing.contains("-1.00000"), "{listing}");
}

#[test]
fn execute_default_var_all_numeric() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0],
        "g" => ["a", "b", "c"],
        "y" => [3.0_f64, 2.0, 1.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), char_meta("g"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = CorrAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        nosimple: false,
        noprob: true,
        nocorr: false,
        var: vec![],
        with: vec![],
        partial: vec![],
        pearson: false,
        spearman: false,
        kendall: false,
        hoeffding: false,
        weight: None,
        outp: None,
        outs: None,
        outk: None,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    // Only x and y (numeric) are analyzed; char g excluded.
    assert!(listing.contains("2 Variables:"), "{listing}");
    assert!(listing.contains("x y"), "{listing}");
}

// --- listing blocks ---

#[test]
fn execute_spearman_block() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0],
        "y" => [1.0_f64, 3.0, 2.0, 4.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let mut ast = base_ast("T");
    ast.spearman = true;
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(
        listing.contains("Spearman Correlation Coefficients"),
        "{listing}"
    );
    // No Pearson block when only spearman requested.
    assert!(
        !listing.contains("Pearson Correlation Coefficients"),
        "{listing}"
    );
    // r_s off-diagonal = 0.80000.
    assert!(listing.contains("0.80000"), "{listing}");
}

#[test]
fn execute_kendall_block() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0],
        "y" => [1.0_f64, 3.0, 2.0, 4.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let mut ast = base_ast("T");
    ast.kendall = true;
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(listing.contains("Kendall Tau b Coefficients"), "{listing}");
    assert!(listing.contains("Prob > |tau|"), "{listing}");
    // tau_b = 0.66667.
    assert!(listing.contains("0.66667"), "{listing}");
}

#[test]
fn execute_all_three_methods() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0],
        "y" => [2.0_f64, 1.0, 4.0, 3.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let mut ast = base_ast("T");
    ast.pearson = true;
    ast.spearman = true;
    ast.kendall = true;
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(
        listing.contains("Pearson Correlation Coefficients"),
        "{listing}"
    );
    assert!(
        listing.contains("Spearman Correlation Coefficients"),
        "{listing}"
    );
    assert!(listing.contains("Kendall Tau b Coefficients"), "{listing}");
}

// --- OUT= datasets ---

#[test]
fn execute_outp_dataset_structure() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0],
        "y" => [2.0_f64, 4.0, 6.0, 8.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let mut ast = base_ast("T");
    ast.outp = Some(DatasetRef {
        libref: Some("WORK".into()),
        name: "C".into(),
    });
    execute(&ast, &mut session).unwrap();

    // Read back the produced TYPE=CORR dataset.
    let (out, _) = session.libs.get("WORK").unwrap().read("C").unwrap();
    // Columns: _TYPE_, _NAME_, x, y.
    let names: Vec<String> = out.vars.iter().map(|v| v.name.clone()).collect();
    assert_eq!(names, vec!["_TYPE_", "_NAME_", "x", "y"]);
    // 3 stats rows + 2 corr rows = 5.
    assert_eq!(out.n_obs(), 5);

    let type_col = decode_column(&out, 0).unwrap();
    let types: Vec<String> = type_col
        .iter()
        .map(|v| match v {
            Value::Char(s) => s.clone(),
            _ => String::new(),
        })
        .collect();
    assert_eq!(types, vec!["MEAN", "STD", "N", "CORR", "CORR"]);

    // _NAME_ on CORR rows = x then y; empty on stats rows.
    let name_col = decode_column(&out, 1).unwrap();
    match &name_col[3] {
        Value::Char(s) => assert_eq!(s, "x"),
        other => panic!("expected x, got {other:?}"),
    }

    // CORR row for x: r(x,x)=1, r(x,y)=1 (perfect). Column "x" idx 2.
    let xcorr = decode_column(&out, 2).unwrap();
    assert!((value_to_num(&xcorr[3]).unwrap() - 1.0).abs() < 1e-12);
    // N row value = 4.
    assert!((value_to_num(&xcorr[2]).unwrap() - 4.0).abs() < 1e-12);
    // MEAN of x = 2.5.
    assert!((value_to_num(&xcorr[0]).unwrap() - 2.5).abs() < 1e-12);

    // _LAST_ updated.
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.C"));
}

#[test]
fn execute_outs_outk_methods() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0],
        "y" => [1.0_f64, 3.0, 2.0, 4.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let mut ast = base_ast("T");
    ast.spearman = true;
    ast.kendall = true;
    ast.outs = Some(DatasetRef {
        libref: Some("WORK".into()),
        name: "S".into(),
    });
    ast.outk = Some(DatasetRef {
        libref: Some("WORK".into()),
        name: "K".into(),
    });
    execute(&ast, &mut session).unwrap();

    // Spearman OUTS: corr(x,y) row for x = 0.8.
    let (s, _) = session.libs.get("WORK").unwrap().read("S").unwrap();
    let sx = decode_column(&s, 2).unwrap(); // column x
    // row 4 (index 4) is CORR y; row 3 is CORR x → off-diag at col y.
    let sy = decode_column(&s, 3).unwrap(); // column y, CORR x row
    assert!(
        (value_to_num(&sy[3]).unwrap() - 0.8).abs() < 1e-9,
        "{:?}",
        sy[3]
    );
    assert!((value_to_num(&sx[3]).unwrap() - 1.0).abs() < 1e-9);

    // Kendall OUTK: corr(x,y) = 0.6667.
    let (kd, _) = session.libs.get("WORK").unwrap().read("K").unwrap();
    let ky = decode_column(&kd, 3).unwrap();
    assert!(
        (value_to_num(&ky[3]).unwrap() - 4.0 / 6.0).abs() < 1e-9,
        "{:?}",
        ky[3]
    );
}

#[test]
fn execute_weighted_listing_runs() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0],
        "y" => [2.0_f64, 4.0, 6.0, 8.0],
        "wt" => [1.0_f64, 1.0, 1.0, 1.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y"), num_meta("wt")],
    };
    write_dataset(&mut session, "T", ds);

    let mut ast = base_ast("T");
    ast.var = vec!["x".into(), "y".into()];
    ast.weight = Some("wt".into());
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    // With w=1 the weighted r equals the unweighted perfect correlation.
    assert!(listing.contains("1.00000"), "{listing}");
    assert!(
        listing.contains("Pearson Correlation Coefficients"),
        "{listing}"
    );
}
