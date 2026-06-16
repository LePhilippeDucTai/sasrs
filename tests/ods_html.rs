//! Test d'intégration M22.4 : la destination ODS HTML écrit bien un fichier
//! `.html` sur disque avec le contenu attendu (tables CSS, cellules échappées).
//!
//! Le harnais de snapshots (`tests/snapshot.rs`) ne capture que log + listing ;
//! il ne voit pas les fichiers produits. Ce test exécute un programme
//! `ODS HTML FILE=...` dans un répertoire temporaire, puis relit le fichier
//! HTML généré pour vérifier sa structure.

mod common;

use sas_interpreter::{run, RunOptions};

/// Exécute un programme SAS en mode déterministe avec `base_dir` = tempdir
/// peuplé de `data/class.parquet`, puis renvoie le contenu du fichier HTML
/// `report.html` produit (relatif à base_dir).
#[test]
fn ods_html_writes_table_to_file() {
    let tmp = tempfile::tempdir().unwrap();
    common::write_class_parquet(&tmp.path().join("data"));

    let program = r#"
        libname d 'data';
        data small;
            set d.class;
            if _n_ <= 3;
        run;
        ods html file='report.html';
        proc print data=small;
            var name sex age height;
        run;
        ods html close;
    "#;

    let outcome = run(
        program,
        RunOptions {
            work_dir: None,
            base_dir: Some(tmp.path().to_path_buf()),
            deterministic: true,
            vectorize: false,
        },
    );

    // Le programme s'exécute proprement.
    assert_eq!(outcome.exit_code, 0, "log:\n{}", outcome.log);
    // Le LOG mentionne l'écriture du fichier HTML.
    assert!(
        outcome.log.contains("Writing HTML Body file: report.html"),
        "log:\n{}",
        outcome.log
    );

    // Le fichier HTML existe et porte le contenu attendu.
    let html_path = tmp.path().join("report.html");
    assert!(html_path.exists(), "report.html n'a pas été écrit");
    let html = std::fs::read_to_string(&html_path).unwrap();

    // Structure du document.
    assert!(html.contains("<!DOCTYPE html>"), "html:\n{html}");
    assert!(html.contains("<style>"), "html:\n{html}");
    assert!(html.contains("<table class=\"sas\">"), "html:\n{html}");
    // En-têtes de colonnes.
    assert!(html.contains("<th"), "html:\n{html}");
    assert!(html.contains(">name<"), "html:\n{html}");
    // Une cellule de donnée connue (1ʳᵉ obs de sashelp.class).
    assert!(html.contains(">Alfred<"), "html:\n{html}");
    // Colonne numérique alignée à droite.
    assert!(
        html.contains("text-align:right"),
        "alignement numérique attendu, html:\n{html}"
    );
}

/// `ODS HTML` sans `FILE=` n'écrit aucun fichier (et n'échoue pas).
#[test]
fn ods_html_without_file_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    common::write_class_parquet(&tmp.path().join("data"));

    let program = r#"
        libname d 'data';
        ods html;
        proc print data=d.class;
            var name age;
        run;
        ods html close;
    "#;

    let outcome = run(
        program,
        RunOptions {
            work_dir: None,
            base_dir: Some(tmp.path().to_path_buf()),
            deterministic: true,
            vectorize: false,
        },
    );

    // Pas de crash ; aucun fichier .html parasite dans base_dir.
    assert_eq!(outcome.exit_code, 0, "log:\n{}", outcome.log);
    let stray: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|x| x == "html")
                .unwrap_or(false)
        })
        .collect();
    assert!(stray.is_empty(), "aucun fichier HTML ne devrait être créé");
}
