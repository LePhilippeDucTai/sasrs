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

use crate::ast::{GlobalStmt, OdsAction};
use crate::datastep;
use crate::error::Result;
use crate::log::StepTimer;
use crate::parser::{Block, StatementStream};
use crate::procs;
use crate::session::Session;
use crate::source::SourceFile;
use std::path::PathBuf;

/// M11.5/M11.7 : expansion macro INTERFOLIÉE segment par segment (toujours
/// active désormais — il n'y a plus de feature `macros`).
///
/// On découpe le source ORIGINAL en segments bruts (`RawSegmenter`, coupe sur
/// `run;`/`quit;` de niveau supérieur). Pour CHAQUE segment, dans l'ordre :
/// 1. écho des lignes ORIGINALES du segment (numérotation préservée — cf.
///    divergence ci-dessous) ;
/// 2. `expand_open_code` du texte brut du segment avec l'état VIVANT de
///    l'engine (les `%let`/symput des segments antérieurs sont donc visibles) ;
/// 3. lexing/parsing/exécution du texte expansé via un `StatementStream`
///    transitoire.
///
/// Comme le drain de `CALL SYMPUT` a lieu à la fin de l'étape (donc à la fin
/// du segment qui contient le `run;`), un `&var` du segment SUIVANT voit bien
/// la valeur posée par le symput — c'est l'objectif de M11.5.
///
/// # Écho de numéros de ligne — préservation
/// L'écho reste BLOC PAR BLOC, comme le build par défaut : pour chaque bloc
/// du segment expansé, on écho­te les lignes de son span via
/// `seg_src.lines_of_span(span)`. Le compteur de lignes du `LogWriter`
/// (`src_line`) avance naturellement d'un segment à l'autre. Lorsqu'un
/// segment n'a subi AUCUNE expansion (cas des fixtures sans macro :
/// `expand_open_code` est l'identité), le texte du segment est
/// caractère-pour-caractère la tranche correspondante du source original,
/// donc l'écho et la numérotation sont IDENTIQUES au chemin mono-source de
/// M11.1. La seule divergence POSSIBLE concerne un segment dont l'expansion
/// macro change le nombre/contenu des lignes : on écho­te alors le texte
/// EXPANSÉ de ce segment (pas l'original). C'est sans incidence sur les
/// fixtures de snapshot (aucune n'emploie de macro), et sans fixture dédiée
/// pour ce cas.
pub fn run_program(src: &SourceFile, session: &mut Session) -> Result<()> {
    use crate::preprocess::RawSegmenter;

    let orig = src;
    let mut seg = RawSegmenter::new(&orig.text);
    while let Some((start, end)) = seg.next_segment() {
        let raw = &orig.text[start..end];
        // Expansion avec l'état vivant (visibilité des symput antérieurs).
        let expanded = session.macro_engine.expand_open_code(raw);
        // M19.3 — relayer au log les lignes produites par l'expansion (écho
        // MPRINT/MLOGIC/SYMBOLGEN et sortie de `%put`), AVANT d'exécuter le
        // segment expansé (elles précèdent le code dans le log SAS).
        for line in session.macro_engine.take_pending_log_lines() {
            session.log.put_line(&line);
        }
        // M19.3 — `%call execute(...)` côté macro : mettre en file pour exécution
        // après le segment courant (même file que le CALL EXECUTE des étapes).
        let macro_ce = session.macro_engine.take_pending_call_execute();
        session.call_execute_queue.extend(macro_ce);
        let seg_src = SourceFile::new(expanded);
        let mut stream = match StatementStream::new(&seg_src) {
            Ok(s) => s,
            Err(e) => {
                session.log.error(&e.to_string());
                continue;
            }
        };
        while let Some((block, span)) = stream.next_block() {
            let lines = seg_src.lines_of_span(span);
            let line_texts: Vec<&str> = lines.iter().map(|(_, text)| *text).collect();
            session.log.echo_source(&line_texts);
            run_one_block(block, session);
        }
        // M19.3 — un `%call execute(...)` en code ouvert (hors étape DATA) doit
        // tout de même être rejoué après le segment qui l'a produit. Les DATA
        // steps drainent déjà la file à leur RUN ; ce drain couvre le code
        // ouvert pur (segment sans étape DATA).
        run_call_execute_queue(session);
    }
    Ok(())
}

/// Exécute UN bloc déjà lexé/parsé (commun aux deux builds). L'écho de source
/// est fait par l'appelant (différemment selon le build).
fn run_one_block(block: Result<Block>, session: &mut Session) {
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

fn exec_global(stmt: &GlobalStmt, session: &mut Session) {
    match stmt {
        GlobalStmt::Libname { libref, engine, path } => {
            // M13 : routage cloud. Quand la feature `s3` est active et que le
            // chemin commence par `s3://`, on enregistre une `S3Library`
            // (bucket/prefix) au lieu d'une `DirLibrary`. Le chemin affiché
            // reste l'URI tel quel (pas de résolution relative, pas d'absolu de
            // tempdir → snapshots stables). Sous le build par défaut ce bloc
            // n'est pas compilé : un chemin `s3://...` est traité comme
            // aujourd'hui (résolu comme un répertoire local, qui n'existe pas →
            // erreur runtime habituelle).
            // M13 : routage cloud s3://.
            #[cfg(feature = "s3")]
            if path.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("s3://")) {
                match session.libs.assign_uri(libref, path) {
                    Ok(()) => session.log.note(&format!(
                        "Libref {} was successfully assigned as follows:\n      Engine:        PARQUET\n      Physical Name: {path}",
                        libref.to_uppercase()
                    )),
                    Err(e) => session.log.error(&e.to_string()),
                }
                return;
            }

            // M14.4 : XLSX engine deferral — emit an error and return.
            match engine.as_deref().map(|e| e.to_ascii_uppercase()).as_deref() {
                Some("XLSX") | Some("EXCEL") | Some("XLS") => {
                    session.log.error(
                        "LIBNAME engine XLSX is not yet implemented in this build \
                         (the calamine/rust_xlsxwriter crates are not available).",
                    );
                    return;
                }
                _ => {}
            }

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

            // M14.4 : branch on engine.
            match engine.as_deref().map(|e| e.to_ascii_uppercase()).as_deref() {
                Some("CSV") => {
                    match session.libs.assign_csv(libref, abs) {
                        Ok(()) => session.log.note(&format!(
                            "Libref {} was successfully assigned as follows:\n      Engine:        CSV\n      Physical Name: {shown}",
                            libref.to_uppercase()
                        )),
                        Err(e) => session.log.error(&e.to_string()),
                    }
                }
                // None | Some("PARQUET") | Some("BASE") | Some("V9") | _ → parquet path
                _ => {
                    match session.libs.assign(libref, abs) {
                        Ok(()) => session.log.note(&format!(
                            "Libref {} was successfully assigned as follows:\n      Engine:        PARQUET\n      Physical Name: {shown}",
                            libref.to_uppercase()
                        )),
                        Err(e) => session.log.error(&e.to_string()),
                    }
                }
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
                session.listing.set_title(text.clone());
            }
        }
        GlobalStmt::Options(opts) => {
            for (name, value) in opts {
                if name.eq_ignore_ascii_case("ls") || name.eq_ignore_ascii_case("linesize") {
                    match value.as_deref().and_then(|v| v.parse::<usize>().ok()) {
                        Some(v) if (40..=256).contains(&v) => {
                            session.options.ls = v;
                            session.listing.set_ls(v);
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
                } else if name.eq_ignore_ascii_case("sasautos") {
                    // M19.2 — SASAUTOS= fixe le(s) répertoire(s) de bibliothèques
                    // autocall. On accepte une valeur simple (un répertoire) :
                    //   OPTIONS SASAUTOS='dir';  ou  OPTIONS SASAUTOS=dir;
                    // Les guillemets éventuels sont retirés par le parser
                    // global ; le chemin relatif est résolu contre `base_dir`
                    // (même base que %include/LIBNAME). La forme liste
                    // `(d1 d2)` n'est pas gérée ici (différée).
                    match value.as_deref() {
                        Some(v) if !v.is_empty() => {
                            let dir = session.resolve_path(v);
                            session.macro_engine.set_sasautos_path(vec![dir]);
                        }
                        _ => session
                            .log
                            .error("The value for the SASAUTOS option is missing."),
                    }
                } else if value.is_none() && session.set_ods_option(name) {
                    // M22.2 — options globales ODS booléennes (CENTER/NOCENTER,
                    // DATE/NODATE, NUMBER/NONUMBER) posées sur `session.ods_options`.
                    // Stockées seulement (application au rendu différée M22.3+) :
                    // pas d'effet visible sur le listing texte par défaut.
                } else if let Some(flag) = parse_macro_trace_flag(name) {
                    // M19.3 — options de trace booléennes : MPRINT/MLOGIC/
                    // SYMBOLGEN (et leurs formes NO...). Appliquées à la session
                    // ET propagées au processeur macro (qui décide de l'écho).
                    let (which, on) = flag;
                    match which {
                        "mprint" => {
                            session.options.mprint = on;
                            session.macro_engine.set_mprint(on);
                        }
                        "mlogic" => {
                            session.options.mlogic = on;
                            session.macro_engine.set_mlogic(on);
                        }
                        "symbolgen" => {
                            session.options.symbolgen = on;
                            session.macro_engine.set_symbolgen(on);
                        }
                        _ => {}
                    }
                } else {
                    session.log.warning(&format!(
                        "Option {} is not yet supported.",
                        name.to_uppercase()
                    ));
                }
            }
        }
        GlobalStmt::Ods { destination, action, file, style } => {
            exec_ods(session, destination, *action, file.as_deref(), style.as_deref());
        }
        GlobalStmt::OdsOptions { nocenter, date, number } => {
            session.ods_options.nocenter = *nocenter;
            session.ods_options.date = *date;
            session.ods_options.number = *number;
        }
    }
}

/// M22.2 — exécute un statement `ODS` : ouvre/ferme la destination demandée.
///
/// Invariant : la destination courante reste `session.listing`. `ODS LISTING`
/// réinstalle le listing texte par défaut ; `ODS HTML` ouvre la destination
/// HTML ; RTF/PDF/EXCEL sont des stubs (note « différé M23 »). `CLOSE` ferme la
/// destination nommée. FILE=/STYLE= sont stockés dans l'AST mais utilisés
/// seulement en M22.4+ (aucune action fichier ici).
fn exec_ods(
    session: &mut Session,
    destination: &str,
    action: OdsAction,
    _file: Option<&str>,
    _style: Option<&str>,
) {
    use crate::output::{HtmlDestination, RtfDestination, PdfDestination, ExcelDestination, TextListing};

    let dest = destination.to_ascii_lowercase();
    let ls = session.options.ls;

    match action {
        OdsAction::Close => {
            session.close_destination(&dest);
        }
        OdsAction::Open => match dest.as_str() {
            "listing" => {
                session.open_destination("listing", Box::new(TextListing::new(ls)));
            }
            "html" => {
                session.open_destination("html", Box::new(HtmlDestination::new(ls)));
            }
            "rtf" => {
                session.open_destination("rtf", Box::new(RtfDestination::new(ls)));
                session
                    .log
                    .note("ODS RTF destination rendering is deferred to M23.");
            }
            "pdf" => {
                session.open_destination("pdf", Box::new(PdfDestination::new(ls)));
                session
                    .log
                    .note("ODS PDF destination rendering is deferred to M23.");
            }
            "excel" => {
                session.open_destination("excel", Box::new(ExcelDestination::new(ls)));
                session
                    .log
                    .note("ODS EXCEL destination rendering is deferred to M23.");
            }
            other => {
                session.log.warning(&format!(
                    "ODS destination {} is not supported in this build.",
                    other.to_uppercase()
                ));
            }
        },
        OdsAction::Select | OdsAction::Exclude => {
            // Différé M22.3 ; le parser rejette déjà ces formes, donc inatteignable.
            session
                .log
                .note("ODS SELECT/EXCLUDE is deferred to M22.3.");
        }
    }
}

/// M19.3 — reconnaît une option de trace macro booléenne. Rend
/// `Some((canon, on))` où `canon` est `"mprint"`/`"mlogic"`/`"symbolgen"` et
/// `on` est `false` pour la forme préfixée `NO` (ex. `NOMPRINT`). `None` si
/// l'option n'est pas une option de trace.
fn parse_macro_trace_flag(name: &str) -> Option<(&'static str, bool)> {
    let lower = name.to_ascii_lowercase();
    let (body, on) = match lower.strip_prefix("no") {
        Some(rest) if matches!(rest, "mprint" | "mlogic" | "symbolgen") => (rest.to_string(), false),
        _ => (lower, true),
    };
    let canon = match body.as_str() {
        "mprint" => "mprint",
        "mlogic" => "mlogic",
        "symbolgen" => "symbolgen",
        _ => return None,
    };
    Some((canon, on))
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
    // CALL EXECUTE (M15.6) : le code mis en file pendant l'étape s'exécute
    // APRÈS son RUN. On draine la file et on rejoue le code concaténé comme un
    // programme SAS à part entière (il repasse donc par le processeur macro et
    // les statements globaux/DATA/PROC). Garde de profondeur : le code rejoué
    // peut lui-même générer du CALL EXECUTE, mais on traite la file en boucle
    // tant qu'elle se remplit.
    run_call_execute_queue(session);
}

/// Rejoue (M15.6) le code mis en file par CALL EXECUTE. Chaque entrée est un
/// fragment SAS ; on les concatène (séparés par un saut de ligne) et on les
/// exécute via `run_program`. Si le rejeu re-remplit la file (CALL EXECUTE
/// imbriqué), on boucle, avec une garde de profondeur anti-récursion infinie.
fn run_call_execute_queue(session: &mut Session) {
    let mut depth = 0;
    while !session.call_execute_queue.is_empty() {
        depth += 1;
        if depth > 1000 {
            session.log.error(
                "CALL EXECUTE generated too many nested steps (possible infinite loop); stopping.",
            );
            session.call_execute_queue.clear();
            return;
        }
        let code = std::mem::take(&mut session.call_execute_queue).join("\n");
        let src = SourceFile::new(code);
        let _ = run_program(&src, session);
    }
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
    fn execute_ods_opens_listing_and_html() {
        // ODS LISTING / ODS HTML / ODS CLOSE parsent et s'exécutent sans erreur,
        // et le listing texte reste fonctionnel après bascule.
        let out = run_det(
            "ods listing;\n\
             ods html file='out.html';\n\
             ods html close;\n\
             data a; x = 1; run;\n\
             proc print data=a; run;\n",
        );
        assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
        // Le listing texte par défaut fonctionne toujours après la bascule ODS.
        assert!(out.listing.contains("Obs"), "{}", out.listing);
    }

    #[test]
    fn execute_ods_rtf_emits_deferral_note() {
        let out = run_det("ods rtf;\n");
        assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
        assert!(
            out.log.contains("ODS RTF destination rendering is deferred"),
            "{}",
            out.log
        );
    }

    #[test]
    fn execute_global_ods_options_no_warning() {
        // NOCENTER/NODATE/NONUMBER sont reconnues comme options ODS et ne
        // déclenchent pas de WARNING "not yet supported".
        let out = run_det("options nocenter nodate nonumber;\n");
        assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
        assert!(
            !out.log.contains("is not yet supported"),
            "unexpected warning in log:\n{}",
            out.log
        );
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
        // M22.2 — CENTER/NOCENTER/DATE/NODATE/NUMBER/NONUMBER are now handled
        // as ODS options, so no warning. Test with an actually unknown option.
        let out = run_det("options ls=120 unknownopt;");
        assert_eq!(out.exit_code, 1, "{}", out.log);
        assert!(out.log.contains("WARNING: Option UNKNOWNOPT is not yet supported."));
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
    fn options_sasautos_enables_autocall() {
        // M19.2 — `OPTIONS SASAUTOS='dir';` (chemin relatif résolu contre
        // base_dir) doit câbler la recherche autocall : une macro non définie
        // dans le code est cherchée comme `nom.sas` dans ce répertoire et
        // compilée paresseusement à l'invocation.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("auto")).unwrap();
        std::fs::write(
            dir.path().join("auto").join("greet.sas"),
            "%macro greet(who); %put HELLO &who from autocall; %mend;\n",
        )
        .unwrap();
        // L'option doit être posée AVANT l'expansion du segment qui invoque la
        // macro autocall : on place une frontière de segment (`run;`) entre les
        // deux (l'expansion est interfoliée par segment).
        let out = run(
            "options sasautos='auto';\ndata _null_; run;\n%greet(WORLD);\n",
            RunOptions {
                work_dir: None,
                base_dir: Some(dir.path().to_path_buf()),
                deterministic: true,
                vectorize: false,
            },
        );
        assert_eq!(out.exit_code, 0, "{}", out.log);
        assert!(
            out.log.contains("HELLO WORLD from autocall"),
            "autocall macro did not run; log was:\n{}",
            out.log
        );
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

    /// M11.5 : `CALL SYMPUT` dans une étape pose un symbole macro visible
    /// dans le SEGMENT SUIVANT (le drain a lieu au `run;`). Ici on s'en sert
    /// pour nommer un dataset de l'étape d'après.
    #[test]
    fn symput_visible_in_next_segment_as_dataset_name() {
        let out = run_det(
            "data _null_; call symput('answer','42'); run;\n\
             data tbl_&answer; x=1; run;\n\
             proc print data=tbl_&answer; run;\n",
        );
        assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
        // Le dataset a bien été nommé WORK.TBL_42 (symbole résolu).
        assert!(
            out.log
                .contains("The data set WORK.TBL_42 has 1 observations and 1 variables."),
            "log was:\n{}",
            out.log
        );
        assert!(out
            .log
            .contains("There were 1 observations read from the data set WORK.TBL_42."));
    }

    /// M11.5 : formatage NUMÉRIQUE d'un symput — `42` (et non `          42`).
    #[test]
    fn symput_numeric_value_left_aligned_best12() {
        let out = run_det(
            "data _null_; call symput('n', 42); run;\n\
             data tbl_&n; x=1; run;\n",
        );
        assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
        assert!(
            out.log
                .contains("The data set WORK.TBL_42 has 1 observations and 1 variables."),
            "log was:\n{}",
            out.log
        );
    }

    /// M11.5 : SYMGET lit un `%let` antérieur (table macro → DATA step).
    #[test]
    fn symget_reads_prior_let() {
        let out = run_det(
            "%let x = 5;\n\
             data a; v = symget('x'); run;\n\
             proc print data=a; run;\n",
        );
        assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
        // v est une variable caractère = "5".
        assert!(out.listing.contains('5'), "listing was:\n{}", out.listing);
        assert!(out
            .log
            .contains("The data set WORK.A has 1 observations and 1 variables."));
    }

    /// M11.5 : un symput n'est PAS visible DANS LA MÊME étape. SYMGET lit
    /// l'instantané de DÉBUT d'étape : un `symput('w', ...)` plus tôt dans la
    /// MÊME étape ne s'y reflète pas (le drain n'a lieu qu'au `run;`). Ici
    /// `w` n'existe pas au début de l'étape → symget rend une valeur vide,
    /// alors que l'étape SUIVANTE la verrait.
    #[test]
    fn symput_not_visible_in_same_step() {
        let out = run_det(
            "data a; call symput('w','99'); seen = symget('w'); run;\n\
             data b; later = symget('w'); run;\n\
             proc print data=b; run;\n",
        );
        assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
        // Étape A : `seen` est vide (symput pas encore drainé) → 0 obs avec
        // valeur non vide ; on vérifie surtout que B voit bien 99.
        assert!(
            out.listing.contains("99"),
            "step B should see w=99 via symget; listing was:\n{}",
            out.listing
        );
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

    /// M14.4 — LIBNAME XLSX emits a clear deferral error.
    #[test]
    fn libname_xlsx_engine_deferred_error() {
        let out = run_det("libname xl xlsx '/tmp';");
        // The log must contain an ERROR message about XLSX not being available.
        assert!(
            out.log.contains("ERROR"),
            "expected ERROR in log: {}",
            out.log
        );
        assert!(
            out.log.to_ascii_lowercase().contains("xlsx"),
            "expected 'xlsx' in error: {}",
            out.log
        );
    }

    /// M14.4 — LIBNAME EXCEL (synonym for XLSX) also deferred.
    #[test]
    fn libname_excel_engine_deferred_error() {
        let out = run_det("libname xl excel '/tmp';");
        assert!(out.log.contains("ERROR"), "expected ERROR: {}", out.log);
    }

    /// M14.4 — LIBNAME with CSV engine assigns and reads back a table.
    #[test]
    fn libname_csv_engine_end_to_end() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        // Write a CSV file in the temp dir.
        let csv_path = tmp.path().join("scores.csv");
        let mut f = std::fs::File::create(&csv_path).unwrap();
        writeln!(f, "id,score").unwrap();
        writeln!(f, "1,100").unwrap();
        writeln!(f, "2,200").unwrap();
        drop(f);

        let src = format!(
            "libname csv1 csv '{}';\n\
             data work.out; set csv1.scores; run;\n\
             proc print data=work.out; run;\n",
            tmp.path().display()
        );
        let out = crate::run(
            &src,
            crate::RunOptions {
                work_dir: None,
                base_dir: None,
                deterministic: true,
                vectorize: false,
            },
        );
        assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
        assert!(
            out.log.contains("Engine:        CSV"),
            "expected CSV engine note: {}",
            out.log
        );
        assert!(
            out.log.contains("2 observations"),
            "expected 2 obs note: {}",
            out.log
        );
    }

    /// M14.4 — LIBNAME without engine (no engine field) → parquet path unchanged.
    #[test]
    fn libname_no_engine_uses_parquet_path() {
        let tmp = tempfile::tempdir().unwrap();
        let out = crate::run(
            &format!("libname p '{}';", tmp.path().display()),
            crate::RunOptions {
                work_dir: None,
                base_dir: None,
                deterministic: true,
                vectorize: false,
            },
        );
        assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
        assert!(out.log.contains("Engine:        PARQUET"), "{}", out.log);
    }

    // ---- M15.6 — CALL EXECUTE end-to-end (post-step replay) -------------

    /// CALL EXECUTE queues code that runs AFTER the current step's RUN.
    #[test]
    fn call_execute_runs_queued_step_after_run() {
        let out = run_det(
            "data _null_; call execute('data made; v = 7; output; run;'); run;\n\
             proc print data=made; run;\n",
        );
        assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
        // The queued DATA step created WORK.MADE.
        assert!(
            out.log
                .contains("The data set WORK.MADE has 1 observations and 1 variables."),
            "log was:\n{}",
            out.log
        );
        assert!(out.listing.contains('7'), "listing:\n{}", out.listing);
    }

    /// CALL EXECUTE, one per input row, builds several statements that run in
    /// order after the generating step.
    #[test]
    fn call_execute_per_row_generates_multiple_steps() {
        let out = run_det(
            "data seed; do i = 1 to 3; output; end; run;\n\
             data _null_; set seed; \
               call execute('data g'||left(put(i,1.))||'; x=i_val; run;'); run;\n",
        );
        assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
        // Three datasets were generated (WORK.G1, WORK.G2, WORK.G3).
        assert!(out.log.contains("WORK.G1"), "log:\n{}", out.log);
        assert!(out.log.contains("WORK.G2"), "log:\n{}", out.log);
        assert!(out.log.contains("WORK.G3"), "log:\n{}", out.log);
    }
}
