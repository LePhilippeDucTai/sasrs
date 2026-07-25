use super::*;

#[test]
fn retain_with_init_accumulates_max() {
    let mut s = session();
    write_class(&s, "inp");
    run(
        "data out; set inp; retain maxage 0; if age > maxage then maxage = age; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    let maxage = ds.df.column("maxage").unwrap().f64().unwrap();
    // 14 dès la 1re obs, retenu ensuite (. > 14 faux, 13 > 14 faux).
    assert_eq!(maxage.get(0), Some(14.0));
    assert_eq!(maxage.get(1), Some(14.0));
    assert_eq!(maxage.get(2), Some(14.0));
}

#[test]
fn retain_initial_value_wins_over_sum_zero() {
    let mut s = session();
    write_class(&s, "inp");
    run("data out; set inp; n + 1; retain n 100; run;", &mut s).unwrap();
    let ds = read_work(&s, "out");
    let n = ds.df.column("n").unwrap().f64().unwrap();
    assert_eq!(n.get(0), Some(101.0));
    assert_eq!(n.get(2), Some(103.0));
}

#[test]
fn retain_without_init_keeps_value_across_iterations() {
    let mut s = session();
    write_class(&s, "inp");
    // prev : Name de l'observation précédente ('' à la 1re itération).
    run(
        "data out; set inp; retain prev; output; prev = name; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    let prev = ds.df.column("prev").unwrap().str().unwrap();
    assert_eq!(prev.get(0), Some(""));
    assert_eq!(prev.get(1), Some("Alfred"));
    assert_eq!(prev.get(2), Some("Alice"));
}

#[test]
fn retain_char_init_truncated_to_fixed_length() {
    let mut s = session();
    // c figée Char(3) par LENGTH ; l'init RETAIN 'abcdef' est tronquée
    // par le pdv.set normal au moment de poser les valeurs initiales.
    run("data out; length c $ 3; retain c 'abcdef'; run;", &mut s).unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("c").unwrap().str().unwrap().get(0), Some("abc"));
}

#[test]
fn retain_date_literal_bare_suffix() {
    // `retain d 21710d;` — 21710 est la valeur SAS date (2019-06-14).
    let mut s = session();
    run("data out; retain d 21710d; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "d", 0), Some(21710.0));
}

#[test]
fn retain_date_literal_quoted() {
    // `retain d '01JAN1960'd;` — l'époque SAS = 0.
    let mut s = session();
    run("data out; retain d '01JAN1960'd; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "d", 0), Some(0.0));
    // '02JAN1960'd = 1.
    let mut s2 = session();
    run("data out; retain e '02JAN1960'd; run;", &mut s2).unwrap();
    assert_eq!(num_at(&s2, "out", "e", 0), Some(1.0));
}

#[test]
fn retain_datetime_literal() {
    // `retain dt '01JAN1960 00:00:00'dt;` = 0 secondes depuis l'époque.
    let mut s = session();
    run(
        "data out; retain dt '01JAN1960 00:00:00'dt; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "dt", 0), Some(0.0));
    // '01JAN1960 00:01:00'dt = 60 secondes.
    let mut s2 = session();
    run(
        "data out; retain dt '01JAN1960 00:01:00'dt; run;",
        &mut s2,
    )
    .unwrap();
    assert_eq!(num_at(&s2, "out", "dt", 0), Some(60.0));
}

#[test]
fn retain_date_literal_is_retained_across_iterations() {
    // La valeur initiale issue d'un littéral date est bien RETENUE :
    // on l'incrémente à chaque obs lue.
    let mut s = session();
    write_class(&s, "inp");
    run(
        "data out; set inp; retain d 100d; d = d + 1; run;",
        &mut s,
    )
    .unwrap();
    // 100 (initial) +1 par obs : 101, 102, 103.
    assert_eq!(num_at(&s, "out", "d", 0), Some(101.0));
    assert_eq!(num_at(&s, "out", "d", 1), Some(102.0));
    assert_eq!(num_at(&s, "out", "d", 2), Some(103.0));
}

/// RETAIN _ALL_ : toutes les variables connues sont retenues à travers les
/// itérations.
#[test]
fn retain_all_retains_every_variable() {
    let mut s = session();
    write_num_ds(&s, "inp", &[("x", some(&[1.0, 2.0, 3.0]))]);
    run(
        "data out; set inp; retain a b; a = 0; b = 0; retain _all_; \
         a = a + x; b = b + 1; run;",
        &mut s,
    )
    .unwrap();
    // a et b retenus (cumul) : a = 1,3,6 ; b = 1,2,3. Mais `a=0;b=0;` les
    // remet à 0 AVANT le cumul, à CHAQUE itération → a=x, b=1. On teste donc
    // que la retenue n'empêche pas la ré-assignation explicite : a=1,2,3.
    assert_eq!(col(&s, "out", "a"), vec![Some(1.0), Some(2.0), Some(3.0)]);
    assert_eq!(col(&s, "out", "b"), vec![Some(1.0), Some(1.0), Some(1.0)]);
}

/// RETAIN _ALL_ : effet cumulatif réel (pas de remise à missing entre
/// itérations) pour une variable jamais ré-initialisée. `t` est initialisé
/// à 0 à la 1re itération seulement, puis RETAIN _ALL_ le préserve.
#[test]
fn retain_all_accumulates() {
    let mut s = session();
    write_num_ds(&s, "inp", &[("x", some(&[1.0, 2.0, 3.0, 4.0]))]);
    run(
        "data out; set inp; if _n_ = 1 then t = 0; retain _all_; t = t + x; run;",
        &mut s,
    )
    .unwrap();
    // t=0 à la 1re obs, retenu ensuite (jamais remis à missing) → cumul :
    // 1, 3, 6, 10.
    assert_eq!(
        col(&s, "out", "t"),
        vec![Some(1.0), Some(3.0), Some(6.0), Some(10.0)]
    );
}

/// RETAIN _ALL_ mélangé à un RETAIN explicite avec valeur initiale : la
/// valeur initiale du RETAIN explicite est honorée.
#[test]
fn retain_all_mixed_with_explicit_retain() {
    let mut s = session();
    write_num_ds(&s, "inp", &[("x", some(&[1.0, 2.0, 3.0]))]);
    run(
        "data out; set inp; retain base 100; if _n_ = 1 then sum = 0; \
         retain _all_; sum = sum + x; run;",
        &mut s,
    )
    .unwrap();
    // base retenu avec init 100 (jamais réassigné) ; sum cumulé.
    assert_eq!(
        col(&s, "out", "base"),
        vec![Some(100.0), Some(100.0), Some(100.0)]
    );
    assert_eq!(col(&s, "out", "sum"), vec![Some(1.0), Some(3.0), Some(6.0)]);
}

/// RETAIN _ALL_ n'autorise pas de valeur initiale.
#[test]
fn retain_all_rejects_initial_value() {
    let mut s = session();
    let e = run_err("data out; x = 1; retain _all_ 5; run;", &mut s);
    assert!(e.contains("initial value"), "got: {e}");
}

#[test]
fn length_truncates_longer_assignment() {
    let mut s = session();
    let stats = run("data out; length c $ 3; c = 'abcdef'; run;", &mut s).unwrap();
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 1)]);
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("c").unwrap().str().unwrap().get(0), Some("abc"));
    assert_eq!(ds.vars[0].length, 3);
}

#[test]
fn do_to_sums_one_to_ten() {
    let mut s = session();
    run("data out; s = 0; do i = 1 to 10; s = s + i; end; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "s", 0), Some(55.0));
}

#[test]
fn index_is_four_after_do_one_to_three() {
    let mut s = session();
    // Règle SAS célèbre : à la sortie par le test TO, i vaut la
    // PREMIÈRE valeur qui dépasse.
    run("data out; do i = 1 to 3; end; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "i", 0), Some(4.0));
}

#[test]
fn do_negative_by_runs_three_times() {
    let mut s = session();
    run("data out; do i = 3 to 1 by -1; n + 1; end; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(3.0));
    // Sortie par le test TO : i a dépassé vers le bas.
    assert_eq!(num_at(&s, "out", "i", 0), Some(0.0));
}

#[test]
fn do_fractional_by() {
    let mut s = session();
    // 1, 1.5, 2, 2.5, 3 → 5 tours ; i == 3.5 après.
    run("data out; do i = 1 to 3 by 0.5; n + 1; end; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(5.0));
    assert_eq!(num_at(&s, "out", "i", 0), Some(3.5));
}

#[test]
fn do_while_clause_cuts_iteration() {
    let mut s = session();
    // WHILE testé avant chaque tour : coupe à i = 4 (3 tours).
    run(
        "data out; do i = 1 to 10 while(i < 4); n + 1; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(3.0));
    assert_eq!(num_at(&s, "out", "i", 0), Some(4.0));
}

#[test]
fn do_while_false_runs_zero_times() {
    let mut s = session();
    run("data out; n = 0; do while(0); n + 1; end; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(0.0));
}

#[test]
fn do_until_runs_at_least_once() {
    let mut s = session();
    run("data out; do until(1); n + 1; end; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(1.0));
}

#[test]
fn pure_do_while_loops_until_condition_false() {
    let mut s = session();
    run("data out; x = 0; do while(x < 3); x = x + 1; end; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "x", 0), Some(3.0));
}

#[test]
fn delete_filters_missing_age() {
    let mut s = session();
    write_class(&s, "inp");
    // 3 obs dont 1 age missing → 2 obs en sortie.
    let stats = run("data out; set inp; if age = . then delete; run;", &mut s).unwrap();
    assert_eq!(stats.read, vec![("WORK.INP".to_string(), 3)]);
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 2, 2)]);
    let ds = read_work(&s, "out");
    let name = ds.df.column("Name").unwrap().str().unwrap();
    assert_eq!(name.get(0), Some("Alfred"));
    assert_eq!(name.get(1), Some("Barbara"));
}

#[test]
fn delete_inside_do_exits_loop_and_iteration() {
    let mut s = session();
    write_class(&s, "inp");
    // Chaque itération entre dans le DO et DELETE à i = 2 : le Flow
    // NextIter traverse la boucle → aucune obs en sortie, tout est lu.
    let stats = run(
        "data out; set inp; do i = 1 to 10; if i = 2 then delete; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(stats.read, vec![("WORK.INP".to_string(), 3)]);
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 0, 3)]);
}

#[test]
fn nested_do_loops() {
    let mut s = session();
    run(
        "data out; do i = 1 to 3; do j = 1 to 2; n + 1; end; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(6.0));
    assert_eq!(num_at(&s, "out", "i", 0), Some(4.0));
    assert_eq!(num_at(&s, "out", "j", 0), Some(3.0));
}

#[test]
fn do_bounds_are_evaluated_once_at_entry() {
    let mut s = session();
    // n modifié dans le corps : la borne TO reste celle de l'entrée
    // (3) — règle SAS, les bornes sont figées.
    run(
        "data out; n = 3; do i = 1 to n; n = 0; c + 1; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "c", 0), Some(3.0));
    assert_eq!(num_at(&s, "out", "i", 0), Some(4.0));
}

#[test]
fn missing_do_bound_is_runtime_error() {
    let mut s = session();
    // m jamais assignée → missing : erreur de contrôle de boucle
    // (divergence documentée : stoppe l'étape).
    let err = run("data out; do i = 1 to m; end; run;", &mut s)
        .err()
        .unwrap();
    assert_eq!(err.to_string(), "Invalid DO loop control information.");
}

#[test]
fn missing_subscript_stops_step_with_error() {
    let out = crate::run(
        "data out; array a{3} x y z; i = .; a{i} = 1; run;",
        crate::RunOptions {
            work_dir: None,
            base_dir: None,
            deterministic: true,
            vectorize: false,
        },
    );
    assert_eq!(out.exit_code, 2, "log was:\n{}", out.log);
    assert!(
        out.log.contains("ERROR: Array subscript out of range."),
        "log was:\n{}",
        out.log
    );
}

#[test]
fn missing_by_key_collates_first() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", vec![None, Some(2.0)])]);
    write_num_ds(&s, "b", &[("x", some(&[1.0]))]);
    run("data out; set a b; by x; f = first.x; run;", &mut s).unwrap();
    // `.` < 1 < 2 (les missings se collationnent en premier).
    assert_eq!(col(&s, "out", "x"), vec![None, Some(1.0), Some(2.0)]);
    assert_eq!(col(&s, "out", "f"), some(&[1.0, 1.0, 1.0]));
}

#[test]
fn missing_over_zero_does_not_emit_division_note() {
    let mut s = session();
    run("data out; m = .; r = m / 0; e = _error_; run;", &mut s).unwrap();
    // missing/0 : propagation missing, PAS une division par zéro.
    assert_eq!(num_at(&s, "out", "e", 0), Some(0.0));
    assert_eq!(num_at(&s, "out", "r", 0), None);
    let log = s.log.into_string();
    assert!(!log.contains("Division by zero"), "log was: {log}");
    assert!(log.contains("Missing values were generated"), "log was: {log}");
}

#[test]
fn modifying_index_in_body_affects_loop() {
    let mut s = session();
    // L'index est une variable normale du PDV : i = i + 1 dans le
    // corps saute une valeur sur deux → 5 tours (1,3,5,7,9).
    run(
        "data out; do i = 1 to 10; i = i + 1; n + 1; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(5.0));
    assert_eq!(num_at(&s, "out", "i", 0), Some(11.0));
}

#[test]
fn infinite_do_loop_guard_trips() {
    let mut s = session();
    let err = run("data out; do while(1); end; run;", &mut s)
        .err()
        .unwrap();
    assert_eq!(
        err.to_string(),
        "DO loop exceeded 10000000 iterations; stopping (possible infinite loop)."
    );
}
