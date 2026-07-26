use super::*;
use crate::session::Session;
use crate::source::SourceFile;
use std::path::PathBuf;

fn make_session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

fn parse_printto_src(src: &str) -> Result<PrinttoAst> {
    let source = SourceFile::new(src);
    let mut ts = crate::parser::StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "printto"
    parse(&mut ts)
}

// ── Parse tests ───────────────────────────────────────────────────────────

#[test]
fn parse_reset_bare() {
    let ast = parse_printto_src("proc printto; run;").unwrap();
    assert!(ast.reset);
    assert!(ast.log.is_none());
    assert!(ast.print.is_none());
    assert!(!ast.new);
}

#[test]
fn parse_log_path() {
    let ast = parse_printto_src("proc printto log='/tmp/mylog.txt'; run;").unwrap();
    assert!(!ast.reset);
    assert_eq!(ast.log.as_deref(), Some("/tmp/mylog.txt"));
    assert!(ast.print.is_none());
}

#[test]
fn parse_print_path() {
    let ast = parse_printto_src("proc printto print='/tmp/out.lst'; run;").unwrap();
    assert!(!ast.reset);
    assert_eq!(ast.print.as_deref(), Some("/tmp/out.lst"));
    assert!(ast.log.is_none());
}

#[test]
fn parse_new_option() {
    let ast = parse_printto_src("proc printto log='/tmp/log.txt' new; run;").unwrap();
    assert!(ast.new);
    assert!(ast.log.is_some());
}

#[test]
fn parse_log_fileref() {
    let ast = parse_printto_src("proc printto log=mylog; run;").unwrap();
    assert_eq!(ast.log.as_deref(), Some("mylog"));
}

// ── Execute tests ─────────────────────────────────────────────────────────

#[test]
fn execute_log_stores_path() {
    let mut session = make_session();
    let ast = PrinttoAst {
        log: Some("/tmp/mylog.txt".to_string()),
        print: None,
        new: false,
        reset: false,
    };
    execute(&ast, &mut session).unwrap();

    assert!(session.printto_log.is_some());
    let log = session.log.into_string();
    assert!(log.contains("log redirected"), "log: {log}");
}

#[test]
fn execute_print_stores_path() {
    let mut session = make_session();
    let ast = PrinttoAst {
        log: None,
        print: Some("/tmp/out.lst".to_string()),
        new: false,
        reset: false,
    };
    execute(&ast, &mut session).unwrap();

    assert!(session.printto_print.is_some());
}

#[test]
fn execute_reset_clears_paths() {
    let mut session = make_session();
    session.printto_log = Some(PathBuf::from("/tmp/old.log"));
    session.printto_print = Some(PathBuf::from("/tmp/old.lst"));

    let ast = PrinttoAst {
        log: None,
        print: None,
        new: false,
        reset: true,
    };
    execute(&ast, &mut session).unwrap();

    assert!(session.printto_log.is_none());
    assert!(session.printto_print.is_none());
    let log = session.log.into_string();
    assert!(log.contains("reset"), "log: {log}");
}

#[test]
fn execute_new_noted_in_log() {
    let mut session = make_session();
    let ast = PrinttoAst {
        log: Some("/tmp/newlog.txt".to_string()),
        print: None,
        new: true,
        reset: false,
    };
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    assert!(log.contains("NEW"), "log: {log}");
}

#[test]
fn execute_does_not_affect_listing_output() {
    // Default listing output should be unaffected by PRINTTO
    let mut session = make_session();
    let ast = PrinttoAst {
        log: Some("/tmp/ignored.txt".to_string()),
        print: None,
        new: false,
        reset: false,
    };
    execute(&ast, &mut session).unwrap();

    // listing should still be empty (PRINTTO alone writes nothing to listing)
    let listing = session.listing.into_string();
    assert!(
        listing.is_empty(),
        "listing should be empty after PRINTTO: {listing}"
    );
}
