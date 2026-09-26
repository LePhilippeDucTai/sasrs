use super::*;
use crate::dataset::{TMP_MARKER, fault_point, sidecar_path};

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

    /// Renommage sans orphelin (J04-P2) :
    ///
    /// 1. destination existante → ERROR (sémantique PROC DATASETS CHANGE :
    ///    SAS refuse le CHANGE vers un membre existant), rien n'est déplacé ;
    /// 2. un sidecar orphelin de la destination (parquet absent) est purgé :
    ///    il ne peut pas se faire passer pour les métadonnées de la table
    ///    renommée (son empreinte ne correspondrait de toute façon pas, mais
    ///    le fichier ne doit pas survivre au renommage) ;
    /// 3. si le déplacement du sidecar échoue APRÈS celui du parquet, le
    ///    parquet est rebasculé à son ancien nom : jamais de table renommée
    ///    à moitié (données au nouveau nom, métadonnées à l'ancien).
    fn rename(&self, old: &str, new: &str) -> Result<()> {
        let old_path = self.table_path(old);
        if !old_path.is_file() {
            return Err(SasError::runtime(format!(
                "Table {} does not exist in this library.",
                old.to_uppercase()
            )));
        }
        let new_path = self.table_path(new);
        if new_path.is_file() {
            return Err(SasError::runtime(format!(
                "Table {} already exists in this library; {} was not renamed.",
                new.to_uppercase(),
                old.to_uppercase()
            )));
        }
        let old_sidecar = sidecar_path(&old_path);
        let new_sidecar = sidecar_path(&new_path);
        // Sidecar orphelin de la destination (le parquet n'existe pas) : purge
        // avant le déplacement. Best effort — un répertoire à cet emplacement
        // n'est pas un sidecar et fera échouer le renommage plus loin.
        if new_sidecar.is_file() {
            let _ = std::fs::remove_file(&new_sidecar);
        }

        // 1. Données : rename atomique du parquet.
        std::fs::rename(&old_path, &new_path)?;
        fault_point("after_rename_parquet");

        // 2. Métadonnées : le sidecar suit sa table. En cas d'échec, on
        //    annule le déplacement du parquet — l'état de départ est restauré.
        if old_sidecar.is_file()
            && let Err(e) = std::fs::rename(&old_sidecar, &new_sidecar)
        {
            let rollback = std::fs::rename(&new_path, &old_path);
            fault_point("after_rename_rollback");
            if let Err(rb) = rollback {
                return Err(SasError::runtime(format!(
                    "failed to move metadata sidecar for {}: {e}; rolling back the parquet \
                         move failed too: {rb}",
                    new.to_uppercase()
                )));
            }
            return Err(SasError::runtime(format!(
                "failed to move metadata sidecar for {}: {e}; the parquet move was rolled \
                     back and {} was not renamed.",
                new.to_uppercase(),
                old.to_uppercase()
            )));
        }
        fault_point("after_rename_sidecar");
        Ok(())
    }

    /// Le sidecar `<table>.parquet.sasmeta.json` existe-t-il (même sans le
    /// parquet — sidecar orphelin) ? Sert à `EXCHANGE` pour choisir un nom
    /// temporaire qui n'entre en collision avec AUCUN artefact résiduel.
    fn sidecar_exists(&self, table: &str) -> bool {
        sidecar_path(&self.table_path(table)).is_file()
    }

    fn catalog_dir(&self) -> Option<&std::path::Path> {
        Some(&self.dir)
    }
}
