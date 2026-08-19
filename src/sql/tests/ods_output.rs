//! M42.3 — ODS OUTPUT généralisé : capture de la table « SQL_Results »
//! (nom d'objet ODS SAS réel pour un SELECT nu de PROC SQL).
//!
//! `exec_select` ACCUMULE les tranches (`Session::append_ods_output`) ; la
//! matérialisation en dataset se fait en fin de proc (`flush_ods_output`,
//! appelé par `procs::execute_proc`) — les tests appellent donc le flush
//! explicitement après `run_sql`, comme le ferait le dispatch (voir le même
//! pattern en `procs::freq::tests::ods_output`).

use super::super::*;
use super::*;
use crate::ast::DatasetRef;
use crate::testkit::*;

fn ods_target(session: &mut Session, table: &str, name: &str) {
    session.set_ods_output(&[(
        table.to_string(),
        DatasetRef {
            libref: None,
            name: name.to_string(),
        },
    )]);
}

/// Oracle M42.3 : `ods output sql_results=out;` autour d'un SELECT nu →
/// dataset OUT avec les mêmes colonnes/valeurs TYPÉES que le SELECT.
#[test]
fn sql_results_captured_from_bare_select() {
    let mut s = make_session();
    write_people(&mut s);
    ods_target(&mut s, "SQL_Results", "out");

    run_sql("select name, age from t;", &mut s);
    s.flush_ods_output().unwrap();

    let out = read_work(&mut s, "OUT");
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["name", "age"]);
    assert_eq!(out.n_obs(), 4);
    assert_eq!(s.last_dataset.as_deref(), Some("WORK.OUT"));
}

/// Sans `ODS OUTPUT`, aucune capture : ni dataset, ni tampon en attente —
/// l'invariant byte-identique du chemin par défaut (le listing du SELECT
/// n'est pas affecté).
#[test]
fn sql_results_not_requested_writes_nothing() {
    let mut s = make_session();
    write_people(&mut s);

    run_sql("select name, age from t;", &mut s);
    s.flush_ods_output().unwrap();

    assert!(s.libs.get("WORK").unwrap().read("OUT").is_err());
    assert!(s.ods_output_pending.is_empty());
    // Un SELECT nu ne produit pas de table : `_LAST_` reste inchangé
    // (`write_people` écrit T directement via la bibliothèque, sans passer
    // par un chemin qui pose `last_dataset`).
    assert_eq!(s.last_dataset, None);
}

/// Deux SELECT nus dans le même run-group PROC SQL → UN dataset, tranches
/// empilées avec union diagonale des colonnes (convention générale de ce
/// projet pour l'accumulation ODS OUTPUT au sein d'un même step — voir
/// `OneWayFreqs` en PROC FREQ ; le comportement SAS réel pour plusieurs
/// SELECT n'a pas pu être confirmé, ce test documente le choix retenu ici).
#[test]
fn sql_results_two_selects_stack_diagonally() {
    let mut s = make_session();
    write_people(&mut s);
    ods_target(&mut s, "SQL_Results", "out");

    run_sql("select name, age from t; select sex from t;", &mut s);
    s.flush_ods_output().unwrap();

    let out = read_work(&mut s, "OUT");
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    // Colonnes en ordre de première apparition : la tranche du premier
    // SELECT d'abord, puis les colonnes propres au second ajoutées à droite.
    assert_eq!(names, vec!["name", "age", "sex"]);
    assert_eq!(out.n_obs(), 8, "4 lignes du premier SELECT + 4 du second");

    // Les colonnes hors de leur tranche d'origine sont missing.
    let name_col = out.df.column("name").unwrap();
    let sex_col = out.df.column("sex").unwrap();
    assert_eq!(
        name_col.null_count(),
        4,
        "missing sur les lignes du 2e SELECT"
    );
    assert_eq!(
        sex_col.null_count(),
        4,
        "missing sur les lignes du 1er SELECT"
    );
}

/// Le WARNING générique « Output '…' was not created. » reste déclenché
/// quand `ods output sql_results=out;` est actif mais que le step PROC SQL
/// ne contient aucun SELECT nu (ici, seulement un CREATE TABLE — objet ODS
/// distinct, pas SQL_Results).
#[test]
fn sql_results_requested_without_select_warns() {
    let mut s = make_session();
    write_people(&mut s);
    ods_target(&mut s, "SQL_Results", "out");

    run_sql("create table cap as select * from t;", &mut s);
    s.flush_ods_output().unwrap();
    s.ods_output_step_boundary();

    assert_eq!(s.log.warnings, 1);
    assert!(s.libs.get("WORK").unwrap().read("OUT").is_err());
    // La table CREATE reste bien produite, indépendamment du warning ODS.
    assert!(s.libs.get("WORK").unwrap().read("CAP").is_ok());
}
