//! Registre des PROCs : chaque PROC possède son propre parser (grammaire
//! contextuelle SAS) et son exécuteur.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `parse_proc(name, ts)` est appelé par `parser::next_block()` APRÈS
//! consommation de `proc <name>` ; le sous-parser consomme tout jusqu'à
//! `run;` (ou `quit;` pour les run-group procs : SQL, DATASETS).
//! PROC inconnue → ERROR "Procedure XXX not found." + récupération.
//!
//! `execute_proc` dispatche, entoure d'un `StepTimer`, et écrit la NOTE
//! de fin "NOTE: PROCEDURE XXX used (Total process time): ...".
//!
//! Convention commune : `data=` absent → `session.last_dataset` (_LAST_) ;
//! aucun dataset créé dans la session → ERROR (comme SAS _LAST_ vide).

pub mod append;
pub mod contents;
pub mod datasets;
pub mod format;
pub mod freq;
pub mod means;
pub mod print;
pub mod sort;
pub mod transpose;
pub mod univariate;

use crate::error::{Result, SasError};
use crate::log::StepTimer;
use crate::parser::StatementStream;
use crate::session::Session;

pub enum ProcAst {
    Print(print::PrintAst),
    Sort(sort::SortAst),
    Means(means::MeansAst),
    Freq(freq::FreqAst),
    Transpose(transpose::TransposeAst),
    Univariate(univariate::UnivariateAst),
    Contents(contents::ContentsAst),
    Datasets(datasets::DatasetsAst),
    Append(append::AppendAst),
    Format(format::FormatAst),
    Sql(crate::sql::ast::SqlProgram),
}

/// Parse a PROC block. Called AFTER `proc <name>` has been consumed.
/// Dispatches to the appropriate sub-parser by proc name.
///
/// - PRINT (M1): fully implemented.
/// - Known procs not yet implemented: skip to step boundary, return Err.
/// - Unknown proc name: return Err "Procedure XXX not found."
pub fn parse_proc(name: &str, ts: &mut StatementStream) -> Result<ProcAst> {
    match name.to_ascii_lowercase().as_str() {
        "print" => {
            let ast = print::parse(ts)?;
            Ok(ProcAst::Print(ast))
        }
        "sort" => {
            let ast = sort::parse(ts)?;
            Ok(ProcAst::Sort(ast))
        }
        "format" => {
            let ast = format::parse(ts)?;
            Ok(ProcAst::Format(ast))
        }
        // Procs connues du périmètre, pas encore implémentées : consommer
        // le bloc pour rester synchronisé, puis ERROR. Finir d'abord le
        // statement courant (on est au MILIEU du statement PROC : un ident
        // comme `data` dans `proc sort data=x;` serait sinon pris pour une
        // frontière par skip_to_step_boundary).
        "means" | "summary" | "freq" | "transpose" | "univariate" | "contents"
        | "datasets" | "append" | "sql" => {
            ts.skip_to_semi();
            ts.skip_to_step_boundary();
            Err(SasError::runtime(format!(
                "Procedure {} is not yet implemented.",
                name.to_uppercase()
            )))
        }
        _ => {
            // Proc inconnue : finir le statement courant ; le caller
            // (parser::parse_block) saute ensuite jusqu'à la frontière.
            ts.skip_to_semi();
            Err(SasError::runtime(format!(
                "Procedure {} not found.",
                name.to_uppercase()
            )))
        }
    }
}

/// Execute a previously parsed PROC AST. Wraps with a StepTimer and writes
/// the "NOTE: PROCEDURE XXX used (Total process time):" note.
pub fn execute_proc(name: &str, ast: &ProcAst, session: &mut Session) -> Result<()> {
    let timer = StepTimer::start();

    let result = match ast {
        ProcAst::Print(a) => print::execute(a, session),
        ProcAst::Sort(a) => sort::execute(a, session),
        ProcAst::Means(a) => means::execute(a, session),
        ProcAst::Freq(a) => freq::execute(a, session),
        ProcAst::Transpose(a) => transpose::execute(a, session),
        ProcAst::Univariate(a) => univariate::execute(a, session),
        ProcAst::Contents(a) => contents::execute(a, session),
        ProcAst::Datasets(a) => datasets::execute(a, session),
        ProcAst::Append(a) => append::execute(a, session),
        ProcAst::Format(a) => format::execute(a, session),
        ProcAst::Sql(a) => crate::sql::execute(a, session),
    };

    // Write timing NOTE even on success (SAS always prints this).
    // On error the caller may still want the timing, but we follow SAS: only
    // write it on success.
    if result.is_ok() {
        session
            .log
            .step_used(&format!("PROCEDURE {}", name.to_uppercase()), &timer);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::session::Session;
    use crate::source::SourceFile;
    use crate::value::VarType;
    use polars::prelude::*;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn parse_proc_src(src: &str) -> Result<(String, ProcAst)> {
        let source = SourceFile::new(src);
        let mut ts = crate::parser::StatementStream::new(&source).unwrap();
        // consume "proc"
        ts.next();
        // consume proc name
        let name = ts.next().ident().unwrap().to_string();
        let ast = parse_proc(&name, &mut ts)?;
        Ok((name, ast))
    }

    // --- parse tests ---

    #[test]
    fn parse_proc_print_minimal() {
        let (name, ast) = parse_proc_src("proc print; run;").unwrap();
        assert_eq!(name.to_ascii_lowercase(), "print");
        assert!(matches!(ast, ProcAst::Print(_)));
    }

    #[test]
    fn parse_proc_print_with_data_and_noobs() {
        let (_, ast) = parse_proc_src("proc print data=work.foo noobs; run;").unwrap();
        if let ProcAst::Print(a) = ast {
            assert!(a.noobs);
            assert_eq!(a.data.as_ref().unwrap().name, "foo");
        } else {
            panic!("expected ProcAst::Print");
        }
    }

    #[test]
    fn parse_proc_print_var_statement() {
        let (_, ast) = parse_proc_src("proc print data=work.x; var a b; run;").unwrap();
        if let ProcAst::Print(a) = ast {
            assert_eq!(a.vars, Some(vec!["a".to_string(), "b".to_string()]));
        } else {
            panic!("expected ProcAst::Print");
        }
    }

    #[test]
    fn parse_known_unimplemented_proc_errors_with_correct_message() {
        for proc_name in &["means", "freq", "transpose", "univariate",
                           "contents", "datasets", "append", "sql"] {
            let src = format!("proc {}; run;", proc_name);
            let source = SourceFile::new(&src);
            let mut ts = crate::parser::StatementStream::new(&source).unwrap();
            ts.next(); // "proc"
            ts.next(); // proc_name
            let result = parse_proc(proc_name, &mut ts);
            assert!(result.is_err(), "proc {proc_name} should error");
            let msg = result.err().unwrap().to_string();
            let expected_fragment = "is not yet implemented.";
            assert!(
                msg.contains(expected_fragment),
                "proc {proc_name}: expected '{}' in '{}'",
                expected_fragment,
                msg
            );
            assert!(
                msg.contains(&proc_name.to_ascii_uppercase()),
                "proc {proc_name}: message should contain uppercase name, got: {msg}"
            );
        }
    }

    #[test]
    fn parse_unknown_proc_errors_with_not_found_message() {
        let source = SourceFile::new("proc frobnicate; run;");
        let mut ts = crate::parser::StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "frobnicate"
        let result = parse_proc("frobnicate", &mut ts);
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(
            msg.contains("not found."),
            "expected 'not found.' in: {msg}"
        );
        assert!(
            msg.contains("FROBNICATE"),
            "expected uppercase proc name in: {msg}"
        );
    }

    #[test]
    fn unknown_proc_message_does_not_say_not_yet_implemented() {
        let source = SourceFile::new("proc xyzzy; run;");
        let mut ts = crate::parser::StatementStream::new(&source).unwrap();
        ts.next();
        ts.next();
        let result = parse_proc("xyzzy", &mut ts);
        let msg = result.err().unwrap().to_string();
        assert!(
            !msg.contains("not yet implemented"),
            "unknown proc should say 'not found', not 'not yet implemented': {msg}"
        );
    }

    #[test]
    fn parse_proc_format_succeeds() {
        let (name, ast) = parse_proc_src("proc format; value f 1='x'; run;").unwrap();
        assert_eq!(name.to_ascii_lowercase(), "format");
        assert!(matches!(ast, ProcAst::Format(_)));
    }

    #[test]
    fn execute_proc_format_registers_and_notes() {
        use crate::formats::FormatSpec;
        use crate::value::Value;

        let mut session = make_session();
        let (_, ast) = parse_proc_src("proc format; value sexfmt 1='Male' 2='Female'; run;").unwrap();
        execute_proc("format", &ast, &mut session).unwrap();

        let spec = FormatSpec::parse("SEXFMT.").unwrap();
        let result = session.format_catalog.format(&Value::Num(1.0), &spec);
        assert!(result.contains("Male"), "result: {result}");

        let log = session.log.into_string();
        assert!(log.contains("Format SEXFMT has been output."), "log: {log}");
        assert!(log.contains("PROCEDURE FORMAT used"), "log: {log}");
    }

    // --- execute_proc tests ---

    fn write_test_dataset(session: &mut Session) {
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0],
            "y" => ["a", "b", "c"]
        ]
        .unwrap();
        let vars = vec![
            VarMeta {
                name: "x".to_string(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            },
            VarMeta {
                name: "y".to_string(),
                ty: VarType::Char,
                length: 1,
                format: None,
                label: None,
            },
        ];
        let ds = SasDataset { df, vars };
        session.libs.get("WORK").unwrap().write("TEST", &ds).unwrap();
        session.last_dataset = Some("WORK.TEST".to_string());
    }

    #[test]
    fn execute_proc_print_writes_timing_note() {
        let mut session = make_session();
        write_test_dataset(&mut session);

        let ast = ProcAst::Print(print::PrintAst {
            data: Some(crate::ast::DatasetRef {
                libref: Some("WORK".into()),
                name: "TEST".into(),
            }),
            vars: None,
            noobs: false,
            label: false,
        });

        execute_proc("print", &ast, &mut session).unwrap();

        let log = session.log.into_string();
        assert!(
            log.contains("PROCEDURE PRINT used (Total process time):"),
            "log: {log}"
        );
        assert!(log.contains("real time"), "log: {log}");
        assert!(log.contains("cpu time"), "log: {log}");
    }

    #[test]
    fn execute_proc_print_full_pipeline() {
        let mut session = make_session();
        write_test_dataset(&mut session);

        let ast = ProcAst::Print(print::PrintAst {
            data: Some(crate::ast::DatasetRef {
                libref: Some("WORK".into()),
                name: "TEST".into(),
            }),
            vars: None,
            noobs: false,
            label: false,
        });

        execute_proc("print", &ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("The SAS System") || listing.contains("SAS"), "listing: {listing}");
        assert!(listing.contains("Obs"), "listing: {listing}");

        let log = session.log.into_string();
        assert!(
            log.contains("There were 3 observations read from the data set WORK.TEST"),
            "log: {log}"
        );
    }
}
