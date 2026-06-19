//! `sas_interpreter` — interpréteur du langage SAS (référence SAS 9.4
//! classique) sur Polars, tables au format Parquet.
//!
//! Voir PLAN.md pour l'architecture complète, les jalons M1–M8 et la
//! répartition implémenté/squelette.

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
pub mod token;
pub mod value;

use session::Session;
use source::SourceFile;
use std::path::PathBuf;

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

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions {
            work_dir: None,
            base_dir: None,
            deterministic: false,
            vectorize: false,
        }
    }
}

pub struct RunOutcome {
    pub log: String,
    pub listing: String,
    /// 0 = propre, 1 = warnings, 2 = erreurs (esprit des codes retour SAS).
    pub exit_code: i32,
}

/// Exécute un programme SAS complet et rend log + listing.
pub fn run(source_text: &str, opts: RunOptions) -> RunOutcome {
    let base_dir = opts
        .base_dir
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    let mut session = match Session::new(opts.work_dir, base_dir, opts.deterministic) {
        Ok(s) => s,
        Err(e) => {
            return RunOutcome {
                log: format!("ERROR: {e}\n"),
                listing: String::new(),
                exit_code: 2,
            };
        }
    };
    session.vectorize = opts.vectorize;

    // M11.1 : l'expansion macro n'est plus pilotée ici. L'état macro vit dans
    // `Session::macro_engine` et l'expansion est désormais conduite par
    // l'`executor` (cf. `run_program`). Le source brut est passé tel quel.
    let src = SourceFile::new(source_text.to_string());

    if let Err(e) = executor::run_program(&src, &mut session) {
        session.log.error(&e.to_string());
    }

    // M23 — filet de sécurité : si une destination avec fichier cible est encore
    // ouverte à la fin du programme (fixture sans `ODS CLOSE`), on l'écrit
    // maintenant. La NOTE va dans le log AVANT `log.into_string()`.
    if let Some((path, bytes)) = session.listing.finalize_to_bytes() {
        let label = session.listing.dest_type_label();
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("output")
            .to_string();
        match std::fs::write(&path, &bytes) {
            Ok(()) => {
                session.log.note(&format!("Writing {} file: {}", label, file_name));
            }
            Err(e) => {
                session.log.note(&format!(
                    "WARNING: Could not write {} file {}: {}",
                    label, file_name, e
                ));
            }
        }
    } else if let Some((path, content)) = session.listing.finalize() {
        let label = session.listing.dest_type_label();
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("output.html")
            .to_string();
        match std::fs::write(&path, &content) {
            Ok(()) => {
                session.log.note(&format!("Writing {} file: {}", label, file_name));
            }
            Err(e) => {
                session.log.note(&format!(
                    "WARNING: Could not write {} file {}: {}",
                    label, file_name, e
                ));
            }
        }
    }

    let exit_code = if session.log.errors > 0 {
        2
    } else if session.log.warnings > 0 {
        1
    } else {
        0
    };
    RunOutcome {
        log: session.log.into_string(),
        listing: session.listing.into_string(),
        // NB : `Session.listing` est désormais `Box<dyn OutputDestination>` ;
        // `into_string` prend `&mut self` (drain) au lieu de consommer.
        exit_code,
    }
}
