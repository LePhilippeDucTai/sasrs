use super::*;

/// END= sur un seul dataset : 0 sauf la dernière obs.
#[test]
fn end_option_single_dataset() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[10.0, 20.0, 30.0]))]);
    run("data out; set a end=eof; flag = eof; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "flag"), some(&[0.0, 0.0, 1.0]));
    // eof n'est PAS écrite en sortie (variable automatique).
    assert!(read_work(&s, "out").df.column("eof").is_err());
}

/// END= permet une logique « dernière observation » (totaux).
#[test]
fn end_option_drives_last_obs_logic() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0, 3.0, 4.0]))]);
    run(
        "data out; set a end=eof; retain total 0; total + x; \
         if eof then output; run;",
        &mut s,
    )
    .unwrap();
    // Une seule obs sortie : le total final.
    assert_eq!(read_work(&s, "out").n_obs(), 1);
    assert_eq!(num_at(&s, "out", "total", 0), Some(10.0));
}

/// END= avec plusieurs datasets : 1 seulement après la dernière obs du
/// DERNIER dataset.
#[test]
fn end_option_multiple_datasets() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0]))]);
    write_num_ds(&s, "b", &[("x", some(&[3.0]))]);
    run("data out; set a b end=eof; flag = eof; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "flag"), some(&[0.0, 0.0, 1.0]));
}

/// END= avec WHERE= : la dernière obs RETENUE porte eof=1.
#[test]
fn end_option_with_where() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0, 3.0, 4.0]))]);
    run(
        "data out; set a(where=(x <= 2)) end=eof; flag = eof; run;",
        &mut s,
    )
    .unwrap();
    // Seules x=1 et x=2 passent ; eof=1 sur x=2.
    assert_eq!(col(&s, "out", "x"), some(&[1.0, 2.0]));
    assert_eq!(col(&s, "out", "flag"), some(&[0.0, 1.0]));
}

/// END= avec BY (interclassement) : 1 sur la toute dernière obs servie.
#[test]
fn end_option_with_by() {
    let mut s = session();
    write_num_ds(&s, "a", &[("k", some(&[1.0, 3.0]))]);
    write_num_ds(&s, "b", &[("k", some(&[2.0, 4.0]))]);
    run(
        "data out; set a b end=eof; by k; flag = eof; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "k"), some(&[1.0, 2.0, 3.0, 4.0]));
    assert_eq!(col(&s, "out", "flag"), some(&[0.0, 0.0, 0.0, 1.0]));
}

/// END=/NOBS= combinés : compteur de fin + total.
#[test]
fn end_and_nobs_combined() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[5.0, 6.0, 7.0]))]);
    run(
        "data out; set a end=eof nobs=n; \
         if eof then last_total = n; retain last_total; run;",
        &mut s,
    )
    .unwrap();
    // n est connu partout ; last_total posé sur eof.
    assert_eq!(num_at(&s, "out", "last_total", 2), Some(3.0));
}

/// NOBS= : disponible AVANT la boucle (somme d'observations).
#[test]
fn nobs_option_available_before_loop() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[10.0, 20.0, 30.0]))]);
    run("data out; set a nobs=n; cnt = n; run;", &mut s).unwrap();
    // n est constant = 3 pour chaque obs.
    assert_eq!(col(&s, "out", "cnt"), some(&[3.0, 3.0, 3.0]));
    assert_eq!(col(&s, "out", "n"), some(&[3.0, 3.0, 3.0]));
}

/// NOBS= total sur plusieurs datasets.
#[test]
fn nobs_option_total_across_datasets() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0]))]);
    write_num_ds(&s, "b", &[("x", some(&[3.0, 4.0, 5.0]))]);
    run("data out; set a b nobs=n; cnt = n; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "cnt"), some(&[5.0, 5.0, 5.0, 5.0, 5.0]));
}

/// NOBS= utilisable pour une initialisation AVANT toute lecture (le test
/// le plus parlant : un `_N_ = 1` avec `if _n_ = 1` initialise un tableau
/// dimensionné par n). Ici, on vérifie juste l'accès dès la 1re itération.
#[test]
fn nobs_usable_for_initialization() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[7.0, 8.0]))]);
    run(
        "data out; set a nobs=n; if _n_ = 1 then half = n / 2; \
         retain half; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "half"), some(&[1.0, 1.0]));
}

/// POINT= : accès direct via une boucle DO 1..NOBS + OUTPUT explicite.
#[test]
fn point_option_direct_access_loop() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0, 33.0]))]);
    run(
        "data out; do i = 1 to n; set a point=i nobs=n; output; end; stop; run;",
        &mut s,
    )
    .unwrap();
    // Toutes les obs, dans l'ordre de l'index.
    assert_eq!(col(&s, "out", "x"), some(&[11.0, 22.0, 33.0]));
}

/// POINT= : lecture inverse (index décroissant).
#[test]
fn point_option_reverse_order() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0, 33.0]))]);
    run(
        "data out; do i = n to 1 by -1; set a point=i nobs=n; output; end; stop; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "x"), some(&[33.0, 22.0, 11.0]));
}

/// POINT= : accès à UNE obs précise (1-based).
#[test]
fn point_option_single_index() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0, 33.0]))]);
    run(
        "data out; p = 2; set a point=p; output; stop; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(read_work(&s, "out").n_obs(), 1);
    assert_eq!(num_at(&s, "out", "x", 0), Some(22.0));
}

/// POINT= désactive l'output implicite : sans OUTPUT, rien n'est écrit.
#[test]
fn point_option_disables_implicit_output() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0]))]);
    // OUTPUT absent → 0 obs écrite (et STOP évite la boucle infinie).
    run("data out; p = 1; set a point=p; stop; run;", &mut s).unwrap();
    assert_eq!(read_work(&s, "out").n_obs(), 0);
}

/// POINT= index missing → erreur runtime « Error in variable ».
#[test]
fn point_option_missing_index_errors() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0]))]);
    // p jamais affecté → missing.
    let e = run_err("data out; set a point=p; output; stop; run;", &mut s);
    assert!(e.contains("Error in variable"), "got: {e}");
}

/// POINT= index hors bornes (0) → erreur runtime.
#[test]
fn point_option_zero_index_errors() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0]))]);
    let e = run_err("data out; p = 0; set a point=p; output; stop; run;", &mut s);
    assert!(e.contains("Error in variable"), "got: {e}");
}

/// POINT= index trop grand → erreur runtime.
#[test]
fn point_option_out_of_bounds_errors() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0]))]);
    let e = run_err("data out; p = 99; set a point=p; output; stop; run;", &mut s);
    assert!(e.contains("Error in variable"), "got: {e}");
}

/// POINT= avec plusieurs datasets : index GLOBAL sur la concaténation.
#[test]
fn point_option_multiple_datasets_global_index() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0]))]);
    write_num_ds(&s, "b", &[("x", some(&[33.0, 44.0]))]);
    run(
        "data out; do i = 1 to n; set a b point=i nobs=n; output; end; stop; run;",
        &mut s,
    )
    .unwrap();
    // n = 4 (total), index 1..4 parcourt a puis b.
    assert_eq!(col(&s, "out", "x"), some(&[11.0, 22.0, 33.0, 44.0]));
}

/// POINT= incompatible avec BY → erreur de compilation/exécution.
#[test]
fn point_option_with_by_errors() {
    let mut s = session();
    write_num_ds(&s, "a", &[("k", some(&[1.0, 2.0]))]);
    let e = run_err(
        "data out; set a point=p; by k; output; stop; run;",
        &mut s,
    );
    assert!(e.contains("POINT="), "got: {e}");
}

/// POINT= + END= : eof=1 quand l'index pointe la dernière obs.
#[test]
fn point_option_with_end() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0, 33.0]))]);
    run(
        "data out; do i = 1 to n; set a point=i nobs=n end=eof; \
         flag = eof; output; end; stop; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "flag"), some(&[0.0, 0.0, 1.0]));
}

/// POINT= : re-lecture de la même obs (contrôle d'itération manuel).
#[test]
fn point_option_reread_same_obs() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0, 33.0]))]);
    run(
        "data out; do i = 1, 1, 3; set a point=i; output; end; stop; run;",
        &mut s,
    )
    .unwrap();
    // Index 1, 1, 3 → re-lecture autorisée.
    assert_eq!(col(&s, "out", "x"), some(&[11.0, 11.0, 33.0]));
}
