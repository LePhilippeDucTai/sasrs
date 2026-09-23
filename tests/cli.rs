//! Harnais de tests CLI réels : le binaire `sasrs` de ce dépôt est exécuté
//! comme un vrai processus (et non via [`sasrs::run`]) dans un répertoire
//! temporaire, pour verrouiller le contrat de `src/main.rs`.
//!
//! Couvert (J01-P3) : codes retour 0/1/2, script illisible → 2 + message
//! stderr, `--log`/`--print` écrivent les fichiers attendus, défauts log →
//! stderr et listing → stdout, `--version` aligné sur `Cargo.toml`, chemins
//! LIBNAME relatifs résolus depuis le dossier du SCRIPT.
//!
//! Hors périmètre ici : les échecs d'écriture `--log`/`--print` (J02-P1
//! corrige le comportement puis le teste).

mod common;

use std::path::Path;
use std::process::Command;

/// Programme propre : un DATA step minimal + PROC PRINT (listing non vide).
const CLEAN_PRINT: &str = r#"
data work.a;
    x = 42;
run;

proc print data=work.a;
run;
"#;

/// Programme dont la SEULE anomalie est un WARNING : option inconnue demandée
/// à PROC OPTIONS (« Option XXX is not a recognized SAS option. »).
const UNKNOWN_OPTION: &str = r#"
proc options ZORKOPTION;
run;
"#;

/// Programme avec une ERROR : procédure inconnue (récupération de session,
/// mais le code retour doit signaler l'échec).
const UNKNOWN_PROC: &str = r#"
data work.a;
    x = 1;
run;

proc frobnicate data=work.a;
run;
"#;

/// Programme qui lit une libref déclarée en chemin RELATIF : `data` n'existe
/// que dans le dossier du script, pas dans le répertoire courant du processus.
const RELATIVE_LIBNAME: &str = r#"
libname d 'data';

proc print data=d.class;
run;
"#;

/// Une exécution du binaire : sorties décodées + code retour
/// (`None` = processus tué par un signal).
struct CliRun {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

impl CliRun {
    /// Code retour, en échouant vite si le binaire est mort par signal.
    fn code(&self) -> i32 {
        self.code
            .expect("sasrs terminé par un signal, sans code retour")
    }
}

/// Exécute `sasrs <args…>` avec `cwd` comme répertoire courant du processus.
///
/// `env!("CARGO_BIN_EXE_sasrs")` est posé par cargo : il garantit que le
/// binaire est (re)construit avant les tests d'intégration et pointe sur lui.
fn sasrs(cwd: &Path, args: &[&str]) -> CliRun {
    let out = Command::new(env!("CARGO_BIN_EXE_sasrs"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("impossible de lancer le binaire sasrs");
    CliRun {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        code: out.status.code(),
    }
}

/// Tempdir d'une exécution, gardé vivant le temps des assertions.
struct Case {
    tmp: tempfile::TempDir,
    run: CliRun,
}

impl Case {
    fn dir(&self) -> &Path {
        self.tmp.path()
    }
}

/// Helper réutilisable : tempdir frais, `prog` écrit dans `prog.sas`, puis
/// exécution de `sasrs <args…> /…/prog.sas` depuis le tempdir lui-même.
/// Rend stdout/stderr/code via [`Case::run`].
fn run_sas(prog: &str, args: &[&str]) -> Case {
    let tmp = tempfile::tempdir().unwrap();
    let script = tmp.path().join("prog.sas");
    std::fs::write(&script, prog).unwrap();
    let mut argv: Vec<&str> = args.to_vec();
    argv.push(script.to_str().unwrap());
    let run = sasrs(tmp.path(), &argv);
    Case { tmp, run }
}

/// Programme sans WARNING ni ERROR → code retour 0.
#[test]
fn clean_program_exits_zero() {
    let case = run_sas(CLEAN_PRINT, &[]);
    assert_eq!(case.run.code(), 0, "stderr:\n{}", case.run.stderr);
    assert!(
        !case.run.stderr.contains("ERROR:"),
        "stderr inattendue : {}",
        case.run.stderr
    );
}

/// Un WARNING sans ERROR → code retour 1 (esprit des codes retour SAS).
#[test]
fn warning_program_exits_one() {
    let case = run_sas(UNKNOWN_OPTION, &[]);
    assert_eq!(case.run.code(), 1, "stderr:\n{}", case.run.stderr);
    assert!(
        case.run.stderr.contains("WARNING:"),
        "WARNING absent du log : {}",
        case.run.stderr
    );
    assert!(
        case.run.stderr.contains("ZORKOPTION"),
        "le WARNING devrait nommer l'option : {}",
        case.run.stderr
    );
}

/// Une ERROR → code retour 2, même si la session récupère et continue.
#[test]
fn error_program_exits_two() {
    let case = run_sas(UNKNOWN_PROC, &[]);
    assert_eq!(case.run.code(), 2, "stderr:\n{}", case.run.stderr);
    assert!(
        case.run.stderr.contains("ERROR:"),
        "ERROR absent du log : {}",
        case.run.stderr
    );
}

/// Script illisible (ici : inexistant) → code retour 2 et message sur stderr.
#[test]
fn unreadable_script_exits_two_with_stderr_message() {
    let tmp = tempfile::tempdir().unwrap();
    let run = sasrs(tmp.path(), &["absent.sas"]);
    assert_eq!(run.code(), 2, "stderr:\n{}", run.stderr);
    assert!(
        run.stderr.contains("cannot read") && run.stderr.contains("absent.sas"),
        "message stderr inattendu : {}",
        run.stderr
    );
}

/// `--log F` et `--print F` détournent log et listing vers les fichiers
/// attendus : les deux fichiers existent avec le bon contenu, et stdout comme
/// stderr restent vides.
#[test]
fn log_and_print_flags_write_expected_files() {
    let case = run_sas(CLEAN_PRINT, &["--log", "out.log", "--print", "out.lst"]);

    assert_eq!(case.run.code(), 0, "stderr:\n{}", case.run.stderr);
    assert!(case.run.stdout.is_empty(), "stdout:\n{}", case.run.stdout);
    assert!(case.run.stderr.is_empty(), "stderr:\n{}", case.run.stderr);

    let log = std::fs::read_to_string(case.dir().join("out.log")).unwrap();
    let listing = std::fs::read_to_string(case.dir().join("out.lst")).unwrap();
    assert!(
        log.contains("NOTE:") && log.contains("PROCEDURE PRINT"),
        "contenu du fichier log :\n{log}"
    );
    assert!(
        listing.contains("The SAS System") && listing.contains("42"),
        "contenu du fichier listing :\n{listing}"
    );
}

/// Sans option, la log part sur stderr et le listing sur stdout — jamais
/// l'inverse, et jamais les deux mélangés sur un même flux.
#[test]
fn defaults_log_to_stderr_and_listing_to_stdout() {
    let case = run_sas(CLEAN_PRINT, &[]);

    assert!(
        case.run.stdout.contains("The SAS System"),
        "listing absent de stdout :\n{}",
        case.run.stdout
    );
    assert!(
        !case.run.stdout.contains("NOTE:"),
        "la log a fui sur stdout :\n{}",
        case.run.stdout
    );
    assert!(
        case.run.stderr.contains("NOTE:"),
        "log absent de stderr :\n{}",
        case.run.stderr
    );
    assert!(
        !case.run.stderr.contains("The SAS System"),
        "le listing a fui sur stderr :\n{}",
        case.run.stderr
    );
}

/// `--version` rend la version de `Cargo.toml` du paquet.
#[test]
fn version_reports_cargo_toml_version() {
    let tmp = tempfile::tempdir().unwrap();
    let run = sasrs(tmp.path(), &["--version"]);
    assert_eq!(run.code(), 0, "stderr:\n{}", run.stderr);
    assert!(
        run.stdout.contains("sasrs") && run.stdout.contains(env!("CARGO_PKG_VERSION")),
        "sortie --version inattendue : {}",
        run.stdout
    );
}

/// Les chemins LIBNAME relatifs sont résolus depuis le DOSSIER DU SCRIPT :
/// ici `data/class.parquet` vit à côté de `prog.sas`, tandis que le
/// répertoire courant du processus est un autre répertoire vide.
#[test]
fn relative_libname_resolved_from_script_dir() {
    let script_dir = tempfile::tempdir().unwrap();
    common::write_class_parquet(&script_dir.path().join("data"));
    let script = script_dir.path().join("prog.sas");
    std::fs::write(&script, RELATIVE_LIBNAME).unwrap();

    let elsewhere = tempfile::tempdir().unwrap();
    let run = sasrs(elsewhere.path(), &[script.to_str().unwrap()]);

    assert_eq!(run.code(), 0, "stderr:\n{}", run.stderr);
    assert!(
        run.stdout.contains("Alfred"),
        "d.class n'a pas été lu depuis le dossier du script :\n{}",
        run.stdout
    );
}
