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
pub mod lexer;
pub mod library;
pub mod listing;
pub mod log;
pub mod missing;
pub mod parser;
pub mod preprocess;
pub mod procs;
pub mod session;
pub mod source;
pub mod sql;
pub mod token;
pub mod value;

use preprocess::TextStage;
#[cfg(not(feature = "macros"))]
use preprocess::IdentityMacroStage;
#[cfg(feature = "macros")]
use preprocess::MacroStage;
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
}

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions {
            work_dir: None,
            base_dir: None,
            deterministic: false,
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

    // Couture du processeur macro : identité par défaut, spike %let/&var
    // sous la feature `macros`.
    #[cfg(not(feature = "macros"))]
    let preprocessed = IdentityMacroStage.process(source_text);
    #[cfg(feature = "macros")]
    let preprocessed = MacroStage::default().process(source_text);
    let src = SourceFile::new(preprocessed);

    if let Err(e) = executor::run_program(&src, &mut session) {
        session.log.error(&e.to_string());
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
        exit_code,
    }
}
