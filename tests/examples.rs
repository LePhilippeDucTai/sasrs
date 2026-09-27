//! J06-P3 — Tests des exemples autonomes.
//!
//! `examples/cli/analysis.sas` est exécuté via le binaire `sasrs`
//! (`env!("CARGO_BIN_EXE_sasrs")`) : le test vérifie le code retour, l'absence
//! d'ERROR dans la log, la table produite (`../out/summary.csv`, copiée hors
//! du dépôt pour ne pas polluer l'arbre de travail) et le listing attendu,
//! documenté dans l'en-tête de l'exemple.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Racine du dépôt (celle du manifeste, où vit `examples/`).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Normalise un champ CSV : toute valeur numérique est réimprimée via
/// `f64` (33.0 → "33") pour être insensible au choix d'écriture "33"/"33.0".
fn normalize_field(field: &str) -> String {
    field
        .trim()
        .parse::<f64>()
        .map(|v| {
            if v.fract() == 0.0 && v.is_finite() {
                format!("{}", v as i64)
            } else {
                format!("{v}")
            }
        })
        .unwrap_or_else(|_| field.trim().to_string())
}

/// Normalise une ligne CSV complète (champs séparés par des virgules).
fn normalize_line(line: &str) -> String {
    line.split(',')
        .map(normalize_field)
        .collect::<Vec<_>>()
        .join(",")
}

#[test]
fn cli_example_runs_and_produces_the_summary_table() {
    let root = repo_root();

    // Miroir de la disposition examples/{cli,data} dans un tempdir : les
    // chemins relatifs de l'exemple résolvent comme dans le dépôt, mais la
    // sortie est écrite hors de l'arbre de travail.
    let tmp = tempfile::tempdir().unwrap();
    let cli_dir = tmp.path().join("examples").join("cli");
    let data_dir = tmp.path().join("examples").join("data");
    let out_dir = tmp.path().join("examples").join("out");
    fs::create_dir_all(&cli_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&out_dir).unwrap();
    fs::copy(
        root.join("examples/cli/analysis.sas"),
        cli_dir.join("analysis.sas"),
    )
    .unwrap();
    fs::copy(
        root.join("examples/data/patients.csv"),
        data_dir.join("patients.csv"),
    )
    .unwrap();

    let script = cli_dir.join("analysis.sas");
    let out = Command::new(env!("CARGO_BIN_EXE_sasrs"))
        .arg("--deterministic")
        .arg(&script)
        .current_dir(tmp.path())
        .output()
        .expect("impossible de lancer le binaire sasrs");

    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();

    // Code retour 0 : programme propre (contrat documenté dans l'exemple).
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr :\n{stderr}\nstdout :\n{stdout}"
    );
    assert!(!stderr.contains("ERROR:"), "log inattendue :\n{stderr}");

    // La table produite : summary.csv avec en-tête + une ligne par sexe.
    let summary_path = out_dir.join("summary.csv");
    let summary = fs::read_to_string(&summary_path)
        .unwrap_or_else(|e| panic!("summary.csv illisible ({e}) : stderr :\n{stderr}"));
    let lines: Vec<String> = summary.lines().map(normalize_line).collect();
    assert_eq!(lines.len(), 3, "summary.csv inattendu :\n{summary}");
    assert_eq!(lines[0].to_ascii_uppercase(), "SEX,N_PATIENTS,MEAN_AGE");
    let sorted = {
        let mut rest = lines[1..].to_vec();
        rest.sort();
        rest
    };
    assert_eq!(sorted, vec!["F,3,33".to_string(), "M,2,48".to_string()]);

    // Le listing (stdout) montre le résumé des deux groupes.
    assert!(
        stdout.contains("F") && stdout.contains("M"),
        "listing sans les lignes du résumé :\n{stdout}"
    );
    assert!(
        stdout.contains("Resume des patients par sexe"),
        "titre attendu dans le listing :\n{stdout}"
    );
}

/// Propriété de non-régression : le test principal est DISCRIMINANT —
/// si la donnée d'entrée change (ici, un seul sexe), la table produite
/// change aussi, donc les assertions de contenu exact ci-dessus ne
/// passent pas par accident.
#[test]
fn cli_example_fails_on_broken_input() {
    let root = repo_root();
    let tmp = tempfile::tempdir().unwrap();
    let cli_dir = tmp.path().join("examples").join("cli");
    let data_dir = tmp.path().join("examples").join("data");
    let out_dir = tmp.path().join("examples").join("out");
    fs::create_dir_all(&cli_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&out_dir).unwrap();
    fs::copy(
        root.join("examples/cli/analysis.sas"),
        cli_dir.join("analysis.sas"),
    )
    .unwrap();
    // Données d'entrée modifiées : un seul sexe (F). La table est produite,
    // mais DIFFÈRE de la sortie attendue de l'exemple : le test principal
    // (contenu exact de summary.csv) est bien discriminant — il échoue si
    // la donnée change, donc il ne passe pas par accident.
    fs::write(
        data_dir.join("patients.csv"),
        "Name,Sex,Age,Weight\nAlice,F,32,55.5\nBob,F,45,80.0\n",
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_sasrs"))
        .arg("--deterministic")
        .arg(cli_dir.join("analysis.sas"))
        .current_dir(tmp.path())
        .output()
        .expect("impossible de lancer le binaire sasrs");
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr :\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let summary = fs::read_to_string(out_dir.join("summary.csv")).unwrap();
    let lines: Vec<String> = summary.lines().map(normalize_line).collect();
    assert_ne!(
        lines,
        vec![
            "Sex,N_PATIENTS,MEAN_AGE".to_string(),
            "F,3,33".to_string(),
            "M,2,48".to_string()
        ],
        "la sortie doit refléter la donnée modifiée, pas la sortie attendue"
    );
}

/// Vérification de cohérence du normalisateur (utilisé ci-dessus).
#[test]
fn normalize_field_handles_numeric_and_text() {
    assert_eq!(normalize_line("F,3,33"), "F,3,33");
    assert_eq!(normalize_line("F,3,33.0"), "F,3,33");
    assert_eq!(normalize_line("Name,55.5"), "Name,55.5");
}
