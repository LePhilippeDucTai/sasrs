//! Harnais de tests CLI réels : le binaire `sasrs` de ce dépôt est exécuté
//! comme un vrai processus (et non via [`sasrs::run`]) dans un répertoire
//! temporaire, pour verrouiller le contrat de `src/main.rs`.
//!
//! Couvert (J01-P3) : codes retour 0/1/2, script illisible → 2 + message
//! stderr, `--log`/`--print` écrivent les fichiers attendus, défauts log →
//! stderr et listing → stdout, `--version` aligné sur `Cargo.toml`, chemins
//! LIBNAME relatifs résolus depuis le dossier du SCRIPT.
//!
//! J02-P1 : échecs d’écriture CLI et cycle de vie des destinations ODS.

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

// J02-P1: expected behavior comes from the coordinator's write-failure and
// ODS lifecycle contract. Compare recovered bytes with the existing clean CLI
// behavior; no new snapshots or implementation-derived formatting oracle.
fn assert_write_failure(flag: &str, target: &str) {
    let baseline = run_sas(CLEAN_PRINT, &["--deterministic"]);
    let case = run_sas(CLEAN_PRINT, &["--deterministic", flag, target]);
    assert_eq!(case.run.code(), 2, "{}", case.run.stderr);
    assert!(case.run.stderr.contains("ERROR: cannot write"));
    assert!(case.run.stderr.contains(target));
    let recovered = if flag == "--log" {
        &baseline.run.stderr
    } else {
        &baseline.run.stdout
    };
    assert!(case.run.stderr.contains(recovered), "{}", case.run.stderr);
    assert!(!case.run.stderr.contains("panicked"));
}

#[test]
fn write_failure_log_directory() {
    assert_write_failure("--log", ".");
}

#[test]
fn write_failure_print_directory() {
    assert_write_failure("--print", ".");
}

#[test]
fn write_failure_missing_directory() {
    for flag in ["--log", "--print"] {
        assert_write_failure(flag, "absent/output.txt");
    }
}

#[cfg(unix)]
#[test]
fn write_failure_permission() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("readonly.txt");
    std::fs::write(&target, "preserved").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o444)).unwrap();
    // A privileged test runner must not silently pass without exercising EACCES.
    assert!(
        std::fs::OpenOptions::new()
            .write(true)
            .open(&target)
            .is_err()
    );
    for flag in ["--log", "--print"] {
        assert_write_failure(flag, target.to_str().unwrap());
    }
    assert_eq!(std::fs::read_to_string(target).unwrap(), "preserved");
}

#[cfg(unix)]
#[test]
fn write_failure_closed_stdout() {
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    use std::process::Stdio;
    let tmp = tempfile::tempdir().unwrap();
    let script = tmp.path().join("prog.sas");
    std::fs::write(&script, CLEAN_PRINT).unwrap();
    let (writer, reader) = UnixStream::pair().unwrap();
    drop(reader);
    let out = Command::new(env!("CARGO_BIN_EXE_sasrs"))
        .arg(script)
        .stdout(Stdio::from(OwnedFd::from(writer)))
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("ERROR: cannot write stdout"), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
}

#[test]
fn ods_unwritable_file_close_and_end_of_run() {
    for destination in ["html", "rtf", "pdf", "excel"] {
        for close in [
            String::new(),
            "ods _all_ close;".into(),
            "ods close;".into(),
            format!("ods {destination} close;"),
        ] {
            let prog = format!("ods {destination} file='absent/result';\n{CLEAN_PRINT}\n{close}");
            let case = run_sas(&prog, &[]);
            assert_eq!(case.run.code(), 2, "{}", case.run.stderr);
            assert!(case.run.stderr.contains("ERROR: Could not write"));
            assert!(
                case.run
                    .stderr
                    .contains(case.dir().join("absent/result").to_str().unwrap())
            );
            assert!(!case.run.stderr.contains("NOTE: WARNING"));
        }
    }
}

#[test]
fn ods_successive_destinations_preserve_files_and_listing() {
    for second in ["html", "rtf", "pdf", "excel"] {
        let prog = format!(
            "title 'BEFORE_ODS'; {CLEAN_PRINT}\n\
             ods html file='first.html'; title 'FIRST_ODS'; proc print data=a; run;\n\
             ods {second} file='second.out'; title 'SECOND_ODS'; proc print data=a; run;\n\
             ods _all_ close; title 'AFTER_ODS'; proc print data=a; run;"
        );
        let case = run_sas(&prog, &[]);
        assert_eq!(case.run.code(), 0, "{}", case.run.stderr);
        let first = std::fs::read_to_string(case.dir().join("first.html")).unwrap();
        assert!(first.contains("FIRST_ODS"));
        assert!(!first.contains("SECOND_ODS"));
        let second = std::fs::read(case.dir().join("second.out")).unwrap();
        assert!(!second.is_empty());
        assert!(case.run.stdout.contains("BEFORE_ODS"));
        assert!(case.run.stdout.contains("AFTER_ODS"));
        assert!(!case.run.stdout.contains("FIRST_ODS"));
        assert!(!case.run.stdout.contains("SECOND_ODS"));
        assert_eq!(case.run.stderr.matches("NOTE: Writing").count(), 2);
    }
}

#[test]
fn ods_empty_destination_note() {
    for destination in ["html", "rtf", "pdf", "excel"] {
        for close in ["", "ods _all_ close;"] {
            let case = run_sas(&format!("ods {destination} file='empty.out'; {close}"), &[]);
            assert_eq!(case.run.code(), 0, "{}", case.run.stderr);
            assert!(
                case.run.stderr.contains("NOTE: ODS") && case.run.stderr.contains("no output"),
                "{}",
                case.run.stderr
            );
            assert!(!case.dir().join("empty.out").exists());
        }
    }
}

#[test]
fn ods_replacement_after_write_failure_continues() {
    let case = run_sas(
        &format!(
            "ods html file='absent/first.html'; {CLEAN_PRINT}\n\
             ods html file='second.html'; title 'SECOND_ODS'; proc print data=a; run;\n\
             ods html close; ods _all_ close;"
        ),
        &[],
    );
    assert_eq!(case.run.code(), 2, "{}", case.run.stderr);
    assert_eq!(case.run.stderr.matches("ERROR: Could not write").count(), 1);
    assert!(
        case.run
            .stderr
            .contains(case.dir().join("absent/first.html").to_str().unwrap())
    );
    assert_eq!(
        case.run
            .stderr
            .matches("NOTE: Writing HTML Body file: second.html")
            .count(),
        1
    );
    assert!(
        std::fs::read_to_string(case.dir().join("second.html"))
            .unwrap()
            .contains("SECOND_ODS")
    );
}

#[cfg(feature = "graphics")]
#[test]
fn ods_image_write_failure_is_error() {
    for procedure in [
        "proc reg data=a; model y=x; run;",
        "proc reg data=a plots=(fit); model y=x; run;",
        "proc univariate data=a noprint; var x; histogram x; run;",
    ] {
        let case = run_sas(
            &format!(
                "data a; do x=1 to 20; y=x*x; output; end; run;\n\
                 ods graphics on / imagename='absent/image'; {procedure}"
            ),
            &[],
        );
        assert_eq!(case.run.code(), 2, "{}", case.run.stderr);
        assert!(
            case.run.stderr.contains("ERROR: could not write image"),
            "{}",
            case.run.stderr
        );
        assert!(
            case.run
                .stderr
                .contains(case.dir().join("absent/image").to_str().unwrap())
        );
        assert!(!case.run.stderr.contains("NOTE: WARNING"));
    }
}

// ── UTF-8 / BOM (contrat D-001, docs/encoding.md — J03-P3) ────────────────

/// Un BOM UTF-8 en tête du source `.sas` est ignoré : programme propre,
/// code retour 0 (comme SAS, qui saute le BOM).
#[test]
fn utf8_bom_in_source_is_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    let script = tmp.path().join("prog.sas");
    std::fs::write(&script, format!("\u{feff}{CLEAN_PRINT}")).unwrap();
    let run = sasrs(tmp.path(), &[script.to_str().unwrap()]);
    assert_eq!(run.code(), 0, "stderr:\n{}", run.stderr);
    assert!(
        !run.stderr.contains("ERROR:"),
        "le BOM ne doit produire aucune ERROR : {}",
        run.stderr
    );
}

/// Un source `.sas` qui n'est pas valide UTF-8 est refusé : code retour 2 et
/// message qui NOMME le fichier (contrat D-001 — aucun repli lossy).
#[test]
fn utf8_invalid_source_is_error_naming_file() {
    let tmp = tempfile::tempdir().unwrap();
    let script = tmp.path().join("prog.sas");
    std::fs::write(&script, b"data a; x = \"caf\xe9\"; run;\n").unwrap();
    let run = sasrs(tmp.path(), &[script.to_str().unwrap()]);
    assert_eq!(run.code(), 2, "stderr:\n{}", run.stderr);
    assert!(
        run.stderr.contains("ERROR") && run.stderr.contains("prog.sas"),
        "l'ERROR devrait nommer le fichier : {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("UTF-8"),
        "l'ERROR devrait mentionner UTF-8 : {}",
        run.stderr
    );
}
