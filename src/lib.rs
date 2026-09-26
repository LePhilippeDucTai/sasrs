//! `sas_interpreter` — interpréteur du langage SAS (référence SAS 9.4
//! classique) sur Polars, tables au format Parquet.
//!
//! Voir PLAN.md pour l'architecture et les décisions actées, PROGRESS.md pour
//! l'état d'avancement jalon par jalon.

pub mod ast;
pub mod dataset;
pub mod datastep;
pub mod error;
pub mod executor;
pub mod formats;
pub mod graphics;
pub mod lexer;
pub mod library;
pub mod listing;
pub mod log;
pub mod macros;
pub mod missing;
pub mod ods_graphics;
pub mod output;
pub mod parser;
pub mod preprocess;
pub mod procs;
pub mod session;
pub mod source;
pub mod sql;
pub mod stat;
#[cfg(test)]
pub mod testkit;
pub mod token;
pub mod value;

use session::Session;
use source::SourceFile;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

#[derive(Default)]
pub struct RunOptions {
    /// Répertoire WORK ; None = répertoire temporaire détruit en fin de
    /// session.
    pub work_dir: Option<PathBuf>,
    /// Base de résolution des chemins LIBNAME relatifs (défaut : cwd).
    pub base_dir: Option<PathBuf>,
    /// Fige les temps (et toute sortie non reproductible) pour les
    /// snapshots de test.
    pub deterministic: bool,
    /// Active le fast-path vectorisé OPTIONNEL des étapes DATA simples
    /// (cf. `datastep::fastpath`). OFF par défaut.
    pub vectorize: bool,
}

pub struct RunOutcome {
    pub log: String,
    pub listing: String,
    /// Without an explicit session request: 0 = clean, 1 = warnings, 2 = errors.
    /// An explicit request supplies the return code, with a minimum of 1 when
    /// errors were counted (and 0 otherwise).
    pub exit_code: i32,
}

/// Exécute un programme SAS complet et rend log + listing.
pub fn run(source_text: &str, opts: RunOptions) -> RunOutcome {
    let base_dir = opts
        .base_dir
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    let base_dir = if base_dir.is_absolute() {
        base_dir
    } else {
        std::env::current_dir().unwrap_or_default().join(base_dir)
    };
    let created = catch_unwind(AssertUnwindSafe(|| {
        Session::new(opts.work_dir, base_dir, opts.deterministic)
    }));
    let mut session = match created {
        Ok(Ok(s)) => s,
        other => {
            let e = match other {
                Ok(Err(e)) => e.to_string(),
                Err(payload) => internal_error(payload.as_ref()),
                Ok(Ok(_)) => unreachable!(),
            };
            return RunOutcome {
                log: format!("ERROR: {e}\n"),
                listing: String::new(),
                exit_code: 2,
            };
        }
    };
    session.vectorize = opts.vectorize;

    run_in_session(session, |session| {
        let src = SourceFile::new(source_text.to_string());
        executor::run_program(&src, session);
    })
}

fn internal_error(payload: &(dyn std::any::Any + Send)) -> String {
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("unknown panic payload");
    format!("internal error: {message}")
}

// Keep ownership of the session outside the unwind boundary so partial log and
// output survive an executor panic. Finalization has its own boundary because
// destination rendering can panic too.
fn run_in_session(mut session: Session, execute: impl FnOnce(&mut Session)) -> RunOutcome {
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| execute(&mut session))) {
        session.log.error(&internal_error(payload.as_ref()));
    }
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| session.finish_destination())) {
        session.log.error(&internal_error(payload.as_ref()));
    }
    let listing = session.take_completed_listing();
    let exit_code = if let Some(requested) = session.take_requested_exit_code() {
        requested.max(i32::from(session.log.errors > 0))
    } else if session.log.errors > 0 {
        2
    } else if session.log.warnings > 0 {
        1
    } else {
        0
    };
    RunOutcome {
        log: session.log.into_string(),
        listing,
        exit_code,
    }
}

#[cfg(test)]
mod run_failure_tests {
    use super::*;

    #[test]
    fn exit_code_requested_return_reaches_outcome() {
        // SAS %ABORT RETURN n passes n to the host; J02-P10 supplies the
        // session channel that the executor can use for that return code.
        // https://support.sas.com/kb/23/addl/fusion23211_1_abort.html
        for counted_error in [false, true] {
            let session = Session::new(None, std::env::temp_dir(), true).unwrap();
            let outcome = run_in_session(session, |session| {
                session.request_exit_code(8);
                if counted_error {
                    session.log.error("counted failure");
                }
            });
            assert_eq!(outcome.exit_code, 8);
            assert_eq!(
                outcome.log,
                if counted_error {
                    "ERROR: counted failure\n"
                } else {
                    ""
                }
            );
        }
    }

    #[test]
    fn exit_code_requested_zero_preserves_counted_failure() {
        let session = Session::new(None, std::env::temp_dir(), true).unwrap();
        let outcome = run_in_session(session, |session| {
            session.request_exit_code(0);
            session.log.error("counted failure");
        });
        assert_eq!(outcome.exit_code, 1);
        assert_eq!(outcome.log, "ERROR: counted failure\n");
    }

    #[test]
    fn exit_code_requested_zero_without_errors_succeeds() {
        let session = Session::new(None, std::env::temp_dir(), true).unwrap();
        let outcome = run_in_session(session, |session| session.request_exit_code(0));
        assert_eq!(outcome.exit_code, 0);
        assert!(outcome.log.is_empty());
    }

    #[test]
    fn exit_code_requested_zero_preserves_finalization_failure() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::new(None, dir.path().to_path_buf(), true).unwrap();
        let outcome = run_in_session(session, |session| {
            session.request_exit_code(0);
            // Writing an HTML file to an existing directory must fail.
            session.open_destination(
                "HTML",
                Box::new(output::HtmlDestination::with_file(
                    96,
                    dir.path().to_path_buf(),
                )),
            );
            session.listing.write_line("partial output");
        });
        assert_eq!(outcome.exit_code, 1);
        assert!(outcome.log.contains("ERROR: Could not write"));
    }

    #[test]
    fn exit_code_without_request_keeps_log_classification() {
        for (warnings, errors, expected) in [(0, 0, 0), (1, 0, 1), (0, 1, 2), (1, 1, 2)] {
            let session = Session::new(None, std::env::temp_dir(), true).unwrap();
            let outcome = run_in_session(session, |session| {
                for _ in 0..warnings {
                    session.log.warning("counted warning");
                }
                for _ in 0..errors {
                    session.log.error("counted failure");
                }
            });
            assert_eq!(outcome.exit_code, expected);
        }
    }

    #[test]
    fn panic_preserves_partial_log_and_listing() {
        let session = Session::new(None, std::env::temp_dir(), true).unwrap();
        let outcome = run_in_session(session, |session| {
            session.log.note("before panic");
            session.listing.write_line("partial listing");
            panic!("test execution failure");
        });
        assert_eq!(outcome.exit_code, 2);
        assert!(outcome.log.contains("NOTE: before panic\n"));
        assert!(
            outcome
                .log
                .contains("ERROR: internal error: test execution failure\n")
        );
        assert_eq!(outcome.listing, "partial listing\n");
    }

    #[test]
    fn panic_still_finalizes_open_ods_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.html");
        let session = Session::new(None, dir.path().to_path_buf(), true).unwrap();
        let outcome = run_in_session(session, |session| {
            session.open_destination(
                "HTML",
                Box::new(output::HtmlDestination::with_file(96, path.clone())),
            );
            session.listing.write_line("partial output");
            std::panic::panic_any(17);
        });
        assert_eq!(outcome.exit_code, 2);
        assert!(
            outcome
                .log
                .contains("ERROR: internal error: unknown panic payload")
        );
        assert!(
            std::fs::read_to_string(path)
                .unwrap()
                .contains("partial output")
        );
    }
}
