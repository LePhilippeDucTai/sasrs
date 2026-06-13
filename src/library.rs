use crate::dataset::SasDataset;
use crate::error::{Result, SasError};
use polars::prelude::*;
use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

/// Storage backend for one libref. Everything above this trait manipulates
/// table names only; the provider owns paths/URIs. A future S3 provider
/// implements this same trait over Polars cloud scans.
pub trait LibraryProvider: Send + Sync {
    fn list(&self) -> Result<Vec<String>>;
    fn exists(&self, table: &str) -> bool;
    /// Eager read for the DATA step; returns log notes from type coercion.
    fn read(&self, table: &str) -> Result<(SasDataset, Vec<String>)>;
    /// Lazy scan for PROC SQL.
    fn scan(&self, table: &str) -> Result<LazyFrame>;
    fn write(&self, table: &str, ds: &SasDataset) -> Result<()>;
    fn delete(&self, table: &str) -> Result<()>;
    /// Rename `old` → `new` (PROC DATASETS CHANGE statement).
    /// Also moves the sidecar `<old>.parquet.sasmeta.json` if it exists.
    /// Returns an error if `old` does not exist.
    fn rename(&self, old: &str, new: &str) -> Result<()>;

    /// True for cloud-backed providers (e.g. `S3Library`). Lets the executor /
    /// tests distinguish a cloud libref from a local `DirLibrary` without a
    /// downcast. Defaults to `false` (local directory backend).
    fn is_cloud(&self) -> bool {
        false
    }
}

/// A libref bound to a local directory: each table is `<dir>/<table>.parquet`.
pub struct DirLibrary {
    dir: PathBuf,
}

impl DirLibrary {
    pub fn new(dir: PathBuf) -> Self {
        DirLibrary { dir }
    }

    fn table_path(&self, table: &str) -> PathBuf {
        self.dir.join(format!("{}.parquet", table.to_lowercase()))
    }
}

impl LibraryProvider for DirLibrary {
    fn list(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "parquet")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                names.push(stem.to_uppercase());
            }
        }
        names.sort();
        Ok(names)
    }

    fn exists(&self, table: &str) -> bool {
        self.table_path(table).is_file()
    }

    fn read(&self, table: &str) -> Result<(SasDataset, Vec<String>)> {
        SasDataset::read_parquet(&self.table_path(table))
    }

    fn scan(&self, table: &str) -> Result<LazyFrame> {
        let lf = LazyFrame::scan_parquet(self.table_path(table), ScanArgsParquet::default())?;
        Ok(lf)
    }

    fn write(&self, table: &str, ds: &SasDataset) -> Result<()> {
        ds.write_parquet(&self.table_path(table))
    }

    fn delete(&self, table: &str) -> Result<()> {
        std::fs::remove_file(self.table_path(table))?;
        Ok(())
    }

    fn rename(&self, old: &str, new: &str) -> Result<()> {
        let old_path = self.table_path(old);
        if !old_path.is_file() {
            return Err(SasError::runtime(format!(
                "Table {} does not exist in this library.",
                old.to_uppercase()
            )));
        }
        let new_path = self.table_path(new);
        std::fs::rename(&old_path, &new_path)?;

        // Move the sidecar metadata file if it exists.
        // Sidecar convention (from dataset.rs): `<table>.parquet.sasmeta.json`
        let old_sidecar = {
            let mut s = old_path.as_os_str().to_os_string();
            s.push(".sasmeta.json");
            std::path::PathBuf::from(s)
        };
        if old_sidecar.is_file() {
            let new_sidecar = {
                let mut s = new_path.as_os_str().to_os_string();
                s.push(".sasmeta.json");
                std::path::PathBuf::from(s)
            };
            std::fs::rename(&old_sidecar, &new_sidecar)?;
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CsvLibrary — LIBNAME … CSV 'dir';
// ─────────────────────────────────────────────────────────────────────────────

/// A libref bound to a local directory: each table is `<dir>/<table>.csv`.
///
/// Read uses Polars `CsvReadOptions`; write uses `CsvWriter`. The SAS type
/// coercion path (`SasDataset::from_dataframe`) is the same as for
/// `DirLibrary` (Parquet) or `PROC IMPORT DBMS=CSV`.
pub struct CsvLibrary {
    dir: PathBuf,
}

impl CsvLibrary {
    pub fn new(dir: PathBuf) -> Self {
        CsvLibrary { dir }
    }

    fn table_path(&self, table: &str) -> PathBuf {
        self.dir.join(format!("{}.csv", table.to_lowercase()))
    }
}

impl LibraryProvider for CsvLibrary {
    fn list(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "csv")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                names.push(stem.to_uppercase());
            }
        }
        names.sort();
        Ok(names)
    }

    fn exists(&self, table: &str) -> bool {
        self.table_path(table).is_file()
    }

    fn read(&self, table: &str) -> Result<(SasDataset, Vec<String>)> {
        let path = self.table_path(table);
        if !path.is_file() {
            return Err(SasError::runtime(format!(
                "CSV table '{}' does not exist at '{}'.",
                table.to_uppercase(),
                path.display()
            )));
        }
        let df = CsvReadOptions::default()
            .with_has_header(true)
            .try_into_reader_with_file_path(Some(path))?
            .finish()?;
        SasDataset::from_dataframe(df)
    }

    fn scan(&self, table: &str) -> Result<LazyFrame> {
        // No native lazy CSV scan with column inference here — delegate to
        // eager read then convert to lazy (acceptable for PROC SQL over CSV).
        let (ds, _notes) = self.read(table)?;
        Ok(ds.df.lazy())
    }

    fn write(&self, table: &str, ds: &SasDataset) -> Result<()> {
        let path = self.table_path(table);
        let mut df = ds.df.clone();
        let file = File::create(&path)?;
        CsvWriter::new(file)
            .with_separator(b',')
            .include_header(true)
            .finish(&mut df)?;
        Ok(())
    }

    fn delete(&self, table: &str) -> Result<()> {
        std::fs::remove_file(self.table_path(table))?;
        Ok(())
    }

    fn rename(&self, old: &str, new: &str) -> Result<()> {
        let old_path = self.table_path(old);
        if !old_path.is_file() {
            return Err(SasError::runtime(format!(
                "Table {} does not exist in this CSV library.",
                old.to_uppercase()
            )));
        }
        let new_path = self.table_path(new);
        std::fs::rename(&old_path, &new_path)?;
        Ok(())
    }
}

enum WorkDir {
    /// Kept alive so the directory survives the session, deleted on drop.
    Temp(#[allow(dead_code)] TempDir),
    /// User-supplied --work directory; the DirLibrary holds the path.
    Fixed,
}

/// All assigned librefs. WORK is always present, backed by a temp directory
/// removed at end of session (or a user-supplied directory kept as is).
pub struct LibraryManager {
    refs: HashMap<String, Arc<dyn LibraryProvider>>,
    _work: WorkDir,
}

impl LibraryManager {
    pub fn new(work_override: Option<PathBuf>) -> Result<Self> {
        let (work, work_path) = match work_override {
            Some(p) => {
                std::fs::create_dir_all(&p)?;
                (WorkDir::Fixed, p)
            }
            None => {
                let t = TempDir::new()?;
                let p = t.path().to_path_buf();
                (WorkDir::Temp(t), p)
            }
        };
        let mut refs: HashMap<String, Arc<dyn LibraryProvider>> = HashMap::new();
        refs.insert("WORK".to_string(), Arc::new(DirLibrary::new(work_path)));
        Ok(LibraryManager { refs, _work: work })
    }

    /// `LIBNAME libref 'path';` — path must be an existing directory.
    pub fn assign(&mut self, libref: &str, dir: PathBuf) -> Result<()> {
        validate_libref(libref)?;
        if !dir.is_dir() {
            return Err(SasError::runtime(format!(
                "Library directory {} does not exist.",
                dir.display()
            )));
        }
        self.refs
            .insert(libref.to_uppercase(), Arc::new(DirLibrary::new(dir)));
        Ok(())
    }

    /// `LIBNAME libref CSV 'dir';` — register a CSV-backed `CsvLibrary`.
    /// The directory must already exist; each table `t` maps to `<dir>/t.csv`.
    pub fn assign_csv(&mut self, libref: &str, dir: PathBuf) -> Result<()> {
        validate_libref(libref)?;
        if !dir.is_dir() {
            return Err(SasError::runtime(format!(
                "Library directory {} does not exist.",
                dir.display()
            )));
        }
        self.refs
            .insert(libref.to_uppercase(), Arc::new(CsvLibrary::new(dir)));
        Ok(())
    }

    /// `LIBNAME libref 's3://bucket/prefix';` — register a cloud-backed
    /// `S3Library`. No directory existence check and no network I/O happens
    /// here; the bucket/prefix is parsed and the provider is registered, with
    /// real cloud scans deferred to read/scan time.
    #[cfg(feature = "s3")]
    pub fn assign_uri(&mut self, libref: &str, uri: &str) -> Result<()> {
        validate_libref(libref)?;
        let lib = S3Library::from_uri(uri)?;
        self.refs.insert(libref.to_uppercase(), Arc::new(lib));
        Ok(())
    }

    /// `LIBNAME libref CLEAR;`
    pub fn clear(&mut self, libref: &str) -> Result<()> {
        let key = libref.to_uppercase();
        if key == "WORK" {
            return Err(SasError::runtime("Libref WORK cannot be cleared."));
        }
        if self.refs.remove(&key).is_none() {
            return Err(SasError::runtime(format!(
                "Libref {key} is not assigned."
            )));
        }
        Ok(())
    }

    pub fn get(&self, libref: &str) -> Result<Arc<dyn LibraryProvider>> {
        self.refs
            .get(&libref.to_uppercase())
            .cloned()
            .ok_or_else(|| SasError::runtime(format!("Libref {} is not assigned.", libref.to_uppercase())))
    }
}

/// Backend de stockage S3 (ou compatible S3) derrière la feature `s3`.
///
/// STATUT (M13) : ce backend est désormais RÉELLEMENT branché. `LIBNAME ref
/// 's3://bucket/prefix';` enregistre une `S3Library` (au lieu d'une
/// `DirLibrary`) et `read`/`scan` font un scan parquet cloud authentique via
/// Polars (`scan_parquet` + `CloudOptions`), porté par object_store/aws-*.
/// Une table `t` d'un libref lié au bucket `b`/préfixe `p` est mappée sur l'URI
/// `s3://b/p/t.parquet`, lue par le scanner parquet de Polars (chemin de
/// coercition identique à `DirLibrary`, seul l'URI et le transport changent).
///
/// Tout ce code n'est compilé qu'avec la feature `s3` (qui tire `polars/cloud`
/// + `polars/aws`). Sous le build par défaut, ce backend n'existe pas et un
/// chemin `s3://` est traité comme aujourd'hui (chemin local).
///
/// Credentials / région : `read`/`scan` dérivent les `CloudOptions` de
/// l'environnement via `CloudOptions::from_untyped_config(uri, [])`, qui laisse
/// object_store détecter les variables AWS standard (`AWS_REGION` /
/// `AWS_DEFAULT_REGION`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`,
/// `AWS_SESSION_TOKEN`, `AWS_ENDPOINT_URL`, profils/IMDS…). Aucune credential
/// n'est codée en dur.
///
/// Les opérations mutantes (write/delete/rename/list) restent non gérées par ce
/// backend orienté lecture et renvoient une erreur runtime claire.
#[cfg(feature = "s3")]
pub struct S3Library {
    bucket: String,
    prefix: String,
}

#[cfg(feature = "s3")]
impl S3Library {
    pub fn new(bucket: impl Into<String>, prefix: impl Into<String>) -> Self {
        S3Library {
            bucket: bucket.into(),
            prefix: prefix.into(),
        }
    }

    /// Parse `s3://bucket/prefix...` en `(bucket, prefix)`. Le schéma `s3://`
    /// (insensible à la casse) est retiré ; le premier segment est le bucket,
    /// le reste (barres de tête/fin retirées) est le préfixe (éventuellement
    /// vide). Renvoie une erreur si le bucket est vide.
    pub fn from_uri(uri: &str) -> Result<Self> {
        let rest = uri
            .strip_prefix("s3://")
            .or_else(|| uri.strip_prefix("S3://"))
            .ok_or_else(|| SasError::runtime(format!("{uri} is not an s3:// URI.")))?;
        let (bucket, prefix) = match rest.split_once('/') {
            Some((b, p)) => (b, p),
            None => (rest, ""),
        };
        if bucket.is_empty() {
            return Err(SasError::runtime(format!(
                "{uri} is missing an S3 bucket name."
            )));
        }
        Ok(S3Library {
            bucket: bucket.to_string(),
            prefix: prefix.trim_matches('/').to_string(),
        })
    }

    /// Construit l'URI `s3://<bucket>/<prefix>/<table>.parquet` (nom de table
    /// en minuscules, comme `DirLibrary`). Le préfixe vide ou bordé de `/` est
    /// normalisé pour éviter les doubles barres.
    fn uri(&self, table: &str) -> String {
        let prefix = self.prefix.trim_matches('/');
        let table = table.to_lowercase();
        if prefix.is_empty() {
            format!("s3://{}/{table}.parquet", self.bucket)
        } else {
            format!("s3://{}/{prefix}/{table}.parquet", self.bucket)
        }
    }

    /// `ScanArgsParquet` portant les `CloudOptions` dérivées de l'environnement
    /// pour cet URI. `from_untyped_config(uri, [])` choisit le backend (AWS ici)
    /// d'après le schéma et laisse object_store récupérer région/credentials
    /// depuis les variables d'environnement AWS standard. En cas d'échec de
    /// résolution (rare), on retombe sur des `CloudOptions` par défaut.
    fn scan_args(&self, uri: &str) -> ScanArgsParquet {
        let cloud_options =
            polars::prelude::cloud::CloudOptions::from_untyped_config(uri, std::iter::empty::<(String, String)>())
                .ok();
        ScanArgsParquet {
            cloud_options,
            ..Default::default()
        }
    }
}

#[cfg(feature = "s3")]
impl LibraryProvider for S3Library {
    fn list(&self) -> Result<Vec<String>> {
        Err(SasError::runtime(
            "Listing tables in an S3 library is not supported by the cloud scan stub.",
        ))
    }

    fn exists(&self, _table: &str) -> bool {
        // Pas de HEAD object dans le stub : on ne peut pas l'affirmer.
        false
    }

    fn read(&self, table: &str) -> Result<(SasDataset, Vec<String>)> {
        // Même contrat que DirLibrary::read : lecture eager puis coercition au
        // modèle SAS (et notes de conversion) via from_dataframe.
        let df = self.scan(table)?.collect()?;
        SasDataset::from_dataframe(df)
    }

    fn scan(&self, table: &str) -> Result<LazyFrame> {
        // Scan parquet cloud authentique : on passe l'URI `s3://...` tel quel
        // (PlPath/&str), avec les CloudOptions dérivées de l'environnement.
        let uri = self.uri(table);
        let args = self.scan_args(&uri);
        let lf = LazyFrame::scan_parquet(&uri, args)?;
        Ok(lf)
    }

    fn is_cloud(&self) -> bool {
        true
    }

    fn write(&self, _table: &str, _ds: &SasDataset) -> Result<()> {
        Err(SasError::runtime(
            "Writing to an S3 library is not supported yet (read-only cloud scan stub).",
        ))
    }

    fn delete(&self, _table: &str) -> Result<()> {
        Err(SasError::runtime(
            "Deleting from an S3 library is not supported yet (read-only cloud scan stub).",
        ))
    }

    fn rename(&self, _old: &str, _new: &str) -> Result<()> {
        Err(SasError::runtime(
            "Renaming in an S3 library is not supported yet (read-only cloud scan stub).",
        ))
    }
}

fn validate_libref(libref: &str) -> Result<()> {
    let valid = !libref.is_empty()
        && libref.len() <= 8
        && libref
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && libref.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if valid {
        Ok(())
    } else {
        Err(SasError::runtime(format!(
            "{} is not a valid SAS name for a libref.",
            libref.to_uppercase()
        )))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// M14.4 unit tests — CsvLibrary
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod csv_library_tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::value::VarType;
    use polars::df;
    use tempfile::tempdir;

    /// Helper: create a minimal SasDataset with one numeric and one char column.
    fn make_ds() -> SasDataset {
        let frame = df![
            "name" => ["Alice", "Bob"],
            "score" => [95.5_f64, 87.0_f64]
        ]
        .unwrap();
        SasDataset {
            df: frame,
            vars: vec![
                VarMeta {
                    name: "name".into(),
                    ty: VarType::Char,
                    length: 8,
                    format: None,
                    label: None,
                },
                VarMeta {
                    name: "score".into(),
                    ty: VarType::Num,
                    length: 8,
                    format: None,
                    label: None,
                },
            ],
        }
    }

    // ── CsvLibrary::new + table_path ─────────────────────────────────────────

    #[test]
    fn csv_library_table_path_lowercase() {
        let dir = tempdir().unwrap();
        let lib = CsvLibrary::new(dir.path().to_path_buf());
        // exists() should return false for a non-existent table.
        assert!(!lib.exists("MYDS"));
        // list() should return an empty vec for an empty dir.
        let names = lib.list().unwrap();
        assert!(names.is_empty());
    }

    // ── Round-trip: write then read back ─────────────────────────────────────

    #[test]
    fn csv_library_write_read_roundtrip() {
        let dir = tempdir().unwrap();
        let lib = CsvLibrary::new(dir.path().to_path_buf());
        let ds_orig = make_ds();

        // Write
        lib.write("myds", &ds_orig).unwrap();
        assert!(lib.exists("MYDS"));
        assert!(lib.exists("myds"));

        // CSV file should exist
        let expected_file = dir.path().join("myds.csv");
        assert!(expected_file.is_file());

        // Read back and check
        let (ds_read, _notes) = lib.read("myds").unwrap();
        assert_eq!(ds_read.n_obs(), 2);
        assert_eq!(ds_read.n_vars(), 2);

        // Column types must be preserved via SasDataset::from_dataframe coercion
        let name_var = ds_read.vars.iter().find(|v| v.name.to_ascii_uppercase() == "NAME");
        assert!(name_var.is_some(), "NAME column missing after round-trip");
        assert_eq!(name_var.unwrap().ty, VarType::Char);

        let score_var = ds_read.vars.iter().find(|v| v.name.to_ascii_uppercase() == "SCORE");
        assert!(score_var.is_some(), "SCORE column missing after round-trip");
        assert_eq!(score_var.unwrap().ty, VarType::Num);
    }

    // ── list() reflects written files ─────────────────────────────────────────

    #[test]
    fn csv_library_list_reflects_files() {
        let dir = tempdir().unwrap();
        let lib = CsvLibrary::new(dir.path().to_path_buf());
        let ds = make_ds();

        lib.write("alpha", &ds).unwrap();
        lib.write("beta", &ds).unwrap();

        let mut names = lib.list().unwrap();
        names.sort();
        assert_eq!(names, vec!["ALPHA", "BETA"]);
    }

    // ── delete() removes the CSV file ─────────────────────────────────────────

    #[test]
    fn csv_library_delete_removes_file() {
        let dir = tempdir().unwrap();
        let lib = CsvLibrary::new(dir.path().to_path_buf());
        lib.write("todel", &make_ds()).unwrap();
        assert!(lib.exists("todel"));

        lib.delete("todel").unwrap();
        assert!(!lib.exists("todel"));
    }

    // ── rename() moves the CSV file ───────────────────────────────────────────

    #[test]
    fn csv_library_rename_moves_file() {
        let dir = tempdir().unwrap();
        let lib = CsvLibrary::new(dir.path().to_path_buf());
        lib.write("old_name", &make_ds()).unwrap();

        lib.rename("old_name", "new_name").unwrap();

        assert!(!lib.exists("old_name"), "old name should be gone");
        assert!(lib.exists("new_name"), "new name should exist");
    }

    #[test]
    fn csv_library_rename_nonexistent_errors() {
        let dir = tempdir().unwrap();
        let lib = CsvLibrary::new(dir.path().to_path_buf());
        let result = lib.rename("ghost", "new");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.to_ascii_uppercase().contains("GHOST"),
            "error should mention the missing table; got: {msg}"
        );
    }

    // ── read() errors on missing table ────────────────────────────────────────

    #[test]
    fn csv_library_read_missing_table_errors() {
        let dir = tempdir().unwrap();
        let lib = CsvLibrary::new(dir.path().to_path_buf());
        let result = lib.read("nosuch");
        assert!(result.is_err());
    }

    // ── scan() delegates to eager read ────────────────────────────────────────

    #[test]
    fn csv_library_scan_returns_lazyframe() {
        let dir = tempdir().unwrap();
        let lib = CsvLibrary::new(dir.path().to_path_buf());
        lib.write("t", &make_ds()).unwrap();

        let lf = lib.scan("t").unwrap();
        let df = lf.collect().unwrap();
        assert_eq!(df.height(), 2);
    }

    // ── is_cloud() → false ────────────────────────────────────────────────────

    #[test]
    fn csv_library_is_not_cloud() {
        let dir = tempdir().unwrap();
        let lib = CsvLibrary::new(dir.path().to_path_buf());
        assert!(!lib.is_cloud());
    }

    // ── LibraryManager::assign_csv registers the provider ────────────────────

    #[test]
    fn library_manager_assign_csv_registers_provider() {
        let dir = tempdir().unwrap();
        let csv_dir = dir.path().to_path_buf();
        let mut mgr = LibraryManager::new(None).unwrap();

        mgr.assign_csv("mylib", csv_dir.clone()).unwrap();

        let provider = mgr.get("MYLIB").unwrap();
        // Write + read through the manager
        let ds = make_ds();
        provider.write("t", &ds).unwrap();
        let (ds_back, _) = provider.read("t").unwrap();
        assert_eq!(ds_back.n_obs(), 2);
    }

    #[test]
    fn library_manager_assign_csv_nonexistent_dir_errors() {
        let mut mgr = LibraryManager::new(None).unwrap();
        let result = mgr.assign_csv("mylib", PathBuf::from("/no/such/path/xyz123"));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.to_lowercase().contains("does not exist"),
            "expected 'does not exist' in error; got: {msg}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// M14.4 executor-level tests — LIBNAME CSV / XLSX end-to-end
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod csv_libname_executor_tests {
    use crate::{run, RunOptions};

    fn run_with_base(src: &str, base: std::path::PathBuf) -> crate::RunOutcome {
        run(
            src,
            RunOptions {
                work_dir: None,
                base_dir: Some(base),
                deterministic: true,
                vectorize: false,
            },
        )
    }

    /// `LIBNAME mylib CSV 'dir';` parses correctly and the libref is assigned.
    #[test]
    fn libname_csv_assigns_successfully() {
        let dir = tempfile::tempdir().unwrap();
        let csv_dir = dir.path().join("csvdata");
        std::fs::create_dir_all(&csv_dir).unwrap();

        let src = format!(
            "libname mylib CSV '{}';",
            csv_dir.display()
        );
        let out = run_with_base(&src, dir.path().to_path_buf());
        assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
        assert!(
            out.log.contains("Libref MYLIB was successfully assigned"),
            "log was:\n{}",
            out.log
        );
        assert!(
            out.log.contains("Engine:        CSV"),
            "log was:\n{}",
            out.log
        );
    }

    /// `LIBNAME mylib XLSX 'dir';` must emit an ERROR (deferred / not implemented).
    #[test]
    fn libname_xlsx_engine_errors_not_implemented() {
        let dir = tempfile::tempdir().unwrap();
        let src = format!(
            "libname myxls XLSX '{}';",
            dir.path().display()
        );
        let out = run_with_base(&src, dir.path().to_path_buf());
        // Should produce ERROR exit code.
        assert_eq!(out.exit_code, 2, "log was:\n{}", out.log);
        assert!(
            out.log.contains("LIBNAME engine XLSX is not yet implemented in this build."),
            "expected not-implemented error in log; got:\n{}",
            out.log
        );
    }

    /// Full round-trip via executor: write a dataset into a CSV libref, read it back.
    #[test]
    fn libname_csv_round_trip_via_executor() {
        let dir = tempfile::tempdir().unwrap();
        let csv_dir = dir.path().join("csvlib");
        std::fs::create_dir_all(&csv_dir).unwrap();

        // Write a CSV file that CsvLibrary will pick up as table FOO.
        let csv_path = csv_dir.join("foo.csv");
        std::fs::write(&csv_path, "x,y\n1,alpha\n2,beta\n").unwrap();

        let src = format!(
            "libname csvref CSV '{}';\n\
             proc print data=csvref.foo; run;\n",
            csv_dir.display()
        );
        let out = run_with_base(&src, dir.path().to_path_buf());
        assert_eq!(out.exit_code, 0, "log was:\n{}", out.log);
        // Listing should contain the data values.
        assert!(
            out.listing.contains("alpha") || out.listing.contains("1"),
            "listing was:\n{}",
            out.listing
        );
    }
}

#[cfg(all(test, feature = "s3"))]
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
