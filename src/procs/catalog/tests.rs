use super::*;
use crate::source::SourceFile;
use crate::testkit::*;

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
fn contract_catalog_mutations_error() {
    for stmt in ["delete myfmt / et=format", "copy out=work.cat2"] {
        let err = parse_catalog_src(&format!("proc catalog catalog=work.cat; {stmt}; quit;"))
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("not supported in PROC CATALOG"), "{err}");
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
fn contract_catalog_execute_rejects_unsupported_ast() {
    for stmt in [
        CatalogStmt::Delete {
            entries: vec!["MYFORMAT".into()],
        },
        CatalogStmt::Copy {
            out: Some("WORK.CAT2".into()),
        },
    ] {
        let mut session = make_session();
        let ast = CatalogAst {
            catalog: "WORK.FORMATS".into(),
            stmts: vec![stmt],
        };
        let err = execute(&ast, &mut session).unwrap_err().to_string();
        assert!(err.contains("not supported in PROC CATALOG"), "{err}");
    }
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
