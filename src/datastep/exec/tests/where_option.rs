//! Sémantique de l'option de dataset `WHERE=` posée sur une table lue par
//! une étape DATA (`data test; set table(where=(x = 1)); run;`).
//!
//! Le parsing (espaces, parenthèses, casse) est verrouillé côté parseur dans
//! `parser::datastep::tests::dsopts` ; ici on vérifie que l'option est
//! réellement APPLIQUÉE : filtre pré-chargement (les obs rejetées n'entrent
//! jamais au PDV et ne comptent pas dans la NOTE « observations read »),
//! indifférence à l'écriture, et interaction avec `keep=`/`rename=`.

use super::*;

/// Table de travail : `x` = 1..4, `y` = 10*x.
fn write_xy(session: &Session, table: &str) {
    write_num_ds(
        session,
        table,
        &[
            ("x", some(&[1.0, 2.0, 3.0, 4.0])),
            ("y", some(&[10.0, 20.0, 30.0, 40.0])),
        ],
    );
}

/// Cas de la demande : `set table (where = (x = 1));` — l'option EST prise en
/// compte (une seule obs en sortie) et le compteur de lecture ne compte que
/// les obs retenues.
#[test]
fn where_option_filters_rows() {
    let mut s = session();
    write_xy(&s, "table");
    let stats = run("data test; set table (where = (x = 1)); run;", &mut s).unwrap();
    assert_eq!(col(&s, "test", "x"), some(&[1.0]));
    assert_eq!(col(&s, "test", "y"), some(&[10.0]));
    assert_eq!(stats.read, vec![("WORK.TABLE".to_string(), 1)]);
    assert_eq!(stats.written, vec![("WORK.TEST".to_string(), 1, 2)]);
    let log = s.log.into_string();
    assert!(
        log.contains("There were 1 observations read from the data set WORK.TABLE."),
        "log was: {log}"
    );
}

/// ORACLE d'écriture : espacée (`table (where = (x = 1))`) ou collée
/// (`table(where=(x=1))`), même sortie et mêmes compteurs.
#[test]
fn where_option_spacing_does_not_change_results() {
    let mut s = session();
    write_xy(&s, "table");
    let spaced = run("data a; set table (where = (x > 2)); run;", &mut s).unwrap();
    let tight = run("data b; set table(where=(x>2)); run;", &mut s).unwrap();
    assert_eq!(col(&s, "a", "x"), some(&[3.0, 4.0]));
    assert_eq!(col(&s, "a", "x"), col(&s, "b", "x"));
    assert_eq!(spaced.read[0].1, tight.read[0].1);
}

/// Parenthèses imbriquées dans l'expression : `((x = 1) or (x = 3))` — le
/// groupement est respecté, et l'option se termine bien à SA parenthèse.
#[test]
fn where_option_with_nested_parentheses() {
    let mut s = session();
    write_xy(&s, "table");
    run(
        "data test; set table(where=((x = 1) or (x = 3))); run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "test", "x"), some(&[1.0, 3.0]));
}

/// Le groupement par parenthèses CHANGE le résultat quand il contredit la
/// priorité par défaut (`and` lie plus fort que `or`) : sans parenthèses
/// `x = 1 or x = 2 and y = 30` ne retient que x=1 ; avec, x=1 puis rien.
#[test]
fn where_option_parentheses_override_operator_precedence() {
    let mut s = session();
    write_xy(&s, "table");
    run(
        "data a; set table(where=(x = 1 or x = 2 and y = 30)); run;",
        &mut s,
    )
    .unwrap();
    run(
        "data b; set table(where=((x = 1 or x = 2) and y = 30)); run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "a", "x"), some(&[1.0]));
    assert_eq!(col(&s, "b", "x"), Vec::new());
}

/// Une expression `where=` fausse partout donne une table vide (0 obs lue,
/// 0 obs écrite) — pas d'erreur.
#[test]
fn where_option_matching_nothing_yields_empty_dataset() {
    let mut s = session();
    write_xy(&s, "table");
    let stats = run("data test; set table(where=(x = 99)); run;", &mut s).unwrap();
    assert_eq!(col(&s, "test", "x"), Vec::new());
    assert_eq!(stats.read, vec![("WORK.TABLE".to_string(), 0)]);
    assert_eq!(stats.written, vec![("WORK.TEST".to_string(), 0, 2)]);
}

/// `where=` sur une variable caractère (comparaison de chaînes).
#[test]
fn where_option_on_character_variable() {
    let mut s = session();
    write_class_full(&s, "class");
    run("data test; set class (where = (Sex = 'F')); run;", &mut s).unwrap();
    assert_eq!(
        col_str(&s, "test", "Name"),
        vec![Some("Alice".to_string()), Some("Barbara".to_string())]
    );
}

/// Le filtre est PRÉ-chargement : `_N_` ne compte que les obs retenues
/// (différence observable avec un subsetting IF, qui les compte).
#[test]
fn where_option_is_prefilter_unlike_subsetting_if() {
    let mut s = session();
    write_xy(&s, "table");
    let stats_where = run("data a; set table(where=(x > 2)); n = _n_; run;", &mut s).unwrap();
    let stats_if = run("data b; set table; if x > 2; n = _n_; run;", &mut s).unwrap();
    // Mêmes lignes retenues…
    assert_eq!(col(&s, "a", "x"), some(&[3.0, 4.0]));
    assert_eq!(col(&s, "b", "x"), col(&s, "a", "x"));
    // …mais _N_ et le compteur de lecture diffèrent.
    assert_eq!(col(&s, "a", "n"), some(&[1.0, 2.0]));
    assert_eq!(col(&s, "b", "n"), some(&[3.0, 4.0]));
    assert_eq!(stats_where.read, vec![("WORK.TABLE".to_string(), 2)]);
    assert_eq!(stats_if.read, vec![("WORK.TABLE".to_string(), 4)]);
}

/// `where=` combiné à `keep=` : le filtre s'applique, et seules les
/// variables gardées sortent (l'ordre des deux options est indifférent).
#[test]
fn where_option_combines_with_keep() {
    let mut s = session();
    write_xy(&s, "table");
    run("data a; set table(keep=x where=(x = 3)); run;", &mut s).unwrap();
    run("data b; set table(where=(x = 3) keep=(x)); run;", &mut s).unwrap();
    assert_eq!(col(&s, "a", "x"), some(&[3.0]));
    let out_a = read_work(&s, "a");
    let cols: Vec<&str> = out_a.df.get_column_names_str();
    assert_eq!(cols, vec!["x"]);
    assert_eq!(col(&s, "b", "x"), col(&s, "a", "x"));
}

/// `where=` sur une variable retirée par `keep=` : erreur SAS « Variable …
/// is not on file … » (le filtre voit la table APRÈS keep/drop).
#[test]
fn where_option_on_dropped_variable_errors() {
    let mut s = session();
    write_xy(&s, "table");
    let err = run("data test; set table(keep=x where=(y = 30)); run;", &mut s)
        .err()
        .unwrap();
    assert_eq!(err.to_string(), "Variable y is not on file WORK.TABLE.");
}

/// `where=` combiné à `rename=` : le filtre s'écrit avec le NOUVEAU nom.
#[test]
fn where_option_uses_renamed_variable_name() {
    let mut s = session();
    write_xy(&s, "table");
    run(
        "data test; set table(rename=(x=z) where=(z = 2)); run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "test", "z"), some(&[2.0]));
}

/// …et l'ANCIEN nom n'est plus visible du `where=`.
#[test]
fn where_option_on_old_name_after_rename_errors() {
    let mut s = session();
    write_xy(&s, "table");
    let err = run(
        "data test; set table(rename=(x=z) where=(x = 2)); run;",
        &mut s,
    )
    .err()
    .unwrap();
    assert_eq!(err.to_string(), "Variable x is not on file WORK.TABLE.");
}

/// Une variable CALCULÉE dans l'étape n'est pas visible du `where=` (le
/// filtre est appliqué à la lecture, avant tout code).
#[test]
fn where_option_cannot_see_step_variables() {
    let mut s = session();
    write_xy(&s, "table");
    let err = run(
        "data test; set table(where=(double = 2)); double = 2 * x; run;",
        &mut s,
    )
    .err()
    .unwrap();
    assert_eq!(
        err.to_string(),
        "Variable double is not on file WORK.TABLE."
    );
}

/// Deux lectures de la MÊME table dans une étape : chaque site applique son
/// propre `where=` (ici deux flux appariés ligne à ligne).
#[test]
fn where_option_is_independent_per_read_site() {
    let mut s = session();
    write_xy(&s, "table");
    run(
        "data test; set table(where=(x <= 2)); set table(rename=(x=x2 y=y2) where=(x2 >= 3)); run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "test", "x"), some(&[1.0, 2.0]));
    assert_eq!(col(&s, "test", "x2"), some(&[3.0, 4.0]));
}

/// `where=` sur le dataset de SORTIE de l'étape DATA : refusé (option
/// d'entrée uniquement), avec le message SAS.
#[test]
fn where_option_on_output_dataset_errors() {
    let mut s = session();
    write_xy(&s, "table");
    let err = run("data test(where=(x = 1)); set table; run;", &mut s)
        .err()
        .unwrap();
    assert_eq!(
        err.to_string(),
        "WHERE= is not a valid data set option for output data sets."
    );
}
