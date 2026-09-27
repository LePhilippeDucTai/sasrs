//! J06-P1 — tests de la façade publique `sasrs::api` (ADR 0002).
//!
//! Couvre le contrat de la façade :
//! - deux soumissions partageant WORK et le moteur macro ;
//! - injection (`register_dataset`) puis relecture (`dataset`) avec
//!   métadonnées persistées (sidecar ADR 0001) ;
//! - fermeture (`close`) : rapport final, code retour global.

use polars::prelude::*;
use sasrs::api::{self, ApiError, Options, VarMeta, VarType};

fn session_in(dir: &std::path::Path) -> api::Session {
    api::Session::new(Options {
        base_dir: Some(dir.to_path_buf()),
        deterministic: true,
        ..Options::default()
    })
    .unwrap()
}

#[test]
fn two_submissions_share_work_and_macros() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session_in(dir.path());

    // Soumission 1 : crée WORK.BASE avec une variable macro.
    let first = session.submit(
        "%let factor = 3;\n\
         data work.base;\n\
         \x20 length name $8;\n\
         \x20 do i = 1 to 2;\n\
         \x20   name = cats('row', i);\n\
         \x20   amount = &factor * i;\n\
         \x20   output;\n\
         \x20 end;\n\
         \x20 drop i;\n\
         run;\n",
    );
    assert_eq!(
        first.exit_code, 0,
        "log de la soumission 1 :\n{}",
        first.log
    );
    assert_eq!(first.errors, 0);
    assert!(first.log.contains("WORK.BASE"), "log :\n{}", first.log);

    // Soumission 2 : LIT WORK.BASE (WORK partagé) et résout &factor
    // (moteur macro partagé entre soumissions).
    let second = session.submit(
        "data work.scaled;\n\
         \x20 set work.base;\n\
         \x20 amount = amount + &factor;\n\
         run;\n",
    );
    assert_eq!(
        second.exit_code, 0,
        "log de la soumission 2 :\n{}",
        second.log
    );
    assert_eq!(second.errors, 0);
    // Une seule soumission à la fois : le log rendu ne contient QUE celle-ci.
    assert!(
        !second.log.contains("data work.base"),
        "le log doit être limité à la soumission courante :\n{}",
        second.log
    );

    let (df, vars) = session.dataset("work", "scaled").unwrap();
    assert_eq!(df.height(), 2);
    assert_eq!(vars.len(), 2);
    // Les noms de colonnes conservent la casse du source SAS (ici minuscule).
    let value = df
        .column("amount")
        .unwrap()
        .f64()
        .unwrap()
        .into_no_null_iter()
        .collect::<Vec<f64>>();
    // &factor * i = 3, 6 puis + &factor → 6, 9.
    assert_eq!(value, vec![6.0, 9.0]);

    let report = session.close();
    assert_eq!(report.exit_code, 0);
    assert_eq!(report.errors, 0);
}

#[test]
fn register_then_read_back_with_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session_in(dir.path());

    let df = df![
        "ID" => [1.0f64, 2.0, 3.0],
        "CIV" => ["M", "MME", "M"],
    ]
    .unwrap();

    let metadata = vec![
        VarMeta {
            name: "ID".to_string(),
            ty: VarType::Num,
            length: 8,
            format: Some("8.2".to_string()),
            label: Some("Identifiant client".to_string()),
        },
        VarMeta {
            name: "CIV".to_string(),
            ty: VarType::Char,
            length: 3,
            format: Some("$3.".to_string()),
            label: Some("Civilité".to_string()),
        },
    ];
    session
        .register_dataset("WORK", "clients", df, Some(metadata))
        .unwrap();

    // Relire : données ET métadonnées (label/format persistés via sidecar).
    let (back, vars) = session.dataset("WORK", "CLIENTS").unwrap();
    assert_eq!(back.height(), 3);
    assert_eq!(vars.len(), 2);
    assert_eq!(vars[0].name, "ID");
    assert_eq!(vars[0].label.as_deref(), Some("Identifiant client"));
    assert_eq!(vars[0].format.as_deref(), Some("8.2"));
    assert_eq!(vars[1].name, "CIV");
    assert_eq!(vars[1].label.as_deref(), Some("Civilité"));
    assert_eq!(vars[1].format.as_deref(), Some("$3."));
    assert_eq!(vars[1].length, 3);
    let civ = back
        .column("CIV")
        .unwrap()
        .str()
        .unwrap()
        .into_no_null_iter()
        .collect::<Vec<&str>>();
    assert_eq!(civ, vec!["M", "MME", "M"]);

    // La table injectée est lisible depuis une soumission SAS (_LAST_).
    let sub = session.submit(
        "data work.check;\n\
         \x20 set work.clients;\n\
         \x20 keep ID;\n\
         run;\n",
    );
    assert_eq!(sub.exit_code, 0, "log :\n{}", sub.log);
    let (check, _) = session.dataset("WORK", "CHECK").unwrap();
    assert_eq!(check.height(), 3);

    let report = session.close();
    assert_eq!(report.exit_code, 0);
}

#[test]
fn register_dataset_rejects_incoherent_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session_in(dir.path());

    let df = df![
        "A" => [1.0f64, 2.0],
        "B" => ["x", "y"],
    ]
    .unwrap();

    // Trop de métadonnées.
    let bad_len = vec![
        VarMeta {
            name: "A".to_string(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        };
        3
    ];
    assert_eq!(
        session
            .register_dataset("WORK", "T1", df.clone(), Some(bad_len))
            .unwrap_err(),
        ApiError::MetadataMismatch {
            columns: 2,
            provided: 3,
            detail: "one VarMeta per column expected".to_string(),
        }
    );

    // Noms qui ne correspondent pas aux colonnes.
    let bad_name = vec![
        VarMeta {
            name: "A".to_string(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
        VarMeta {
            name: "Z".to_string(),
            ty: VarType::Char,
            length: 1,
            format: None,
            label: None,
        },
    ];
    assert!(matches!(
        session
            .register_dataset("WORK", "T2", df, Some(bad_name))
            .unwrap_err(),
        ApiError::MetadataMismatch { .. }
    ));
}

#[test]
fn dataset_errors_are_typed() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session_in(dir.path());

    assert_eq!(
        session.dataset("NOSUCH", "T").unwrap_err(),
        ApiError::UnknownLibrary {
            libref: "NOSUCH".to_string()
        }
    );
    assert_eq!(
        session.dataset("WORK", "MISSING").unwrap_err(),
        ApiError::UnknownDataset {
            libref: "WORK".to_string(),
            name: "MISSING".to_string()
        }
    );
    assert_eq!(
        session.close().exit_code,
        0,
        "les erreurs typées ne comptent pas au log"
    );
}

#[test]
fn close_reports_global_exit_code_and_drains() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session_in(dir.path());

    let ok = session.submit("data work.a;\n\x20 x = 1;\nrun;\n");
    assert_eq!(ok.exit_code, 0);
    let bad = session.submit("data work.b;\n\x20 set work.does_not_exist;\nrun;\n");
    assert_eq!(bad.exit_code, 2, "log :\n{}", bad.log);
    assert_eq!(bad.errors, 1);

    let report = session.close();
    // Bilan global : les erreurs de la soumission 2 restent comptées.
    assert_eq!(report.exit_code, 2);
    assert_eq!(report.errors, 1);
    assert_eq!(report.warnings, 0);
}
