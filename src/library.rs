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
