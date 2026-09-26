//! Exécuteur du corpus de conformité (J05-P1).
//!
//! Chaque cas vit dans `conformance/cases/<groupe>/<id>/` (schéma complet
//! dans `conformance/schema.md`) :
//!
//! - `program.sas` — le programme exécuté ;
//! - `data/*.csv` — entrées, converties en parquet par l'exécuteur avec
//!   les types déclarés dans `case.json` (les missings `.`/`._`/`.A`..`.Z`
//!   sont encodés comme missing SAS, pas comme du texte) ;
//! - `expected/<ds>.csv` — datasets attendus, comparés aux tables WORK
//!   produites par le programme (relues via l'API publique
//!   `sasrs::dataset`, pas par un lecteur parquet ad hoc) ;
//! - `case.json` — métadonnées, tolérances, attentes de log.
//!
//! Un cas `validated` doit passer TOUTES les vérifications. Un cas
//! `known-divergence` doit en ÉCHOUER au moins une : s'il passe, le test
//! signale « à promouver » et échoue (la promotion est une décision
//! humaine, cf. CONTRIBUTING.md §2).

use polars::prelude::*;
use sasrs::dataset::SasDataset;
use sasrs::missing::{decode_nan, encode_special};
use sasrs::value::{MissingKind, VarType};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// ── Schéma case.json (sous-ensemble exécuté ; conformance/schema.md fait
//    autorité pour la description complète) ─────────────────────────────

#[derive(Deserialize)]
struct CaseSpec {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    provenance: Provenance,
    validates: String,
    #[serde(default)]
    tolerance: TolSpec,
    #[serde(default)]
    log: LogSpec,
    #[serde(default)]
    exit_code: Option<i32>,
    status: String,
    /// Fichier CSV (sous `data/`) → colonne → `num`|`char`.
    #[serde(default)]
    data_types: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    issue: Option<String>,
}

#[derive(Deserialize, Default)]
struct Provenance {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    sas_version: String,
    #[serde(default)]
    options: Vec<String>,
}

#[derive(Deserialize, Default)]
struct TolSpec {
    #[serde(default = "default_tol")]
    abs: f64,
    #[serde(default = "default_tol")]
    rel: f64,
    #[serde(default)]
    columns: BTreeMap<String, ColumnTol>,
}

#[derive(Deserialize, Default)]
struct ColumnTol {
    #[serde(default = "default_tol")]
    abs: f64,
    #[serde(default = "default_tol")]
    rel: f64,
}

fn default_tol() -> f64 {
    1e-9
}

#[derive(Deserialize, Default)]
struct LogSpec {
    #[serde(default)]
    required: Vec<String>,
    /// Motifs (regex fancy-regex) qui ne doivent PAS apparaître dans la log.
    #[serde(default)]
    forbidden: Vec<String>,
}

struct Case {
    dir: PathBuf,
    spec: CaseSpec,
}

// ── Cellules numériques avec missings SAS ──────────────────────────────

/// Une valeur numérique attendue/lue : missing (ordinaire ou spécial) ou f64.
#[derive(Debug, PartialEq)]
enum NumCell {
    Missing(MissingKind),
    Value(f64),
}

/// Analyse une cellule CSV comme SAS le ferait : `.`/vide = missing
/// ordinaire, `._`/`.A`..`.Z` = missings spéciaux, sinon un flottant.
fn parse_num_cell(dataset: &str, column: &str, cell: &str) -> Result<NumCell, String> {
    let cell = cell.trim();
    if cell.is_empty() || cell == "." {
        return Ok(NumCell::Missing(MissingKind::Dot));
    }
    if cell == "._" {
        return Ok(NumCell::Missing(MissingKind::Underscore));
    }
    if cell.len() == 2
        && cell.starts_with('.')
        && let Some(kind) = cell.chars().nth(1).and_then(MissingKind::from_letter)
    {
        return Ok(NumCell::Missing(kind));
    }
    cell.parse::<f64>()
        .map(NumCell::Value)
        .map_err(|e| format!("{dataset}.{column} : cellule « {cell} » non numérique ({e})"))
}

/// Cellule du parquet WORK → valeur SAS : null = `.`, NaN à payload =
/// missing spécial décodé par l'API publique du crate.
fn actual_num(v: Option<f64>) -> NumCell {
    match v {
        None => NumCell::Missing(MissingKind::Dot),
        Some(f) if f.is_nan() => NumCell::Missing(decode_nan(f)),
        Some(f) => NumCell::Value(f),
    }
}

fn kind_label(k: MissingKind) -> String {
    match k {
        MissingKind::Dot => ".".to_string(),
        MissingKind::Underscore => "._".to_string(),
        MissingKind::Letter(i) => format!(".{}", (b'A' + i) as char),
    }
}

fn close_enough(got: f64, want: f64, abs: f64, rel: f64) -> bool {
    (got - want).abs() <= abs || (got - want).abs() <= rel * want.abs().max(got.abs())
}

// ── Découverte du corpus ───────────────────────────────────────────────

fn collect_cases(root: &Path) -> Vec<Case> {
    let mut cases = Vec::new();
    let Ok(groups) = fs::read_dir(root) else {
        return cases;
    };
    for group in groups {
        let group = group.expect("lecture du groupe").path();
        if !group.is_dir() {
            continue;
        }
        for case_dir in fs::read_dir(&group).expect("lecture des cas") {
            let case_dir = case_dir.expect("lecture du cas").path();
            let case_json = case_dir.join("case.json");
            if !case_json.is_file() {
                continue;
            }
            let text = fs::read_to_string(&case_json)
                .unwrap_or_else(|e| panic!("lecture de {} : {e}", case_json.display()));
            let spec: CaseSpec = serde_json::from_str(&text)
                .unwrap_or_else(|e| panic!("case.json invalide ({}) : {e}", case_json.display()));
            cases.push(Case {
                dir: case_dir,
                spec,
            });
        }
    }
    cases.sort_by(|a, b| a.dir.cmp(&b.dir));
    cases
}

// ── Exécution d'un cas ─────────────────────────────────────────────────

/// Convertit un CSV d'entrée en parquet `<tmp>/data/<stem>.parquet` avec
/// les types déclarés dans `case.json` (colonne `num` → f64 encodant les
/// missings spéciaux comme NaN à payload, colonne `char` → String).
fn convert_inputs(case: &Case, data_dir: &Path) -> Result<(), String> {
    let src_dir = case.dir.join("data");
    if !src_dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&src_dir).map_err(|e| e.to_string())? {
        let csv_path = entry.map_err(|e| e.to_string())?.path();
        if csv_path.extension().is_some_and(|e| e == "csv") {
            let name = csv_path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("nom de fichier non-UTF8")?
                .to_string();
            let types =
                case.spec.data_types.get(&name).ok_or_else(|| {
                    format!("data/{name} : types absents de case.json (data_types)")
                })?;
            write_parquet_from_csv(&csv_path, data_dir, &name, types)?;
        }
    }
    Ok(())
}

fn write_parquet_from_csv(
    csv_path: &Path,
    data_dir: &Path,
    name: &str,
    types: &BTreeMap<String, String>,
) -> Result<(), String> {
    let text = fs::read_to_string(csv_path).map_err(|e| e.to_string())?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header: Vec<String> = lines
        .next()
        .ok_or_else(|| format!("data/{name} : CSV vide"))?
        .split(',')
        .map(|c| c.trim().to_string())
        .collect();
    let rows: Vec<Vec<String>> = lines
        .map(|l| l.split(',').map(|c| c.trim().to_string()).collect())
        .collect();

    let ctx = format!("data/{name}");
    let mut columns: Vec<Column> = Vec::with_capacity(header.len());
    for (j, col) in header.iter().enumerate() {
        let ty = types
            .get(col)
            .ok_or_else(|| format!("{ctx} : type manquant pour la colonne « {col} »"))?;
        match ty.as_str() {
            "num" => {
                let mut vals = Vec::with_capacity(rows.len());
                for (i, row) in rows.iter().enumerate() {
                    let cell = row.get(j).map(String::as_str).unwrap_or("");
                    match parse_num_cell(&ctx, col, cell)
                        .map_err(|e| format!("ligne {} : {e}", i + 2))?
                    {
                        NumCell::Missing(MissingKind::Dot) => vals.push(None),
                        NumCell::Missing(k) => vals.push(Some(encode_special(k))),
                        NumCell::Value(f) => vals.push(Some(f)),
                    }
                }
                columns.push(Series::new(col.clone().into(), vals).into());
            }
            "char" => {
                let vals: Vec<String> = rows
                    .iter()
                    .map(|row| row.get(j).cloned().unwrap_or_default())
                    .collect();
                columns.push(Series::new(col.clone().into(), vals).into());
            }
            other => return Err(format!("{ctx} : type inconnu « {other} » (num|char)")),
        }
    }
    let mut df = DataFrame::new(columns).map_err(|e| format!("{ctx} : {e}"))?;
    let target = data_dir.join(format!(
        "{}.parquet",
        csv_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
    ));
    let mut file = fs::File::create(&target).map_err(|e| format!("{ctx} : {e}"))?;
    ParquetWriter::new(&mut file)
        .finish(&mut df)
        .map_err(|e| format!("{ctx} : {e}"))?;
    Ok(())
}

/// Toutes les vérifications d'un cas ; rend la liste des problèmes
/// (vide = cas passé).
fn run_case(case: &Case) -> Vec<String> {
    let mut problems = Vec::new();

    // L'id doit correspondre au nom du répertoire du cas.
    if let Some(dir_name) = case.dir.file_name().and_then(|n| n.to_str())
        && case.spec.id != dir_name
    {
        problems.push(format!(
            "id « {} » ≠ nom du répertoire « {dir_name} »",
            case.spec.id
        ));
    }

    // Statut et validates : vocabulaire fermé.
    if !matches!(case.spec.status.as_str(), "validated" | "known-divergence") {
        problems.push(format!("status invalide : {}", case.spec.status));
    }
    if !matches!(case.spec.validates.as_str(), "math" | "sas-behaviour") {
        problems.push(format!("validates invalide : {}", case.spec.validates));
    }
    if case.spec.status == "known-divergence" && case.spec.issue.is_none() {
        problems.push("known-divergence sans « issue »".to_string());
    }
    if !matches!(
        case.spec.provenance.kind.as_str(),
        "sas-doc" | "sas-run" | "independent-oracle"
    ) {
        problems.push(format!(
            "provenance.kind invalide : {}",
            case.spec.provenance.kind
        ));
    }
    if case.spec.provenance.source.trim().is_empty() {
        problems.push("provenance.source absent".to_string());
    }
    if case.spec.provenance.sas_version.trim().is_empty() {
        problems.push("provenance.sas_version absent".to_string());
    }
    // L'exécuteur ne gère qu'un jeu d'options fermé : toute option
    // déclarée doit y figurer, sinon le cas s'exécute différemment de sa
    // définition.
    for opt in &case.spec.provenance.options {
        if opt != "--deterministic" {
            problems.push(format!(
                "provenance.options : « {opt} » non géré par l'exécuteur"
            ));
        }
    }

    // Préparation du bac à sable : program.sas + data/*.csv → parquet.
    let tmp = tempfile::tempdir().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let work_dir = tmp.path().join("work");
    if let Err(e) = fs::create_dir_all(&data_dir) {
        problems.push(format!("création de data/ : {e}"));
        return problems;
    }
    if let Err(e) = fs::create_dir_all(&work_dir) {
        problems.push(format!("création de work/ : {e}"));
        return problems;
    }
    let program = tmp.path().join("program.sas");
    if let Err(e) = fs::copy(case.dir.join("program.sas"), &program) {
        problems.push(format!("copie de program.sas : {e}"));
        return problems;
    }
    if let Err(e) = convert_inputs(case, &data_dir) {
        problems.push(format!("conversion des entrées : {e}"));
        return problems;
    }

    // Exécution du binaire (motif établi de tests/storage_integrity.rs).
    let log_file = tmp.path().join("run.log");
    let print_file = tmp.path().join("run.lst");
    let output = Command::new(env!("CARGO_BIN_EXE_sasrs"))
        .arg("--work")
        .arg(&work_dir)
        .arg("--log")
        .arg(&log_file)
        .arg("--print")
        .arg(&print_file)
        .arg("--deterministic")
        .arg(&program)
        .current_dir(tmp.path())
        .output()
        .expect("lancement du binaire sasrs");
    let code = output.status.code().unwrap_or(-1);
    let expected_code = case.spec.exit_code.unwrap_or(0);
    if code != expected_code {
        problems.push(format!(
            "code retour {code}, attendu {expected_code} — stderr : {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    // Log : requise = sous-chaînes ; interdites = regex fancy-regex.
    let log = fs::read_to_string(&log_file)
        .unwrap_or_else(|_| String::from_utf8_lossy(&output.stderr).into_owned());
    for req in &case.spec.log.required {
        if !log.contains(req.as_str()) {
            problems.push(format!("log : ligne requise absente « {req} »"));
        }
    }
    for pattern in &case.spec.log.forbidden {
        match fancy_regex::Regex::new(pattern) {
            Ok(re) => {
                if re.is_match(&log).unwrap_or(false) {
                    problems.push(format!("log : motif interdit présent « {pattern} »"));
                }
            }
            Err(e) => problems.push(format!(
                "log.forbidden : regex invalide « {pattern} » ({e})"
            )),
        }
    }

    // Datasets attendus vs tables WORK (relecture via l'API publique).
    let expected_dir = case.dir.join("expected");
    let mut expected_csvs: Vec<PathBuf> = match fs::read_dir(&expected_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "csv"))
            .collect(),
        Err(_) => Vec::new(),
    };
    expected_csvs.sort();
    if expected_csvs.is_empty() {
        problems.push("aucun dataset attendu (expected/*.csv vide ou absent)".to_string());
    }
    for csv in &expected_csvs {
        let table = csv.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        let parquet = work_dir.join(format!("{table}.parquet"));
        let text = match fs::read_to_string(csv) {
            Ok(t) => t,
            Err(e) => {
                problems.push(format!("expected/{table}.csv illisible : {e}"));
                continue;
            }
        };
        if !parquet.is_file() {
            problems.push(format!(
                "WORK.{table} absent du WORK après exécution ({})",
                parquet.display()
            ));
            continue;
        }
        match SasDataset::read_parquet(&parquet) {
            Ok((ds, _notes)) => {
                let n = compare_dataset(&text, &ds, &case.spec.tolerance);
                for diff in n {
                    problems.push(format!("WORK.{table} : {diff}"));
                }
            }
            Err(e) => problems.push(format!("WORK.{table} : relecture impossible : {e}")),
        }
    }

    problems
}

// ── Comparaison dataset attendu / dataset produit ──────────────────────

fn compare_dataset(expected_csv: &str, ds: &SasDataset, tol: &TolSpec) -> Vec<String> {
    let mut diffs = Vec::new();
    let mut lines = expected_csv.lines().filter(|l| !l.trim().is_empty());
    let header: Vec<String> = match lines.next() {
        Some(h) => h.split(',').map(|c| c.trim().to_uppercase()).collect(),
        None => {
            diffs.push("CSV attendu vide".to_string());
            return diffs;
        }
    };

    // Colonnes : mêmes noms, même ordre (ordre de compilation SAS).
    let actual: Vec<String> = ds.vars.iter().map(|v| v.name.to_uppercase()).collect();
    if actual != header {
        diffs.push(format!(
            "colonnes : produites {actual:?}, attendues {header:?}"
        ));
        return diffs;
    }

    let rows: Vec<Vec<String>> = lines
        .map(|l| l.split(',').map(|c| c.trim().to_string()).collect())
        .collect();
    if rows.len() != ds.n_obs() {
        diffs.push(format!(
            "observations : {} produites, {} attendues",
            ds.n_obs(),
            rows.len()
        ));
        return diffs;
    }

    for (j, var) in ds.vars.iter().enumerate() {
        let col_tol = tol.columns.get(&var.name.to_uppercase());
        let (abs, rel) = match col_tol {
            Some(ct) => (ct.abs, ct.rel),
            None => (tol.abs, tol.rel),
        };
        for (i, row) in rows.iter().enumerate() {
            let cell = row.get(j).cloned().unwrap_or_default();
            let at = format!("obs {}, colonne {} : ", i + 1, var.name);
            match var.ty {
                VarType::Num => {
                    let got = ds.df.get_columns()[j]
                        .as_materialized_series()
                        .f64()
                        .expect("colonne numérique")
                        .get(i);
                    match (parse_num_cell("WORK", &var.name, &cell), actual_num(got)) {
                        (Ok(a), b) => match (&a, &b) {
                            (NumCell::Missing(k1), NumCell::Missing(k2)) => {
                                if k1 != k2 {
                                    diffs.push(format!(
                                        "{at}missing {} attendu, {} produit",
                                        kind_label(*k1),
                                        kind_label(*k2)
                                    ));
                                }
                            }
                            (NumCell::Missing(k), NumCell::Value(v)) => {
                                diffs.push(format!(
                                    "{at}missing {} attendu, valeur {v} produite",
                                    kind_label(*k)
                                ));
                            }
                            (NumCell::Value(v), NumCell::Missing(k)) => {
                                diffs.push(format!(
                                    "{at}valeur {v} attendue, missing {} produit",
                                    kind_label(*k)
                                ));
                            }
                            (NumCell::Value(want), NumCell::Value(got_v)) => {
                                if !close_enough(*got_v, *want, abs, rel) {
                                    diffs.push(format!(
                                        "{at}{got_v} produit, {want} attendu (|écarts| > abs {abs} / rel {rel})"
                                    ));
                                }
                            }
                        },
                        (Err(e), _) => diffs.push(format!("{at}attendu illisible : {e}")),
                    }
                }
                VarType::Char => {
                    let got = ds.df.get_columns()[j]
                        .as_materialized_series()
                        .str()
                        .expect("colonne caractère")
                        .get(i);
                    let got_trimmed = got.unwrap_or_default().trim_end();
                    if got_trimmed != cell.trim_end() {
                        diffs.push(format!(
                            "{at}« {got_trimmed} » produit, « {} » attendu",
                            cell.trim_end()
                        ));
                    }
                }
            }
        }
    }
    diffs
}

// ── Test agrégé : TOUT le corpus, rapport par cas ──────────────────────

#[test]
fn conformance_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("conformance")
        .join("cases");
    let cases = collect_cases(&root);
    assert!(
        !cases.is_empty(),
        "corpus vide : aucun case.json sous {}",
        root.display()
    );

    let mut failures: Vec<String> = Vec::new();
    let mut report = String::from("Rapport du corpus de conformité :\n");
    for case in &cases {
        let rel = case
            .dir
            .strip_prefix(root.parent().unwrap().parent().unwrap())
            .unwrap_or(&case.dir)
            .display();
        let problems = run_case(case);
        match (case.spec.status.as_str(), problems.is_empty()) {
            ("validated", true) => {
                report.push_str(&format!("  PASS  {rel} — {}\n", case.spec.title));
            }
            ("validated", false) => {
                report.push_str(&format!("  FAIL  {rel} — {}\n", case.spec.title));
                failures.extend(
                    problems
                        .into_iter()
                        .map(|p| format!("{rel} (validated) : {p}")),
                );
            }
            ("known-divergence", true) => {
                // Toutes les vérifications passent alors que le cas documente
                // une divergence : à promouvoir en validated (décision humaine).
                report.push_str(&format!(
                    "  PROMOTE {rel} — {} (divergence documentée désormais conforme)\n",
                    case.spec.title
                ));
                failures.push(format!(
                    "{rel} (known-divergence {}) : à promouvoir — toutes les vérifications \
                     passent ; statut obsolète, décision de promotion requise",
                    case.spec.issue.as_deref().unwrap_or("?")
                ));
            }
            ("known-divergence", false) => {
                report.push_str(&format!(
                    "  DIVERGENT {rel} — {} (issue {}) : {} divergence(s) attendue(s)\n",
                    case.spec.title,
                    case.spec.issue.as_deref().unwrap_or("?"),
                    problems.len()
                ));
            }
            _ => {
                report.push_str(&format!("  BAD-STATUS {rel} — {}\n", case.spec.title));
                failures.extend(
                    problems
                        .into_iter()
                        .map(|p| format!("{rel} (statut invalide) : {p}")),
                );
            }
        }
    }
    println!("{report}");
    assert!(
        failures.is_empty(),
        "corpus de conformité en échec :\n{}",
        failures.join("\n")
    );
}
