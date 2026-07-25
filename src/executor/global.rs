use super::*;

/// Valide une option numérique bornée : renvoie la valeur si elle parse et
/// tombe dans `range`, sinon émet le message d'erreur SAS
/// « The value X is not a valid LABEL value (lo..hi). » et renvoie `None`.
pub(super) fn parse_bounded_usize(
    session: &mut Session,
    value: Option<&str>,
    range: std::ops::RangeInclusive<usize>,
    label: &str,
) -> Option<usize> {
    match value.and_then(|v| v.parse::<usize>().ok()) {
        Some(v) if range.contains(&v) => Some(v),
        _ => {
            session.log.error(&format!(
                "The value {} is not a valid {label} value ({}..{}).",
                value.unwrap_or(""),
                range.start(),
                range.end()
            ));
            None
        }
    }
}

/// Applique UNE option de `OPTIONS name=value;` (ou boolénne sans valeur) à la
/// session. Une option inconnue émet le WARNING « not yet supported ».
pub(super) fn apply_option(session: &mut Session, name: &str, value: Option<&str>) {
    match name.to_ascii_lowercase().as_str() {
        "ls" | "linesize" => {
            if let Some(v) = parse_bounded_usize(session, value, 40..=256, "LINESIZE") {
                session.options.ls = v;
                session.listing.set_ls(v);
            }
        }
        // OBS=MAX (or unset) → no limit; OBS=n → process up to obs n.
        "obs" => match value {
            Some(v) if v.eq_ignore_ascii_case("max") => session.options.obs = None,
            Some(v) => match v.parse::<usize>() {
                Ok(n) => session.options.obs = Some(n),
                Err(_) => session
                    .log
                    .error(&format!("The value {v} is not a valid OBS value.")),
            },
            None => session.options.obs = None,
        },
        // FIRSTOBS=MAX is unusual; treat any non-number as an error.
        "firstobs" => match value {
            Some(v) if v.eq_ignore_ascii_case("max") => session.options.firstobs = usize::MAX,
            Some(v) => match v.parse::<usize>() {
                Ok(n) if n >= 1 => session.options.firstobs = n,
                _ => session
                    .log
                    .error(&format!("The value {v} is not a valid FIRSTOBS value.")),
            },
            None => {}
        },
        // M38.2 — PAGESIZE=/PS= : page length for listing output.
        // Valid range: 15..=32767. Stored but no pagination yet.
        "ps" | "pagesize" => {
            if let Some(v) = parse_bounded_usize(session, value, 15..=32767, "PAGESIZE") {
                session.options.pagesize = v;
            }
        }
        // M38.2 — MISSING= : single character used to display ordinary
        // numeric missing values (`.`) in the listing. Default '.'.
        // Spec: value is a single character (quoted or unquoted).
        "missing" => match value {
            Some(v) if v.chars().count() == 1 => {
                session.options.missing_char = v.chars().next().expect("length checked");
            }
            Some(v) if v.is_empty() => {
                // OPTIONS MISSING=''; → space (SAS behaviour)
                session.options.missing_char = ' ';
            }
            _ => session
                .log
                .error("The value for the MISSING option must be a single character."),
        },
        // M38.2 — YEARCUTOFF= : lower bound of the 100-year sliding
        // window for interpreting 2-digit years. Valid: 1582..=9999.
        "yearcutoff" => match value.and_then(|v| v.parse::<u16>().ok()) {
            Some(v) if v >= 1582 => {
                session.options.yearcutoff = v;
            }
            _ => session.log.error(&format!(
                "The value {} is not a valid YEARCUTOFF value.",
                value.unwrap_or("")
            )),
        },
        // M38.2 — FMTSEARCH= : list of library refs / catalogues for
        // format search order. Stored; multi-library resolution deferred
        // to M39. Value arrives as space-separated entries (parser joins
        // the parenthesised list with spaces).
        "fmtsearch" => {
            let entries: Vec<String> = value
                .unwrap_or("")
                .split_whitespace()
                .map(|s| s.to_ascii_uppercase())
                .collect();
            session.options.fmtsearch = entries;
        }
        // M19.2 — SASAUTOS= fixe le(s) répertoire(s) de bibliothèques
        // autocall. On accepte une valeur simple (un répertoire) :
        //   OPTIONS SASAUTOS='dir';  ou  OPTIONS SASAUTOS=dir;
        // Les guillemets éventuels sont retirés par le parser
        // global ; le chemin relatif est résolu contre `base_dir`
        // (même base que %include/LIBNAME). La forme liste
        // `(d1 d2)` n'est pas gérée ici (différée).
        "sasautos" => match value {
            Some(v) if !v.is_empty() => {
                let dir = session.resolve_path(v);
                session.macro_engine.set_sasautos_path(vec![dir]);
            }
            _ => session
                .log
                .error("The value for the SASAUTOS option is missing."),
        },
        // M22.2 — options globales ODS booléennes (CENTER/NOCENTER,
        // DATE/NODATE, NUMBER/NONUMBER) posées sur `session.ods_options`.
        // Stockées seulement (application au rendu différée M22.3+) :
        // pas d'effet visible sur le listing texte par défaut.
        _ if value.is_none() && session.set_ods_option(name) => {}
        // M19.3 — options de trace booléennes : MPRINT/MLOGIC/
        // SYMBOLGEN (et leurs formes NO...). Appliquées à la session
        // ET propagées au processeur macro (qui décide de l'écho).
        _ if parse_macro_trace_flag(name).is_some() => {
            match parse_macro_trace_flag(name) {
                Some(("mprint", on)) => {
                    session.options.mprint = on;
                    session.macro_engine.set_mprint(on);
                }
                Some(("mlogic", on)) => {
                    session.options.mlogic = on;
                    session.macro_engine.set_mlogic(on);
                }
                Some(("symbolgen", on)) => {
                    session.options.symbolgen = on;
                    session.macro_engine.set_symbolgen(on);
                }
                _ => {}
            }
        }
        _ => {
            session.log.warning(&format!(
                "Option {} is not yet supported.",
                name.to_uppercase()
            ));
        }
    }
}

/// NOTE de succès (ou ERROR) commune aux trois moteurs de LIBNAME.
pub(super) fn log_libref_assignment(
    session: &mut Session,
    libref: &str,
    engine: &str,
    physical: &str,
    result: crate::error::Result<()>,
) {
    match result {
        Ok(()) => session.log.note(&format!(
            "Libref {} was successfully assigned as follows:\n      Engine:        {engine}\n      Physical Name: {physical}",
            libref.to_uppercase()
        )),
        Err(e) => session.log.error(&e.to_string()),
    }
}

pub(super) fn exec_global(stmt: &GlobalStmt, session: &mut Session) {
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
        GlobalStmt::Filename { fileref, path, device } => {
            // M35.2 — registre minimal fileref → chemin pour `%include fileref;`.
            // La forme `FILENAME ref 'chemin';` (ou chemin nu) enregistre le
            // chemin résolu (même base que %include/LIBNAME/SASAUTOS). Les
            // formes device/options (`TEMP`, pipes, URL, …) sont acceptées mais
            // ignorées (NOTE), car non supportées dans ce build.
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
        GlobalStmt::Ods { destination, action, file, style } => {
            exec_ods(session, destination, *action, file.as_deref(), style.as_deref());
        }
        GlobalStmt::OdsOptions { nocenter, date, number } => {
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
        GlobalStmt::OdsGraphics(stmt) => {
            exec_ods_graphics(session, stmt);
        }
    }
}

/// M29.1 — exécute un statement `ODS GRAPHICS`.
///
/// Met à jour l'état GLOBAL `session.ods_graphics` à partir des options
/// PAR-STATEMENT, puis émet la NOTE de log. La NOTE reflète UNIQUEMENT ce que
/// CE statement a porté : les dimensions ne sont affichées que si WIDTH=/HEIGHT=
/// ont été fournis dans ce statement précis (même si la session conserve des
/// valeurs antérieures). C'est pourquoi on construit la NOTE AVANT/à partir du
/// statement, pas en relisant `session.ods_graphics`.
pub(super) fn exec_ods_graphics(session: &mut Session, stmt: &crate::ast::OdsGraphicsStmt) {
    use crate::ast::OdsGraphicsToggle;

    // 1) Appliquer les options de config à l'état de session (persistantes).
    if let Some(w) = stmt.width {
        session.ods_graphics.width = w;
    }
    if let Some(h) = stmt.height {
        session.ods_graphics.height = h;
    }
    if let Some(fmt) = stmt.imagefmt {
        session.ods_graphics.image_format = fmt;
    }
    if let Some(ref name) = stmt.imagename {
        session.ods_graphics.file_stem = Some(name.clone());
    }

    // 2) Appliquer la bascule ON/OFF (persistante).
    match stmt.toggle {
        OdsGraphicsToggle::On => session.ods_graphics.enabled = true,
        OdsGraphicsToggle::Off => session.ods_graphics.enabled = false,
        OdsGraphicsToggle::None => {}
    }

    // 3) Émettre la NOTE — pilotée par CE statement seulement.
    //    Les dimensions n'apparaissent que si elles ont été fournies ici.
    let dims_suffix = match (stmt.width, stmt.height) {
        (Some(w), Some(h)) => Some(format!("(width={}, height={})", w, h)),
        (Some(w), None) => Some(format!("(width={})", w)),
        (None, Some(h)) => Some(format!("(height={})", h)),
        (None, None) => None,
    };
    match stmt.toggle {
        OdsGraphicsToggle::On => match dims_suffix {
            Some(d) => session.log.note(&format!("ODS GRAPHICS ON {}.", d)),
            None => session.log.note("ODS GRAPHICS ON."),
        },
        OdsGraphicsToggle::Off => session.log.note("ODS GRAPHICS OFF."),
        OdsGraphicsToggle::None => match dims_suffix {
            Some(d) => session.log.note(&format!("ODS GRAPHICS {}.", d)),
            None => session.log.note("ODS GRAPHICS."),
        },
    }
}

/// M22.2/M22.4 — exécute un statement `ODS` : ouvre/ferme la destination demandée.
///
/// Invariant : la destination courante reste `session.listing`. `ODS LISTING`
/// réinstalle le listing texte par défaut ; `ODS HTML` ouvre la destination
/// HTML (M22.4 : avec fichier si FILE= est fourni) ; RTF/PDF/EXCEL sont des
/// stubs (note « différé M23 »). `CLOSE` ferme la destination nommée (M22.4 :
/// déclenche l'écriture du fichier HTML si applicable).
pub(super) fn exec_ods(
    session: &mut Session,
    destination: &str,
    action: OdsAction,
    file: Option<&str>,
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
                // M22.4 : si FILE= est fourni, ouvrir avec un fichier cible ;
                // sinon émettre une NOTE informant que la sortie n'est pas
                // matérialisée (aucun fichier).
                if let Some(f) = file {
                    let path = session.resolve_path(f);
                    session.open_destination(
                        "html",
                        Box::new(HtmlDestination::with_file(ls, path)),
                    );
                } else {
                    session.open_destination("html", Box::new(HtmlDestination::new(ls)));
                    session.log.note(
                        "ODS HTML sans FILE= : la sortie HTML n\u{2019}est pas mat\u{e9}rialis\u{e9}e (v1).",
                    );
                }
            }
            "rtf" => {
                if let Some(f) = file {
                    let path = session.resolve_path(f);
                    session.open_destination("rtf", Box::new(RtfDestination::with_file(ls, path)));
                } else {
                    session.open_destination("rtf", Box::new(RtfDestination::new(ls)));
                    session.log.note("ODS RTF sans FILE= : la sortie RTF n'est pas materialisee (v1).");
                }
            }
            "pdf" => {
                if let Some(f) = file {
                    let path = session.resolve_path(f);
                    session.open_destination("pdf", Box::new(PdfDestination::with_file(ls, path)));
                } else {
                    session.open_destination("pdf", Box::new(PdfDestination::new(ls)));
                    session.log.note("ODS PDF sans FILE= : la sortie PDF n'est pas materialisee (v1).");
                }
            }
            "excel" => {
                if let Some(f) = file {
                    let path = session.resolve_path(f);
                    session.open_destination("excel", Box::new(ExcelDestination::with_file(ls, path)));
                } else {
                    session.open_destination("excel", Box::new(ExcelDestination::new(ls)));
                    session.log.note("ODS EXCEL sans FILE= : la sortie Excel n'est pas materialisee (v1).");
                }
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
pub(super) fn parse_macro_trace_flag(name: &str) -> Option<(&'static str, bool)> {
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
