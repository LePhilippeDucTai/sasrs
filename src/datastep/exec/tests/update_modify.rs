use super::*;
use crate::parser::StatementStream;
use crate::source::SourceFile;

// ----- UPDATE -----

/// UPDATE de base : transaction superpose le maître par clé (match).
#[test]
fn update_basic_overlay() {
    let mut s = session();
    // maître : id=1,2,3 ; x=10,20,30
    write_num_ds(&s, "mas", &[("id", some(&[1.0, 2.0, 3.0])), ("x", some(&[10.0, 20.0, 30.0]))]);
    // transaction : id=2 ; x=99
    write_num_ds(&s, "tra", &[("id", some(&[2.0])), ("x", some(&[99.0]))]);
    let stats = run("data mas; update mas tra key=id; run;", &mut s).unwrap();
    assert_eq!(col(&s, "mas", "id"), some(&[1.0, 2.0, 3.0]));
    // id=2 mis à jour à 99 ; les autres inchangés.
    assert_eq!(col(&s, "mas", "x"), some(&[10.0, 99.0, 30.0]));
    assert_eq!(stats.written, vec![("WORK.MAS".to_string(), 3, 2)]);
    // Deux NOTEs de lecture (maître + transaction).
    assert_eq!(
        stats.read,
        vec![("WORK.MAS".to_string(), 3), ("WORK.TRA".to_string(), 1)]
    );
}

/// UPDATE : une clé maître sans transaction correspondante reste inchangée.
#[test]
fn update_no_match_unchanged() {
    let mut s = session();
    write_num_ds(&s, "mas", &[("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))]);
    write_num_ds(&s, "tra", &[("id", some(&[9.0])), ("x", some(&[99.0]))]);
    run("data mas; update mas tra key=id; run;", &mut s).unwrap();
    assert_eq!(col(&s, "mas", "x"), some(&[10.0, 20.0]));
}

/// UPDATE : une valeur transaction MANQUANTE ne superpose pas (no-update).
#[test]
fn update_missing_transaction_skips_overlay() {
    let mut s = session();
    write_num_ds(
        &s,
        "mas",
        &[("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0])), ("y", some(&[1.0, 2.0]))],
    );
    // transaction id=1 : x=. (manquant → pas de MAJ), y=77 (MAJ).
    write_num_ds(
        &s,
        "tra",
        &[("id", some(&[1.0])), ("x", vec![None]), ("y", some(&[77.0]))],
    );
    run("data mas; update mas tra key=id; run;", &mut s).unwrap();
    // x inchangé (transaction manquante) ; y mis à jour.
    assert_eq!(col(&s, "mas", "x"), some(&[10.0, 20.0]));
    assert_eq!(col(&s, "mas", "y"), some(&[77.0, 2.0]));
}

/// UPDATE : la variable clé n'est jamais écrasée par la transaction.
#[test]
fn update_key_not_overwritten() {
    let mut s = session();
    write_num_ds(&s, "mas", &[("id", some(&[5.0])), ("x", some(&[1.0]))]);
    // La transaction porte la même clé 5 ; x=42.
    write_num_ds(&s, "tra", &[("id", some(&[5.0])), ("x", some(&[42.0]))]);
    run("data mas; update mas tra key=id; run;", &mut s).unwrap();
    assert_eq!(col(&s, "mas", "id"), some(&[5.0]));
    assert_eq!(col(&s, "mas", "x"), some(&[42.0]));
}

/// UPDATE : plusieurs transactions pour une clé → seule la PREMIÈRE compte.
#[test]
fn update_multiple_transactions_first_wins() {
    let mut s = session();
    write_num_ds(&s, "mas", &[("id", some(&[1.0])), ("x", some(&[10.0]))]);
    write_num_ds(&s, "tra", &[("id", some(&[1.0, 1.0])), ("x", some(&[20.0, 30.0]))]);
    run("data mas; update mas tra key=id; run;", &mut s).unwrap();
    // Première transaction (20) appliquée, la seconde (30) ignorée.
    assert_eq!(col(&s, "mas", "x"), some(&[20.0]));
}

/// UPDATE : une transaction sans maître correspondant est IGNORÉE (v1).
#[test]
fn update_unmatched_transaction_ignored() {
    let mut s = session();
    write_num_ds(&s, "mas", &[("id", some(&[1.0])), ("x", some(&[10.0]))]);
    write_num_ds(&s, "tra", &[("id", some(&[1.0, 2.0])), ("x", some(&[11.0, 22.0]))]);
    let stats = run("data mas; update mas tra key=id; run;", &mut s).unwrap();
    // id=2 (sans maître) n'est PAS inséré : 1 obs en sortie.
    assert_eq!(col(&s, "mas", "id"), some(&[1.0]));
    assert_eq!(col(&s, "mas", "x"), some(&[11.0]));
    assert_eq!(stats.written, vec![("WORK.MAS".to_string(), 1, 2)]);
}

/// UPDATE avec WHERE= sur le maître : les obs filtrées ne sont ni mises à
/// jour ni sorties.
#[test]
fn update_master_where() {
    let mut s = session();
    write_num_ds(&s, "mas", &[("id", some(&[1.0, 2.0, 3.0])), ("x", some(&[10.0, 20.0, 30.0]))]);
    write_num_ds(&s, "tra", &[("id", some(&[2.0])), ("x", some(&[99.0]))]);
    let stats = run(
        "data out; update mas(where=(id>=2)) tra key=id; run;",
        &mut s,
    )
    .unwrap();
    // id=1 filtré ; id=2 mis à jour, id=3 inchangé.
    assert_eq!(col(&s, "out", "id"), some(&[2.0, 3.0]));
    assert_eq!(col(&s, "out", "x"), some(&[99.0, 30.0]));
    // 2 obs maître lues (id=1 rejeté).
    assert_eq!(stats.read[0], ("WORK.MAS".to_string(), 2));
}

/// UPDATE avec plusieurs variables clé.
#[test]
fn update_multiple_keys() {
    let mut s = session();
    write_num_ds(
        &s,
        "mas",
        &[("k1", some(&[1.0, 1.0])), ("k2", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))],
    );
    // Met à jour seulement (1,2).
    write_num_ds(
        &s,
        "tra",
        &[("k1", some(&[1.0])), ("k2", some(&[2.0])), ("x", some(&[99.0]))],
    );
    run("data mas; update mas tra key=k1 k2; run;", &mut s).unwrap();
    assert_eq!(col(&s, "mas", "x"), some(&[10.0, 99.0]));
}

/// UPDATE avec clé CARACTÈRE (insensible aux blancs finaux).
#[test]
fn update_char_key() {
    let mut s = session();
    write_keyed_ds(&s, "mas", "name", &["a", "b", "c"], &[("x", some(&[1.0, 2.0, 3.0]))]);
    write_keyed_ds(&s, "tra", "name", &["b"], &[("x", some(&[20.0]))]);
    run("data mas; update mas tra key=name; run;", &mut s).unwrap();
    assert_eq!(col(&s, "mas", "x"), some(&[1.0, 20.0, 3.0]));
}

/// UPDATE : la transaction apporte une NOUVELLE variable absente du maître.
#[test]
fn update_new_variable_from_transaction() {
    let mut s = session();
    write_num_ds(&s, "mas", &[("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))]);
    write_num_ds(&s, "tra", &[("id", some(&[1.0])), ("z", some(&[5.0]))]);
    run("data mas; update mas tra key=id; run;", &mut s).unwrap();
    // z existe (du maître absent → missing), posée pour id=1.
    assert_eq!(col(&s, "mas", "z"), vec![Some(5.0), None]);
}

/// UPDATE : KEY= absente d'un dataset → erreur de compilation.
#[test]
fn update_key_not_on_transaction_errors() {
    let mut s = session();
    write_num_ds(&s, "mas", &[("id", some(&[1.0])), ("x", some(&[10.0]))]);
    write_num_ds(&s, "tra", &[("other", some(&[1.0])), ("x", some(&[20.0]))]);
    let e = run_err("data mas; update mas tra key=id; run;", &mut s);
    assert!(e.contains("KEY variable id"), "got: {e}");
}

/// UPDATE : KEY= obligatoire (erreur de parsing si absente).
#[test]
fn update_requires_key_option() {
    // Parsing seul : KEY= absente → erreur de parsing (pas d'exécution).
    let file = SourceFile::new("data mas; update mas tra; run;");
    let mut ts = StatementStream::new(&file).unwrap();
    assert!(ts.next().is_kw("data"));
    let err = crate::parser::datastep::parse_data_step(&mut ts).unwrap_err();
    assert!(err.to_string().to_uppercase().contains("KEY"), "got: {err}");
}

/// UPDATE avec BY : FIRST./LAST. exposés sur les groupes BY du maître.
#[test]
fn update_with_by_first_last() {
    let mut s = session();
    // maître trié par g : g=1,1,2 ; x=10,20,30.
    write_num_ds(
        &s,
        "mas",
        &[("g", some(&[1.0, 1.0, 2.0])), ("id", some(&[1.0, 2.0, 3.0])), ("x", some(&[10.0, 20.0, 30.0]))],
    );
    write_num_ds(&s, "tra", &[("id", some(&[2.0])), ("x", some(&[99.0]))]);
    run(
        "data out; update mas tra key=id; by g; \
         f = first.g; l = last.g; run;",
        &mut s,
    )
    .unwrap();
    // id=2 mis à jour ; FIRST.g sur les 1res obs de chaque groupe g.
    assert_eq!(col(&s, "out", "x"), some(&[10.0, 99.0, 30.0]));
    assert_eq!(col(&s, "out", "f"), some(&[1.0, 0.0, 1.0]));
    assert_eq!(col(&s, "out", "l"), some(&[0.0, 1.0, 1.0]));
}

/// UPDATE avec BY : la mise à jour reste pilotée par KEY= au sein des
/// groupes BY (chaque obs maître conserve son comportement).
#[test]
fn update_with_by_groups_update() {
    let mut s = session();
    write_num_ds(
        &s,
        "mas",
        &[("g", some(&[1.0, 2.0])), ("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))],
    );
    write_num_ds(&s, "tra", &[("id", some(&[1.0, 2.0])), ("x", some(&[100.0, 200.0]))]);
    run("data out; update mas tra key=id; by g; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "x"), some(&[100.0, 200.0]));
}

/// UPDATE : le corps peut calculer des variables dérivées.
#[test]
fn update_with_derived_body_statement() {
    let mut s = session();
    write_num_ds(&s, "mas", &[("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))]);
    write_num_ds(&s, "tra", &[("id", some(&[1.0])), ("x", some(&[100.0]))]);
    run("data out; update mas tra key=id; d = x * 2; run;", &mut s).unwrap();
    // x après MAJ : 100, 20 ; d = 200, 40.
    assert_eq!(col(&s, "out", "x"), some(&[100.0, 20.0]));
    assert_eq!(col(&s, "out", "d"), some(&[200.0, 40.0]));
}

/// UPDATE/MODIFY exclusif : pas plus d'une source par étape.
#[test]
fn update_after_set_is_error() {
    let mut s = session();
    write_num_ds(&s, "a", &[("id", some(&[1.0]))]);
    write_num_ds(&s, "b", &[("id", some(&[1.0]))]);
    let e = run_err("data out; set a; update a b key=id; run;", &mut s);
    assert!(e.contains("Only one SET, MERGE, UPDATE, or MODIFY"), "got: {e}");
}

// ----- MODIFY -----

/// MODIFY de base : une modification par assignation persiste en place.
#[test]
fn modify_basic_assign_persists() {
    let mut s = session();
    write_num_ds(&s, "d", &[("id", some(&[1.0, 2.0, 3.0])), ("x", some(&[10.0, 20.0, 30.0]))]);
    let stats = run("data d; modify d; x = x + 1; run;", &mut s).unwrap();
    assert_eq!(col(&s, "d", "x"), some(&[11.0, 21.0, 31.0]));
    // Réécriture en place : même nombre d'obs/variables.
    assert_eq!(stats.written, vec![("WORK.D".to_string(), 3, 2)]);
    assert_eq!(stats.read, vec![("WORK.D".to_string(), 3)]);
    assert_eq!(s.last_dataset.as_deref(), Some("WORK.D"));
}

/// MODIFY : conditionnel (modifie seulement certaines obs).
#[test]
fn modify_conditional_update() {
    let mut s = session();
    write_num_ds(&s, "d", &[("id", some(&[1.0, 2.0, 3.0])), ("x", some(&[10.0, 20.0, 30.0]))]);
    run("data d; modify d; if id = 2 then x = 999; run;", &mut s).unwrap();
    assert_eq!(col(&s, "d", "x"), some(&[10.0, 999.0, 30.0]));
}

/// MODIFY : OUTPUT explicite est INTERDIT (erreur de compilation).
#[test]
fn modify_output_not_allowed() {
    let mut s = session();
    write_num_ds(&s, "d", &[("x", some(&[1.0]))]);
    let e = run_err("data d; modify d; output; run;", &mut s);
    assert!(e.contains("OUTPUT statement is not allowed"), "got: {e}");
}

/// MODIFY avec KEY= (lecture séquentielle, clés présentes).
#[test]
fn modify_with_key() {
    let mut s = session();
    write_num_ds(&s, "d", &[("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))]);
    run("data d; modify d key=id; x = x * 10; run;", &mut s).unwrap();
    assert_eq!(col(&s, "d", "x"), some(&[100.0, 200.0]));
}

/// MODIFY + NOBS= : le total est disponible avant la boucle.
#[test]
fn modify_nobs_available() {
    let mut s = session();
    write_num_ds(&s, "d", &[("x", some(&[5.0, 6.0, 7.0]))]);
    run("data d; modify d nobs=n; x = n; run;", &mut s).unwrap();
    // Chaque obs reçoit le total = 3.
    assert_eq!(col(&s, "d", "x"), some(&[3.0, 3.0, 3.0]));
}

/// MODIFY + POINT= : accès direct piloté par un DO, modifie toutes les obs.
#[test]
fn modify_point_loop_all() {
    let mut s = session();
    write_num_ds(&s, "d", &[("x", some(&[10.0, 20.0, 30.0]))]);
    let stats = run(
        "data d; do p = 1 to 3; modify d point=p; x = x + 100; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "d", "x"), some(&[110.0, 120.0, 130.0]));
    // 3 obs traitées, 3 réécrites.
    assert_eq!(stats.read, vec![("WORK.D".to_string(), 3)]);
    assert_eq!(stats.written, vec![("WORK.D".to_string(), 3, 1)]);
}

/// MODIFY + POINT= : accès direct ciblé (une seule obs modifiée).
#[test]
fn modify_point_single_row() {
    let mut s = session();
    write_num_ds(&s, "d", &[("x", some(&[10.0, 20.0, 30.0]))]);
    run(
        "data d; do p = 2 to 2; modify d point=p; x = 999; end; run;",
        &mut s,
    )
    .unwrap();
    // Seule la 2e obs change.
    assert_eq!(col(&s, "d", "x"), some(&[10.0, 999.0, 30.0]));
}

/// MODIFY : char + num, modification d'une colonne char persiste.
#[test]
fn modify_char_column() {
    let mut s = session();
    write_keyed_ds(&s, "d", "grp", &["a", "a", "b"], &[("x", some(&[1.0, 2.0, 3.0]))]);
    run("data d; modify d; if x >= 2 then grp = 'z'; run;", &mut s).unwrap();
    assert_eq!(
        col_str(&s, "d", "grp"),
        vec![Some("a".into()), Some("z".into()), Some("z".into())]
    );
}

/// MODIFY : KEY= absente du dataset → erreur de compilation.
#[test]
fn modify_key_not_present_errors() {
    let mut s = session();
    write_num_ds(&s, "d", &[("x", some(&[1.0]))]);
    let e = run_err("data d; modify d key=nope; run;", &mut s);
    assert!(e.contains("KEY variable nope"), "got: {e}");
}

/// MODIFY après MODIFY → erreur.
#[test]
fn modify_twice_is_error() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0]))]);
    write_num_ds(&s, "b", &[("x", some(&[1.0]))]);
    let e = run_err("data a; modify a; modify b; run;", &mut s);
    assert!(e.contains("Only one SET, MERGE, UPDATE, or MODIFY"), "got: {e}");
}
