use super::*;
use crate::source::SourceFile;

use std::path::PathBuf;

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

// ── Execute tests (J07-P5 — routage réel) ─────────────────────────────────

/// Session dont les chemins relatifs se résolvent dans un tempdir isolé.
fn session_in_tempdir() -> (crate::session::Session, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let session = crate::session::Session::new(None, dir.path().to_path_buf(), true).unwrap();
    (session, dir)
}

fn log_ast(path: &std::path::Path, new: bool) -> PrinttoAst {
    PrinttoAst {
        log: Some(path.display().to_string()),
        print: None,
        new,
        reset: false,
    }
}

fn print_ast(path: &std::path::Path, new: bool) -> PrinttoAst {
    PrinttoAst {
        log: None,
        print: Some(path.display().to_string()),
        new,
        reset: false,
    }
}

fn reset_ast() -> PrinttoAst {
    PrinttoAst {
        log: None,
        print: None,
        new: false,
        reset: true,
    }
}

#[test]
fn execute_log_routes_really_appends() {
    let (mut session, dir) = session_in_tempdir();
    let target = dir.path().join("routed.log");
    std::fs::write(&target, "OLD CONTENT\n").unwrap();

    execute(&log_ast(&target, false), &mut session).unwrap();
    assert!(session.log.route_active());
    session.log.note("ROUTED LINE");

    // Le segment routé part au fichier au reset, PAS au log par défaut.
    execute(&reset_ast(), &mut session).unwrap();
    assert!(!session.log.route_active());
    let file = std::fs::read_to_string(&target).unwrap();
    assert!(file.contains("OLD CONTENT"), "{file}");
    assert!(file.contains("ROUTED LINE"), "{file}");
    let default_log = session.log.current_text();
    assert!(!default_log.contains("ROUTED LINE"), "{default_log}");
    assert!(default_log.contains("reset to default"), "{default_log}");
}

#[test]
fn execute_log_new_replaces_file() {
    let (mut session, dir) = session_in_tempdir();
    let target = dir.path().join("routed.log");
    std::fs::write(&target, "OLD CONTENT\n").unwrap();

    execute(&log_ast(&target, true), &mut session).unwrap();
    session.log.note("FRESH LINE");
    execute(&reset_ast(), &mut session).unwrap();

    let file = std::fs::read_to_string(&target).unwrap();
    assert!(!file.contains("OLD CONTENT"), "{file}");
    assert!(file.contains("FRESH LINE"), "{file}");
}

#[test]
fn execute_log_open_failure_is_counted_error() {
    let (mut session, dir) = session_in_tempdir();
    // Le répertoire parent n'existe pas : l'ouverture doit échouer.
    let target = dir.path().join("no/such/dir/routed.log");
    let errors_before = session.log.errors;

    execute(&log_ast(&target, false), &mut session).unwrap();

    assert!(!session.log.route_active());
    assert_eq!(session.log.errors, errors_before + 1);
    let default_log = session.log.current_text();
    assert!(
        default_log.contains("ERROR: PROCEDURE PRINTTO: cannot open log destination"),
        "{default_log}"
    );
}

#[test]
fn execute_log_route_flushed_at_consumption() {
    // Route laissée ouverte (pas de PROC PRINTTO nu) : le drain final du
    // LogWriter (into_parts, façade api) vide quand même le segment routé.
    let (mut session, dir) = session_in_tempdir();
    let target = dir.path().join("routed.log");
    execute(&log_ast(&target, false), &mut session).unwrap();
    session.log.note("TAIL LINE");
    // Même protocole que la façade api : l'écrivain est remplacé puis consommé.
    let writer = std::mem::replace(&mut session.log, crate::log::LogWriter::new(true));
    let default_log = writer.into_string();
    assert!(!default_log.contains("TAIL LINE"), "{default_log}");
    let file = std::fs::read_to_string(&target).unwrap();
    assert!(file.contains("TAIL LINE"), "{file}");
}

#[test]
fn execute_print_routes_listing_to_file() {
    let (mut session, dir) = session_in_tempdir();
    let target = dir.path().join("routed.lst");
    std::fs::write(&target, "OLD LISTING\n").unwrap();

    execute(&print_ast(&target, false), &mut session).unwrap();
    assert_eq!(session.printto_print.as_deref(), Some(target.as_path()));
    session.listing.write_line("ROUTED LISTING LINE");

    execute(&reset_ast(), &mut session).unwrap();
    assert!(session.printto_print.is_none());
    let file = std::fs::read_to_string(&target).unwrap();
    assert!(file.contains("OLD LISTING"), "{file}");
    assert!(file.contains("ROUTED LISTING LINE"), "{file}");
    // Le listing par défaut (après reset) ne contient pas le segment routé.
    session.listing.write_line("BACK TO DEFAULT");
    let listing = session.take_completed_listing() + &session.listing.take_string();
    assert!(!listing.contains("ROUTED LISTING LINE"), "{listing}");
    assert!(listing.contains("BACK TO DEFAULT"), "{listing}");
}

#[test]
fn execute_print_new_replaces_file() {
    let (mut session, dir) = session_in_tempdir();
    let target = dir.path().join("routed.lst");
    std::fs::write(&target, "OLD LISTING\n").unwrap();

    execute(&print_ast(&target, true), &mut session).unwrap();
    session.listing.write_line("FRESH LISTING");
    execute(&reset_ast(), &mut session).unwrap();

    let file = std::fs::read_to_string(&target).unwrap();
    assert!(!file.contains("OLD LISTING"), "{file}");
    assert!(file.contains("FRESH LISTING"), "{file}");
}

#[test]
fn execute_print_open_failure_is_counted_error() {
    let (mut session, dir) = session_in_tempdir();
    let target = dir.path().join("no/such/dir/routed.lst");
    let errors_before = session.log.errors;

    execute(&print_ast(&target, false), &mut session).unwrap();

    assert!(session.printto_print.is_none());
    assert_eq!(session.log.errors, errors_before + 1);
    let default_log = session.log.current_text();
    assert!(
        default_log.contains("ERROR: PROCEDURE PRINTTO: cannot open print destination"),
        "{default_log}"
    );
}

#[test]
fn execute_log_stores_path() {
    let (mut session, dir) = session_in_tempdir();
    let target = dir.path().join("mylog.txt");
    execute(&log_ast(&target, false), &mut session).unwrap();
    execute(&reset_ast(), &mut session).unwrap();

    // After reset the destination is back to default; during the route the
    // path was traced in printto_log.
    assert!(session.printto_log.is_none());
}

#[test]
fn execute_reset_clears_paths() {
    let (mut session, _dir) = session_in_tempdir();
    session.printto_log = Some(PathBuf::from("/tmp/old.log"));

    execute(&reset_ast(), &mut session).unwrap();

    assert!(session.printto_log.is_none());
    assert!(session.printto_print.is_none());
    let log = session.log.current_text();
    assert!(log.contains("reset"), "log: {log}");
}

#[test]
fn execute_does_not_affect_listing_output() {
    // Default listing output should be unaffected by a LOG= route
    let (mut session, dir) = session_in_tempdir();
    execute(
        &log_ast(&dir.path().join("ignored.txt"), false),
        &mut session,
    )
    .unwrap();
    execute(&reset_ast(), &mut session).unwrap();

    // listing should still be empty (PRINTTO alone writes nothing to listing)
    let listing = session.listing.take_string();
    assert!(
        listing.is_empty(),
        "listing should be empty after PRINTTO: {listing}"
    );
}
