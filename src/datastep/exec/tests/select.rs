use super::*;

// ── SELECT / WHEN / OTHERWISE (M16.1) ────────────────────────────────

#[test]
fn select_selector_form_matches_first_value() {
    // Sélecteur numérique : age 14 → "teen", . → autre, 13 → "kid".
    let mut s = session();
    write_class(&s, "inp");
    run(
        "data out; set inp; length grp $8; \
         select (age); \
           when (13) grp='kid'; \
           when (14, 15) grp='teen'; \
           otherwise grp='other'; \
         end; \
         run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    // age = 14, missing, 13.
    assert_eq!(
        str_col(&ds, "grp"),
        vec!["teen".to_string(), "other".to_string(), "kid".to_string()]
    );
}

#[test]
fn select_selector_multiple_values_in_one_when() {
    // Une seule clause liste plusieurs valeurs ; n'importe laquelle suffit.
    let mut s = session();
    run(
        "data out; \
         do x = 1 to 4; \
           select (x); \
             when (1, 3) flag = 1; \
             otherwise flag = 0; \
           end; \
           output; \
         end; \
         run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "flag", 0), Some(1.0)); // x=1
    assert_eq!(num_at(&s, "out", "flag", 1), Some(0.0)); // x=2
    assert_eq!(num_at(&s, "out", "flag", 2), Some(1.0)); // x=3
    assert_eq!(num_at(&s, "out", "flag", 3), Some(0.0)); // x=4
}

#[test]
fn select_selector_char_form() {
    // Sélecteur caractère ; comparaison ignore les blancs finaux (sas_cmp).
    let mut s = session();
    run(
        "data out; length sex $1 desc $8; \
         sex = 'F'; \
         select (sex); \
           when ('M') desc = 'male'; \
           when ('F') desc = 'female'; \
           otherwise desc = 'unknown'; \
         end; \
         output; \
         run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "desc"), vec!["female".to_string()]);
}

#[test]
fn select_boolean_form_first_true_wins() {
    // Forme booléenne : conditions évaluées dans l'ordre, première vraie.
    let mut s = session();
    run(
        "data out; length band $8; \
         do x = 5 to 25 by 10; \
           select; \
             when (x < 10) band = 'low'; \
             when (x < 20) band = 'mid'; \
             otherwise band = 'high'; \
           end; \
           output; \
         end; \
         run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    // x = 5, 15, 25.
    assert_eq!(
        str_col(&ds, "band"),
        vec!["low".to_string(), "mid".to_string(), "high".to_string()]
    );
}

#[test]
fn select_boolean_form_range_condition() {
    // Plage exprimée par une condition booléenne 1 <= x <= 10.
    let mut s = session();
    run(
        "data out; length r $8; \
         do x = 0 to 15 by 5; \
           select; \
             when (x >= 1 and x <= 10) r = 'in'; \
             otherwise r = 'out'; \
           end; \
           output; \
         end; \
         run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    // x = 0(out), 5(in), 10(in), 15(out).
    assert_eq!(
        str_col(&ds, "r"),
        vec![
            "out".to_string(),
            "in".to_string(),
            "in".to_string(),
            "out".to_string()
        ]
    );
}

#[test]
fn select_when_do_block_runs_all_statements() {
    // Le corps d'un WHEN peut être un do; ... end; (plusieurs statements).
    let mut s = session();
    run(
        "data out; x = 2; \
         select (x); \
           when (2) do; a = 10; b = 20; end; \
           otherwise do; a = 0; b = 0; end; \
         end; \
         output; \
         run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "a", 0), Some(10.0));
    assert_eq!(num_at(&s, "out", "b", 0), Some(20.0));
}

#[test]
fn select_no_fall_through() {
    // Pas de fall-through : seule la PREMIÈRE clause vraie s'exécute,
    // même si une clause suivante correspondrait aussi.
    let mut s = session();
    run(
        "data out; x = 1; n = 0; \
         select (x); \
           when (1) n = n + 1; \
           when (1) n = n + 100; \
           otherwise n = n + 1000; \
         end; \
         output; \
         run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(1.0));
}

#[test]
fn select_missing_value_matches_dot() {
    // `. = .` est vrai en SAS : un WHEN (.) capture le sélecteur missing.
    let mut s = session();
    write_class(&s, "inp");
    run(
        "data out; set inp; length tag $8; \
         select (age); \
           when (.) tag = 'na'; \
           otherwise tag = 'ok'; \
         end; \
         run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    // age = 14, missing, 13.
    assert_eq!(
        str_col(&ds, "tag"),
        vec!["ok".to_string(), "na".to_string(), "ok".to_string()]
    );
}

#[test]
fn select_no_otherwise_no_match_is_runtime_error() {
    // Sans OTHERWISE et sans WHEN correspondant : erreur runtime (SAS).
    let mut s = session();
    let err = run(
        "data out; x = 99; select (x); when (1) y = 1; end; run;",
        &mut s,
    )
    .err()
    .unwrap();
    assert!(
        err.to_string().contains("does not match any clause"),
        "got: {err}"
    );
}

#[test]
fn select_no_otherwise_with_match_is_ok() {
    // Sans OTHERWISE mais avec un WHEN correspondant : pas d'erreur.
    let mut s = session();
    run(
        "data out; x = 1; select (x); when (1) y = 7; end; output; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "y", 0), Some(7.0));
}

#[test]
fn select_empty_when_body_is_noop() {
    // `when (1) ;` corps vide : la clause est prise mais ne fait rien
    // (pas de fall-through vers OTHERWISE).
    let mut s = session();
    run(
        "data out; x = 1; y = 5; \
         select (x); \
           when (1) ; \
           otherwise y = 0; \
         end; \
         output; \
         run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "y", 0), Some(5.0));
}

#[test]
fn select_selector_evaluated_once_via_subsetting() {
    // Le sélecteur est une expression : 2*x. x=3 → 6 → "six".
    let mut s = session();
    run(
        "data out; length w $8; x = 3; \
         select (2 * x); \
           when (6) w = 'six'; \
           otherwise w = 'no'; \
         end; \
         output; \
         run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "w"), vec!["six".to_string()]);
}

// ── M16.3 : DO sur liste de valeurs, DO OVER, RETAIN littéraux date ───

#[test]
fn do_list_numeric_explicit_values() {
    // `do i = 1, 3, 5, 7;` — somme et dernière valeur.
    let mut s = session();
    run(
        "data out; s = 0; do i = 1, 3, 5, 7; s = s + i; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "s", 0), Some(16.0));
    // À la sortie d'une liste, l'index garde la DERNIÈRE valeur (≠ TO).
    assert_eq!(num_at(&s, "out", "i", 0), Some(7.0));
}

#[test]
fn do_list_unordered_values() {
    // Ordre quelconque honoré tel quel : 5, 1, 9.
    let mut s = session();
    run(
        "data out; n = 0; do i = 5, 1, 9; n + 1; last = i; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(3.0));
    assert_eq!(num_at(&s, "out", "last", 0), Some(9.0));
}

#[test]
fn do_list_single_value() {
    // `do i = 42;` — liste à un élément (boucle une fois).
    let mut s = session();
    run("data out; c = 0; do i = 42; c + 1; end; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "c", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "i", 0), Some(42.0));
}

#[test]
fn do_list_character_values() {
    // `do color = 'red', 'blue', 'green';` — char.
    let mut s = session();
    run(
        "data out; length color $5; n = 0; \
         do color = 'red', 'blue', 'green'; n + 1; end; run;",
        &mut s,
    )
    .unwrap();
    // Dernière valeur conservée ; n = 3.
    assert_eq!(num_at(&s, "out", "n", 0), Some(3.0));
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "color"), vec!["green".to_string()]);
}

#[test]
fn do_list_mixed_range_and_explicit() {
    // `do i = 1 to 5 by 2, 10, 20 to 22;` → 1,3,5,10,20,21,22 (7 valeurs).
    let mut s = session();
    run(
        "data out; n = 0; s = 0; \
         do i = 1 to 5 by 2, 10, 20 to 22; n + 1; s = s + i; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(7.0));
    // 1+3+5+10+20+21+22 = 82.
    assert_eq!(num_at(&s, "out", "s", 0), Some(82.0));
    // Dernière valeur = 22.
    assert_eq!(num_at(&s, "out", "i", 0), Some(22.0));
}

#[test]
fn do_list_range_first_then_values() {
    // `1 to 12 by 2, 0` : c'est une LISTE (à cause de la virgule) → le
    // range énumère 1,3,5,7,9,11 puis la valeur 0 ; 7 tours, index final 0.
    let mut s = session();
    run(
        "data out; n = 0; do month = 1 to 12 by 2, 0; n + 1; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(7.0));
    // En LISTE, l'index garde la dernière valeur (≠ TO classique).
    assert_eq!(num_at(&s, "out", "month", 0), Some(0.0));
}

#[test]
fn do_over_1d_iterates_all_elements() {
    // DO OVER 1-D : `arr` nu = élément courant ; on double chaque élément.
    let mut s = session();
    run(
        "data out; array a{5} v1-v5; \
         do i = 1 to 5; a{i} = i; end; \
         do over a; a = a * 10; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "v1", 0), Some(10.0));
    assert_eq!(num_at(&s, "out", "v2", 0), Some(20.0));
    assert_eq!(num_at(&s, "out", "v3", 0), Some(30.0));
    assert_eq!(num_at(&s, "out", "v4", 0), Some(40.0));
    assert_eq!(num_at(&s, "out", "v5", 0), Some(50.0));
}

#[test]
fn do_over_1d_reads_current_element_into_accumulator() {
    // `arr` en lecture nue dans une accumulation.
    let mut s = session();
    run(
        "data out; array a{4} v1-v4 (3 6 9 12); \
         tot = 0; do over a; tot = tot + a; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "tot", 0), Some(30.0));
}

#[test]
fn do_over_static_indexed_access_inside_loop() {
    // Accès indexé `a{1}` reste STATIQUE même dans DO OVER : on lit le
    // premier élément à chaque tour.
    let mut s = session();
    run(
        "data out; array a{3} v1-v3 (5 6 7); \
         firstsum = 0; do over a; firstsum = firstsum + a{1}; end; run;",
        &mut s,
    )
    .unwrap();
    // a{1}=5 lu 3 fois → 15.
    assert_eq!(num_at(&s, "out", "firstsum", 0), Some(15.0));
}

#[test]
fn do_over_multidim_row_major_order() {
    // DO OVER sur un array 2×3 : itération row-major (= ordre des slots).
    // On affecte des valeurs croissantes par tour pour vérifier l'ordre.
    let mut s = session();
    run(
        "data out; array m{2,3} v1-v6; \
         k = 0; do over m; k + 1; m = k; end; run;",
        &mut s,
    )
    .unwrap();
    // Row-major : v1=1, v2=2, ..., v6=6.
    assert_eq!(num_at(&s, "out", "v1", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "v2", 0), Some(2.0));
    assert_eq!(num_at(&s, "out", "v3", 0), Some(3.0));
    assert_eq!(num_at(&s, "out", "v4", 0), Some(4.0));
    assert_eq!(num_at(&s, "out", "v5", 0), Some(5.0));
    assert_eq!(num_at(&s, "out", "v6", 0), Some(6.0));
}

#[test]
fn do_over_char_array() {
    // DO OVER sur array caractère : uppercase de chaque élément.
    let mut s = session();
    run(
        "data out; array c{3} $3 a b cc; \
         a = 'foo'; b = 'bar'; cc = 'baz'; \
         do over c; c = upcase(c); end; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "a"), vec!["FOO".to_string()]);
    assert_eq!(str_col(&ds, "b"), vec!["BAR".to_string()]);
    assert_eq!(str_col(&ds, "cc"), vec!["BAZ".to_string()]);
}

#[test]
fn do_over_then_index_value_independent() {
    // Intégration M16.2 : DO OVER puis accès indexé hors boucle.
    let mut s = session();
    run(
        "data out; array a{3} x y z (1 2 3); \
         do over a; a = a + 100; end; \
         p = a{2}; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "x", 0), Some(101.0));
    assert_eq!(num_at(&s, "out", "y", 0), Some(102.0));
    assert_eq!(num_at(&s, "out", "z", 0), Some(103.0));
    assert_eq!(num_at(&s, "out", "p", 0), Some(102.0));
}
