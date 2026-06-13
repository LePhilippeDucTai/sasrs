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
    // M11.1 : l'expansion macro est désormais pilotée par l'executor (et non
    // plus par `lib.rs`), avec l'état dans `Session::macro_engine`. Pour CETTE
    // unité, on expanse le source ENTIER une seule fois, en tête de boucle.
    // C'est la couture : la table macro vit dans la `Session` et l'expansion
    // est conduite ici. Le découpage en segments bruts (`RawSegmenter`) et
    // l'expansion interfoliée bloc-par-bloc — requis par `CALL SYMPUT` (M11.5)
    // — sont DÉFÉRÉS afin de préserver la garantie byte-identical : sous le
    // build par défaut `expand_open_code` est l'identité stricte, donc `src`
    // est inchangé et le lexing / l'écho de n° de ligne restent identiques.
    let expanded = session.macro_engine.expand_open_code(&src.text);
    let owned_src;
    let src: &SourceFile = if expanded == src.text {
        src
    } else {
        owned_src = SourceFile::new(expanded);
        &owned_src
    };

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
            // Sous --deterministic, le chemin affiché est celui du source
            // (un chemin absolu de tempdir casserait les snapshots).
            let shown = if session.deterministic {
                path.clone()
            } else {
                abs.display().to_string()
            };
            match session.libs.assign(libref, abs) {
                Ok(()) => session.log.note(&format!(
                    "Libref {} was successfully assigned as follows:\n      Engine:        PARQUET\n      Physical Name: {shown}",
                    libref.to_uppercase()
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
                } else if name.eq_ignore_ascii_case("obs") {
                    // OBS=MAX (or unset) → no limit; OBS=n → process up to obs n.
                    match value.as_deref() {
                        Some(v) if v.eq_ignore_ascii_case("max") => session.options.obs = None,
                        Some(v) => match v.parse::<usize>() {
                            Ok(n) => session.options.obs = Some(n),
                            Err(_) => session.log.error(&format!(
                                "The value {v} is not a valid OBS value."
                            )),
                        },
                        None => session.options.obs = None,
                    }
                } else if name.eq_ignore_ascii_case("firstobs") {
                    // FIRSTOBS=MAX is unusual; treat any non-number as an error.
                    match value.as_deref() {
                        Some(v) if v.eq_ignore_ascii_case("max") => {
                            session.options.firstobs = usize::MAX
                        }
                        Some(v) => match v.parse::<usize>() {
                            Ok(n) if n >= 1 => session.options.firstobs = n,
                            _ => session.log.error(&format!(
                                "The value {v} is not a valid FIRSTOBS value."
                            )),
                        },
                        None => {}
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
                vectorize: false,
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
    fn options_firstobs_and_obs_window_input() {
        // Build a 5-row data set, then read it with FIRSTOBS=2 OBS=4 → obs 2..4
        // (3 observations). The window applies to the physical SET input.
        let out = run_det(
            "data a; do i = 1 to 5; output; end; run;\n\
             options firstobs=2 obs=4;\n\
             data b; set a; run;\n",
        );
        assert_eq!(out.exit_code, 0, "{}", out.log);
        assert!(
            out.log
                .contains("The data set WORK.A has 5 observations and 1 variables."),
            "{}",
            out.log
        );
        assert!(
            out.log
                .contains("The data set WORK.B has 3 observations and 1 variables."),
            "{}",
            out.log
        );
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
                vectorize: false,
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

    /// M11.1 : l'expansion macro est conduite par l'executor (état dans
    /// `Session::macro_engine`). Un programme avec `%let`/`&var` doit produire
    /// EXACTEMENT le même résultat que son équivalent sans macro.
    #[cfg(feature = "macros")]
    #[test]
    fn macro_let_ref_runs_through_executor() {
        let with_macro = run_det(
            "%let lib=work; data &lib..a; x=1; run; proc print data=&lib..a; run;",
        );
        let without_macro = run_det(
            "data work.a; x=1; run; proc print data=work.a; run;",
        );
        assert_eq!(with_macro.exit_code, 0, "log was:\n{}", with_macro.log);
        assert_eq!(
            with_macro.listing, without_macro.listing,
            "macro listing differs:\nMACRO:\n{}\nPLAIN:\n{}",
            with_macro.listing, without_macro.listing
        );
        // Les NOTEs de l'étape DATA / PROC doivent correspondre.
        assert!(with_macro
            .log
            .contains("The data set WORK.A has 1 observations and 1 variables."));
        assert!(with_macro
            .log
            .contains("There were 1 observations read from the data set WORK.A."));
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
