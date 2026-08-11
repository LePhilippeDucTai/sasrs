use super::*;

mod libname;
mod ods;
mod options;

pub(super) use libname::*;
pub(super) use ods::*;
pub(super) use options::*;

pub(super) fn exec_global(stmt: &GlobalStmt, session: &mut Session) {
    match stmt {
        GlobalStmt::Libname {
            libref,
            engine,
            path,
        } => {
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
            if path
                .get(..5)
                .is_some_and(|p| p.eq_ignore_ascii_case("s3://"))
            {
                let result = session.libs.assign_uri(libref, path);
                log_libref_assignment(session, libref, "PARQUET", path, result);
                return;
            }

            let engine_up = engine.as_deref().map(|e| e.to_ascii_uppercase());

            // M14.4 : XLSX engine deferral — emit an error and return.
            match engine_up.as_deref() {
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
            // None | Some("PARQUET") | Some("BASE") | Some("V9") | _ → parquet path
            let (engine_name, result) = match engine_up.as_deref() {
                Some("CSV") => ("CSV", session.libs.assign_csv(libref, abs)),
                _ => ("PARQUET", session.libs.assign(libref, abs)),
            };
            log_libref_assignment(session, libref, engine_name, &shown, result);
        }
        GlobalStmt::LibnameClear { libref } => match session.libs.clear(libref) {
            Ok(()) => session.log.note(&format!(
                "Libref {} has been deassigned.",
                libref.to_uppercase()
            )),
            Err(e) => session.log.error(&e.to_string()),
        },
        GlobalStmt::Filename {
            fileref,
            path,
            device,
        } => {
            // M35.2 — registre minimal fileref → chemin pour `%include fileref;`.
            // La forme `FILENAME ref 'chemin';` (ou chemin nu) enregistre le
            // chemin résolu (même base que %include/LIBNAME/SASAUTOS). Les
            // formes device (`TEMP`, `PIPE`, `URL`, …) sont reconnues mais
            // différées (NOTE) car non supportées dans ce build ; le device est
            // tout de même mémorisé (M38.5) pour un diagnostic fidèle au moment
            // de l'usage (`%include fileref;`).
            //
            // CAVEAT segment : un FILENAME ne devient visible pour un `%include`
            // que si ce dernier est dans un SEGMENT/STATEMENT ULTÉRIEUR — chaque
            // segment est expansé (où %include résout) puis exécuté avant le
            // suivant ; le FILENAME registre son fileref à l'exécution de son
            // propre segment.
            match (path, device) {
                (Some(p), _) => {
                    let resolved = session.resolve_path(p);
                    session.macro_engine.set_fileref(fileref, resolved);
                }
                (None, Some(dev)) => {
                    // M38.5 — mémoriser l'assignation device pour qu'un
                    // `%include fileref;` ultérieur émette une NOTE de
                    // déferrement fidèle au lieu d'un « cannot read » trompeur.
                    session.macro_engine.set_fileref_device(fileref, dev);
                    session.log.note(&format!(
                        "FILENAME device {} is not supported in this build; statement ignored.",
                        dev
                    ));
                }
                (None, None) => {
                    // `FILENAME ref ;` dégénéré : no-op silencieux.
                }
            }
        }
        GlobalStmt::Title { n, text } => {
            // M38.1 : TITLE1..TITLE9 multi-niveaux avec sémantique d'effacement
            // SAS, état global de session poussé à la destination courante.
            session.set_title_level(*n, text.clone());
        }
        GlobalStmt::Footnote { n, text } => {
            // M38.1 : FOOTNOTE1..FOOTNOTE9, même sémantique que TITLE.
            session.set_footnote_level(*n, text.clone());
        }
        GlobalStmt::Options(opts) => {
            for (name, value) in opts {
                apply_option(session, name, value.as_deref());
            }
        }
        GlobalStmt::Ods {
            destination,
            action,
            file,
            style,
        } => {
            exec_ods(
                session,
                destination,
                *action,
                file.as_deref(),
                style.as_deref(),
            );
        }
        GlobalStmt::OdsOptions {
            nocenter,
            date,
            number,
        } => {
            session.ods_options.nocenter = *nocenter;
            session.ods_options.date = *date;
            session.ods_options.number = *number;
        }
        GlobalStmt::OdsOutput { mappings, close } => {
            if *close {
                session.clear_ods_output();
            } else {
                session.set_ods_output(mappings);
            }
        }
        GlobalStmt::OdsSelect { exclude, items } => {
            // M38.4 — remplace la liste de sélection ODS de la session.
            // Silencieux, comme SAS (aucune NOTE de log). Cycle de vie
            // (consommation au step suivant, ALL/NONE persistants) :
            // `session::ods_select`.
            session.set_ods_selection(*exclude, items);
        }
        GlobalStmt::OdsGraphics(stmt) => {
            exec_ods_graphics(session, stmt);
        }
    }
}
