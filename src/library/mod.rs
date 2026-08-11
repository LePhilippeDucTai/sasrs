//! Bibliothèques SAS : `libref` → stockage des tables.
//!
//! Le trait [`LibraryProvider`] abstrait le stockage ; [`LibraryManager`]
//! tient la table des librefs. `WORK` est un répertoire temporaire détruit
//! avec la session ; un LIBNAME classique pointe un dossier local où chaque
//! table est un `<nom>.parquet`. Le backend S3 vit derrière la feature `s3`.

use crate::dataset::SasDataset;
use crate::error::{Result, SasError};
use polars::prelude::*;
use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

mod csv;
mod dir;
// Tout le contenu de `s3` est derrière la feature `s3` : sans elle le module
// serait vide et le ré-export ne résoudrait pas.
#[cfg(feature = "s3")]
mod s3;

pub use csv::CsvLibrary;
pub use dir::DirLibrary;
#[cfg(feature = "s3")]
pub use s3::S3Library;

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

    /// M39.1 — physical directory backing this libref, for providers that
    /// have one (`DirLibrary`/`CsvLibrary`). `None` for cloud/other providers
    /// without a local root (e.g. `S3Library`). Used solely to locate the
    /// per-libref format-catalog sidecar (`formats.sascat.json`); a `None`
    /// libref simply cannot persist a format catalog to disk in this build.
    fn catalog_dir(&self) -> Option<&std::path::Path> {
        None
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
            return Err(SasError::runtime(format!("Libref {key} is not assigned.")));
        }
        Ok(())
    }

    pub fn get(&self, libref: &str) -> Result<Arc<dyn LibraryProvider>> {
        self.refs
            .get(&libref.to_uppercase())
            .cloned()
            .ok_or_else(|| {
                SasError::runtime(format!("Libref {} is not assigned.", libref.to_uppercase()))
            })
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

fn validate_libref(libref: &str) -> Result<()> {
    let valid = !libref.is_empty()
        && libref.len() <= 8
        && libref
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && libref
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_');
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
mod tests;
