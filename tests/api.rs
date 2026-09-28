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
            informat: None,
        },
        VarMeta {
            name: "CIV".to_string(),
            ty: VarType::Char,
            length: 3,
            format: Some("$3.".to_string()),
            label: Some("Civilité".to_string()),
            informat: None,
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
            informat: None,
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
            informat: None,
        },
        VarMeta {
            name: "Z".to_string(),
            ty: VarType::Char,
            length: 1,
            format: None,
            label: None,
            informat: None,
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

// ===========================================================================
// J06-P2 — diagnostics structurés et fichiers produits
// =========================================================================//

use sasrs::api::{Diagnostic, ProducedFile, ProducedFileKind, Severity};

/// Programme couvrant les trois sévérités : NOTEs d'étape DATA, ERROR de
/// PROC PRINT sur une table absente, WARNING d'une option non supportée.
fn mixed_program() -> String {
    [
        "data work.a;",
        "  x = 1;",
        "  output;",
        "run;",
        "proc print data=work.nope;",
        "run;",
        "options fancy;",
        "",
    ]
    .join("\n")
}

fn count_log_prefix(log: &str, prefix: &str) -> usize {
    log.lines()
        .filter(|l| l.starts_with(&format!("{prefix}: ")))
        .count()
}

#[test]
fn diagnostics_classify_severities_steps_and_lines() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session_in(dir.path());

    let sub = session.submit(&mixed_program());
    assert_eq!(sub.errors, 1, "log :\n{}", sub.log);
    assert_eq!(sub.warnings, 1, "log :\n{}", sub.log);

    // Sévérités : exactement un Error (table absente) et un Warning
    // (option non supportée), au moins un Note (étape DATA).
    let errors: Vec<&Diagnostic> = sub
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    let warnings: Vec<&Diagnostic> = sub
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .collect();
    assert_eq!(errors.len(), 1, "diagnostics :{:?}", sub.diagnostics);
    assert_eq!(warnings.len(), 1, "diagnostics :{:?}", sub.diagnostics);
    assert!(sub.diagnostics.iter().any(|d| d.severity == Severity::Note));

    // L'erreur est attribuée à l'étape PROC PRINT, pas à DATA.
    assert_eq!(errors[0].step, "PROC PRINT", "diagnostics :{:?}", errors);
    assert!(
        !errors[0].message.is_empty(),
        "le message structuré est rempli"
    );

    // Les NOTEs de l'étape DATA portent le step "DATA" et une ligne de
    // source (l'écho précède le message).
    let data_note = sub
        .diagnostics
        .iter()
        .find(|d| d.severity == Severity::Note && d.step == "DATA")
        .expect("au moins une NOTE de l'étape DATA");
    assert!(data_note.line.is_some(), "diagnostics :{:?}", data_note);

    // Le WARNING d'option arrive APRÈS le proc : ligne >= ligne de l'erreur.
    assert!(warnings[0].line.unwrap_or(0) >= errors[0].line.unwrap_or(0));
}

#[test]
fn diagnostics_mirror_log_prefix_lines() {
    // Oracle croisé de non-régression : chaque message NOTE/WARNING/ERROR
    // du texte du log a exactement son pendant structuré (même sévérité,
    // même nombre). Échoue si le texte ou les diagnostics dérivent.
    let dir = tempfile::tempdir().unwrap();
    let mut session = session_in(dir.path());

    let sub = session.submit(&mixed_program());
    for (prefix, severity) in [
        ("NOTE", Severity::Note),
        ("WARNING", Severity::Warning),
        ("ERROR", Severity::Error),
    ] {
        let text_count = count_log_prefix(&sub.log, prefix);
        let struct_count = sub
            .diagnostics
            .iter()
            .filter(|d| d.severity == severity)
            .count();
        assert_eq!(
            text_count, struct_count,
            " {prefix} : texte = {text_count}, structurés = {struct_count}\nlog :\n{}",
            sub.log
        );
    }

    // Le message structuré est le message du log sans son préfixe.
    let first_error = sub
        .diagnostics
        .iter()
        .find(|d| d.severity == Severity::Error)
        .unwrap();
    let log_line = sub
        .log
        .lines()
        .find(|l| l.starts_with("ERROR: "))
        .unwrap()
        .strip_prefix("ERROR: ")
        .unwrap();
    assert_eq!(first_error.message, log_line);

    session.close();
}

#[test]
fn diagnostics_are_drained_per_submission() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session_in(dir.path());

    let first = session.submit(&mixed_program());
    assert!(!first.diagnostics.is_empty());

    // La soumission suivante repart d'une liste vide : les diagnostics
    // sont propres à CHAQUE soumission (même protocole que le log).
    let second = session.submit("data work.b;\n  x = 2;\nrun;\n");
    assert_eq!(second.exit_code, 0, "log :\n{}", second.log);
    assert!(
        second
            .diagnostics
            .iter()
            .all(|d| d.severity == Severity::Note),
        "diagnostics :{:?}",
        second.diagnostics
    );
    assert!(
        second.diagnostics.iter().all(|d| d.step == "DATA"),
        "diagnostics :{:?}",
        second.diagnostics
    );
    assert!(
        second
            .diagnostics
            .iter()
            .any(|d| d.message.contains("WORK.B")),
        "diagnostics :{:?}",
        second.diagnostics
    );

    // Fermeture : le reliquat (ici vide) est transmis sans doublon des
    // soumissions déjà rendues.
    let report = session.close();
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
}

#[test]
fn diagnostics_reach_run_outcome() {
    // La fonction libre `run` expose les diagnostics agrégés de toute
    // l'exécution (champ ajouté à RunOutcome).
    let dir = tempfile::tempdir().unwrap();
    let outcome = sasrs::run(
        &mixed_program(),
        sasrs::RunOptions {
            base_dir: Some(dir.path().to_path_buf()),
            deterministic: true,
            ..Default::default()
        },
    );
    assert_eq!(outcome.exit_code, 2);
    assert_eq!(
        outcome
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count(),
        1,
        "diagnostics :{:?}",
        outcome.diagnostics
    );
    // Cohérence texte/structuré sur l'agrégat complet.
    assert_eq!(
        count_log_prefix(&outcome.log, "ERROR"),
        outcome
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count()
    );
}

#[test]
fn produced_files_registers_ods_destinations() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session_in(dir.path());

    let program = [
        "data work.a;",
        "  x = 1;",
        "  output;",
        "run;",
        "ods html file='rep.html';",
        " proc print data=work.a;",
        "run;",
        "ods html close;",
        "ods rtf file='rep.rtf';",
        " proc print data=work.a;",
        "run;",
        "ods rtf close;",
        "",
    ]
    .join("\n");
    let sub = session.submit(&program);
    assert_eq!(sub.exit_code, 0, "log :\n{}", sub.log);

    // Deux fichiers produits : HTML puis RTF, dans l'ordre d'écriture.
    assert_eq!(sub.produced_files.len(), 2, "{:?}", sub.produced_files);
    let html: &ProducedFile = &sub.produced_files[0];
    let rtf: &ProducedFile = &sub.produced_files[1];
    assert_eq!(html.kind, ProducedFileKind::Html);
    assert_eq!(html.proc_name, "HTML");
    assert_eq!(
        html.path.file_name().and_then(|n| n.to_str()),
        Some("rep.html")
    );
    assert_eq!(rtf.kind, ProducedFileKind::Rtf);
    assert_eq!(rtf.proc_name, "RTF");

    // Les fichiers existent réellement au chemin enregistré.
    assert!(html.path.is_file(), "attendu :{}", html.path.display());
    assert!(rtf.path.is_file(), "attendu :{}", rtf.path.display());
    assert!(
        std::fs::read_to_string(&html.path)
            .unwrap()
            .contains("<table class=\"sas\">"),
        "le HTML enregistré porte bien la table du PROC PRINT"
    );

    // Le registre est drainé par soumission puis à la fermeture.
    let report = session.close();
    assert!(
        report.produced_files.is_empty(),
        "{:?}",
        report.produced_files
    );
}

#[test]
fn produced_files_empty_without_ods_destinations() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session_in(dir.path());

    let sub = session.submit("data work.a;\n  x = 1;\n  output;\nrun;\n");
    assert_eq!(sub.exit_code, 0);
    assert!(
        sub.produced_files.is_empty(),
        "aucun fichier ODS : registre vide, {:?}",
        sub.produced_files
    );
    let report = session.close();
    assert!(report.produced_files.is_empty());
}
