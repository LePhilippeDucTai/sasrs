use super::*;
use crate::session::Session;
use crate::source::SourceFile;
use std::path::PathBuf;

fn make_session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

fn parse_options_src(src: &str) -> crate::error::Result<OptionsAst> {
    let source = SourceFile::new(src);
    let mut ts = crate::parser::StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "options"
    parse(&mut ts)
}

// ── Parse tests ───────────────────────────────────────────────────────────

#[test]
fn parse_no_options() {
    let ast = parse_options_src("proc options; run;").unwrap();
    assert!(ast.option_names.is_empty());
    assert!(!ast.short);
    assert!(!ast.long);
}

#[test]
fn parse_specific_options() {
    let ast = parse_options_src("proc options obs linesize; run;").unwrap();
    assert_eq!(ast.option_names, vec!["OBS", "LINESIZE"]);
}

#[test]
fn parse_short_flag() {
    let ast = parse_options_src("proc options short; run;").unwrap();
    assert!(ast.short);
    // "short" is not added to option_names
    assert!(ast.option_names.is_empty());
}

#[test]
fn parse_long_flag() {
    let ast = parse_options_src("proc options long; run;").unwrap();
    assert!(ast.long);
}

// ── Execute tests ─────────────────────────────────────────────────────────

#[test]
fn execute_all_options_writes_to_log() {
    let mut session = make_session();
    let ast = OptionsAst {
        option_names: vec![],
        short: false,
        long: false,
    };
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    assert!(log.contains("OBS="), "log: {log}");
    assert!(log.contains("LINESIZE="), "log: {log}");
    assert!(log.contains("FIRSTOBS="), "log: {log}");
    // Boolean options appear with NO prefix when false
    assert!(log.contains("NODATE") || log.contains("DATE"), "log: {log}");
}

#[test]
fn execute_specific_option_obs() {
    let mut session = make_session();
    let ast = OptionsAst {
        option_names: vec!["OBS".to_string()],
        short: false,
        long: false,
    };
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    assert!(log.contains("OBS=MAX"), "log: {log}");
    // Should not contain other options
    assert!(!log.contains("LINESIZE="), "should not contain LINESIZE: {log}");
}

#[test]
fn execute_specific_option_linesize() {
    let mut session = make_session();
    let ast = OptionsAst {
        option_names: vec!["LINESIZE".to_string()],
        short: false,
        long: false,
    };
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    assert!(log.contains("LINESIZE=96"), "log: {log}");
}

#[test]
fn execute_unknown_option_warns() {
    let mut session = make_session();
    let ast = OptionsAst {
        option_names: vec!["UNKNOWNOPTION123".to_string()],
        short: false,
        long: false,
    };
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    assert!(log.contains("WARNING") || log.contains("not a recognized"), "log: {log}");
}

#[test]
fn execute_boolean_option_format() {
    let mut session = make_session();
    // Default: MPRINT = false → should appear as NOMPRINT
    let ast = OptionsAst {
        option_names: vec!["MPRINT".to_string()],
        short: false,
        long: false,
    };
    execute(&ast, &mut session).unwrap();

    let log = session.log.into_string();
    assert!(log.contains("NOMPRINT"), "log: {log}");
}

#[test]
fn execute_does_not_write_to_listing() {
    let mut session = make_session();
    let ast = OptionsAst {
        option_names: vec![],
        short: false,
        long: false,
    };
    execute(&ast, &mut session).unwrap();

    // PROC OPTIONS writes to log only, not listing
    let listing = session.listing.into_string();
    assert!(listing.is_empty(), "listing should be empty: {listing}");
}
