use super::*;
use crate::dataset::SasDataset;
use crate::session::Session;
use crate::source::SourceFile;
use crate::testkit::*;
use crate::value::VarType;
use polars::df;

fn parse_transpose(src: &str) -> Result<TransposeAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "transpose"
    parse(&mut ts)
}

fn read_col(session: &Session, table: &str, col: &str) -> Vec<Value> {
    let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
    let idx = ds.vars.iter().position(|m| m.name == col).unwrap();
    decode_column(&ds, idx).unwrap()
}

fn out_ref(name: &str) -> DatasetRef {
    DatasetRef {
        libref: Some("WORK".into()),
        name: name.into(),
    }
}

fn data_ref(name: &str) -> Option<DatasetRef> {
    Some(DatasetRef {
        libref: Some("WORK".into()),
        name: name.into(),
    })
}

// ───────────────────────────── parse tests ─────────────────────────────

#[test]
fn parse_full_statement() {
    let ast =
        parse_transpose("proc transpose data=a out=b prefix=p; by g; id k; var x y; run;").unwrap();
    assert_eq!(ast.data.as_ref().unwrap().name, "a");
    assert_eq!(ast.out.as_ref().unwrap().name, "b");
    assert_eq!(ast.prefix.as_deref(), Some("p"));
    assert_eq!(ast.by, vec!["g".to_string()]);
    assert_eq!(ast.id.as_deref(), Some("k"));
    assert_eq!(ast.var, vec!["x".to_string(), "y".to_string()]);
}

#[test]
fn parse_name_option() {
    let ast = parse_transpose("proc transpose data=a out=b name=src; var x; run;").unwrap();
    assert_eq!(ast.name.as_deref(), Some("src"));
}

#[test]
fn parse_unknown_option_errors() {
    let r = parse_transpose("proc transpose data=a bogus; run;");
    assert!(r.is_err());
    let msg = r.err().unwrap().to_string();
    assert!(msg.contains("BOGUS"), "msg: {msg}");
}

#[test]
fn parse_unknown_substatement_skipped() {
    // The DELETE substatement is unrecognized and should be skipped.
    let ast = parse_transpose("proc transpose data=a out=b; delete foo; var x; run;").unwrap();
    assert_eq!(ast.var, vec!["x".to_string()]);
}

// ───────────────────────── normalize_name tests ────────────────────────

#[test]
fn normalize_name_rules() {
    assert_eq!(normalize_name("abc"), "abc");
    assert_eq!(normalize_name("1x"), "_1x");
    assert_eq!(normalize_name(""), "_");
    assert_eq!(normalize_name("a b"), "a_b");
    assert_eq!(normalize_name("a-b"), "a_b");
}

// ───────────────────────────── execute tests ───────────────────────────

#[test]
fn execute_simple_no_by_no_id() {
    let mut session = make_session();
    let df = df!["x" => [10.0_f64, 20.0, 30.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = TransposeAst {
        data: data_ref("T"),
        out: Some(out_ref("O")),
        prefix: None,
        by: vec![],
        id: None,
        var: vec!["x".into()],
        name: None,
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    assert_eq!(out.n_obs(), 1);
    // _NAME_ = "x"
    let name = read_col(&session, "O", "_NAME_");
    assert_eq!(name, vec![Value::Char("x".into())]);
    // COL1..COL3 = 10,20,30
    assert_eq!(read_col(&session, "O", "COL1"), vec![Value::Num(10.0)]);
    assert_eq!(read_col(&session, "O", "COL2"), vec![Value::Num(20.0)]);
    assert_eq!(read_col(&session, "O", "COL3"), vec![Value::Num(30.0)]);
}

#[test]
fn execute_prefix_renames_cols() {
    let mut session = make_session();
    let df = df!["x" => [1.0_f64, 2.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = TransposeAst {
        data: data_ref("T"),
        out: Some(out_ref("O")),
        prefix: Some("V".into()),
        by: vec![],
        id: None,
        var: vec!["x".into()],
        name: None,
    };
    execute(&ast, &mut session).unwrap();

    assert_eq!(read_col(&session, "O", "V1"), vec![Value::Num(1.0)]);
    assert_eq!(read_col(&session, "O", "V2"), vec![Value::Num(2.0)]);
}

#[test]
fn execute_with_by_pads_shorter_group() {
    let mut session = make_session();
    // group g=1 has 2 rows, g=2 has 1 row -> max 2 cols, g=2 padded.
    let df = df![
        "g" => [1.0_f64, 1.0, 2.0],
        "x" => [10.0_f64, 11.0, 20.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("g"), num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = TransposeAst {
        data: data_ref("T"),
        out: Some(out_ref("O")),
        prefix: None,
        by: vec!["g".into()],
        id: None,
        var: vec!["x".into()],
        name: None,
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    assert_eq!(out.n_obs(), 2); // one row per (group × var)

    let g = read_col(&session, "O", "g");
    let c1 = read_col(&session, "O", "COL1");
    let c2 = read_col(&session, "O", "COL2");
    assert_eq!(g, vec![Value::Num(1.0), Value::Num(2.0)]);
    assert_eq!(c1, vec![Value::Num(10.0), Value::Num(20.0)]);
    // g=1 -> 11; g=2 padded with missing.
    assert_eq!(c2[0], Value::Num(11.0));
    assert!(c2[1].is_missing());
}

#[test]
fn execute_with_id_names_columns() {
    let mut session = make_session();
    let df = df![
        "k" => ["red", "blue"],
        "x" => [1.0_f64, 2.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("k", 4), num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = TransposeAst {
        data: data_ref("T"),
        out: Some(out_ref("O")),
        prefix: None,
        by: vec![],
        id: Some("k".into()),
        var: vec!["x".into()],
        name: None,
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    assert_eq!(out.n_obs(), 1);
    // Columns named by ID values in first-appearance order: red, blue.
    let cols: Vec<String> = out.vars.iter().map(|m| m.name.clone()).collect();
    assert!(cols.contains(&"red".to_string()), "cols: {cols:?}");
    assert!(cols.contains(&"blue".to_string()), "cols: {cols:?}");
    assert_eq!(read_col(&session, "O", "red"), vec![Value::Num(1.0)]);
    assert_eq!(read_col(&session, "O", "blue"), vec![Value::Num(2.0)]);
}

#[test]
fn execute_with_id_numeric_values_normalized() {
    let mut session = make_session();
    // numeric ID values 1,2 -> names "_1","_2" (start with digit).
    let df = df![
        "k" => [1.0_f64, 2.0],
        "x" => [7.0_f64, 8.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("k"), num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = TransposeAst {
        data: data_ref("T"),
        out: Some(out_ref("O")),
        prefix: None,
        by: vec![],
        id: Some("k".into()),
        var: vec!["x".into()],
        name: None,
    };
    execute(&ast, &mut session).unwrap();

    let cols: Vec<String> = {
        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        out.vars.iter().map(|m| m.name.clone()).collect()
    };
    assert!(cols.contains(&"_1".to_string()), "cols: {cols:?}");
    assert!(cols.contains(&"_2".to_string()), "cols: {cols:?}");
    assert_eq!(read_col(&session, "O", "_1"), vec![Value::Num(7.0)]);
    assert_eq!(read_col(&session, "O", "_2"), vec![Value::Num(8.0)]);
}

#[test]
fn execute_duplicate_id_in_group_errors() {
    let mut session = make_session();
    // Two rows with the same ID "a" in the (single) BY group -> ERROR.
    let df = df![
        "k" => ["a", "a"],
        "x" => [1.0_f64, 2.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("k", 1), num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = TransposeAst {
        data: data_ref("T"),
        out: Some(out_ref("O")),
        prefix: None,
        by: vec![],
        id: Some("k".into()),
        var: vec!["x".into()],
        name: None,
    };
    let r = execute(&ast, &mut session);
    assert!(r.is_err());
    let msg = r.err().unwrap().to_string();
    assert!(
        msg.contains("The ID value \"a\" occurs twice in the same BY group."),
        "msg: {msg}"
    );
}

#[test]
fn execute_mixing_char_and_numeric_makes_char_cols() {
    let mut session = make_session();
    // var x (num), var y (char) -> all COL columns become char.
    let df = df![
        "x" => [1.0_f64, 2.0],
        "y" => ["a", "b"]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), char_meta("y", 1)],
    };
    write_dataset(&mut session, "T", ds);

    let ast = TransposeAst {
        data: data_ref("T"),
        out: Some(out_ref("O")),
        prefix: None,
        by: vec![],
        id: None,
        var: vec!["x".into(), "y".into()],
        name: None,
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    // Two output rows: one per var.
    assert_eq!(out.n_obs(), 2);
    // COL columns must be char.
    for nm in ["COL1", "COL2"] {
        let meta = out.vars.iter().find(|m| m.name == nm).unwrap();
        assert_eq!(meta.ty, VarType::Char, "col {nm} should be char");
    }
    // Row 0 = var x: numeric values rendered as char "1","2".
    let c1 = read_col(&session, "O", "COL1");
    let c2 = read_col(&session, "O", "COL2");
    assert_eq!(c1[0], Value::Char("1".into()));
    assert_eq!(c2[0], Value::Char("2".into()));
    // Row 1 = var y: char values "a","b".
    assert_eq!(c1[1], Value::Char("a".into()));
    assert_eq!(c2[1], Value::Char("b".into()));

    // _NAME_ rows are the source names x, y.
    let name = read_col(&session, "O", "_NAME_");
    assert_eq!(name, vec![Value::Char("x".into()), Value::Char("y".into())]);
}

#[test]
fn execute_default_var_all_numeric_excludes_by_and_id() {
    let mut session = make_session();
    // var list empty -> all numeric except BY(g) and ID(k): only x.
    let df = df![
        "g" => [1.0_f64],
        "k" => [5.0_f64],
        "x" => [9.0_f64]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("g"), num_meta("k"), num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = TransposeAst {
        data: data_ref("T"),
        out: Some(out_ref("O")),
        prefix: None,
        by: vec!["g".into()],
        id: Some("k".into()),
        var: vec![],
        name: None,
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    // Single var x -> one output row.
    assert_eq!(out.n_obs(), 1);
    let name = read_col(&session, "O", "_NAME_");
    assert_eq!(name, vec![Value::Char("x".into())]);
}

#[test]
fn execute_name_option_renames_name_col() {
    let mut session = make_session();
    let df = df!["x" => [1.0_f64]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = TransposeAst {
        data: data_ref("T"),
        out: Some(out_ref("O")),
        prefix: None,
        by: vec![],
        id: None,
        var: vec!["x".into()],
        name: Some("source".into()),
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    let cols: Vec<String> = out.vars.iter().map(|m| m.name.clone()).collect();
    assert!(cols.contains(&"source".to_string()), "cols: {cols:?}");
    assert!(!cols.contains(&"_NAME_".to_string()), "cols: {cols:?}");
}

#[test]
fn execute_missing_out_errors() {
    let mut session = make_session();
    let df = df!["x" => [1.0_f64]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = TransposeAst {
        data: data_ref("T"),
        out: None,
        prefix: None,
        by: vec![],
        id: None,
        var: vec!["x".into()],
        name: None,
    };
    let r = execute(&ast, &mut session);
    assert!(r.is_err());
    let msg = r.err().unwrap().to_string();
    assert!(msg.contains("OUT="), "msg: {msg}");
}

#[test]
fn execute_emits_dataset_note() {
    let mut session = make_session();
    let df = df!["x" => [1.0_f64, 2.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = TransposeAst {
        data: data_ref("T"),
        out: Some(out_ref("O")),
        prefix: None,
        by: vec![],
        id: None,
        var: vec!["x".into()],
        name: None,
    };
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    assert!(
        log.contains("The data set WORK.O has 1 observations and"),
        "log: {log}"
    );
}
