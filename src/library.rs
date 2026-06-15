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

// ── CsvLibrary ───────────────────────────────────────────────────────────────

/// Bibliothèque virtuelle CSV : chaque table est un fichier `<dir>/<table>.csv`
/// lu/écrit via le lecteur CSV de Polars. Le moteur est sélectionné par
/// `LIBNAME ref CSV 'dir';`. Les noms de tables sont normalisés en minuscules
/// (comme `DirLibrary` avec les parquets) : `WORK.CLASS` → `class.csv`.
pub struct CsvLibrary {
    dir: PathBuf,
}

impl CsvLibrary {
    /// Crée une bibliothèque CSV pointant sur `dir`.
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
                "Table {} does not exist in this library.",
                table.to_uppercase()
            )));
        }
        let df = CsvReadOptions::default()
            .with_has_header(true)
            .try_into_reader_with_file_path(Some(path.clone()))
            .map_err(|e| {
                SasError::runtime(format!(
                    "CsvLibrary: cannot open '{}': {e}",
                    path.display()
                ))
            })?
            .finish()
            .map_err(|e| {
                SasError::runtime(format!(
                    "CsvLibrary: error reading '{}': {e}",
                    path.display()
                ))
            })?;
        SasDataset::from_dataframe(df)
    }

    fn scan(&self, table: &str) -> Result<LazyFrame> {
        // V1 : lecture eager puis `.lazy()` — acceptable pour PROC SQL sur de
        // petits fichiers CSV. Une vraie implémentation utiliserait
        // `LazyCsvReader` (à activer avec la feature Polars `lazy_csv`).
        Ok(self.read(table)?.0.df.lazy())
    }

    fn write(&self, table: &str, ds: &SasDataset) -> Result<()> {
        let path = self.table_path(table);
        let mut file = File::create(&path).map_err(|e| {
            SasError::runtime(format!(
                "CsvLibrary: cannot create '{}': {e}",
                path.display()
            ))
        })?;
        let mut df = ds.df.clone();
        CsvWriter::new(&mut file)
            .include_header(true)
            .finish(&mut df)
            .map_err(|e| {
                SasError::runtime(format!(
                    "CsvLibrary: error writing '{}': {e}",
                    path.display()
                ))
            })?;
        Ok(())
    }

    fn delete(&self, table: &str) -> Result<()> {
        let path = self.table_path(table);
        std::fs::remove_file(&path).map_err(|e| {
            SasError::runtime(format!(
                "CsvLibrary: cannot delete '{}': {e}",
                path.display()
            ))
        })
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
        Ok(())
    }

    fn is_cloud(&self) -> bool {
        false
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

    /// `LIBNAME libref CSV 'dir';` — bibliothèque virtuelle CSV.
    /// Le répertoire doit exister (même exigence que `assign`).
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

    /// Noms (MAJUSCULES) de toutes les bibliothèques assignées, triés. Sert aux
    /// dictionary tables (`DICTIONARY.TABLES`/`COLUMNS`) qui doivent énumérer
    /// chaque bibliothèque connue de la session (M20.3).
    pub fn librefs(&self) -> Vec<String> {
        let mut v: Vec<String> = self.refs.keys().cloned().collect();
        v.sort();
        v
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

#[cfg(test)]
mod csv_tests {
    use super::*;

    fn make_ds(vals: Vec<i32>, names: Vec<&str>) -> SasDataset {
        // Build a small DataFrame with one numeric column and one char column.
        let numeric = Series::new("x".into(), vals.iter().map(|&v| v as f64).collect::<Vec<_>>());
        let chars   = Series::new("name".into(), names);
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
        let s: Vec<f64> = col.cast(&DataType::Float64).unwrap()
            .f64().unwrap().into_no_null_iter().collect();
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
        let err_msg = lib.read("nobody").err().expect("expected error reading non-existent table").to_string();
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
        let err = mgr.assign_csv("x", PathBuf::from("/nonexistent/path/xyz")).unwrap_err();
        assert!(err.to_string().contains("does not exist"), "{err}");
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
