use super::*;
use crate::dataset::SasDataset;
use crate::session::Session;
use crate::source::SourceFile;
use crate::testkit::*;
use polars::df;

fn parse_append(src: &str) -> Result<AppendAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "append"
    parse(&mut ts)
}

fn read_dataset(session: &Session, table: &str) -> SasDataset {
    let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
    ds
}

// --- Parse tests ---

#[test]
fn parse_basic_fields() {
    let ast = parse_append("proc append base=work.a data=work.b force; run;").unwrap();
    assert_eq!(ast.base.libref.as_deref(), Some("work"));
    assert_eq!(ast.base.name, "a");
    assert_eq!(ast.data.libref.as_deref(), Some("work"));
    assert_eq!(ast.data.name, "b");
    assert!(ast.force);
}

#[test]
fn parse_without_force() {
    let ast = parse_append("proc append base=a data=b; run;").unwrap();
    assert!(!ast.force);
    assert_eq!(ast.base.name, "a");
    assert_eq!(ast.data.name, "b");
}

#[test]
fn parse_missing_base_errors() {
    let result = parse_append("proc append data=work.b; run;");
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("BASE="), "msg: {msg}");
}

#[test]
fn parse_missing_data_errors() {
    let result = parse_append("proc append base=work.a; run;");
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("DATA="), "msg: {msg}");
}

#[test]
fn parse_unknown_option_errors() {
    let result = parse_append("proc append base=a data=b bogus; run;");
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("BOGUS"), "msg: {msg}");
}

// --- Execute tests ---

#[test]
fn execute_base_missing_creates_copy() {
    let mut session = make_session();

    // Write DATA dataset only (no BASE).
    let df = df!["x" => [1.0_f64, 2.0, 3.0]].unwrap();
    let data_ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "DATA_DS", data_ds);

    let ast = AppendAst {
        base: DatasetRef {
            libref: Some("WORK".into()),
            name: "BASE_DS".into(),
        },
        data: DatasetRef {
            libref: Some("WORK".into()),
            name: "DATA_DS".into(),
        },
        force: false,
        nowarn: false,
        appendver: None,
    };
    execute(&ast, &mut session).unwrap();

    // BASE_DS should now exist as a copy of DATA_DS.
    let result = read_dataset(&session, "BASE_DS");
    assert_eq!(result.n_obs(), 3);
    let col = decode_column(&result, 0).unwrap();
    assert_eq!(col, vec![Value::Num(1.0), Value::Num(2.0), Value::Num(3.0)]);

    // last_dataset should point to BASE.
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.BASE_DS"));

    // Log should mention copying.
    let log = session.log.into_string();
    assert!(
        log.contains("DATA file is being copied to BASE file"),
        "log: {log}"
    );
}

#[test]
fn execute_compatible_append_grows_base() {
    let mut session = make_session();

    let base_df = df!["x" => [1.0_f64, 2.0]].unwrap();
    let base_ds = SasDataset {
        df: base_df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "BASE", base_ds);

    let data_df = df!["x" => [3.0_f64, 4.0, 5.0]].unwrap();
    let data_ds = SasDataset {
        df: data_df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "DATA", data_ds);

    let ast = AppendAst {
        base: DatasetRef {
            libref: Some("WORK".into()),
            name: "BASE".into(),
        },
        data: DatasetRef {
            libref: Some("WORK".into()),
            name: "DATA".into(),
        },
        force: false,
        nowarn: false,
        appendver: None,
    };
    execute(&ast, &mut session).unwrap();

    let result = read_dataset(&session, "BASE");
    assert_eq!(result.n_obs(), 5, "base should have 2+3=5 rows");
    let col = decode_column(&result, 0).unwrap();
    assert_eq!(
        col,
        vec![
            Value::Num(1.0),
            Value::Num(2.0),
            Value::Num(3.0),
            Value::Num(4.0),
            Value::Num(5.0),
        ]
    );

    let log = session.log.into_string();
    assert!(log.contains("3 observations read from"), "log: {log}");
    assert!(log.contains("3 observations added"), "log: {log}");
    assert!(log.contains("5 observations and 1 variable"), "log: {log}");
}

#[test]
fn execute_without_force_extra_data_var_anomaly_errors() {
    let mut session = make_session();

    // BASE has only x; DATA has x and y (y is extra — anomaly without FORCE).
    let base_df = df!["x" => [1.0_f64]].unwrap();
    let base_ds = SasDataset {
        df: base_df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "BASE", base_ds);

    let data_df = df!["x" => [2.0_f64], "y" => [99.0_f64]].unwrap();
    let data_ds = SasDataset {
        df: data_df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "DATA", data_ds);

    let ast = AppendAst {
        base: DatasetRef {
            libref: Some("WORK".into()),
            name: "BASE".into(),
        },
        data: DatasetRef {
            libref: Some("WORK".into()),
            name: "DATA".into(),
        },
        force: false,
        nowarn: false,
        appendver: None,
    };
    let result = execute(&ast, &mut session);
    assert!(result.is_err(), "expected anomaly error");
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("anomalies"), "msg: {msg}");

    // BASE should NOT have grown.
    let base_after = read_dataset(&session, "BASE");
    assert_eq!(base_after.n_obs(), 1, "BASE should still have 1 row");
}

#[test]
fn execute_with_force_extra_data_var_dropped_base_only_var_missing() {
    let mut session = make_session();

    // BASE has x and z; DATA has x and y (y is extra, z is base-only).
    let base_df = df!["x" => [1.0_f64], "z" => [10.0_f64]].unwrap();
    let base_ds = SasDataset {
        df: base_df,
        vars: vec![num_meta("x"), num_meta("z")],
    };
    write_dataset(&mut session, "BASE", base_ds);

    let data_df = df!["x" => [2.0_f64], "y" => [99.0_f64]].unwrap();
    let data_ds = SasDataset {
        df: data_df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "DATA", data_ds);

    let ast = AppendAst {
        base: DatasetRef {
            libref: Some("WORK".into()),
            name: "BASE".into(),
        },
        data: DatasetRef {
            libref: Some("WORK".into()),
            name: "DATA".into(),
        },
        force: true,
        nowarn: false,
        appendver: None,
    };
    execute(&ast, &mut session).unwrap();

    let result = read_dataset(&session, "BASE");
    assert_eq!(result.n_obs(), 2, "base should have 1+1=2 rows");
    assert_eq!(result.n_vars(), 2, "base should still have 2 vars (x, z)");

    // x should be [1.0, 2.0].
    let xi = result.vars.iter().position(|v| v.name == "x").unwrap();
    let x_col = decode_column(&result, xi).unwrap();
    assert_eq!(x_col, vec![Value::Num(1.0), Value::Num(2.0)]);

    // z for the appended DATA row should be missing.
    let zi = result.vars.iter().position(|v| v.name == "z").unwrap();
    let z_col = decode_column(&result, zi).unwrap();
    assert_eq!(z_col[0], Value::Num(10.0));
    assert_eq!(z_col[1], Value::Missing(crate::value::MissingKind::Dot));

    // Log should contain a warning about y being dropped.
    let log = session.log.into_string();
    assert!(log.contains("Y") || log.contains("y"), "log: {log}");
    assert!(log.contains("not found on BASE"), "log: {log}");
}

#[test]
fn execute_with_force_char_truncation() {
    let mut session = make_session();

    // BASE has char var name with length 3.
    let base_df = df!["name" => ["abc"]].unwrap();
    let base_ds = SasDataset {
        df: base_df,
        vars: vec![char_meta("name", 3)],
    };
    write_dataset(&mut session, "BASE", base_ds);

    // DATA has char var name with length 8 (longer than BASE).
    let data_df = df!["name" => ["hello!"]].unwrap();
    let data_ds = SasDataset {
        df: data_df,
        vars: vec![char_meta("name", 6)],
    };
    write_dataset(&mut session, "DATA", data_ds);

    let ast = AppendAst {
        base: DatasetRef {
            libref: Some("WORK".into()),
            name: "BASE".into(),
        },
        data: DatasetRef {
            libref: Some("WORK".into()),
            name: "DATA".into(),
        },
        force: true,
        nowarn: false,
        appendver: None,
    };
    execute(&ast, &mut session).unwrap();

    let result = read_dataset(&session, "BASE");
    assert_eq!(result.n_obs(), 2);
    let col = decode_column(&result, 0).unwrap();
    assert_eq!(col[0], Value::Char("abc".to_string()));
    // "hello!" truncated to 3 chars => "hel"
    assert_eq!(col[1], Value::Char("hel".to_string()));
}

#[test]
fn execute_without_force_char_truncation_is_anomaly() {
    let mut session = make_session();

    // BASE has char var name with length 3; DATA has length 6 > 3 → anomaly.
    let base_df = df!["name" => ["abc"]].unwrap();
    let base_ds = SasDataset {
        df: base_df,
        vars: vec![char_meta("name", 3)],
    };
    write_dataset(&mut session, "BASE", base_ds);

    let data_df = df!["name" => ["hello!"]].unwrap();
    let data_ds = SasDataset {
        df: data_df,
        vars: vec![char_meta("name", 6)],
    };
    write_dataset(&mut session, "DATA", data_ds);

    let ast = AppendAst {
        base: DatasetRef {
            libref: Some("WORK".into()),
            name: "BASE".into(),
        },
        data: DatasetRef {
            libref: Some("WORK".into()),
            name: "DATA".into(),
        },
        force: false,
        nowarn: false,
        appendver: None,
    };
    let result = execute(&ast, &mut session);
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("anomalies"), "msg: {msg}");
}

// --- M33.9 new option tests ---

#[test]
fn parse_nowarn_accepted() {
    let ast = parse_append("proc append base=a data=b force nowarn; run;").unwrap();
    assert!(ast.force);
    assert!(ast.nowarn);
    assert!(ast.appendver.is_none());
}

#[test]
fn parse_appendver_accepted() {
    let ast = parse_append("proc append base=a data=b appendver=v6; run;").unwrap();
    assert_eq!(ast.appendver.as_deref(), Some("V6"));
    assert!(!ast.nowarn);
}

#[test]
fn parse_appendver_v9_accepted() {
    let ast = parse_append("proc append base=a data=b appendver=v9; run;").unwrap();
    assert_eq!(ast.appendver.as_deref(), Some("V9"));
}

#[test]
fn execute_nowarn_suppresses_force_warning() {
    // With FORCE + NOWARN, the "Variable ... not found on BASE file" WARNING
    // should NOT appear in the log.
    let mut session = make_session();

    // BASE has x only; DATA has x and y (y is extra).
    let base_df = df!["x" => [1.0_f64]].unwrap();
    let base_ds = SasDataset {
        df: base_df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "BASE", base_ds);

    let data_df = df!["x" => [2.0_f64], "y" => [99.0_f64]].unwrap();
    let data_ds = SasDataset {
        df: data_df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "DATA", data_ds);

    let ast = AppendAst {
        base: DatasetRef {
            libref: Some("WORK".into()),
            name: "BASE".into(),
        },
        data: DatasetRef {
            libref: Some("WORK".into()),
            name: "DATA".into(),
        },
        force: true,
        nowarn: true,
        appendver: None,
    };
    execute(&ast, &mut session).unwrap();

    // Append must have succeeded.
    let result = read_dataset(&session, "BASE");
    assert_eq!(result.n_obs(), 2, "NOWARN still appends");

    // No WARNING in log.
    let log = session.log.into_string();
    assert!(
        !log.to_uppercase().contains("WARNING"),
        "NOWARN should suppress FORCE warnings, log: {log}"
    );
}

#[test]
fn execute_force_without_nowarn_emits_warning() {
    // Sanity: without NOWARN, the warning IS present.
    let mut session = make_session();

    let base_df = df!["x" => [1.0_f64]].unwrap();
    let base_ds = SasDataset {
        df: base_df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "BASE", base_ds);

    let data_df = df!["x" => [2.0_f64], "y" => [99.0_f64]].unwrap();
    let data_ds = SasDataset {
        df: data_df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "DATA", data_ds);

    let ast = AppendAst {
        base: DatasetRef {
            libref: Some("WORK".into()),
            name: "BASE".into(),
        },
        data: DatasetRef {
            libref: Some("WORK".into()),
            name: "DATA".into(),
        },
        force: true,
        nowarn: false,
        appendver: None,
    };
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    assert!(
        log.contains("not found on BASE"),
        "Without NOWARN, warning must appear, log: {log}"
    );
}

#[test]
fn execute_appendver_no_effect_on_output() {
    // APPENDVER= is a no-op; result identical to without it.
    let mut s1 = make_session();
    let mut s2 = make_session();

    for s in [&mut s1, &mut s2] {
        let base_df = df!["x" => [1.0_f64]].unwrap();
        let base_ds = SasDataset {
            df: base_df,
            vars: vec![num_meta("x")],
        };
        write_dataset(s, "BASE", base_ds);

        let data_df = df!["x" => [2.0_f64]].unwrap();
        let data_ds = SasDataset {
            df: data_df,
            vars: vec![num_meta("x")],
        };
        write_dataset(s, "DATA", data_ds);
    }

    // Without APPENDVER.
    let ast_plain = AppendAst {
        base: DatasetRef {
            libref: Some("WORK".into()),
            name: "BASE".into(),
        },
        data: DatasetRef {
            libref: Some("WORK".into()),
            name: "DATA".into(),
        },
        force: false,
        nowarn: false,
        appendver: None,
    };
    execute(&ast_plain, &mut s1).unwrap();

    // With APPENDVER=V6.
    let ast_ver = AppendAst {
        base: DatasetRef {
            libref: Some("WORK".into()),
            name: "BASE".into(),
        },
        data: DatasetRef {
            libref: Some("WORK".into()),
            name: "DATA".into(),
        },
        force: false,
        nowarn: false,
        appendver: Some("V6".to_string()),
    };
    execute(&ast_ver, &mut s2).unwrap();

    // Both outputs must be identical (2 rows, x=[1.0, 2.0]).
    let r1 = read_dataset(&s1, "BASE");
    let r2 = read_dataset(&s2, "BASE");
    assert_eq!(r1.n_obs(), r2.n_obs());
    assert_eq!(
        decode_column(&r1, 0).unwrap(),
        decode_column(&r2, 0).unwrap(),
        "APPENDVER= must not affect output"
    );
}
