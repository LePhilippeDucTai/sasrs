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
    assert_eq!(ast.id, vec!["k".to_string()]);
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
fn contract_transpose_unknown_substatement_errors() {
    // The previous skip silently ran a different program (CONTRIBUTING §5).
    let err = parse_transpose("proc transpose data=a out=b; delete foo; var x; run;")
        .err()
        .unwrap()
        .to_string();
    assert!(err.contains("180-322") && err.contains("DELETE"), "{err}");
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
        id: vec![],
        var: vec!["x".into()],
        name: None,
        suffix: None,
        label: None,
        let_option: false,
        delimiter: None,
        by_desc: vec![],
        notsorted: false,
        idlabel: None,
        copy: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    assert_eq!(out.n_obs(), 1);
    // _NAME_ = "x"
    let name = read_col(&session, "O", "_NAME_");
    assert_eq!(name, vec![Value::Char("X".into())]);
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
        id: vec![],
        var: vec!["x".into()],
        name: None,
        suffix: None,
        label: None,
        let_option: false,
        delimiter: None,
        by_desc: vec![],
        notsorted: false,
        idlabel: None,
        copy: vec![],
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
        id: vec![],
        var: vec!["x".into()],
        name: None,
        suffix: None,
        label: None,
        let_option: false,
        delimiter: None,
        by_desc: vec![],
        notsorted: false,
        idlabel: None,
        copy: vec![],
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
        id: vec!["k".into()],
        var: vec!["x".into()],
        name: None,
        suffix: None,
        label: None,
        let_option: false,
        delimiter: None,
        by_desc: vec![],
        notsorted: false,
        idlabel: None,
        copy: vec![],
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
        id: vec!["k".into()],
        var: vec!["x".into()],
        name: None,
        suffix: None,
        label: None,
        let_option: false,
        delimiter: None,
        by_desc: vec![],
        notsorted: false,
        idlabel: None,
        copy: vec![],
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
        id: vec!["k".into()],
        var: vec!["x".into()],
        name: None,
        suffix: None,
        label: None,
        let_option: false,
        delimiter: None,
        by_desc: vec![],
        notsorted: false,
        idlabel: None,
        copy: vec![],
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
        id: vec![],
        var: vec!["x".into(), "y".into()],
        name: None,
        suffix: None,
        label: None,
        let_option: false,
        delimiter: None,
        by_desc: vec![],
        notsorted: false,
        idlabel: None,
        copy: vec![],
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
    assert_eq!(name, vec![Value::Char("X".into()), Value::Char("Y".into())]);
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
        id: vec!["k".into()],
        var: vec![],
        name: None,
        suffix: None,
        label: None,
        let_option: false,
        delimiter: None,
        by_desc: vec![],
        notsorted: false,
        idlabel: None,
        copy: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    // Single var x -> one output row.
    assert_eq!(out.n_obs(), 1);
    let name = read_col(&session, "O", "_NAME_");
    assert_eq!(name, vec![Value::Char("X".into())]);
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
        id: vec![],
        var: vec!["x".into()],
        name: Some("source".into()),
        suffix: None,
        label: None,
        let_option: false,
        delimiter: None,
        by_desc: vec![],
        notsorted: false,
        idlabel: None,
        copy: vec![],
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
        id: vec![],
        var: vec!["x".into()],
        name: None,
        suffix: None,
        label: None,
        let_option: false,
        delimiter: None,
        by_desc: vec![],
        notsorted: false,
        idlabel: None,
        copy: vec![],
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
        id: vec![],
        var: vec!["x".into()],
        name: None,
        suffix: None,
        label: None,
        let_option: false,
        delimiter: None,
        by_desc: vec![],
        notsorted: false,
        idlabel: None,
        copy: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    assert!(
        log.contains("The data set WORK.O has 1 observations and"),
        "log: {log}"
    );
}

// ─── Tests de compatibilité J07-P4 (cas conformance/cases/compat/transpose) ───
//
// Chaque test exécute un PROGRAMME complet via la façade `crate::run`
// (comme le corpus de conformité et les tests compat de means/compare),
// puis relit WORK via l'API dataset. Les attendus sont ceux des oracles
// gelés J07-P1.

use crate::{RunOptions, run};
use std::collections::BTreeMap;
use std::path::Path;

/// Bac à sable : `data/` rempli par `setup`, WORK isolé, exécution du
/// programme, puis relecture de toutes les tables WORK produites.
fn compat_run_in_sandbox(
    setup: impl FnOnce(&Path),
    program: &str,
) -> (i32, String, BTreeMap<String, SasDataset>) {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    setup(&tmp.path().join("data"));
    let outcome = run(
        program,
        RunOptions {
            work_dir: Some(work.clone()),
            base_dir: Some(tmp.path().to_path_buf()),
            deterministic: true,
            vectorize: false,
        },
    );
    let mut tables = BTreeMap::new();
    for entry in std::fs::read_dir(&work).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_some_and(|e| e == "parquet") {
            let stem = p.file_stem().unwrap().to_str().unwrap().to_string();
            if let Ok((ds, _)) = SasDataset::read_parquet(&p) {
                tables.insert(stem.to_lowercase(), ds);
            }
        }
    }
    (outcome.exit_code, outcome.log, tables)
}

fn compat_write_input(dir: &Path, name: &str, mut df: DataFrame) {
    use std::fs::File;
    std::fs::create_dir_all(dir).unwrap();
    let mut file = File::create(dir.join(format!("{name}.parquet"))).unwrap();
    ParquetWriter::new(&mut file).finish(&mut df).unwrap();
}

fn compat_columns(tables: &BTreeMap<String, SasDataset>, table: &str) -> Vec<String> {
    tables
        .get(table)
        .unwrap_or_else(|| panic!("WORK.{table} absent"))
        .vars
        .iter()
        .map(|m| m.name.to_uppercase())
        .collect()
}

fn compat_num_col(
    tables: &BTreeMap<String, SasDataset>,
    table: &str,
    col: &str,
) -> Vec<Option<f64>> {
    let ds = tables
        .get(table)
        .unwrap_or_else(|| panic!("WORK.{table} absent"));
    let idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(col))
        .unwrap_or_else(|| panic!("colonne {col} absente de WORK.{table}"));
    ds.df.get_columns()[idx]
        .as_materialized_series()
        .f64()
        .unwrap()
        .iter()
        .collect()
}

fn compat_char_col(tables: &BTreeMap<String, SasDataset>, table: &str, col: &str) -> Vec<String> {
    let ds = tables
        .get(table)
        .unwrap_or_else(|| panic!("WORK.{table} absent"));
    let idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(col))
        .unwrap_or_else(|| panic!("colonne {col} absente de WORK.{table}"));
    ds.df.get_columns()[idx]
        .as_materialized_series()
        .str()
        .unwrap()
        .iter()
        .map(|o| o.unwrap_or_default().to_string())
        .collect()
}

/// Reproduit le cas compat transpose-copy-suffix (oracle gelé J07-P1) :
/// instruction COPY (padding a hauteur des observations d'entrée) et
/// option SUFFIX= (colonne transposée nommée d'après la variable source).
#[test]
fn transpose_compat_copy_suffix() {
    let data = |dir: &Path| {
        compat_write_input(
            dir,
            "t",
            df![
                "grader" => ["G1", "G2", "G3"],
                "score" => [90.0_f64, 85.0, 80.0],
            ]
            .unwrap(),
        );
    };
    let program = r#"
libname ind 'data';

proc transpose data=ind.t out=out suffix=_s;
  var score;
  copy grader;
run;
"#;
    let (code, log, tables) = compat_run_in_sandbox(data, program);
    assert_eq!(code, 0, "log:\n{log}");
    assert!(!log.contains("ERROR:"), "log:\n{log}");

    // Colonnes : grader, _NAME_, SCORE_s (oracle).
    assert_eq!(
        compat_columns(&tables, "out"),
        vec!["GRADER", "_NAME_", "SCORE_S"],
        "colonnes : {:?}",
        compat_columns(&tables, "out")
    );
    // 3 observations (autant que l'entrée — COPY).
    assert_eq!(tables.get("out").unwrap().n_obs(), 3, "log:\n{log}");
    // grader recopié tel quel ; _NAME_ = SCORE partout ; SCORE_s = 90
    // puis padding missing (1 seule variable transposée pour 3 obs).
    assert_eq!(
        compat_char_col(&tables, "out", "grader"),
        vec!["G1", "G2", "G3"]
    );
    assert_eq!(
        compat_char_col(&tables, "out", "_NAME_"),
        vec!["SCORE", "SCORE", "SCORE"]
    );
    assert_eq!(
        compat_num_col(&tables, "out", "SCORE_S"),
        vec![Some(90.0), None, None]
    );
}

/// Reproduit le cas compat transpose-id-let-delimiter (oracle gelé
/// J07-P1) : ID multi-variables avec DELIMITER=, LET (dernière occurrence
/// des ID dupliquées, WARNING et non ERROR), IDLABEL et LABEL=.
#[test]
fn transpose_compat_id_let_delimiter() {
    let data = |dir: &Path| {
        compat_write_input(
            dir,
            "u",
            df![
                "grp" => ["A", "A", "A"],
                "metric" => ["ht", "ht", "wt"],
                "value" => [180.0_f64, 181.0, 75.0],
                "lbl" => ["Height cm", "Height dup", "Weight kg"],
            ]
            .unwrap(),
        );
    };
    let program = r#"
libname ind 'data';

proc transpose data=ind.u out=out let delimiter=_ label=mlabel;
  id grp metric;
  idlabel lbl;
  var value;
run;
"#;
    let (code, log, tables) = compat_run_in_sandbox(data, program);
    assert_eq!(code, 0, "log:\n{log}");
    assert!(!log.contains("ERROR:"), "log:\n{log}");
    // LET : l'ID dupliqué (A,ht) produit un WARNING, pas un ERROR.
    assert!(
        log.contains("occurs twice in the same BY group"),
        "log:\n{log}"
    );

    // Colonnes : _NAME_, mlabel, A_ht, A_wt (oracle).
    let cols = compat_columns(&tables, "out");
    assert_eq!(cols, vec!["_NAME_", "MLABEL", "A_HT", "A_WT"], "{cols:?}");
    assert_eq!(tables.get("out").unwrap().n_obs(), 2);

    assert_eq!(
        compat_char_col(&tables, "out", "_NAME_"),
        vec!["VALUE", "VALUE"]
    );
    // Labels de la dernière occurrence de chaque ID (LET).
    assert_eq!(
        compat_char_col(&tables, "out", "mlabel"),
        vec!["Height dup", "Weight kg"]
    );
    // Dernière occurrence de A_ht = 181 ; A_wt = 75 ; hors diagonale :
    // missing (oracle).
    assert_eq!(
        compat_num_col(&tables, "out", "A_ht"),
        vec![Some(181.0), None]
    );
    assert_eq!(
        compat_num_col(&tables, "out", "A_wt"),
        vec![None, Some(75.0)]
    );
}

/// Sans LET, un ID dupliqué dans un groupe arrête la procédure (ERROR
/// SAS exacte) — le test prouve que LET change bien le comportement.
#[test]
fn transpose_compat_duplicate_id_without_let_errors() {
    let data = |dir: &Path| {
        compat_write_input(
            dir,
            "u",
            df![
                "grp" => ["A", "A", "A"],
                "metric" => ["ht", "ht", "wt"],
                "value" => [180.0_f64, 181.0, 75.0],
                "lbl" => ["Height cm", "Height dup", "Weight kg"],
            ]
            .unwrap(),
        );
    };
    let program = r#"
libname ind 'data';

proc transpose data=ind.u out=out delimiter=_;
  id grp metric;
  var value;
run;
"#;
    let (code, log, tables) = compat_run_in_sandbox(data, program);
    // Le programme échoue : ERROR « occurs twice in the same BY group ».
    assert_ne!(code, 0, "log:\n{log}");
    assert!(
        log.contains("The ID value \"A_ht\" occurs twice in the same BY group."),
        "log:\n{log}"
    );
}
