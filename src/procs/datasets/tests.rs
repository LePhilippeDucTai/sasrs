use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::session::Session;
use crate::source::SourceFile;
use crate::testkit::*;
use crate::value::VarType;
use polars::prelude::*;

fn parse_datasets_src(src: &str) -> Result<DatasetsAst> {
    let source = SourceFile::new(src);
    let mut ts = crate::parser::StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "datasets"
    parse(&mut ts)
}

/// Write a simple numeric dataset into WORK.
fn write_simple_dataset(session: &mut Session, name: &str) {
    let df = df!["x" => [1.0_f64, 2.0]].unwrap();
    let vars = vec![VarMeta {
        name: "x".to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }];
    let ds = SasDataset { df, vars };
    session.libs.get("WORK").unwrap().write(name, &ds).unwrap();
}

/// Write a dataset with a format and label so a sidecar file is created.
fn write_dataset_with_meta(session: &mut Session, name: &str) {
    let df = df!["age" => [30.0_f64, 25.0]].unwrap();
    let vars = vec![VarMeta {
        name: "age".to_string(),
        ty: VarType::Num,
        length: 8,
        format: Some("best12.".to_string()),
        label: Some("Age".to_string()),
    }];
    let ds = SasDataset { df, vars };
    session.libs.get("WORK").unwrap().write(name, &ds).unwrap();
}

// ── Parse tests ───────────────────────────────────────────────────────────

#[test]
fn parse_full_example() {
    let src = "proc datasets lib=work nolist; delete a b; change c=d; quit;";
    let ast = parse_datasets_src(src).unwrap();
    assert_eq!(ast.lib, "WORK");
    assert!(ast.nolist);
    assert_eq!(ast.deletes, vec!["A".to_string(), "B".to_string()]);
    assert_eq!(ast.changes, vec![("C".to_string(), "D".to_string())]);
}

#[test]
fn parse_defaults_to_work() {
    let src = "proc datasets nolist; quit;";
    let ast = parse_datasets_src(src).unwrap();
    assert_eq!(ast.lib, "WORK");
    assert!(ast.nolist);
    assert!(ast.deletes.is_empty());
    assert!(ast.changes.is_empty());
}

#[test]
fn parse_library_alias() {
    let src = "proc datasets library=mylib nolist; quit;";
    let ast = parse_datasets_src(src).unwrap();
    assert_eq!(ast.lib, "MYLIB");
    assert!(ast.nolist);
}

#[test]
fn parse_no_nolist_defaults_false() {
    let src = "proc datasets lib=work; quit;";
    let ast = parse_datasets_src(src).unwrap();
    assert!(!ast.nolist);
}

#[test]
fn parse_multiple_changes() {
    let src = "proc datasets lib=work nolist; change a=b c=d; quit;";
    let ast = parse_datasets_src(src).unwrap();
    assert_eq!(
        ast.changes,
        vec![
            ("A".to_string(), "B".to_string()),
            ("C".to_string(), "D".to_string()),
        ]
    );
}

#[test]
fn parse_run_is_noop_separator() {
    // run; between statements should not stop accumulation
    let src = "proc datasets lib=work nolist; delete a; run; change b=c; quit;";
    let ast = parse_datasets_src(src).unwrap();
    assert_eq!(ast.deletes, vec!["A".to_string()]);
    assert_eq!(ast.changes, vec![("B".to_string(), "C".to_string())]);
}

// ── Execute tests ─────────────────────────────────────────────────────────

#[test]
fn execute_delete_removes_table() {
    let mut session = make_session();
    write_simple_dataset(&mut session, "ALPHA");

    assert!(session.libs.get("WORK").unwrap().exists("ALPHA"));

    let ast = DatasetsAst {
        lib: "WORK".to_string(),
        nolist: true,
        deletes: vec!["ALPHA".to_string()],
        changes: vec![],
        ops: vec![],
    };
    execute(&ast, &mut session).unwrap();

    assert!(!session.libs.get("WORK").unwrap().exists("ALPHA"));
    let log = session.log.into_string();
    assert!(log.contains("Deleting"), "log: {log}");
    assert!(log.contains("ALPHA"), "log: {log}");
}

#[test]
fn execute_delete_missing_is_warning_not_error() {
    let mut session = make_session();

    let ast = DatasetsAst {
        lib: "WORK".to_string(),
        nolist: true,
        deletes: vec!["NONEXISTENT".to_string()],
        changes: vec![],
        ops: vec![],
    };
    // Must not return Err
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    assert!(
        log.contains("WARNING") || log.contains("does not exist"),
        "log: {log}"
    );
}

#[test]
fn execute_change_renames_table() {
    let mut session = make_session();
    write_simple_dataset(&mut session, "OLDNAME");

    assert!(session.libs.get("WORK").unwrap().exists("OLDNAME"));
    assert!(!session.libs.get("WORK").unwrap().exists("NEWNAME"));

    let ast = DatasetsAst {
        lib: "WORK".to_string(),
        nolist: true,
        deletes: vec![],
        changes: vec![("OLDNAME".to_string(), "NEWNAME".to_string())],
        ops: vec![],
    };
    execute(&ast, &mut session).unwrap();

    assert!(!session.libs.get("WORK").unwrap().exists("OLDNAME"));
    assert!(session.libs.get("WORK").unwrap().exists("NEWNAME"));

    // Verify data is intact
    let (ds, _) = session.libs.get("WORK").unwrap().read("NEWNAME").unwrap();
    assert_eq!(ds.n_obs(), 2);

    let log = session.log.into_string();
    assert!(log.contains("Changing"), "log: {log}");
    assert!(log.contains("OLDNAME"), "log: {log}");
    assert!(log.contains("NEWNAME"), "log: {log}");
}

#[test]
fn execute_nolist_suppresses_listing() {
    let mut session = make_session();
    write_simple_dataset(&mut session, "T1");

    let ast = DatasetsAst {
        lib: "WORK".to_string(),
        nolist: true,
        deletes: vec![],
        changes: vec![],
        ops: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(
        listing.is_empty(),
        "listing should be empty with nolist: {listing}"
    );
}

#[test]
fn execute_without_nolist_emits_directory() {
    let mut session = make_session();
    write_simple_dataset(&mut session, "T1");
    write_simple_dataset(&mut session, "T2");

    let ast = DatasetsAst {
        lib: "WORK".to_string(),
        nolist: false,
        deletes: vec![],
        changes: vec![],
        ops: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(listing.contains("T1"), "listing: {listing}");
    assert!(listing.contains("T2"), "listing: {listing}");
    assert!(listing.contains("DATA"), "Member Type column: {listing}");
    assert!(listing.contains("Name"), "Name header: {listing}");
}

#[test]
fn execute_rename_moves_sidecar() {
    let mut session = make_session();
    write_dataset_with_meta(&mut session, "WITHFORMAT");

    let ast = DatasetsAst {
        lib: "WORK".to_string(),
        nolist: true,
        deletes: vec![],
        changes: vec![("WITHFORMAT".to_string(), "RENAMED".to_string())],
        ops: vec![],
    };
    execute(&ast, &mut session).unwrap();

    assert!(!session.libs.get("WORK").unwrap().exists("WITHFORMAT"));
    assert!(session.libs.get("WORK").unwrap().exists("RENAMED"));

    // Read back and verify format survived the rename (sidecar was moved)
    let (ds, _) = session.libs.get("WORK").unwrap().read("RENAMED").unwrap();
    let age_var = ds
        .vars
        .iter()
        .find(|v| v.name.eq_ignore_ascii_case("age"))
        .unwrap();
    assert_eq!(
        age_var.format.as_deref(),
        Some("best12."),
        "format should survive rename via sidecar move"
    );
    assert_eq!(
        age_var.label.as_deref(),
        Some("Age"),
        "label should survive rename via sidecar move"
    );
}

// ── M33.8 : COPY / EXCHANGE / SAVE / MODIFY ───────────────────────────────

fn base_ast(lib: &str) -> DatasetsAst {
    DatasetsAst {
        lib: lib.to_string(),
        nolist: true,
        deletes: vec![],
        changes: vec![],
        ops: vec![],
    }
}

#[test]
fn parse_copy_with_select() {
    let src = "proc datasets lib=work nolist; copy out=tgt in=src; select a b; quit;";
    let ast = parse_datasets_src(src).unwrap();
    assert_eq!(
        ast.ops,
        vec![DsOp::Copy {
            out: "TGT".into(),
            r#in: Some("SRC".into()),
            select: vec!["A".into(), "B".into()],
        }]
    );
}

#[test]
fn parse_exchange_save_modify() {
    let src = "proc datasets lib=work nolist; \
               exchange a=b; save keep1 keep2; \
               modify m; rename old=new; label v='hi'; quit;";
    let ast = parse_datasets_src(src).unwrap();
    assert_eq!(
        ast.ops,
        vec![
            DsOp::Exchange("A".into(), "B".into()),
            DsOp::Save(vec!["KEEP1".into(), "KEEP2".into()]),
            DsOp::Modify {
                member: "M".into(),
                renames: vec![("old".into(), "new".into())],
                labels: vec![("v".into(), "hi".into())],
            },
        ]
    );
}

#[test]
fn execute_copy_moves_member_to_other_lib() {
    let mut session = make_session();
    // Source lib = WORK; destination lib = a fresh assigned dir.
    write_simple_dataset(&mut session, "SRCTAB");
    let tmp = tempfile::tempdir().unwrap();
    session
        .libs
        .assign("TGT", tmp.path().to_path_buf())
        .unwrap();

    let ast = DatasetsAst {
        ops: vec![DsOp::Copy {
            out: "TGT".into(),
            r#in: None, // defaults to WORK (the PROC's lib)
            select: vec!["SRCTAB".into()],
        }],
        ..base_ast("WORK")
    };
    execute(&ast, &mut session).unwrap();

    // Original still in WORK, copy present in TGT.
    assert!(session.libs.get("WORK").unwrap().exists("SRCTAB"));
    assert!(session.libs.get("TGT").unwrap().exists("SRCTAB"));
    let (ds, _) = session.libs.get("TGT").unwrap().read("SRCTAB").unwrap();
    assert_eq!(ds.n_obs(), 2);
}

#[test]
fn execute_exchange_swaps_names() {
    let mut session = make_session();
    // ALPHA holds x=[1,2]; BETA holds a single different row so we can tell
    // them apart after the swap.
    write_simple_dataset(&mut session, "ALPHA"); // x = [1, 2]
    let df = df!["x" => [9.0_f64]].unwrap();
    let vars = vec![VarMeta {
        name: "x".into(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }];
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("BETA", &SasDataset { df, vars })
        .unwrap();

    let ast = DatasetsAst {
        ops: vec![DsOp::Exchange("ALPHA".into(), "BETA".into())],
        ..base_ast("WORK")
    };
    execute(&ast, &mut session).unwrap();

    // After exchange: ALPHA now has BETA's content (1 row), BETA has 2 rows.
    let (a, _) = session.libs.get("WORK").unwrap().read("ALPHA").unwrap();
    let (b, _) = session.libs.get("WORK").unwrap().read("BETA").unwrap();
    assert_eq!(a.n_obs(), 1, "ALPHA should now be old BETA");
    assert_eq!(b.n_obs(), 2, "BETA should now be old ALPHA");
}

#[test]
fn execute_save_deletes_all_but_listed() {
    let mut session = make_session();
    write_simple_dataset(&mut session, "KEEPME");
    write_simple_dataset(&mut session, "DROP1");
    write_simple_dataset(&mut session, "DROP2");

    let ast = DatasetsAst {
        ops: vec![DsOp::Save(vec!["KEEPME".into()])],
        ..base_ast("WORK")
    };
    execute(&ast, &mut session).unwrap();

    assert!(session.libs.get("WORK").unwrap().exists("KEEPME"));
    assert!(!session.libs.get("WORK").unwrap().exists("DROP1"));
    assert!(!session.libs.get("WORK").unwrap().exists("DROP2"));
}

#[test]
fn execute_modify_renames_variable_and_sets_label() {
    let mut session = make_session();
    write_dataset_with_meta(&mut session, "MTAB"); // var "age"

    let ast = DatasetsAst {
        ops: vec![DsOp::Modify {
            member: "MTAB".into(),
            renames: vec![("age".into(), "years".into())],
            labels: vec![("years".into(), "Years old".into())],
        }],
        ..base_ast("WORK")
    };
    execute(&ast, &mut session).unwrap();

    let (ds, _) = session.libs.get("WORK").unwrap().read("MTAB").unwrap();
    // Variable renamed (no "age", has "years"), and label updated.
    assert!(ds.vars.iter().all(|v| !v.name.eq_ignore_ascii_case("age")));
    let years = ds
        .vars
        .iter()
        .find(|v| v.name.eq_ignore_ascii_case("years"))
        .unwrap();
    assert_eq!(years.label.as_deref(), Some("Years old"));
    // DataFrame column was renamed too.
    assert!(ds.df.column("years").is_ok(), "df column renamed");
}
