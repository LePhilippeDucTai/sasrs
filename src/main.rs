//! `sasrs` — exécute un programme SAS en batch.
//!
//! Défauts : log sur stderr, listing sur stdout (comme un SAS batch qui
//! écrirait .log/.lst ; les fichiers via --log/--print).

use clap::Parser;
use sasrs::{RunOptions, run};
use std::io::{self, Write};
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
            let _ = writeln!(
                io::stderr().lock(),
                "ERROR: cannot read {}: {e}",
                cli.script.display()
            );
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

    // Attempt both outputs even if the first fails.
    let log_ok = write_output(&cli.log, &outcome.log, false);
    let listing_ok = write_output(&cli.print, &outcome.listing, true);
    ExitCode::from(if log_ok && listing_ok {
        outcome.exit_code as u8
    } else {
        2
    })
}

fn write_output(target: &Option<PathBuf>, content: &str, stdout: bool) -> bool {
    let result = match target {
        Some(path) => std::fs::write(path, content),
        None if stdout => write_stream(&mut io::stdout().lock(), content),
        None => write_stream(&mut io::stderr().lock(), content),
    };
    if let Err(error) = result {
        let name = target.as_ref().map_or_else(
            || if stdout { "stdout" } else { "stderr" }.to_string(),
            |path| path.display().to_string(),
        );
        let mut stderr = io::stderr().lock();
        let _ = writeln!(stderr, "ERROR: cannot write {name}: {error}");
        // Preserve the entire buffered output when a requested file fails.
        // Explicit Write calls also make broken pipes errors instead of panics.
        if target.is_some() || stdout {
            let _ = write_stream(&mut stderr, content);
        }
        return false;
    }
    true
}

fn write_stream(stream: &mut impl Write, content: &str) -> io::Result<()> {
    stream.write_all(content.as_bytes())?;
    stream.flush()
}
