//! Boucle d'exécution principale : tire les blocs un à un et les exécute.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! ```text
//! run_program(src, session):
//!   stream = StatementStream::new(src)            // erreur lexer → ERROR + stop
//!   tant que Some((bloc, span)) = stream.next_block():
//!     session.log.echo_source(src.lines_of_span(span))   // AVANT exécution
//!     selon bloc :
//!       Err(e)            → log.error(e) ; continuer (récupération déjà
//!                            faite par le stream)
//!       Global(stmt)      → exécuter ici même :
//!           Libname    : résoudre le chemin (relatif → session.base_dir),
//!                        libs.assign ; NOTE "Libref XXX was successfully
//!                        assigned as follows: ... Physical Name: ..."
//!           LibnameClear : libs.clear + NOTE
//!           Title      : session.listing.title = texte (M1 : title1)
//!           Options    : appliquer ls= ; autres → WARNING "not supported"
//!       DataStep(ast)     → timer = StepTimer::start()
//!                           datastep::compile(&ast, session) ;
//!                             Err → ERROR + NOTE "The SAS System stopped
//!                                   processing this step because of errors."
//!                           Ok  → datastep::exec::execute → NOTEs lues/écrites
//!                           log.step_used("DATA statement", &timer)
//!       Proc{name, ast}   → procs::execute_proc (timing inclus)
//!       Empty             → rien
//! ```
//! Aucune erreur n'arrête la session (style batch SAS) sauf l'échec du
//! lexer. Le code retour est dérivé des compteurs du LogWriter par
//! lib.rs (0 propre / 1 warnings / 2 erreurs).

use crate::ast::GlobalStmt;
use crate::datastep;
use crate::error::Result;
use crate::log::StepTimer;
use crate::parser::{Block, StatementStream};
use crate::procs;
use crate::session::Session;
use crate::source::SourceFile;
use std::path::PathBuf;

pub fn run_program(src: &SourceFile, session: &mut Session) -> Result<()> {
    let mut stream = StatementStream::new(src)?;
    while let Some((block, span)) = stream.next_block() {
        let lines = src.lines_of_span(span);
        let line_texts: Vec<&str> = lines.iter().map(|(_, text)| *text).collect();
        session.log.echo_source(&line_texts);

        match block {
            Err(e) => {
                // La récupération de flux est déjà faite par le stream.
                session.log.error(&e.to_string());
            }
            Ok(Block::Empty) => {}
            Ok(Block::Global(stmt)) => exec_global(&stmt, session),
            Ok(Block::DataStep(ast)) => exec_data_step(&ast, session),
            Ok(Block::Proc { name, ast }) => {
                if let Err(e) = procs::execute_proc(&name, &ast, session) {
                    session.log.error(&e.to_string());
                    session
                        .log
                        .note("The SAS System stopped processing this step because of errors.");
                }
            }
        }
    }
    Ok(())
}

fn exec_global(stmt: &GlobalStmt, session: &mut Session) {
    match stmt {
        GlobalStmt::Libname { libref, path } => {
            let p = PathBuf::from(path);
            let abs = if p.is_absolute() {
                p
            } else {
                session.base_dir.join(p)
            };
            match session.libs.assign(libref, abs.clone()) {
                Ok(()) => session.log.note(&format!(
                    "Libref {} was successfully assigned as follows:\n      Engine:        PARQUET\n      Physical Name: {}",
                    libref.to_uppercase(),
                    abs.display()
                )),
                Err(e) => session.log.error(&e.to_string()),
            }
        }
        GlobalStmt::LibnameClear { libref } => match session.libs.clear(libref) {
            Ok(()) => session.log.note(&format!(
                "Libref {} has been deassigned.",
                libref.to_uppercase()
            )),
            Err(e) => session.log.error(&e.to_string()),
        },
        GlobalStmt::Title { n, text } => {
            // M1 : seul TITLE1 est rendu par le listing ; les autres niveaux
            // sont acceptés sans effet.
            if *n == 1 {
                session.listing.title = text.clone();
            }
        }
        GlobalStmt::Options(opts) => {
            for (name, value) in opts {
                if name.eq_ignore_ascii_case("ls") || name.eq_ignore_ascii_case("linesize") {
                    match value.as_deref().and_then(|v| v.parse::<usize>().ok()) {
                        Some(v) if (40..=256).contains(&v) => {
                            session.options.ls = v;
                            session.listing.ls = v;
                        }
                        _ => session.log.error(&format!(
                            "The value {} is not a valid LINESIZE value (40..256).",
                            value.as_deref().unwrap_or("")
                        )),
                    }
                } else {
                    session.log.warning(&format!(
                        "Option {} is not yet supported.",
                        name.to_uppercase()
                    ));
                }
            }
        }
    }
}

fn exec_data_step(ast: &crate::ast::DataStepAst, session: &mut Session) {
    let timer = StepTimer::start();
    let compiled = datastep::compile(ast, session);
    match compiled {
        Err(e) => {
            session.log.error(&e.to_string());
            session
                .log
                .note("The SAS System stopped processing this step because of errors.");
        }
        Ok(prog) => {
            if let Err(e) = datastep::exec::execute(prog, session) {
                session.log.error(&e.to_string());
                session
                    .log
                    .note("The SAS System stopped processing this step because of errors.");
            }
        }
    }
    // SAS imprime la NOTE de timing même quand l'étape a échoué.
    session.log.step_used("DATA statement", &timer);
}

#[cfg(test)]
mod tests {
    use crate::{run, RunOptions};

    fn run_det(src: &str) -> crate::RunOutcome {
        run(
            src,
            RunOptions {
                work_dir: None,
                base_dir: None,
                deterministic: true,
            },
        )
    }

    #[test]
    fn end_to_end_data_then_print() {
        let out = run_det(
            "title 'Essai';\n\
             data a; x = 1; y = 'ab'; run;\n\
             proc print data=a; run;\n",
        );
        assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
        // Écho numéroté du source.
        assert!(out.log.contains("1     title 'Essai';"), "{}", out.log);
        assert!(out.log.contains("data a; x = 1; y = 'ab'; run;"));
        // NOTEs de l'étape DATA.
        assert!(out
            .log
            .contains("The data set WORK.A has 1 observations and 2 variables."));
        assert!(out.log.contains("DATA statement used (Total process time):"));
        assert!(out.log.contains("real time           0.00 seconds"));
        // PROC PRINT : timing + listing avec titre.
        assert!(out.log.contains("PROCEDURE PRINT used (Total process time):"));
        assert!(out.listing.contains("Essai"), "{}", out.listing);
        assert!(out.listing.contains("Obs"));
    }

    #[test]
    fn error_recovery_continues_session() {
        let out = run_det(
            "frobnicate;\n\
             data a; x = 1; run;\n",
        );
        assert_eq!(out.exit_code, 2);
        assert!(out.log.contains("ERROR: Statement 'FROBNICATE' is not valid"));
        // L'étape suivante s'exécute malgré l'erreur.
        assert!(out
            .log
            .contains("The data set WORK.A has 1 observations and 1 variables."));
    }

    #[test]
    fn unknown_proc_errors_and_continues() {
        let out = run_det(
            "proc nosuchproc data=a; run;\n\
             data b; x = 1; run;\n",
        );
        assert_eq!(out.exit_code, 2);
        assert!(out.log.contains("ERROR: Procedure NOSUCHPROC not found."));
        assert!(out
            .log
            .contains("The data set WORK.B has 1 observations and 1 variables."));
    }

    #[test]
    fn missing_input_dataset_stops_step_with_notes() {
        let out = run_det("data a; set nosuch; run;");
        assert_eq!(out.exit_code, 2);
        assert!(out.log.contains("ERROR: File WORK.NOSUCH.DATA does not exist."));
        assert!(out
            .log
            .contains("The SAS System stopped processing this step because of errors."));
        // Timing imprimé malgré l'erreur.
        assert!(out.log.contains("DATA statement used (Total process time):"));
    }

    #[test]
    fn options_ls_applied_and_unknown_option_warns() {
        let out = run_det("options ls=120 nocenter;");
        assert_eq!(out.exit_code, 1, "{}", out.log);
        assert!(out.log.contains("WARNING: Option NOCENTER is not yet supported."));
    }

    #[test]
    fn libname_relative_resolution_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("dat")).unwrap();
        let out = run(
            "libname mylib 'dat';\nlibname mylib clear;\n",
            RunOptions {
                work_dir: None,
                base_dir: Some(dir.path().to_path_buf()),
                deterministic: true,
            },
        );
        assert_eq!(out.exit_code, 0, "{}", out.log);
        assert!(out
            .log
            .contains("Libref MYLIB was successfully assigned as follows:"));
        assert!(out.log.contains("Physical Name:"));
        assert!(out.log.contains("Libref MYLIB has been deassigned."));
    }

    #[test]
    fn data_null_no_listing_no_dataset_note() {
        let out = run_det("data _null_; x = 1; run;");
        assert_eq!(out.exit_code, 0);
        assert!(!out.log.contains("has 1 observations"));
        assert!(out.listing.is_empty());
    }

    #[test]
    fn proc_print_uses_last_dataset() {
        let out = run_det(
            "data zz; v = 3.5; run;\n\
             proc print; run;\n",
        );
        assert_eq!(out.exit_code, 0, "{}", out.log);
        assert!(out
            .log
            .contains("There were 1 observations read from the data set WORK.ZZ."));
        assert!(out.listing.contains("3.5"));
    }
}
