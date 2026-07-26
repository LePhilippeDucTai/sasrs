use super::*;
use crate::session::Session;
use crate::source::SourceFile;
use std::path::PathBuf;

fn make_session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

fn parse_catalog_src(src: &str) -> crate::error::Result<CatalogAst> {
    let source = SourceFile::new(src);
    let mut ts = crate::parser::StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "catalog"
    parse(&mut ts)
}

// ── Parse tests ───────────────────────────────────────────────────────────

#[test]
fn parse_catalog_minimal() {
    let ast = parse_catalog_src("proc catalog catalog=work.formats; quit;").unwrap();
    assert_eq!(ast.catalog, "WORK.FORMATS");
    assert!(ast.stmts.is_empty());
}

#[test]
fn parse_catalog_with_contents() {
    let ast = parse_catalog_src("proc catalog catalog=work.cat; contents; quit;").unwrap();
    assert_eq!(ast.stmts.len(), 1);
    assert!(matches!(ast.stmts[0], CatalogStmt::Contents));
}

#[test]
fn parse_catalog_delete_entry() {
    let ast =
        parse_catalog_src("proc catalog catalog=sasuser.profile; delete myfmt / et=format; quit;")
            .unwrap();
    assert_eq!(ast.stmts.len(), 1);
    match &ast.stmts[0] {
        CatalogStmt::Delete { entries } => {
            assert_eq!(entries, &["MYFMT".to_string()]);
        }
        _ => panic!("expected Delete statement"),
    }
}

#[test]
fn parse_catalog_copy() {
    let ast =
        parse_catalog_src("proc catalog catalog=work.cat; copy out=work.cat2; quit;").unwrap();
    assert_eq!(ast.stmts.len(), 1);
    match &ast.stmts[0] {
        CatalogStmt::Copy { out } => {
            assert_eq!(out.as_deref(), Some("WORK.CAT2"));
        }
        _ => panic!("expected Copy statement"),
    }
}

#[test]
fn parse_catalog_quit_terminates() {
    // Anything after quit; should be ignored
    let ast =
        parse_catalog_src("proc catalog catalog=work.cat; contents; quit; contents;").unwrap();
    // Should have only 1 statement (the one before quit)
    assert_eq!(ast.stmts.len(), 1);
}

// ── Execute tests ─────────────────────────────────────────────────────────

#[test]
fn execute_contents_empty_catalog() {
    let mut session = make_session();
    let ast = CatalogAst {
        catalog: "WORK.FORMATS".to_string(),
        stmts: vec![CatalogStmt::Contents],
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(listing.contains("WORK.FORMATS"), "listing: {listing}");
    // No user formats yet → empty listing with note
    assert!(
        listing.contains("No entries") || listing.is_empty() || listing.contains("Catalog"),
        "listing: {listing}"
    );
}

#[test]
fn execute_delete_noop_with_note() {
    let mut session = make_session();
    let ast = CatalogAst {
        catalog: "WORK.FORMATS".to_string(),
        stmts: vec![CatalogStmt::Delete {
            entries: vec!["MYFORMAT".to_string()],
        }],
    };
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    assert!(
        log.contains("DELETE") || log.contains("no-op"),
        "log: {log}"
    );
    assert!(log.contains("MYFORMAT"), "log: {log}");
}

#[test]
fn execute_copy_noop_with_note() {
    let mut session = make_session();
    let ast = CatalogAst {
        catalog: "WORK.CAT".to_string(),
        stmts: vec![CatalogStmt::Copy {
            out: Some("WORK.CAT2".to_string()),
        }],
    };
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    assert!(log.contains("COPY") || log.contains("no-op"), "log: {log}");
}

#[test]
fn execute_quit_recognized() {
    // Just parsing and executing with no stmts should succeed
    let mut session = make_session();
    let ast = CatalogAst {
        catalog: "WORK.CAT".to_string(),
        stmts: vec![],
    };
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("Processing catalog"), "log: {log}");
}
