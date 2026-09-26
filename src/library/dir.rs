use super::*;
use crate::dataset::{TMP_MARKER, sidecar_path};

/// A libref bound to a local directory: each table is `<dir>/<table>.parquet`.
pub struct DirLibrary {
    dir: PathBuf,
}

impl DirLibrary {
    pub fn new(dir: PathBuf) -> Self {
        DirLibrary { dir }
    }

    pub(super) fn table_path(&self, table: &str) -> PathBuf {
        self.dir.join(format!("{}.parquet", table.to_lowercase()))
    }
}

impl LibraryProvider for DirLibrary {
    fn list(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let path = entry?.path();
            // Temporaires d'écriture atomique (ADR 0001) : orphelins d'une
            // écriture interrompue, JAMAIS des tables. Garde explicite (le
            // suffixe fait qu'ils n'ont de toute façon pas l'extension
            // parquet) — un futur changement de nommage ne pourrait pas les
            // faire réapparaître dans le catalogue.
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains(TMP_MARKER))
            {
                continue;
            }
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

    /// Atomic publish (ADR 0001): the whole protocol — temporaires dans le
    /// même dossier + fsync + rename, sidecar fingerprinté écrit APRÈS le
    /// parquet — vit dans `SasDataset::write_parquet`, qui purge au passage
    /// les temporaires orphelins de la même cible.
    fn write(&self, table: &str, ds: &SasDataset) -> Result<()> {
        ds.write_parquet(&self.table_path(table))
    }

    fn delete(&self, table: &str) -> Result<()> {
        let path = self.table_path(table);
        std::fs::remove_file(&path)?;
        // Le sidecar accompagne sa table : le retirer aussi évite qu'un
        // sidecar orphelin ne corresponde par coïncidence à une future
        // réécriture de même empreinte (ADR 0001, récupération).
        let _ = std::fs::remove_file(sidecar_path(&path));
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
        let old_sidecar = sidecar_path(&old_path);
        if old_sidecar.is_file() {
            std::fs::rename(&old_sidecar, sidecar_path(&new_path))?;
        }
        Ok(())
    }

    fn catalog_dir(&self) -> Option<&std::path::Path> {
        Some(&self.dir)
    }
}
