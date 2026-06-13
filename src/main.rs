//! `sasrs` — exécute un programme SAS en batch.
//!
//! Défauts : log sur stderr, listing sur stdout (comme un SAS batch qui
//! écrirait .log/.lst ; les fichiers via --log/--print).

use clap::Parser;
use sas_interpreter::{RunOptions, run};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "sasrs",
    version,
    about = "Interpréteur SAS (SAS 9.4 classique) sur Polars — tables Parquet"
)]
struct Cli {
    /// Programme SAS à exécuter (.sas)
    script: PathBuf,

    /// Écrire la log dans un fichier au lieu de stderr
    #[arg(long)]
    log: Option<PathBuf>,

    /// Écrire le listing dans un fichier au lieu de stdout
    #[arg(long)]
    print: Option<PathBuf>,

    /// Répertoire WORK (défaut : répertoire temporaire jeté en fin de session)
    #[arg(long)]
    work: Option<PathBuf>,

    /// Sortie déterministe (temps figés) — utilisé par les tests snapshot
    #[arg(long)]
    deterministic: bool,

    /// Active le fast-path vectorisé OPTIONNEL des étapes DATA simples
    /// (SET + assignations numériques) ; sinon repli sur la boucle.
    #[arg(long)]
    vectorize: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let source = match std::fs::read_to_string(&cli.script) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ERROR: cannot read {}: {e}", cli.script.display());
            return ExitCode::from(2);
        }
    };

    let base_dir = cli
        .script
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(PathBuf::from);

    let outcome = run(
        &source,
        RunOptions {
            work_dir: cli.work,
            base_dir,
            deterministic: cli.deterministic,
            vectorize: cli.vectorize,
        },
    );

    let write_or = |target: &Option<PathBuf>, content: &str, fallback_stdout: bool| {
        match target {
            Some(path) => {
                if let Err(e) = std::fs::write(path, content) {
                    eprintln!("ERROR: cannot write {}: {e}", path.display());
                }
            }
            None if fallback_stdout => print!("{content}"),
            None => eprint!("{content}"),
        }
    };

    write_or(&cli.log, &outcome.log, false);
    write_or(&cli.print, &outcome.listing, true);

    ExitCode::from(outcome.exit_code as u8)
}
