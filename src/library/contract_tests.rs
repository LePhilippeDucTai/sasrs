//! J02-P8 — contrat des bibliothèques : DICTIONARY.TABLES/COLUMNS et S3
//! (replis silencieux supprimés).
//!
//! Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
//! le diagnostic (texte, code de sortie). Oracle de sévérité : CONTRIBUTING
//! §5. Les tests S3 (`ra_j02_p8_s3_*`, feature `s3`) ne font aucun accès
//! réseau : l'assignation d'une libref S3 n'ouvre rien, et la lecture est
//! testée à partir du DataFrame que le scan cloud aurait rendu.

use super::*;

/// Exécute un programme complet (mode déterministe) : log, listing, code.
fn run_sas(src: &str, work_dir: Option<PathBuf>) -> crate::RunOutcome {
    crate::run(
        src,
        crate::RunOptions {
            work_dir,
            deterministic: true,
            ..Default::default()
        },
    )
}

/// Base : une table illisible (parquet corrompu) était omise en silence de
/// DICTIONARY.TABLES et DICTIONARY.COLUMNS (code 0). WARNING nommant la
/// table ; les autres tables restent listées.
#[test]
fn ra_j02_p8_dictionary_unreadable_table_warns() {
    let work = tempfile::tempdir().unwrap();
    std::fs::write(work.path().join("bad.parquet"), b"not a parquet file").unwrap();
    for (table, cols) in [
        ("dictionary.tables", "memname, nobs"),
        ("dictionary.columns", "memname, name"),
    ] {
        let out = run_sas(
            &format!(
                "data good; x = 1; run;
                 proc sql; select {cols} from {table} where libname = 'WORK'; quit;"
            ),
            Some(work.path().to_path_buf()),
        );
        let what = table.to_uppercase();
        assert_eq!(out.exit_code, 1, "{table}: {}", out.log);
        let needle =
            format!("WARNING: Table WORK.BAD could not be read and is not included in {what}: ");
        assert!(
            out.log.contains(&needle),
            "{table}: missing «{needle}»\n{}",
            out.log
        );
        assert!(out.listing.contains("GOOD"), "{table}: {}", out.listing);
        assert!(!out.listing.contains("BAD"), "{table}: {}", out.listing);
    }
}

#[cfg(feature = "s3")]
mod s3 {
    use super::*;

    /// Base (feature s3) : `libname c csv 's3://…';` assignait une
    /// bibliothèque S3 lue en parquet — le moteur CSV était ignoré (XLSX
    /// aussi, au lieu de son ERROR de moteur non implémenté). ERROR, libref
    /// non assignée ; PARQUET (le moteur qui lit S3) reste honoré, sans accès
    /// réseau.
    #[test]
    fn ra_j02_p8_s3_libname_engine_is_not_ignored() {
        for engine in ["csv", "xlsx", "nosuch"] {
            let out = run_sas(
                &format!(
                    "libname c {engine} 's3://bucket/data';
                     proc datasets lib=c nolist; quit;"
                ),
                None,
            );
            assert_eq!(out.exit_code, 2, "{engine}: {}", out.log);
            let msg = format!(
                "ERROR: The {} engine is not supported for the S3 path s3://bucket/data: an S3 \
                 library is read with the PARQUET engine. Libref C was not assigned.",
                engine.to_uppercase()
            );
            assert!(
                out.log.contains(&msg),
                "{engine}: missing «{msg}»\n{}",
                out.log
            );
            assert!(
                out.log.contains("ERROR: Libref C is not assigned."),
                "{engine}: {}",
                out.log
            );
        }

        for engine in ["", "parquet", "base", "v9"] {
            let out = run_sas(&format!("libname c {engine} 's3://bucket/data';"), None);
            assert_eq!(out.exit_code, 0, "{engine}: {}", out.log);
            assert!(
                out.log.contains("Engine:        PARQUET"),
                "{engine}: {}",
                out.log
            );
        }
    }

    /// Base (feature s3) : `S3Library::read` rendait `from_dataframe(df)` —
    /// aucune note, le sidecar `<table>.parquet.sasmeta.json` (formats,
    /// informats, libellés) jamais lu. WARNING nommant la table à chaque
    /// lecture, jusqu'à J12-P1. `read` = scan cloud (réseau) puis
    /// `dataset_from_scan`, testé ici sur le DataFrame qu'aurait rendu le
    /// scan.
    #[test]
    fn ra_j02_p8_s3_sidecar_not_read_warns() {
        let lib = S3Library::from_uri("s3://bucket/data").unwrap();
        let df = df!["x" => [1.0_f64, 2.0]].unwrap();
        let (ds, notes) = lib.dataset_from_scan("Class", df).unwrap();
        assert_eq!(ds.n_obs(), 2);
        assert_eq!(
            notes,
            [
                "WARNING: The metadata sidecar s3://bucket/data/class.parquet.sasmeta.json of \
                 table CLASS is not read from an S3 library in this build: its stored formats, \
                 informats and labels are not applied (planned: roadmap-avancee J12-P1)."
            ]
        );
    }

    /// Base (feature s3) : DICTIONARY.TABLES (et COLUMNS) sautait en silence
    /// une libref S3, dont `list` n'est pas supporté (code 0, aucune ligne
    /// pour la libref). WARNING nommant la libref, jusqu'à J12-P1.
    #[test]
    fn ra_j02_p8_s3_dictionary_names_the_skipped_library() {
        for table in ["dictionary.tables", "dictionary.columns"] {
            let out = run_sas(
                &format!(
                    "libname c 's3://bucket/data';
                     proc sql; select libname, memname from {table}; quit;"
                ),
                None,
            );
            assert_eq!(out.exit_code, 1, "{table}: {}", out.log);
            let msg = format!(
                "WARNING: The members of library C are not included in {}: listing the tables \
                 of an S3 library is not supported in this build (planned: roadmap-avancee \
                 J12-P1).",
                table.to_uppercase()
            );
            assert!(
                out.log.contains(&msg),
                "{table}: missing «{msg}»\n{}",
                out.log
            );
        }
    }
}
