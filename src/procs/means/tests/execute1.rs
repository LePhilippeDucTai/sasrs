use super::super::*;
use super::*;
use crate::dataset::SasDataset;
use polars::df;

#[test]
fn execute_ways_restricts_output_rows() {
    let mut session = make_session();
    // class g {a,b}, h {1,2}; combos (a,1)(a,2)(b,1).
    let df = df![
        "g" => ["a", "a", "b"],
        "h" => [1.0_f64, 2.0, 1.0],
        "x" => [5.0_f64, 7.0, 9.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("g"), num_meta("h"), num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let mut ast = MeansAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        summary: false,
        noprint: true,
        stats: vec![],
        class: vec!["g".into(), "h".into()],
        var: vec!["x".into()],
        by: vec![],
        weight: None,
        alpha: 0.05,
        printalltypes: false,
        ways: vec![1],
        types: vec![],
        output: Some(MeansOutput {
            out: DatasetRef {
                libref: Some("WORK".into()),
                name: "O".into(),
            },
            specs: vec![("sum".into(), "x".into(), "sx".into())],
        }),
    };
    execute(&ast, &mut session).unwrap();
    // WAYS 1 → only _TYPE_=1 (h levels {1,2}: 2 rows) and _TYPE_=2 (g
    // levels {a,b}: 2 rows). Total 4 rows; no _TYPE_=0 or 3.
    let ty = read_num_col(&session, "O", "_TYPE_");
    let set: std::collections::BTreeSet<i64> = ty
        .iter()
        .map(|v| match v {
            Value::Num(f) => *f as i64,
            _ => panic!(),
        })
        .collect();
    assert_eq!(set, [1i64, 2].iter().copied().collect());
    assert_eq!(ty.len(), 4);

    // TYPES (g) → only _TYPE_=2 (2 rows).
    ast.ways = vec![];
    ast.types = vec![vec!["g".into()]];
    execute(&ast, &mut session).unwrap();
    let ty = read_num_col(&session, "O", "_TYPE_");
    assert_eq!(ty.len(), 2);
    for v in &ty {
        assert_eq!(*v, Value::Num(2.0));
    }
}

#[test]
fn execute_output_k0_no_class() {
    let mut session = make_session();
    let df = df!["x" => [Some(2.0_f64), Some(4.0), Some(6.0), None]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = MeansAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        summary: false,
        noprint: true,
        stats: vec![],
        class: vec![],
        var: vec!["x".into()],
        by: vec![],
        weight: None,
        alpha: 0.05,
        printalltypes: false,
        ways: vec![],
        types: vec![],
        output: Some(MeansOutput {
            out: DatasetRef {
                libref: Some("WORK".into()),
                name: "O".into(),
            },
            specs: vec![
                ("mean".into(), "x".into(), "m".into()),
                ("n".into(), "x".into(), "cnt".into()),
                ("nmiss".into(), "x".into(), "nm".into()),
            ],
        }),
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    assert_eq!(out.n_obs(), 1);
    let ty = read_num_col(&session, "O", "_TYPE_");
    let freq = read_num_col(&session, "O", "_FREQ_");
    let m = read_num_col(&session, "O", "m");
    let cnt = read_num_col(&session, "O", "cnt");
    let nm = read_num_col(&session, "O", "nm");
    assert_eq!(ty, vec![Value::Num(0.0)]);
    assert_eq!(freq, vec![Value::Num(4.0)]); // all rows incl. missing
    assert_eq!(m, vec![Value::Num(4.0)]);
    assert_eq!(cnt, vec![Value::Num(3.0)]);
    assert_eq!(nm, vec![Value::Num(1.0)]);
}

#[test]
fn execute_output_k1() {
    let mut session = make_session();
    // group g: a(1,3) b(10)  -> means a=2, b=10
    let df = df![
        "g" => ["a", "a", "b"],
        "x" => [1.0_f64, 3.0, 10.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("g"), num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = MeansAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        summary: false,
        noprint: true,
        stats: vec![],
        class: vec!["g".into()],
        var: vec!["x".into()],
        by: vec![],
        weight: None,
        alpha: 0.05,
        printalltypes: false,
        ways: vec![],
        types: vec![],
        output: Some(MeansOutput {
            out: DatasetRef {
                libref: Some("WORK".into()),
                name: "O".into(),
            },
            specs: vec![("mean".into(), "x".into(), "mx".into())],
        }),
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    // _TYPE_ 0 (overall) + 2 levels = 3 rows.
    assert_eq!(out.n_obs(), 3);

    let ty = read_num_col(&session, "O", "_TYPE_");
    let freq = read_num_col(&session, "O", "_FREQ_");
    let mx = read_num_col(&session, "O", "mx");
    let g = read_num_col(&session, "O", "g"); // char col decoded as Value::Char

    // Row 0 = overall (_TYPE_=0): freq 3, mean (1+3+10)/3.
    assert_eq!(ty[0], Value::Num(0.0));
    assert_eq!(freq[0], Value::Num(3.0));
    assert_eq!(mx[0], Value::Num((1.0 + 3.0 + 10.0) / 3.0));
    // The overall class cell is blank (inactive char -> empty/null).
    assert_eq!(g[0], Value::Char(String::new()));

    // Rows 1,2 = per-level (_TYPE_=1), ordered a then b.
    assert_eq!(ty[1], Value::Num(1.0));
    assert_eq!(ty[2], Value::Num(1.0));
    assert_eq!(g[1], Value::Char("a".into()));
    assert_eq!(g[2], Value::Char("b".into()));
    assert_eq!(freq[1], Value::Num(2.0));
    assert_eq!(freq[2], Value::Num(1.0));
    assert_eq!(mx[1], Value::Num(2.0));
    assert_eq!(mx[2], Value::Num(10.0));
}

#[test]
fn execute_output_k2_type_set_and_rowcount() {
    let mut session = make_session();
    // c0 (g) has 2 levels {a,b}; c1 (h) has 2 levels {1,2}.
    // combos present: (a,1),(a,2),(b,1) -> 3 combos.
    let df = df![
        "g" => ["a", "a", "b"],
        "h" => [1.0_f64, 2.0, 1.0],
        "x" => [5.0_f64, 7.0, 9.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("g"), num_meta("h"), num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = MeansAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        summary: false,
        noprint: true,
        stats: vec![],
        class: vec!["g".into(), "h".into()],
        var: vec!["x".into()],
        by: vec![],
        weight: None,
        alpha: 0.05,
        printalltypes: false,
        ways: vec![],
        types: vec![],
        output: Some(MeansOutput {
            out: DatasetRef {
                libref: Some("WORK".into()),
                name: "O".into(),
            },
            specs: vec![("sum".into(), "x".into(), "sx".into())],
        }),
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    // Expected rows:
    //   _TYPE_=0 : 1 (overall)
    //   _TYPE_=1 : levels of LAST class (h) = {1,2} -> 2
    //   _TYPE_=2 : levels of FIRST class (g) = {a,b} -> 2
    //   _TYPE_=3 : combos = 3
    // total = 1 + 2 + 2 + 3 = 8
    assert_eq!(out.n_obs(), 8);

    let ty = read_num_col(&session, "O", "_TYPE_");
    let type_set: std::collections::BTreeSet<i64> = ty
        .iter()
        .map(|v| match v {
            Value::Num(f) => *f as i64,
            _ => panic!("type must be numeric"),
        })
        .collect();
    assert_eq!(type_set, [0i64, 1, 2, 3].iter().cloned().collect());

    // _TYPE_ is ascending.
    let tys: Vec<f64> = ty
        .iter()
        .map(|v| match v {
            Value::Num(f) => *f,
            _ => unreachable!(),
        })
        .collect();
    let mut sorted = tys.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(tys, sorted);

    // _TYPE_=3 sum check: combo (a,1) sum=5, (a,2)=7, (b,1)=9; overall freq.
    let freq = read_num_col(&session, "O", "_FREQ_");
    assert_eq!(freq[0], Value::Num(3.0)); // overall _TYPE_=0
}

#[test]
fn execute_report_contains_title_and_var() {
    let mut session = make_session();
    let df = df!["height" => [60.0_f64, 62.0, 64.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("height")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = MeansAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        summary: false,
        noprint: false,
        stats: vec![],
        class: vec![],
        var: vec![],
        by: vec![],
        weight: None,
        alpha: 0.05,
        printalltypes: false,
        ways: vec![],
        types: vec![],
        output: None,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(
        listing.contains("The MEANS Procedure"),
        "listing: {listing}"
    );
    assert!(listing.contains("height"), "listing: {listing}");
    // default stats headers
    assert!(listing.contains("Mean"), "listing: {listing}");
    assert!(listing.contains("Minimum"), "listing: {listing}");
}

#[test]
fn execute_noprint_writes_nothing_to_listing() {
    let mut session = make_session();
    let df = df!["x" => [1.0_f64, 2.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = MeansAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        summary: true,
        noprint: true,
        stats: vec![],
        class: vec![],
        var: vec![],
        by: vec![],
        weight: None,
        alpha: 0.05,
        printalltypes: false,
        ways: vec![],
        types: vec![],
        output: None,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(
        !listing.contains("The MEANS Procedure"),
        "noprint should not emit a report: {listing}"
    );
}

#[test]
fn execute_clm_output_readback() {
    let mut session = make_session();
    // [2,4,4,4,5,5,7,9]: mean 5, lclm≈3.21251, uclm≈6.78749 (alpha 0.05,
    // SAS sample-std CI).
    let df = df!["x" => [2.0_f64, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = MeansAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        summary: false,
        noprint: true,
        stats: vec![],
        class: vec![],
        var: vec!["x".into()],
        by: vec![],
        weight: None,
        alpha: 0.05,
        printalltypes: false,
        ways: vec![],
        types: vec![],
        output: Some(MeansOutput {
            out: DatasetRef {
                libref: Some("WORK".into()),
                name: "O".into(),
            },
            specs: vec![
                ("lclm".into(), "x".into(), "lo".into()),
                ("uclm".into(), "x".into(), "hi".into()),
            ],
        }),
    };
    execute(&ast, &mut session).unwrap();

    let lo = read_num_col(&session, "O", "lo");
    let hi = read_num_col(&session, "O", "hi");
    if let (Value::Num(l), Value::Num(h)) = (&lo[0], &hi[0]) {
        assert!((l - 3.21251).abs() < 1e-3, "lo={l}");
        assert!((h - 6.78749).abs() < 1e-3, "hi={h}");
    } else {
        panic!("lclm/uclm numeric");
    }
}

#[test]
fn execute_clm_report_headers() {
    let mut session = make_session();
    let df = df!["x" => [2.0_f64, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = MeansAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        summary: false,
        noprint: false,
        stats: vec!["mean".into(), "clm".into()],
        class: vec![],
        var: vec!["x".into()],
        by: vec![],
        weight: None,
        alpha: 0.05,
        printalltypes: false,
        ways: vec![],
        types: vec![],
        output: None,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(
        listing.contains("Lower 95% CL for Mean"),
        "listing: {listing}"
    );
    assert!(
        listing.contains("Upper 95% CL for Mean"),
        "listing: {listing}"
    );
}

// ───────────────────────────── BY tests ────────────────────────────────

#[test]
fn execute_by_per_group_report_and_headings() {
    let mut session = make_session();
    // Sorted by sex: F,F,M,M.
    let df = df![
        "sex" => ["F", "F", "M", "M"],
        "x" => [2.0_f64, 4.0, 10.0, 20.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("sex"), num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = MeansAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        summary: false,
        noprint: false,
        stats: vec!["mean".into()],
        class: vec![],
        var: vec!["x".into()],
        by: vec![("sex".into(), false)],
        weight: None,
        alpha: 0.05,
        printalltypes: false,
        ways: vec![],
        types: vec![],
        output: None,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    // Title once, BY headings for each group.
    assert!(
        listing.contains("The MEANS Procedure"),
        "listing: {listing}"
    );
    assert!(listing.contains("sex=F"), "listing: {listing}");
    assert!(listing.contains("sex=M"), "listing: {listing}");
    // The F group mean is 3, the M group mean is 15.
    assert!(listing.contains("15"), "listing: {listing}");
}
