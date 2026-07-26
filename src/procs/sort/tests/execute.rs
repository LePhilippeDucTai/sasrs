use super::super::*;
use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::missing::encode_special;
use crate::value::{MissingKind, VarType};
use polars::df;

#[test]
fn execute_ascending_missing_collation() {
    // Mix special and ordinary missings with numbers:
    // ._ (underscore), . (dot=null), .A (letter 0), 5, 2.
    // Expected ascending order: ._ < . < .A < 2 < 5.
    let mut session = make_session();
    let xs = vec![
        Some(5.0),
        None,                                          // .
        Some(encode_special(MissingKind::Letter(0))),  // .A
        Some(encode_special(MissingKind::Underscore)), // ._
        Some(2.0),
    ];
    write_num_dataset(&mut session, "T", "x", xs);

    let ast = SortAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        out: None,
        by: vec![("x".to_string(), false)],
        nodupkey: false,
        noduprecs: false,
        tagsort: false,
        sortseq: SortSeq::Ascii,
    };
    execute(&ast, &mut session).unwrap();

    let got = read_num_col(&session, "T", "x");
    assert_eq!(
        got,
        vec![
            Value::Missing(MissingKind::Underscore),
            Value::Missing(MissingKind::Dot),
            Value::Missing(MissingKind::Letter(0)),
            Value::Num(2.0),
            Value::Num(5.0),
        ]
    );
}

#[test]
fn execute_descending() {
    let mut session = make_session();
    write_num_dataset(
        &mut session,
        "T",
        "x",
        vec![Some(1.0), Some(3.0), Some(2.0)],
    );

    let ast = SortAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        out: None,
        by: vec![("x".to_string(), true)],
        nodupkey: false,
        noduprecs: false,
        tagsort: false,
        sortseq: SortSeq::Ascii,
    };
    execute(&ast, &mut session).unwrap();

    let got = read_num_col(&session, "T", "x");
    assert_eq!(got, vec![Value::Num(3.0), Value::Num(2.0), Value::Num(1.0)]);
}

#[test]
fn execute_multikey_num_then_char() {
    let mut session = make_session();
    let df = df![
        "g" => [2.0_f64, 1.0, 1.0, 2.0],
        "s" => ["b", "z", "a", "a"]
    ]
    .unwrap();
    let vars = vec![
        VarMeta {
            name: "g".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
        VarMeta {
            name: "s".into(),
            ty: VarType::Char,
            length: 1,
            format: None,
            label: None,
        },
    ];
    let ds = SasDataset { df, vars };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let ast = SortAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        out: None,
        by: vec![("g".to_string(), false), ("s".to_string(), false)],
        nodupkey: false,
        noduprecs: false,
        tagsort: false,
        sortseq: SortSeq::Ascii,
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("T").unwrap();
    let g: Vec<f64> = out
        .df
        .column("g")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    let s: Vec<String> = out
        .df
        .column("s")
        .unwrap()
        .str()
        .unwrap()
        .iter()
        .map(|o| o.unwrap().to_string())
        .collect();
    // (g=1,s=a),(g=1,s=z),(g=2,s=a),(g=2,s=b)
    assert_eq!(g, vec![1.0, 1.0, 2.0, 2.0]);
    assert_eq!(s, vec!["a", "z", "a", "b"]);
}

#[test]
fn execute_nodupkey_deletes_and_notes() {
    let mut session = make_session();
    write_num_dataset(
        &mut session,
        "T",
        "x",
        vec![Some(1.0), Some(1.0), Some(2.0), Some(2.0), Some(2.0)],
    );

    let ast = SortAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        out: None,
        by: vec![("x".to_string(), false)],
        nodupkey: true,
        noduprecs: false,
        tagsort: false,
        sortseq: SortSeq::Ascii,
    };
    execute(&ast, &mut session).unwrap();

    let got = read_num_col(&session, "T", "x");
    assert_eq!(got, vec![Value::Num(1.0), Value::Num(2.0)]);

    let log = session.log.into_string();
    assert!(
        log.contains("3 observations with duplicate key values were deleted."),
        "log: {log}"
    );
}

#[test]
fn execute_noduprecs_whole_row() {
    // Same key but different other column => NODUPKEY would drop, but
    // NODUPRECS keeps (rows differ). Then a true full duplicate is dropped.
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 1.0, 1.0],
        "y" => ["a", "b", "b"]
    ]
    .unwrap();
    let vars = vec![
        VarMeta {
            name: "x".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
        VarMeta {
            name: "y".into(),
            ty: VarType::Char,
            length: 1,
            format: None,
            label: None,
        },
    ];
    let ds = SasDataset { df, vars };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let ast = SortAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        out: None,
        by: vec![("x".to_string(), false)],
        nodupkey: false,
        noduprecs: true,
        tagsort: false,
        sortseq: SortSeq::Ascii,
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("T").unwrap();
    // After sort by x: rows (1,a),(1,b),(1,b) => drop the 3rd (full dup of 2nd).
    assert_eq!(out.n_obs(), 2);
    let log = session.log.into_string();
    assert!(
        log.contains("1 duplicate observations were deleted."),
        "log: {log}"
    );
}

#[test]
fn execute_out_creates_new_leaves_input() {
    let mut session = make_session();
    write_num_dataset(
        &mut session,
        "IN",
        "x",
        vec![Some(3.0), Some(1.0), Some(2.0)],
    );

    let ast = SortAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "IN".into(),
        }),
        out: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "OUT".into(),
        }),
        by: vec![("x".to_string(), false)],
        nodupkey: false,
        noduprecs: false,
        tagsort: false,
        sortseq: SortSeq::Ascii,
    };
    execute(&ast, &mut session).unwrap();

    // OUT is sorted.
    let out = read_num_col(&session, "OUT", "x");
    assert_eq!(out, vec![Value::Num(1.0), Value::Num(2.0), Value::Num(3.0)]);
    // IN is untouched (original order).
    let inp = read_num_col(&session, "IN", "x");
    assert_eq!(inp, vec![Value::Num(3.0), Value::Num(1.0), Value::Num(2.0)]);
    // last_dataset points at OUT.
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.OUT"));
}

#[test]
fn execute_no_out_replaces_input() {
    let mut session = make_session();
    write_num_dataset(
        &mut session,
        "T",
        "x",
        vec![Some(3.0), Some(1.0), Some(2.0)],
    );

    let ast = SortAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        out: None,
        by: vec![("x".to_string(), false)],
        nodupkey: false,
        noduprecs: false,
        tagsort: false,
        sortseq: SortSeq::Ascii,
    };
    execute(&ast, &mut session).unwrap();

    let got = read_num_col(&session, "T", "x");
    assert_eq!(got, vec![Value::Num(1.0), Value::Num(2.0), Value::Num(3.0)]);
}

#[test]
fn execute_uses_last_when_no_data() {
    let mut session = make_session();
    write_num_dataset(&mut session, "LASTONE", "x", vec![Some(2.0), Some(1.0)]);
    // last_dataset = WORK.LASTONE set by helper.

    let ast = SortAst {
        data: None,
        out: None,
        by: vec![("x".to_string(), false)],
        nodupkey: false,
        noduprecs: false,
        tagsort: false,
        sortseq: SortSeq::Ascii,
    };
    execute(&ast, &mut session).unwrap();

    let got = read_num_col(&session, "LASTONE", "x");
    assert_eq!(got, vec![Value::Num(1.0), Value::Num(2.0)]);
}

#[test]
fn execute_unknown_by_var_errors() {
    let mut session = make_session();
    write_num_dataset(&mut session, "T", "x", vec![Some(1.0)]);

    let ast = SortAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        out: None,
        by: vec![("nope".to_string(), false)],
        nodupkey: false,
        noduprecs: false,
        tagsort: false,
        sortseq: SortSeq::Ascii,
    };
    let result = execute(&ast, &mut session);
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(
        msg.contains("NOPE") && msg.contains("not found"),
        "msg: {msg}"
    );
}

#[test]
fn execute_tagsort_identical_output() {
    // TAGSORT is a no-op hint; output must be identical to a plain sort.
    let mut s1 = make_session();
    let mut s2 = make_session();
    let xs = vec![Some(3.0), Some(1.0), Some(2.0)];
    write_num_dataset(&mut s1, "T", "x", xs.clone());
    write_num_dataset(&mut s2, "T", "x", xs);

    // Plain sort.
    let plain = SortAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        out: None,
        by: vec![("x".to_string(), false)],
        nodupkey: false,
        noduprecs: false,
        tagsort: false,
        sortseq: SortSeq::Ascii,
    };
    execute(&plain, &mut s1).unwrap();

    // TAGSORT sort.
    let tagged = SortAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        out: None,
        by: vec![("x".to_string(), false)],
        nodupkey: false,
        noduprecs: false,
        tagsort: true,
        sortseq: SortSeq::Ascii,
    };
    execute(&tagged, &mut s2).unwrap();

    assert_eq!(
        read_num_col(&s1, "T", "x"),
        read_num_col(&s2, "T", "x"),
        "TAGSORT must produce identical output"
    );
}

#[test]
fn execute_sortseq_ascii_identical_output() {
    // SORTSEQ=ASCII is equivalent to the default; output must be identical.
    let mut s1 = make_session();
    let mut s2 = make_session();
    let xs = vec![Some(3.0), Some(1.0), Some(2.0)];
    write_num_dataset(&mut s1, "T", "x", xs.clone());
    write_num_dataset(&mut s2, "T", "x", xs);

    let plain = SortAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        out: None,
        by: vec![("x".to_string(), false)],
        nodupkey: false,
        noduprecs: false,
        tagsort: false,
        sortseq: SortSeq::Ascii,
    };
    execute(&plain, &mut s1).unwrap();

    let ascii = SortAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        out: None,
        by: vec![("x".to_string(), false)],
        nodupkey: false,
        noduprecs: false,
        tagsort: false,
        sortseq: SortSeq::Ascii,
    };
    execute(&ascii, &mut s2).unwrap();

    assert_eq!(
        read_num_col(&s1, "T", "x"),
        read_num_col(&s2, "T", "x"),
        "SORTSEQ=ASCII must produce identical output to default"
    );
}

#[test]
fn execute_key_descending_order() {
    // KEY=age / descending → ages sorted largest to smallest.
    // Uses f64 for age (SAS numeric = float64).
    let mut session = make_session();
    let df = polars::df![
        "name" => ["Alfred", "Alice", "Barbara"],
        "age"  => [14.0_f64, 13.0, 13.0],
    ]
    .unwrap();
    use crate::dataset::VarMeta;
    use crate::value::VarType;
    let vars = vec![
        VarMeta {
            name: "name".into(),
            ty: VarType::Char,
            length: 10,
            format: None,
            label: None,
        },
        VarMeta {
            name: "age".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
    ];
    let ds = crate::dataset::SasDataset { df, vars };
    session.libs.get("WORK").unwrap().write("CLS", &ds).unwrap();

    let ast = SortAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "CLS".into(),
        }),
        out: None,
        // KEY=age / descending (set programmatically as already resolved).
        by: vec![("age".to_string(), true)],
        nodupkey: false,
        noduprecs: false,
        tagsort: false,
        sortseq: SortSeq::Ascii,
    };
    execute(&ast, &mut session).unwrap();

    // Verify via decode_column (uses Value, avoids dtype mismatch).
    let ages = read_num_col(&session, "CLS", "age");
    // Descending: 14, 13, 13.
    assert_eq!(
        ages,
        vec![Value::Num(14.0), Value::Num(13.0), Value::Num(13.0)],
        "KEY=age/descending should sort largest first"
    );
}
