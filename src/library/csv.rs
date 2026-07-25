use super::*;

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

    pub(super) fn table_path(&self, table: &str) -> PathBuf {
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
