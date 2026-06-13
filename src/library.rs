use crate::dataset::SasDataset;
use crate::error::{Result, SasError};
use polars::prelude::*;
use std::collections::HashMap;
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
/// STATUT (spike M8) : ce module pose la couture `LibraryProvider` au-dessus
/// d'un stockage objet et n'est **délibérément PAS branché** dans
/// `LibraryManager`/`LIBNAME` — il prouve que le même trait suffit à brancher
/// le cloud plus tard. Une table `t` d'un libref lié au bucket `b`/préfixe `p`
/// est mappée sur l'URI `s3://b/p/t.parquet`, lue paresseusement par le scanner
/// parquet de Polars (chemin identique à `DirLibrary`, seul l'URI change).
///
/// L'I/O cloud réelle exige de compiler Polars avec ses features `cloud`/`aws`
/// (suite non incluse ici, pour ne pas alourdir les dépendances par défaut) :
/// sans elles, `scan`/`read` résolvent l'URI comme un chemin local et échouent
/// à l'exécution. Les opérations mutantes (write/delete/rename) ne sont pas
/// gérées par ce stub orienté lecture et renvoient une erreur runtime claire.
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
        // PathBuf, exactement comme DirLibrary : Polars résout l'URI cloud quand
        // ses features cloud sont actives, un chemin local sinon.
        let lf = LazyFrame::scan_parquet(PathBuf::from(self.uri(table)), ScanArgsParquet::default())?;
        Ok(lf)
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
}
