use super::*;

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
