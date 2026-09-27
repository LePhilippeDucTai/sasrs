//! J06-P3 — Quickstart de la façade publique `sasrs::api` (ADR 0002).
//!
//! Démontre le cycle de vie complet d'une session :
//! 1. créer une session (WORK temporaire, exécution déterministe) ;
//! 2. soumettre un programme SAS qui importe `examples/data/patients.csv`
//!    et résume les âges par sexe (PROC IMPORT + PROC SQL) ;
//! 3. relire la table produite avec [`Session::dataset`] (données +
//!    métadonnées) ;
//! 4. afficher un diagnostic structuré du log (`api::Diagnostic`) ;
//! 5. fermer la session ([`Session::close`], WORK jeté).
//!
//! La dernière ligne imprimée est toujours `OK` (contrat J06-P3).

use sasrs::api::{Options, Session};
use std::path::PathBuf;

fn main() {
    // Racine du dépôt : le quickstart référence le jeu de données d'exemple
    // sans dépendre du répertoire courant du processus.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let csv = root.join("examples").join("data").join("patients.csv");

    // 1. Session déterministe ; la résolution des chemins relatifs est
    //    ancrée sur la racine du dépôt.
    let mut session = Session::new(Options {
        base_dir: Some(root.clone()),
        deterministic: true,
        ..Options::default()
    })
    .expect("la session doit s'initialiser");

    // 2. Soumission : import CSV + agrégation SQL.
    let code = format!(
        "proc import datafile='{}' out=work.patients dbms=csv replace;\n\
         \x20 getnames=yes;\n\
         run;\n\
         proc sql;\n\
         \x20 create table work.summary as\n\
         \x20 select Sex, count(*) as n_patients, mean(Age) as mean_age\n\
         \x20 from work.patients group by Sex;\n\
         quit;\n",
        csv.display()
    );
    let submission = session.submit(&code);
    println!("code retour de la soumission : {}", submission.exit_code);
    assert_eq!(
        submission.errors, 0,
        "erreurs dans la log :\n{}",
        submission.log
    );
    assert_eq!(submission.exit_code, 0, "log :\n{}", submission.log);

    // 3. Relecture de la table produite (données + métadonnées).
    let (df, vars) = session.dataset("work", "summary").unwrap();
    println!(
        "table WORK.SUMMARY relue : {} observation(s), {} variable(s) ({})",
        df.height(),
        vars.len(),
        vars.iter()
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    assert_eq!(df.height(), 2, "deux sexes attendus");
    assert_eq!(vars.len(), 3);

    // 4. Un diagnostic structuré de la soumission (NOTE/WARNING/ERROR).
    let first = submission
        .diagnostics
        .first()
        .expect("au moins un diagnostic");
    let severity = match first.severity {
        sasrs::api::Severity::Note => "NOTE",
        sasrs::api::Severity::Warning => "WARNING",
        sasrs::api::Severity::Error => "ERROR",
    };
    println!("diagnostic [{}] : {}", severity, first.message);

    // 5. Fermeture : bilan global, WORK temporaire jeté.
    let report = session.close();
    assert_eq!(report.errors, 0, "log final :\n{}", report.log);
    assert_eq!(report.exit_code, 0);

    println!("OK");
}
