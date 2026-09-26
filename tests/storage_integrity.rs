//! Tests adversariaux d'intégrité du stockage (J04-P5) — feature
//! `fault-injection`. Le binaire `sasrs` réel est exécuté en SOUS-PROCESSUS
//! avec `SASRS_FAULT_INJECT` posé sur UN point d'injection : il meurt
//! (`exit(86)`, message `SASRS fault point hit: <nom>` sur stderr) en plein
//! protocole d'écriture atomique parquet + sidecar (ADR 0001), comme un
//! `kill -9` entre deux renommages. Le parent relit ensuite l'état disque
//! dans un run PROPRE et exige l'invariant central :
//!
//!   après une interruption, le run suivant voit SOIT l'ancien état cohérent,
//!   SOIT le nouvel état cohérent — jamais « données nouvelles + métadonnées
//!   fausses » sans diagnostic nommant le sidecar et la cause.
//!
//! Couvert aussi : round-trip données + métadonnées (formats, libellés,
//! longueurs, missing spéciaux) à travers deux sessions distinctes, et
//! sidecars corrompus (troncature, octets arbitraires, empreinte trafiquée)
//! qui doivent produire un WARNING, jamais mentir.
//!
//! AUCUNE modification de `src/` : tout passe par le CLI public.

#![cfg(feature = "fault-injection")]

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Programme d'amorce : crée `d.class` (3 obs) PUIS `d.work` avec
/// métadonnées explicites (longueur caractère, format, libellés) qui
/// doivent passer par le sidecar pour survivre à la session.
const SETUP: &str = r#"
libname d 'data';
data d.class;
  length name $ 8;
  name='Alfred'; sex='M'; age=14; height=69.0; output;
  name='Alice'; sex='F'; age=13; height=56.5; output;
  name='Barbara'; sex='F'; age=14; height=65.3; output;
run;
data d.work;
  length name $ 12;
  set d.class;
  format height 8.2;
  label name='Full name' height='Height (in)';
run;
"#;

/// Remplacement de `d.work` par des données différentes, SANS métadonnées :
/// l'ancien sidecar doit devenir inapplicable (empreinte périmée).
const REPLACE_PLAIN: &str = r#"
libname d 'data';
data d.work;
  set d.class;
  z = age * 2;
run;
"#;

/// Remplacement de `d.work` avec de NOUVELLES métadonnées : après
/// interruption puis réécriture propre, ce sont elles qui s'appliquent.
const REPLACE_WITH_META: &str = r#"
libname d 'data';
data d.work;
  set d.class;
  format height 6.1;
  label name='Nom';
run;
"#;

/// Lecture complète d'une table : la liste des variables (proc contents,
/// y compris formats et libellés) puis les données (proc print).
fn read_program(table: &str) -> String {
    format!(
        "libname d 'data';\nproc contents data=d.{table};\nrun;\n\
         proc print data=d.{table};\nrun;\n"
    )
}

/// Renommage d'une table via le CLI réel (`proc datasets change`).
fn rename_program(from: &str, to: &str) -> String {
    format!("libname d 'data';\nproc datasets lib=d nolist;\n  change {from}={to};\nquit;\n")
}

/// Un répertoire de travail jetable : `<tmp>/data/class.parquet` (généré
/// programmatiquement, cf. `tests/common`) + scripts SAS écrits dedans.
struct Scratch {
    root: tempfile::TempDir,
}

impl Scratch {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        fs::create_dir(&data).unwrap();
        common::write_class_parquet(&data);
        Scratch { root }
    }

    fn dir(&self) -> &Path {
        self.root.path()
    }

    fn data_dir(&self) -> PathBuf {
        self.root.path().join("data")
    }

    fn table(&self, name: &str) -> PathBuf {
        self.data_dir().join(format!("{name}.parquet"))
    }

    fn sidecar(&self, name: &str) -> PathBuf {
        self.data_dir().join(format!("{name}.parquet.sasmeta.json"))
    }

    fn write_script(&self, name: &str, source: &str) -> PathBuf {
        let path = self.dir().join(name);
        fs::write(&path, source).unwrap();
        path
    }
}

/// Sorties décodées d'une exécution du CLI.
struct Run {
    stdout: String,
    stderr: String,
    code: i32,
}

/// Exécute `sasrs <script>` avec `cwd` = dossier du script (les libnames
/// relatifs y sont résolus), avec ou sans point d'injection posé.
fn run(dir: &Path, script: &Path, fault: Option<&str>) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sasrs"));
    cmd.arg(script).current_dir(dir);
    if let Some(point) = fault {
        cmd.env("SASRS_FAULT_INJECT", point);
    }
    let Output {
        status,
        stdout,
        stderr,
    } = cmd.output().expect("lancement du binaire sasrs");
    Run {
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        code: status.code().unwrap_or_else(|| {
            panic!("sasrs tué par un signal sans code retour — stderr: {}", {
                String::from_utf8_lossy(&stderr)
            })
        }),
    }
}

/// Une exécution qui doit mourir au point d'injection : exit 86 + message
/// du point de panne sur stderr, comme le contrat `SASRS_FAULT_INJECT`.
fn run_killed(scratch: &Scratch, script_name: &str, source: &str, point: &str) {
    let script = scratch.write_script(script_name, source);
    let run = run(scratch.dir(), &script, Some(point));
    assert_eq!(
        run.code, 86,
        "l'enfant n'est pas mort au point de panne {point} — stderr: {}",
        run.stderr
    );
    assert!(
        run.stderr
            .contains(&format!("SASRS fault point hit: {point}")),
        "message du point de panne absent de stderr: {}",
        run.stderr
    );
}

/// Une exécution propre : exit 0 (aucun WARNING dans la log).
fn run_clean(scratch: &Scratch, script_name: &str, source: &str) -> Run {
    let script = scratch.write_script(script_name, source);
    let run = run(scratch.dir(), &script, None);
    assert_eq!(run.code, 0, "run propre en échec — stderr: {}", run.stderr);
    run
}

/// Temporaires orphelins (`<cible>.sasrs-tmp.<pid>`) encore présents dans
/// le dossier de données.
fn orphan_temps(scratch: &Scratch) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(scratch.data_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".sasrs-tmp"))
        .collect();
    names.sort();
    names
}

/// Lecture propre d'une table : exit 0 — aucun WARNING dans la log.
fn read_clean(scratch: &Scratch, table: &str) -> Run {
    run_clean(scratch, "read.sas", &read_program(table))
}

/// Lecture d'une table dont on ATTEND un diagnostic de sidecar : le WARNING
/// doit nommer le fichier — la fonction échoue sinon.
fn read_diagnosed(scratch: &Scratch, table: &str) -> Run {
    let run = run(
        scratch.dir(),
        &scratch.write_script("read.sas", &read_program(table)),
        None,
    );
    assert_diagnostic(&run, &format!("{table}.parquet.sasmeta.json"));
    run
}

/// Le listing des données de `d.work` (proc print) contient-il cette
/// valeur ? (les libellés/formats passent par proc contents).
fn listing_mentions(run: &Run, needle: &str) -> bool {
    run.stdout.contains(needle)
}

/// Met en place `d.work` AVEC métadonnées (sidecar frais, empreinte juste).
fn setup_with_meta(scratch: &Scratch) {
    run_clean(scratch, "setup.sas", SETUP);
    assert!(scratch.table("work").is_file());
    assert!(scratch.sidecar("work").is_file());
}

/// Le WARNING de sidecar périmé/illisible doit nommer le fichier et figurer
/// dans la log — jamais de métadonnées appliquées en silence.
fn assert_diagnostic(run: &Run, sidecar_name: &str) {
    assert!(
        run.stderr.contains("WARNING") && run.stderr.contains(sidecar_name),
        "diagnostic de sidecar attendu nommant {sidecar_name} — stderr: {}",
        run.stderr
    );
}

// ---------------------------------------------------------------- ------- //
// Écriture d'un dataset (src/dataset.rs::write_parquet)
// ---------------------------------------------------------------- ------- //

/// Première écriture tuée AVANT la publication du parquet : rien n'est
/// publié, il ne reste qu'un temporaire orphelin, et la réécriture suivante
/// purge l'orphelin et aboutit.
#[test]
fn first_write_killed_before_parquet_publish() {
    let scratch = Scratch::new();
    let write = r#"
libname d 'data';
data d.work;
  set d.class;
run;
"#;

    run_killed(&scratch, "kill.sas", write, "after_parquet_tmp");
    assert!(!scratch.table("work").exists(), "rien ne doit être publié");
    assert!(
        !orphan_temps(&scratch).is_empty(),
        "un temporaire orphelin est attendu après le kill"
    );

    // Reprise : la réécriture purge les orphelins et aboutit proprement.
    run_clean(&scratch, "recover.sas", write);
    assert!(
        orphan_temps(&scratch).is_empty(),
        "les temporaires orphelins doivent être purgés: {:?}",
        orphan_temps(&scratch)
    );
    let read = read_clean(&scratch, "work");
    assert!(listing_mentions(&read, "Alfred"), "{}", read.stdout);
}

/// Première écriture tuée APRÈS la publication du parquet : les données
/// nouvelles sont là, sans sidecar — lecture propre, zéro diagnostic
/// (pas de métadonnées, pas de mensonge).
#[test]
fn first_write_killed_after_parquet_publish() {
    let scratch = Scratch::new();
    let write = r#"
libname d 'data';
data d.work;
  set d.class;
run;
"#;

    run_killed(&scratch, "kill.sas", write, "after_parquet_rename");
    assert!(scratch.table("work").is_file());
    assert!(!scratch.sidecar("work").exists());

    let read = read_clean(&scratch, "work");
    assert!(listing_mentions(&read, "Alfred"), "{}", read.stdout);
    assert!(
        !read.stderr.contains("WARNING"),
        "aucun diagnostic attendu sans sidecar: {}",
        read.stderr
    );
}

/// Remplacement tué entre la publication du parquet et celle du sidecar :
/// les données NOUVELLES coexistent avec l'ANCIEN sidecar. Interdit de
/// l'appliquer en silence : la lecture doit diagnostiquer l'empreinte
/// périmée en nommant le sidecar, et montrer les données nouvelles.
#[test]
fn replace_killed_between_parquet_and_sidecar_is_diagnosed() {
    let scratch = Scratch::new();
    setup_with_meta(&scratch);

    run_killed(&scratch, "kill.sas", REPLACE_PLAIN, "after_parquet_rename");

    let read = read_diagnosed(&scratch, "work");
    assert_diagnostic(&read, "work.parquet.sasmeta.json");
    assert!(
        read.stderr.contains("no longer matches"),
        "la cause attendue est une empreinte périmée: {}",
        read.stderr
    );
    // Les données nouvelles sont visibles (colonne z, 3 obs) — jamais les
    // anciennes métadonnées appliquées sans dire mot.
    assert!(listing_mentions(&read, "Alfred"), "{}", read.stdout);

    // Reprise : réécrire remplace le sidecar par une empreinte fraîche —
    // plus aucun diagnostic.
    run_clean(&scratch, "recover.sas", REPLACE_WITH_META);
    let read = read_clean(&scratch, "work");
    assert!(
        !read.stderr.contains("WARNING"),
        "après réécriture, le sidecar frais ne doit plus diagnostiquer: {}",
        read.stderr
    );
    // Les NOUVELLES métadonnées s'appliquent (pas les anciennes).
    assert!(listing_mentions(&read, "Nom"), "{}", read.stdout);
    assert!(!listing_mentions(&read, "Full name"), "{}", read.stdout);
}

/// Première écriture avec métadonnées, tuée pendant la préparation du
/// sidecar : parquet publié, sidecar absent — lecture propre, et la
/// réécriture purge le temporaire orphelin du sidecar.
#[test]
fn first_write_killed_during_sidecar_prep() {
    let scratch = Scratch::new();
    let write = r#"
libname d 'data';
data d.work;
  set d.class;
  label name='Full name';
run;
"#;

    run_killed(&scratch, "kill.sas", write, "after_sidecar_tmp");
    assert!(scratch.table("work").is_file());
    assert!(!scratch.sidecar("work").exists());
    assert!(!orphan_temps(&scratch).is_empty());

    let read = read_clean(&scratch, "work");
    assert!(listing_mentions(&read, "Alfred"), "{}", read.stdout);
    assert!(
        !read.stderr.contains("WARNING"),
        "parquet sans sidecar = lecture honnête sans diagnostic: {}",
        read.stderr
    );

    run_clean(&scratch, "recover.sas", write);
    assert!(orphan_temps(&scratch).is_empty());
    assert!(scratch.sidecar("work").is_file());
}

/// Remplacement tué pendant la préparation du sidecar : parquet nouveau +
/// ancien sidecar → diagnostic d'empreinte périmée à la lecture, données
/// nouvelles cohérentes.
#[test]
fn replace_killed_during_sidecar_prep_is_diagnosed() {
    let scratch = Scratch::new();
    setup_with_meta(&scratch);

    run_killed(&scratch, "kill.sas", REPLACE_PLAIN, "after_sidecar_tmp");
    assert!(!orphan_temps(&scratch).is_empty());

    let read = read_diagnosed(&scratch, "work");
    assert_diagnostic(&read, "work.parquet.sasmeta.json");
    assert!(listing_mentions(&read, "Alfred"), "{}", read.stdout);
}

/// Écriture tuée juste APRÈS la publication du sidecar : tout est en place,
/// la lecture voit le nouvel état COMPLET (données + métadonnées fraîches,
/// aucune divergence) — pas de faux positif.
#[test]
fn write_killed_after_sidecar_publish_is_fully_consistent() {
    let scratch = Scratch::new();
    setup_with_meta(&scratch);

    run_killed(
        &scratch,
        "kill.sas",
        REPLACE_WITH_META,
        "after_sidecar_rename",
    );

    let read = read_clean(&scratch, "work");
    assert!(listing_mentions(&read, "Nom"), "{}", read.stdout);
    assert!(listing_mentions(&read, "6.1"), "{}", read.stdout);
    assert!(
        !read.stderr.contains("WARNING"),
        "état complet et cohérent: {}",
        read.stderr
    );
}

// ---------------------------------------------------------------- ------- //
// Renommage d'une table (src/library/dir.rs, `proc datasets change`)
// ---------------------------------------------------------------- ------- //

/// Renommage tué après le déplacement du parquet : la table existe au
/// NOUVEAU nom sans métadonnées (lecture propre), l'ANCIEN nom a disparu,
/// et le sidecar orphelin resté à l'ancien nom ne fait pas revivre
/// l'ancienne table.
#[test]
fn rename_killed_after_parquet_move() {
    let scratch = Scratch::new();
    setup_with_meta(&scratch);
    let prog = rename_program("work", "final");

    run_killed(&scratch, "kill.sas", &prog, "after_rename_parquet");
    assert!(scratch.table("final").is_file());
    assert!(!scratch.table("work").exists());
    assert!(
        scratch.sidecar("work").is_file(),
        "sidecar orphelin à l'ancien nom"
    );
    assert!(!scratch.sidecar("final").exists());

    let read = read_clean(&scratch, "final");
    assert!(listing_mentions(&read, "Alfred"), "{}", read.stdout);
    assert!(
        !read.stderr.contains("WARNING"),
        "données publiées sans sidecar: lecture propre attendue: {}",
        read.stderr
    );

    // Reprise : recréer l'ancien nom remplace le sidecar orphelin.
    run_clean(&scratch, "recover.sas", REPLACE_WITH_META);
    assert!(scratch.sidecar("work").is_file());
}

/// Renommage tué pendant le ROLLBACK (échec du déplacement du sidecar
/// simulé par un RÉPERTOIRE à la destination du sidecar) : l'état de départ
/// est intégralement restauré — l'ancienne table (données + métadonnées)
/// est intacte, la destination n'existe pas.
#[test]
fn rename_killed_during_rollback_restores_start_state() {
    let scratch = Scratch::new();
    setup_with_meta(&scratch);
    // Un répertoire là où le sidecar devrait aller : le rename du sidecar
    // échoue (EISDIR), le parquet est annulé, et le kill frappe après.
    let decoy = scratch.sidecar("final");
    fs::create_dir(&decoy).unwrap();

    run_killed(
        &scratch,
        "kill.sas",
        &rename_program("work", "final"),
        "after_rename_rollback",
    );

    // État de départ intact.
    assert!(scratch.table("work").is_file());
    assert!(scratch.sidecar("work").is_file());
    assert!(!scratch.table("final").exists());
    let read = read_clean(&scratch, "work");
    assert!(listing_mentions(&read, "Full name"), "{}", read.stdout);

    // Reprise : lever l'obstacle, le renommage aboutit et les métadonnées
    // suivent la table.
    fs::remove_dir(&decoy).unwrap();
    run_clean(&scratch, "recover.sas", &rename_program("work", "final"));
    assert!(scratch.table("final").is_file());
    assert!(scratch.sidecar("final").is_file());
    assert!(!scratch.table("work").exists());
    let read = read_clean(&scratch, "final");
    assert!(listing_mentions(&read, "Full name"), "{}", read.stdout);
}

/// Renommage tué juste après le déplacement du sidecar : tout est publié,
/// la table au nouveau nom porte données ET métadonnées cohérentes.
#[test]
fn rename_killed_after_sidecar_move_is_fully_consistent() {
    let scratch = Scratch::new();
    setup_with_meta(&scratch);

    run_killed(
        &scratch,
        "kill.sas",
        &rename_program("work", "final"),
        "after_rename_sidecar",
    );
    assert!(scratch.table("final").is_file());
    assert!(scratch.sidecar("final").is_file());
    assert!(!scratch.table("work").exists());

    let read = read_clean(&scratch, "final");
    assert!(listing_mentions(&read, "Full name"), "{}", read.stdout);
    assert!(
        !read.stderr.contains("WARNING"),
        "métadonnées déplacées avec la table: {}",
        read.stderr
    );
}

// ---------------------------------------------------------------- ------- //
// Round-trip données + métadonnées à travers deux sessions
// ---------------------------------------------------------------- ------- //

/// Round-trip complet : formats, libellés, longueurs déclarées et missing
/// spéciaux écrits dans une session sont relus IDENTIQUES dans une session
/// suivante (données via parquet, métadonnées via sidecar).
#[test]
fn metadata_and_special_missing_round_trip_across_sessions() {
    let scratch = Scratch::new();
    let write = r#"
libname d 'data';
data d.work;
  length name $ 12;
  set d.class;
  format height 8.2;
  label name='Full name' height='Height (in)';
  if age = 14 then x = .C; else x = age;
run;
"#;

    let first = run_clean(&scratch, "write.sas", write);
    assert!(scratch.sidecar("work").is_file(), "{}", first.stderr);

    // Session SUIVANTE : tout doit revenir à l'identique.
    let read = read_clean(&scratch, "work");
    // Formats et libellés (proc contents).
    assert!(listing_mentions(&read, "Full name"), "{}", read.stdout);
    assert!(listing_mentions(&read, "Height (in)"), "{}", read.stdout);
    assert!(listing_mentions(&read, "8.2"), "{}", read.stdout);
    // Longueur caractère déclarée (12, pas la longueur physique).
    assert!(listing_mentions(&read, "Char"), "{}", read.stdout);
    // Missing spécial .C rendu comme SAS (lettre C, pas 0 ni .).
    assert!(listing_mentions(&read, " C"), "{}", read.stdout);
    assert!(listing_mentions(&read, "Alfred"), "{}", read.stdout);
}

// ---------------------------------------------------------------- ------- //
// Sidecars corrompus : la lecture diagnostique, elle ne mentit pas
// ---------------------------------------------------------------- ------- //

/// Sidecar tronqué (octets coupés en pleine écriture simulée) : WARNING
/// nommant le fichier, données intactes.
#[test]
fn truncated_sidecar_is_diagnosed() {
    let scratch = Scratch::new();
    setup_with_meta(&scratch);
    let sc = scratch.sidecar("work");
    let bytes = fs::read(&sc).unwrap();
    fs::write(&sc, &bytes[..bytes.len() / 2]).unwrap();

    let read = read_diagnosed(&scratch, "work");
    assert_diagnostic(&read, "work.parquet.sasmeta.json");
    assert!(listing_mentions(&read, "Alfred"), "{}", read.stdout);
}

/// Sidecar rempli d'octets arbitraires (corruption disque) : WARNING
/// nommant le fichier et la cause, données intactes — jamais de crash ni
/// de métadonnées inventées.
#[test]
fn garbage_sidecar_is_diagnosed() {
    let scratch = Scratch::new();
    setup_with_meta(&scratch);
    let sc = scratch.sidecar("work");
    fs::write(&sc, [0xffu8, 0x00, 0xde, 0xad, 0xbe, 0xef]).unwrap();

    let read = read_diagnosed(&scratch, "work");
    assert_diagnostic(&read, "work.parquet.sasmeta.json");
    assert!(listing_mentions(&read, "Alfred"), "{}", read.stdout);
}

/// Sidecar JSON valide mais empreinte trafiquée (taille modifiée à la main,
/// copie d'une autre version du parquet) : traité comme périmé, avec
/// diagnostic — pas appliqué aux données.
#[test]
fn tampered_fingerprint_is_diagnosed() {
    let scratch = Scratch::new();
    setup_with_meta(&scratch);
    let sc = scratch.sidecar("work");
    let json = fs::read_to_string(&sc).unwrap();
    let patched = json
        .lines()
        .map(|line| {
            if line.contains("\"size\"") {
                line.replace(|c: char| c.is_ascii_digit(), "9")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_ne!(json, patched, "l'empreinte doit avoir été trafiquée");
    fs::write(&sc, patched).unwrap();

    let read = read_diagnosed(&scratch, "work");
    assert_diagnostic(&read, "work.parquet.sasmeta.json");
    assert!(
        read.stderr.contains("no longer matches"),
        "cause attendue: empreinte périmée: {}",
        read.stderr
    );
    assert!(listing_mentions(&read, "Alfred"), "{}", read.stdout);
}
