//! Tests du provider CSV et de la résolution des librefs.

use super::*;

fn make_ds(vals: Vec<i32>, names: Vec<&str>) -> SasDataset {
    // Build a small DataFrame with one numeric column and one char column.
    let numeric = Series::new(
        "x".into(),
        vals.iter().map(|&v| v as f64).collect::<Vec<_>>(),
    );
    let chars = Series::new("name".into(), names);
    let df = DataFrame::new(vec![numeric.into(), chars.into()]).unwrap();
    SasDataset::from_dataframe(df).unwrap().0
}

#[test]
fn csv_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    let ds = make_ds(vec![1, 2, 3], vec!["a", "b", "c"]);
    lib.write("mytable", &ds).unwrap();

    let path = tmp.path().join("mytable.csv");
    assert!(path.is_file(), "CSV file should exist after write");

    let (ds2, _) = lib.read("mytable").unwrap();
    assert_eq!(ds2.df.height(), 3, "row count");
    assert!(ds2.df.column("x").is_ok(), "numeric column present");
    assert!(ds2.df.column("name").is_ok(), "char column present");
}

#[test]
fn csv_round_trip_values() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    let ds = make_ds(vec![10, 20], vec!["foo", "bar"]);
    lib.write("t", &ds).unwrap();
    let (ds2, _) = lib.read("t").unwrap();
    let col = ds2.df.column("x").unwrap();
    // CSV is read back as floats or ints – check values via to_string.
    let s: Vec<f64> = col
        .cast(&DataType::Float64)
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect();
    assert_eq!(s, vec![10.0, 20.0]);
}

#[test]
fn csv_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    assert!(!lib.exists("none"));
    let ds = make_ds(vec![1], vec!["x"]);
    lib.write("none", &ds).unwrap();
    assert!(lib.exists("none"));
    // Case-insensitive: table name is lowercased for the file.
    assert!(lib.exists("NONE"));
}

#[test]
fn csv_list() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    assert_eq!(lib.list().unwrap(), Vec::<String>::new());
    let ds = make_ds(vec![1], vec!["v"]);
    lib.write("alpha", &ds).unwrap();
    lib.write("beta", &ds).unwrap();
    let names = lib.list().unwrap();
    assert_eq!(names, vec!["ALPHA".to_string(), "BETA".to_string()]);
}

#[test]
fn csv_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    let ds = make_ds(vec![1], vec!["v"]);
    lib.write("todelete", &ds).unwrap();
    assert!(lib.exists("todelete"));
    lib.delete("todelete").unwrap();
    assert!(!lib.exists("todelete"));
}

#[test]
fn csv_rename() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    let ds = make_ds(vec![1], vec!["v"]);
    lib.write("old", &ds).unwrap();
    lib.rename("old", "new").unwrap();
    assert!(!lib.exists("old"));
    assert!(lib.exists("new"));
}

#[test]
fn csv_rename_nonexistent_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    let err = lib.rename("ghost", "new").unwrap_err();
    assert!(err.to_string().contains("does not exist"), "{err}");
}

#[test]
fn csv_read_nonexistent_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    let err_msg = lib
        .read("nobody")
        .err()
        .expect("expected error reading non-existent table")
        .to_string();
    assert!(err_msg.contains("does not exist"), "{err_msg}");
}

#[test]
fn csv_scan_lazy_works() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    let ds = make_ds(vec![42], vec!["q"]);
    lib.write("lazy", &ds).unwrap();
    let lf = lib.scan("lazy").unwrap();
    let df = lf.collect().unwrap();
    assert_eq!(df.height(), 1);
}

#[test]
fn csv_is_not_cloud() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    assert!(!lib.is_cloud());
}

#[test]
fn assign_csv_registers_libref() {
    let tmp = tempfile::tempdir().unwrap();
    let mut mgr = LibraryManager::new(None).unwrap();
    mgr.assign_csv("csvlib", tmp.path().to_path_buf()).unwrap();
    let prov = mgr.get("csvlib").unwrap();
    assert!(!prov.is_cloud());
}

#[test]
fn assign_csv_rejects_missing_dir() {
    let mut mgr = LibraryManager::new(None).unwrap();
    let err = mgr
        .assign_csv("x", PathBuf::from("/nonexistent/path/xyz"))
        .unwrap_err();
    assert!(err.to_string().contains("does not exist"), "{err}");
}

#[cfg(feature = "s3")]
mod s3_tests {
    use super::*;

    #[test]
    fn builds_s3_uri_lowercasing_table() {
        let lib = S3Library::new("my-bucket", "data/sas");
        assert_eq!(lib.uri("Class"), "s3://my-bucket/data/sas/class.parquet");
    }

    #[test]
    fn empty_prefix_has_no_double_slash() {
        let lib = S3Library::new("my-bucket", "");
        assert_eq!(lib.uri("CLASS"), "s3://my-bucket/class.parquet");
    }

    #[test]
    fn surrounding_slashes_in_prefix_are_trimmed() {
        let lib = S3Library::new("my-bucket", "/trimmed/");
        assert_eq!(lib.uri("t"), "s3://my-bucket/trimmed/t.parquet");
    }

    #[test]
    fn mutating_ops_return_runtime_errors() {
        let lib = S3Library::new("b", "p");
        let ds = SasDataset {
            df: DataFrame::empty(),
            vars: Vec::new(),
        };
        assert!(lib.write("t", &ds).is_err());
        assert!(lib.delete("t").is_err());
        assert!(lib.rename("a", "b").is_err());
        assert!(lib.list().is_err());
        assert!(!lib.exists("t"));
    }

    // ── from_uri: parsing s3://bucket/prefix ────────────────────────────────

    #[test]
    fn from_uri_splits_bucket_and_prefix() {
        let lib = S3Library::from_uri("s3://my-bucket/data/sas").unwrap();
        assert_eq!(lib.bucket, "my-bucket");
        assert_eq!(lib.prefix, "data/sas");
        // Round-trips through the URI builder.
        assert_eq!(lib.uri("Class"), "s3://my-bucket/data/sas/class.parquet");
    }

    #[test]
    fn from_uri_bucket_only_has_empty_prefix() {
        let lib = S3Library::from_uri("s3://my-bucket").unwrap();
        assert_eq!(lib.bucket, "my-bucket");
        assert_eq!(lib.prefix, "");
        assert_eq!(lib.uri("CLASS"), "s3://my-bucket/class.parquet");
    }

    #[test]
    fn from_uri_trims_trailing_slash() {
        let lib = S3Library::from_uri("s3://my-bucket/data/sas/").unwrap();
        assert_eq!(lib.prefix, "data/sas");
        // Bucket with a bare trailing slash → empty prefix.
        let lib2 = S3Library::from_uri("s3://my-bucket/").unwrap();
        assert_eq!(lib2.bucket, "my-bucket");
        assert_eq!(lib2.prefix, "");
    }

    #[test]
    fn from_uri_rejects_non_s3_or_empty_bucket() {
        assert!(S3Library::from_uri("/local/path").is_err());
        assert!(S3Library::from_uri("s3:///just/prefix").is_err());
    }

    #[test]
    fn s3_library_reports_cloud_marker() {
        let lib = S3Library::new("b", "p");
        assert!(lib.is_cloud());
    }

    // ── Provider selection via LibraryManager::assign ───────────────────────

    #[test]
    fn assign_s3_uri_selects_cloud_provider() {
        let mgr = LibraryManager::new(None).unwrap();
        // A normal local path → DirLibrary (not cloud).
        let tmp = tempfile::TempDir::new().unwrap();
        let mut mgr = mgr;
        mgr.assign("loc", tmp.path().to_path_buf()).unwrap();
        assert!(!mgr.get("loc").unwrap().is_cloud());

        // An s3:// path → S3Library (cloud), no directory check, no network I/O.
        mgr.assign_uri("cloudlib", "s3://my-bucket/data").unwrap();
        let prov = mgr.get("cloudlib").unwrap();
        assert!(prov.is_cloud());
    }
}

/// CSV : le renommage refuse une destination existante (même sémantique que
/// DirLibrary / PROC DATASETS CHANGE).
#[test]
fn csv_rename_refuses_existing_destination() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());
    let ds = make_ds(vec![1], vec!["a"]);
    lib.write("old", &ds).unwrap();
    lib.write("new", &ds).unwrap();
    let err = lib.rename("OLD", "NEW").unwrap_err();
    assert!(err.to_string().contains("already exists"), "{err}");
    assert!(lib.exists("OLD") && lib.exists("NEW"));
}

/// CSV : `scan_with_notes` transmet les notes de coercition de la lecture
/// eager (WARNING 2**53 pour les entiers i64 trop grands) — `scan` seul les
/// perdait.
#[test]
fn csv_scan_with_notes_forwards_coercion_warning() {
    let tmp = tempfile::tempdir().unwrap();
    // Entier > 2**53 : la coercition vers le numérique SAS (f64) perd de la
    // précision et doit produire un WARNING.
    std::fs::write(tmp.path().join("big.csv"), "x\n9007199254740993\n").unwrap();
    let lib = CsvLibrary::new(tmp.path().to_path_buf());

    let (_, notes) = lib.scan_with_notes("BIG").unwrap();
    assert!(
        notes.iter().any(|n| n.contains("2**53")),
        "coercion warning lost by scan: {notes:?}"
    );
    // Et le read eager (même chemin) produit la même note.
    let (_, read_notes) = lib.read("BIG").unwrap();
    assert!(read_notes.iter().any(|n| n.contains("2**53")));
}

// ── J04-P2 : suppression, renommage et échange sans orphelins ──────────────

use crate::dataset::sidecar_path;

/// Dataset `x`/`name` dont la colonne caractère porte format + libellé :
/// l'écriture DirLibrary produit donc un sidecar `sasmeta.json`.
fn ds_with_meta(rows: usize, fmt: &str, label: &str) -> SasDataset {
    let mut ds = make_ds(vec![42; rows], vec!["aa"; rows]);
    let v = ds.vars.iter_mut().find(|v| v.name == "name").unwrap();
    v.format = Some(fmt.to_string());
    v.label = Some(label.to_string());
    ds
}

/// `delete` emporte le sidecar avec le parquet : aucun sidecar orphelin ne
/// peut survivre à la suppression de sa table.
#[test]
fn orphan_sidecar_delete_removes_sidecar_too() {
    let dir = tempfile::tempdir().unwrap();
    let lib = DirLibrary::new(dir.path().to_path_buf());
    lib.write("t", &ds_with_meta(2, "F1.", "lab")).unwrap();
    let sc = sidecar_path(&dir.path().join("t.parquet"));
    assert!(sc.is_file(), "sidecar expected after meta write");

    lib.delete("t").unwrap();

    assert!(!lib.exists("T"));
    assert!(!sc.is_file(), "sidecar must not outlive its table");
    assert_eq!(lib.list().unwrap(), Vec::<String>::new());
}

/// Destination existante → ERROR (sémantique PROC DATASETS CHANGE), les
/// deux tables sont intactes — ni parquet ni sidecar déplacés.
#[test]
fn orphan_sidecar_rename_refuses_existing_destination() {
    let dir = tempfile::tempdir().unwrap();
    let lib = DirLibrary::new(dir.path().to_path_buf());
    lib.write("src", &ds_with_meta(1, "F1.", "one")).unwrap();
    lib.write("dst", &ds_with_meta(2, "F2.", "two")).unwrap();

    let err = lib.rename("SRC", "DST").unwrap_err();
    assert!(err.to_string().contains("already exists"), "{err}");

    // Rien n'a bougé : les deux tables relisent leurs propres métadonnées.
    let (s, _) = lib.read("SRC").unwrap();
    assert_eq!(
        s.vars.iter().find(|v| v.name == "name").unwrap().label,
        Some("one".to_string())
    );
    let (d, _) = lib.read("DST").unwrap();
    assert_eq!(
        d.vars.iter().find(|v| v.name == "name").unwrap().label,
        Some("two".to_string())
    );
    assert_eq!(
        lib.list().unwrap(),
        vec!["DST".to_string(), "SRC".to_string()]
    );
}

/// Un sidecar orphelin à la destination (parquet absent) est purgé par le
/// renommage : les métadonnées du survivant sont celles de la table renommée.
#[test]
fn orphan_sidecar_rename_purges_orphan_sidecar_at_destination() {
    let dir = tempfile::tempdir().unwrap();
    let lib = DirLibrary::new(dir.path().to_path_buf());
    lib.write("src", &ds_with_meta(2, "F1.", "lab")).unwrap();
    // Orphelin fabriqué à la main : un sidecar résiduel sans parquet.
    let dst_sc = sidecar_path(&dir.path().join("dst.parquet"));
    std::fs::write(&dst_sc, "{\"fingerprint\":0}").unwrap();

    lib.rename("SRC", "DST").unwrap();

    assert!(!lib.exists("SRC"));
    assert!(lib.exists("DST"));
    // Le sidecar présent est bien celui DÉPLACÉ (format/libellé lisibles),
    // pas l'orphelin résiduel.
    let (ds, notes) = lib.read("DST").unwrap();
    assert!(
        notes.is_empty(),
        "no stale sidecar note expected: {notes:?}"
    );
    let v = ds.vars.iter().find(|v| v.name == "name").unwrap();
    assert_eq!(v.format.as_deref(), Some("F1."));
    assert_eq!(v.label.as_deref(), Some("lab"));
}

/// Le déplacement du sidecar échoue APRÈS celui du parquet (un répertoire
/// occupe l'emplacement du sidecar destination) → le parquet est rebasculé à
/// son ancien nom : table intacte, métadonnées intactes, pas de demi-renom.
#[test]
fn orphan_sidecar_rename_rolls_back_parquet_on_sidecar_failure() {
    let dir = tempfile::tempdir().unwrap();
    let lib = DirLibrary::new(dir.path().to_path_buf());
    lib.write("src", &ds_with_meta(2, "F1.", "lab")).unwrap();
    // Répertoire hostile : le rename du sidecar vers cet emplacement échoue.
    std::fs::create_dir(sidecar_path(&dir.path().join("dst.parquet"))).unwrap();

    let err = lib.rename("SRC", "DST").unwrap_err();
    assert!(err.to_string().contains("sidecar"), "{err}");
    assert!(err.to_string().contains("rolled back"), "{err}");

    // État de départ restauré : SRC lisible avec ses métadonnées.
    assert!(lib.exists("SRC"), "parquet move must be rolled back");
    assert!(!lib.exists("DST"));
    let (ds, notes) = lib.read("SRC").unwrap();
    assert!(notes.is_empty(), "metadata must stay valid: {notes:?}");
    let v = ds.vars.iter().find(|v| v.name == "name").unwrap();
    assert_eq!(v.format.as_deref(), Some("F1."));
    assert_eq!(
        dir.path().join("src.parquet.sasmeta.json").is_file(),
        true,
        "sidecar must be back at the old name"
    );
    assert_eq!(lib.list().unwrap(), vec!["SRC".to_string()]);
}

/// Cohérence casse du trio `list`/`exists`/`read` : tout ce que `list`
/// annonce doit être lisible par `exists`/`read` quelle que soit la casse
/// du nom demandé (les chemins sont normalisés en minuscules).
#[test]
fn orphan_sidecar_case_coherent_list_exists_read() {
    let dir = tempfile::tempdir().unwrap();
    let lib = DirLibrary::new(dir.path().to_path_buf());
    lib.write("MiXeD", &ds_with_meta(1, "F1.", "lab")).unwrap();

    assert_eq!(lib.list().unwrap(), vec!["MIXED".to_string()]);
    assert!(lib.exists("MIXED"));
    assert!(lib.exists("mixed"));
    assert!(lib.exists("MiXeD"));
    let (ds_upper, _) = lib.read("MIXED").unwrap();
    let (ds_lower, _) = lib.read("mixed").unwrap();
    assert_eq!(ds_upper.n_obs(), ds_lower.n_obs());
    // Renommage : la détection de destination existante est insensible à la
    // casse elle aussi (chemins normalisés).
    let err = lib.rename("OTHER", "mixed").unwrap_err();
    assert!(err.to_string().contains("does not exist"), "{err}");
}

/// Tests d'interruption du renommage (feature `fault-injection`) : le binaire
/// de test est relancé en PROCESSUS ENFANT avec `SASRS_FAULT_INJECT` posé ;
/// le point de panne tue l'enfant (exit 86) comme un kill en plein rename.
/// L'état sur disque est vérifié par le parent.
#[cfg(feature = "fault-injection")]
mod orphan_sidecar_rename_faults {
    use super::*;
    use std::process::{Command, Output};

    const DRIVER: &str =
        "library::tests::orphan_sidecar_rename_faults::orphan_sidecar_rename_fault_child_driver";

    fn run_child(dir: &std::path::Path, point: &str) -> Output {
        Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg(DRIVER)
            .env("SASRS_TEST_RENAME_DIR", dir)
            .env("SASRS_FAULT_INJECT", point)
            .output()
            .unwrap()
    }

    /// Driver exécuté DANS l'enfant : renomme SRC→DST, tué par le point de
    /// panne en cours de route.
    #[test]
    fn orphan_sidecar_rename_fault_child_driver() {
        let Ok(dir) = std::env::var("SASRS_TEST_RENAME_DIR") else {
            return; // appel direct (cargo test) : rien à faire
        };
        let lib = DirLibrary::new(std::path::Path::new(&dir).to_path_buf());
        lib.rename("SRC", "DST").unwrap();
    }

    /// Kill APRÈS le déplacement du parquet, AVANT celui du sidecar : les
    /// données sont publiées au nouveau nom sans métadonnées (jamais de
    /// métadonnées d'une autre table), et le sidecar resté à l'ancien nom
    /// est un orphelin inerte — une réécriture de SRC le purge.
    #[test]
    fn orphan_sidecar_rename_killed_after_parquet_move() {
        let dir = tempfile::tempdir().unwrap();
        let lib = DirLibrary::new(dir.path().to_path_buf());
        lib.write("src", &ds_with_meta(2, "F1.", "lab")).unwrap();

        let out = run_child(dir.path(), "after_rename_parquet");
        assert_eq!(
            out.status.code(),
            Some(86),
            "child did not die at the fault point — stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        // Données publiées, sans les métadonnées de l'ancienne table.
        assert!(lib.exists("DST"));
        assert!(!lib.exists("SRC"));
        let (ds, notes) = lib.read("DST").unwrap();
        assert_eq!(ds.n_obs(), 2);
        assert!(notes.is_empty(), "no sidecar at DST: {notes:?}");
        assert_eq!(
            ds.vars.iter().find(|v| v.name == "name").unwrap().format,
            None,
            "stale metadata must not follow an interrupted rename"
        );
        // Le sidecar resté à l'ancien nom ne liste pas SRC comme table.
        assert_eq!(lib.list().unwrap(), vec!["DST".to_string()]);

        // Récupération : réécrire SRC remplace le sidecar orphelin par les
        // métadonnées fraîches de la nouvelle table.
        lib.write("src", &ds_with_meta(3, "F9.", "new")).unwrap();
        let (ds, _) = lib.read("SRC").unwrap();
        assert_eq!(ds.n_obs(), 3);
        assert_eq!(
            ds.vars.iter().find(|v| v.name == "name").unwrap().label,
            Some("new".to_string()),
            "fresh metadata must apply after recovery"
        );
    }
}
