//! J08-P6 — programme E2E de migration, exécuté PAR LE BINAIRE (contrat CLI
//! réel, cf. `tests/cli.rs`) ET via la façade `sasrs::api`.
//!
//! Le scénario (cf. [`migration`]) : import CSV + import/export XLSX →
//! étape DATA → SORT → MEANS NWAY avec `OUTPUT` → TRANSPOSE → COMPARE contre
//! une table attendue (formule BMI publiée : 703 × lb / in²) → ODS OUTPUT
//! (`Summary` de MEANS) → EXPORT XLSX → re-import. Codes retour, absence
//! d'ERROR/WARNING, diagnostics attendus et valeurs des tables produites sont
//! vérifiés sur les deux voies.

mod migration;

use polars::prelude::*;
use std::path::Path;
use std::process::Command;

use migration::{
    EXPECTED_MEAN_F, EXPECTED_MEAN_M, EXPECTED_N_F, EXPECTED_N_M, PROGRAM,
    PROGRAM_BROKEN_EXPECTATION, write_patients_csv, write_program,
};

/// Une exécution du binaire : sorties décodées + code retour.
struct CliRun {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

impl CliRun {
    fn code(&self) -> i32 {
        self.code
            .expect("sasrs terminé par un signal, sans code retour")
    }
}

/// Exécute `sasrs <args…>` avec `cwd` comme répertoire courant du processus.
/// `env!("CARGO_BIN_EXE_sasrs")` garantit que le binaire est reconstruit
/// avant les tests d'intégration et pointe sur lui.
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

/// Lit une table WORK persistée par `--work <dir>` (`<table>.parquet`).
fn read_work_table(work_dir: &Path, table: &str) -> DataFrame {
    let path = work_dir.join(format!("{table}.parquet"));
    let file = std::fs::File::open(&path)
        .unwrap_or_else(|e| panic!("table {table} introuvable ({path:?}) : {e}"));
    ParquetReader::new(file)
        .finish()
        .unwrap_or_else(|e| panic!("lecture parquet de {table} : {e}"))
}

/// Résout une colonne sans sensibilité à la casse (le stockage sasrs garde
/// la casse canonique de la variable, ex. `Sex`).
fn find_column(df: &DataFrame, name: &str) -> PolarsResult<Series> {
    let upper = name.to_ascii_uppercase();
    match df
        .get_column_names()
        .iter()
        .find(|c| c.to_ascii_uppercase() == upper)
    {
        Some(found) => df.column(found).map(|c| c.as_materialized_series().clone()),
        None => df.column(name).map(|c| c.as_materialized_series().clone()),
    }
}

/// Valeur d'une colonne numérique, colonne cherchée sans sensibilité à la
/// casse.
fn num_col(df: &DataFrame, name: &str) -> Vec<f64> {
    let series = find_column(df, name)
        .unwrap_or_else(|e| panic!("colonne {name} absente : {e}"))
        .cast(&DataType::Float64)
        .unwrap();
    series.f64().unwrap().into_no_null_iter().collect()
}

/// Valeur d'une colonne caractère (casse-insensible), valeurs en majuscules.
fn char_col_upper(df: &DataFrame, name: &str) -> Vec<String> {
    let series = find_column(df, name)
        .unwrap_or_else(|e| panic!("colonne {name} absente : {e}"))
        .cast(&DataType::String)
        .unwrap();
    series
        .str()
        .unwrap()
        .into_no_null_iter()
        .map(|v| v.to_ascii_uppercase())
        .collect()
}

fn assert_close(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "{what} : {actual} ≠ {expected}"
    );
}

/// Vérifie `work.bmi_stats` (MEANS NWAY OUTPUT) ligne par ligne.
fn assert_bmi_stats(df: &DataFrame) {
    assert_eq!(df.height(), 2, "NWAY : une seule ligne par valeur de SEX");
    let sex = char_col_upper(df, "sex");
    let means = num_col(df, "bmi_mean");
    let ns = num_col(df, "bmi_n");
    for (index, group) in sex.iter().enumerate() {
        match group.as_str() {
            "F" => {
                assert_close(means[index], EXPECTED_MEAN_F, "BMI_MEAN (F)");
                assert_close(ns[index], EXPECTED_N_F, "BMI_N (F)");
            }
            "M" => {
                assert_close(means[index], EXPECTED_MEAN_M, "BMI_MEAN (M)");
                assert_close(ns[index], EXPECTED_N_M, "BMI_N (M)");
            }
            other => panic!("groupe SEX inattendu : {other}"),
        }
    }
}

/// Vérifie `work.bmi_wide` (TRANSPOSE avec ID sex) ligne par ligne.
fn assert_bmi_wide(df: &DataFrame) {
    assert_eq!(df.height(), 2);
    let names = char_col_upper(df, "_name_");
    let f = num_col(df, "f");
    let m = num_col(df, "m");
    assert!(
        names.contains(&"BMI_MEAN".to_string()) && names.contains(&"BMI_N".to_string()),
        "_NAME_ inattendu : {names:?}"
    );
    for (index, row) in names.iter().enumerate() {
        if row == "BMI_MEAN" {
            assert_close(f[index], EXPECTED_MEAN_F, "BMI_WIDE F (BMI_MEAN)");
            assert_close(m[index], EXPECTED_MEAN_M, "BMI_WIDE M (BMI_MEAN)");
        } else {
            assert_close(f[index], EXPECTED_N_F, "BMI_WIDE F (BMI_N)");
            assert_close(m[index], EXPECTED_N_M, "BMI_WIDE M (BMI_N)");
        }
    }
}

fn assert_no_error_or_warning(log: &str, context: &str) {
    for line in log.lines() {
        assert!(
            !line.contains("ERROR:") && !line.contains("WARNING:"),
            "{context} : la log ne doit contenir ni ERROR ni WARNING :\n{line}"
        );
    }
}

/// Vérifie la log d'une migration réussie : diagnostics attendus (NOTE
/// « no unequal values » des deux COMPARE, table ODS capturée).
fn assert_migration_diagnostics(log: &str) {
    let equal_notes = log
        .lines()
        .filter(|l| l.contains("No unequal values were found"))
        .count();
    assert_eq!(
        equal_notes, 2,
        "les deux COMPARE doivent conclure à l'égalité exacte :\n{log}"
    );
    assert!(
        log.to_ascii_uppercase().contains("SUMMARY_CAPTURE"),
        "la capture ODS OUTPUT Summary=summary_capture doit être notée :\n{log}"
    );
}

/// Vérifie les tables produites par le programme de migration dans `work`.
fn assert_migration_tables(work_dir: &Path) {
    let stats = read_work_table(work_dir, "bmi_stats");
    assert_bmi_stats(&stats);

    let wide = read_work_table(work_dir, "bmi_wide");
    assert_bmi_wide(&wide);

    // COMPARE avec OUTNOEQUAL : aucune différence → tables de sortie vides.
    for table in ["wide_diffs", "back_diffs"] {
        let diffs = read_work_table(work_dir, table);
        assert_eq!(
            diffs.height(),
            0,
            "{table} doit être vide (aucune valeur jugée inégale)"
        );
    }

    // ODS OUTPUT Summary : une ligne par groupe BY-class (F et M).
    let capture = read_work_table(work_dir, "summary_capture");
    assert_eq!(
        capture.height(),
        2,
        "ODS OUTPUT Summary : une ligne par groupe SEX"
    );

    // Aller-retour XLSX du résultat : valeurs identiques à la source.
    let back = read_work_table(work_dir, "bmi_wide_back");
    assert_bmi_wide(&back);

    // La table d'enrichissement porte bien la région importée du XLSX.
    let enriched = read_work_table(work_dir, "enriched");
    let regions = char_col_upper(&enriched, "region");
    assert_eq!(regions.len(), 8, "les 8 observations enrichies");
    assert!(regions.iter().all(|r| r == "NORTH" || r == "SOUTH"));
}

#[test]
fn cli_binary_full_migration_run() {
    let tmp = tempfile::tempdir().unwrap();
    let script_dir = tmp.path().join("scripts");
    write_patients_csv(&script_dir);
    write_program(&script_dir, "migration.sas", PROGRAM);
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();

    let run = sasrs(
        &script_dir,
        &[
            "migration.sas",
            "--work",
            work.to_str().unwrap(),
            "--deterministic",
            "--log",
            "migration.log",
            "--print",
            "migration.lst",
        ],
    );
    assert_eq!(
        run.code(),
        0,
        "code retour CLI de la migration ; stderr :\n{}",
        run.stderr
    );
    let log = std::fs::read_to_string(script_dir.join("migration.log")).unwrap();
    assert_no_error_or_warning(&log, "migration CLI");
    assert_migration_diagnostics(&log);
    assert_migration_tables(&work);

    // Le listing (stdout par défaut, ici --print) porte l'exécution batch.
    assert!(
        !run.stdout.is_empty()
            || std::fs::read_to_string(script_dir.join("migration.lst"))
                .unwrap()
                .contains("The SAS System"),
        "le listing doit être produit"
    );

    // Les classeurs XLSX intermédiaires ont bien été écrits à côté du script.
    assert!(script_dir.join("lookup.xlsx").is_file());
    assert!(script_dir.join("bmi_wide.xlsx").is_file());
}

#[test]
fn api_facade_full_migration_run() {
    let tmp = tempfile::tempdir().unwrap();
    write_patients_csv(tmp.path());
    write_program(tmp.path(), "migration.sas", PROGRAM);
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();

    let mut session = sasrs::api::Session::new(sasrs::api::Options {
        base_dir: Some(tmp.path().to_path_buf()),
        work_dir: Some(work.clone()),
        deterministic: true,
        ..sasrs::api::Options::default()
    })
    .expect("la session doit s'initialiser");

    let submission = session.submit(PROGRAM);
    assert_eq!(
        submission.exit_code, 0,
        "code retour api de la migration ; log :\n{}",
        submission.log
    );
    assert_eq!(submission.errors, 0, "log :\n{}", submission.log);
    assert_eq!(submission.warnings, 0, "log :\n{}", submission.log);
    assert_no_error_or_warning(&submission.log, "migration api");
    assert_migration_diagnostics(&submission.log);

    assert_migration_tables(&work);

    // Relecture par la façade : mêmes tables, métadonnées présentes.
    let (stats, vars) = session.dataset("work", "bmi_stats").unwrap();
    assert_eq!(vars.len(), 5, "_TYPE_ _FREQ_ SEX BMI_MEAN BMI_N");
    assert_bmi_stats(&stats);
    let (wide, _) = session.dataset("work", "bmi_wide").unwrap();
    assert_bmi_wide(&wide);
    let (capture, _) = session.dataset("work", "summary_capture").unwrap();
    assert_eq!(capture.height(), 2);
    let (diffs, _) = session.dataset("work", "wide_diffs").unwrap();
    assert_eq!(diffs.height(), 0);

    // Diagnostics structurés : aucun Warning/Error sur la soumission.
    assert!(
        submission.diagnostics.iter().all(|d| !matches!(
            d.severity,
            sasrs::api::Severity::Warning | sasrs::api::Severity::Error
        )),
        "aucun diagnostic Warning/Error attendu : {:?}",
        submission.diagnostics
    );

    let report = session.close();
    assert_eq!(report.exit_code, 0);
}

#[test]
fn compare_detects_injected_difference() {
    // Propriété (CONTRIBUTING.md) : le COMPARE du scénario prouve l'égalité —
    // l'attendu volontairement faux DOIT produire des lignes dans OUT= avec
    // OUTNOEQUAL, et le diagnostic « unequal » doit apparaître.
    let tmp = tempfile::tempdir().unwrap();
    write_patients_csv(tmp.path());
    write_program(tmp.path(), "broken.sas", PROGRAM_BROKEN_EXPECTATION);
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();

    let run = sasrs(
        tmp.path(),
        &[
            "broken.sas",
            "--work",
            work.to_str().unwrap(),
            "--deterministic",
            "--log",
            "broken.log",
            "--print",
            "broken.lst",
        ],
    );
    assert_eq!(run.code(), 0, "COMPARE signale, il ne fait pas planter");
    let log = std::fs::read_to_string(tmp.path().join("broken.log")).unwrap();
    assert!(
        log.contains("unequal"),
        "la log doit signaler les valeurs inégales :\n{log}"
    );
    let diffs = read_work_table(&work, "wide_diffs");
    assert!(
        diffs.height() > 0,
        "OUTNOEQUAL doit écrire les paires jugées inégales"
    );
}

#[test]
fn cli_missing_csv_is_an_error() {
    // Diagnostics d'échec : un fichier d'import absent → ERROR + code 2.
    let tmp = tempfile::tempdir().unwrap();
    write_program(tmp.path(), "missing.sas", PROGRAM);
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();

    let run = sasrs(
        tmp.path(),
        &[
            "missing.sas",
            "--work",
            work.to_str().unwrap(),
            "--deterministic",
        ],
    );
    assert_eq!(run.code(), 2, "fichier d'import absent doit donner 2");
    let combined = format!("{}{}", run.stderr, run.stdout);
    assert!(
        combined.contains("ERROR"),
        "une ERROR explicite est attendue :\n{combined}"
    );
}
