use super::*;

// ── ARRAY 1-D + indexation (M2, lot 3) ───────────────────────────────

#[test]
fn array_fill_via_do_loop_braces() {
    let mut s = session();
    run(
        "data out; array a{3} x y z; do i = 1 to 3; a{i} = i * 10; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "x", 0), Some(10.0));
    assert_eq!(num_at(&s, "out", "y", 0), Some(20.0));
    assert_eq!(num_at(&s, "out", "z", 0), Some(30.0));
}

#[test]
fn array_paren_form_lvalue_and_rvalue() {
    let mut s = session();
    // Lvalue `a(i) = ...` et rvalue `a(i)` (l'array masque la
    // fonction) ; lecture croisée via t = a(1) + a(3).
    run(
        "data out; array a(3) x y z; do i = 1 to 3; a(i) = i * 10; end; \
         t = a(1) + a(3); run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "x", 0), Some(10.0));
    assert_eq!(num_at(&s, "out", "y", 0), Some(20.0));
    assert_eq!(num_at(&s, "out", "z", 0), Some(30.0));
    assert_eq!(num_at(&s, "out", "t", 0), Some(40.0));
}

#[test]
fn array_sum_via_dim() {
    let mut s = session();
    run(
        "data out; array a{3} x y z; do i = 1 to 3; a{i} = i; end; \
         do i = 1 to dim(a); s + a{i}; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "s", 0), Some(6.0));
}

#[test]
fn array_index_rounds_to_nearest() {
    let mut s = session();
    // 1.4 → 1, 2.6 → 3 (arrondi au plus proche, comme SAS).
    run(
        "data out; array a{3} x y z; a{1.4} = 7; a{2.6} = 9; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "x", 0), Some(7.0));
    assert_eq!(num_at(&s, "out", "z", 0), Some(9.0));
    assert_eq!(num_at(&s, "out", "y", 0), None);
}

// ── M16.2 : arrays multi-dimensionnels, valeurs initiales, DIM/HBOUND/
//    LBOUND, _TEMPORARY_/_NUMERIC_/_CHARACTER_/_ALL_ ─────────────────

#[test]
fn array_2d_creation_and_access_row_major() {
    let mut s = session();
    // 2×3 array sur 6 variables ; remplissage row-major v(i,j) = i*10+j.
    run(
        "data out; array m{2,3} v1-v6; do i = 1 to 2; do j = 1 to 3; \
         m{i,j} = i*10 + j; end; end; run;",
        &mut s,
    )
    .unwrap();
    // Ordre row-major : v1=m(1,1), v2=m(1,2), v3=m(1,3), v4=m(2,1)...
    assert_eq!(num_at(&s, "out", "v1", 0), Some(11.0));
    assert_eq!(num_at(&s, "out", "v2", 0), Some(12.0));
    assert_eq!(num_at(&s, "out", "v3", 0), Some(13.0));
    assert_eq!(num_at(&s, "out", "v4", 0), Some(21.0));
    assert_eq!(num_at(&s, "out", "v5", 0), Some(22.0));
    assert_eq!(num_at(&s, "out", "v6", 0), Some(23.0));
}

#[test]
fn array_3d_creation_and_access() {
    let mut s = session();
    // 2×3×2 = 12 slots, éléments auto-nommés t1..t12.
    run(
        "data out; array t{2,3,2}; \
         t{1,1,1} = 1; t{1,1,2} = 2; t{2,3,2} = 99; \
         a = t{1,1,1}; b = t{1,1,2}; c = t{2,3,2}; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "a", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "b", 0), Some(2.0));
    assert_eq!(num_at(&s, "out", "c", 0), Some(99.0));
    // t1 = (1,1,1) ; t12 = (2,3,2).
    assert_eq!(num_at(&s, "out", "t1", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "t12", 0), Some(99.0));
}

#[test]
fn array_linear_index_on_multidim() {
    let mut s = session();
    // Accès linéaire `m{n}` sur un array 2-D (interprétation row-major).
    run(
        "data out; array m{2,3} v1-v6; do n = 1 to 6; m{n} = n*n; end; \
         a = m{1,1}; f = m{2,3}; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "v1", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "v6", 0), Some(36.0));
    assert_eq!(num_at(&s, "out", "a", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "f", 0), Some(36.0));
}

#[test]
fn array_initial_values_row_major() {
    let mut s = session();
    run(
        "data out; array a{2,2} (1, 2, 3, 4); \
         p = a{1,1}; q = a{1,2}; r = a{2,1}; t = a{2,2}; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "p", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "q", 0), Some(2.0));
    assert_eq!(num_at(&s, "out", "r", 0), Some(3.0));
    assert_eq!(num_at(&s, "out", "t", 0), Some(4.0));
}

#[test]
fn array_initial_values_space_separated_1d() {
    let mut s = session();
    run("data out; array a{3} x y z (10 20 30); run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "x", 0), Some(10.0));
    assert_eq!(num_at(&s, "out", "y", 0), Some(20.0));
    assert_eq!(num_at(&s, "out", "z", 0), Some(30.0));
}

#[test]
fn array_dim_hbound_lbound_functions() {
    let mut s = session();
    run(
        "data out; array m{2,3} v1-v6; \
         nd = dim(m); n1 = dim(m, 1); n2 = dim(m, 2); \
         hb = hbound(m); hb2 = hbound(m, 2); \
         lb = lbound(m); lb2 = lbound(m, 2); run;",
        &mut s,
    )
    .unwrap();
    // dim(m) sans n = 1re dimension = 2 ; dim(m,2) = 3.
    assert_eq!(num_at(&s, "out", "nd", 0), Some(2.0));
    assert_eq!(num_at(&s, "out", "n1", 0), Some(2.0));
    assert_eq!(num_at(&s, "out", "n2", 0), Some(3.0));
    // hbound = borne supérieure (= dim, lbound=1).
    assert_eq!(num_at(&s, "out", "hb", 0), Some(2.0));
    assert_eq!(num_at(&s, "out", "hb2", 0), Some(3.0));
    // lbound toujours 1.
    assert_eq!(num_at(&s, "out", "lb", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "lb2", 0), Some(1.0));
}

#[test]
fn array_dim_on_1d_array() {
    let mut s = session();
    run(
        "data out; array a{5} a1-a5; d = dim(a); h = hbound(a); l = lbound(a); run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "d", 0), Some(5.0));
    assert_eq!(num_at(&s, "out", "h", 0), Some(5.0));
    assert_eq!(num_at(&s, "out", "l", 0), Some(1.0));
}

#[test]
fn array_temporary_elements_not_in_output() {
    let mut s = session();
    run(
        "data out; array t{3} _temporary_ (100 200 300); \
         total = t{1} + t{2} + t{3}; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "total", 0), Some(600.0));
    let ds = read_work(&s, "out");
    // Les éléments temporaires ne sont PAS des colonnes de sortie.
    let cols: Vec<&str> = ds
        .df
        .get_column_names()
        .iter()
        .map(|s| s.as_str())
        .collect();
    assert_eq!(cols, vec!["total"], "temporary elements must not be output");
}

#[test]
fn array_temporary_retained_across_iterations() {
    let mut s = session();
    write_class(&s, "inp");
    // Les éléments _TEMPORARY_ sont retenus : un compteur accumule
    // (valeur initiale 0, puis +1 par itération).
    run(
        "data out; set inp; array acc{1} _temporary_ (0); \
         acc{1} = acc{1} + 1; n = acc{1}; run;",
        &mut s,
    )
    .unwrap();
    // 3 observations → n vaut 1, 2, 3 (retenu, pas remis à missing).
    assert_eq!(num_at(&s, "out", "n", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "n", 1), Some(2.0));
    assert_eq!(num_at(&s, "out", "n", 2), Some(3.0));
}

#[test]
fn array_numeric_special_list() {
    let mut s = session();
    // _NUMERIC_ : toutes les variables numériques déjà connues.
    run(
        "data out; x = 1; y = 2; z = 3; array nums{*} _numeric_; \
         d = dim(nums); s = 0; do i = 1 to dim(nums); s = s + nums{i}; end; run;",
        &mut s,
    )
    .unwrap();
    // x, y, z sont les 3 numériques (i, d, s entrent APRÈS l'ARRAY).
    assert_eq!(num_at(&s, "out", "d", 0), Some(3.0));
    assert_eq!(num_at(&s, "out", "s", 0), Some(6.0));
}

#[test]
fn array_character_special_list() {
    let mut s = session();
    run(
        "data out; a = 'foo'; b = 'bar'; array chs{*} $ _character_; \
         d = dim(chs); chs{1} = 'NEW'; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "d", 0), Some(2.0));
    let ds = read_work(&s, "out");
    // chs{1} pointe sur la 1re variable char (a).
    assert_eq!(
        ds.df.column("a").unwrap().str().unwrap().get(0),
        Some("NEW")
    );
}

#[test]
fn array_mixing_1d_and_multidim() {
    let mut s = session();
    // Une étape avec un array 1-D et un array 2-D coexistants.
    run(
        "data out; array a{3} a1-a3; array m{2,2} m1-m4; \
         do i = 1 to 3; a{i} = i; end; \
         m{1,1} = 9; m{2,2} = 8; \
         da = dim(a); dm = dim(m); dm2 = dim(m,2); run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "a2", 0), Some(2.0));
    assert_eq!(num_at(&s, "out", "m1", 0), Some(9.0));
    assert_eq!(num_at(&s, "out", "m4", 0), Some(8.0));
    assert_eq!(num_at(&s, "out", "da", 0), Some(3.0));
    assert_eq!(num_at(&s, "out", "dm", 0), Some(2.0));
    assert_eq!(num_at(&s, "out", "dm2", 0), Some(2.0));
}

#[test]
fn array_2d_out_of_bounds_stops_step() {
    // Indice de dimension hors bornes : arrêt avec ERROR.
    let out = crate::run(
        "data out; array m{2,3} v1-v6; m{3,1} = 1; run;",
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
fn array_2d_wrong_index_count_stops_step() {
    // 2 indices attendus, 3 fournis → hors bornes.
    let out = crate::run(
        "data out; array m{2,3} v1-v6; t = m{1,2,1}; run;",
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
fn array_initial_too_many_values_errors() {
    let mut s = session();
    match run("data out; array a{2} x y (1 2 3); run;", &mut s) {
        Err(e) => assert!(
            e.to_string().contains("Too many initial values"),
            "wrong error message: {e}"
        ),
        Ok(_) => panic!("expected too-many-initial-values error"),
    }
}

#[test]
fn array_dim_count_mismatch_errors() {
    let mut s = session();
    // 2×3 = 6 attendus, 4 variables fournies.
    match run("data out; array m{2,3} a b c d; run;", &mut s) {
        Err(e) => assert!(
            e.to_string().contains("does not match"),
            "wrong error message: {e}"
        ),
        Ok(_) => panic!("expected dimension-mismatch error"),
    }
}

#[test]
fn array_over_input_variables_updates_them() {
    let mut s = session();
    write_class(&s, "inp");
    // Array sur une variable d'input : l'élément référence le slot
    // existant (type/longueur de l'input conservés).
    run(
        "data out; set inp; array nums{1} age; nums{1} = nums{1} * 2; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    let age = ds.df.column("Age").unwrap().f64().unwrap();
    assert_eq!(age.get(0), Some(28.0));
    assert_eq!(age.get(2), Some(26.0));
}

#[test]
fn char_array_with_truncation() {
    let mut s = session();
    run(
        "data out; array c{2} $ 3 u v; c{1} = 'abcdef'; c{2} = 'xy'; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    // Longueur fixe 3 : troncature silencieuse à l'assignation.
    assert_eq!(
        ds.df.column("u").unwrap().str().unwrap().get(0),
        Some("abc")
    );
    assert_eq!(ds.df.column("v").unwrap().str().unwrap().get(0), Some("xy"));
    assert_eq!(ds.vars[0].length, 3);
}

#[test]
fn char_array_default_length_is_8() {
    let mut s = session();
    run(
        "data out; array c{1} $ u; c{1} = 'abcdefghij'; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(
        ds.df.column("u").unwrap().str().unwrap().get(0),
        Some("abcdefgh")
    );
    assert_eq!(ds.vars[0].length, 8);
}

#[test]
fn out_of_range_subscript_stops_step_with_error() {
    // Lvalue hors bornes : l'étape s'arrête avec ERROR (exit code 2).
    let out = crate::run(
        "data out; array a{3} x y z; a{4} = 1; run;",
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
    assert!(
        out.log
            .contains("The SAS System stopped processing this step because of errors.")
    );

    // Rvalue hors bornes (y compris indice 0) : même arrêt.
    let out = crate::run(
        "data out; array a{3} x y z; t = a{0}; run;",
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
fn auto_named_elements_are_usable_as_variables() {
    let mut s = session();
    // a1 a2 a3 auto-nommés : adressables par indice ET par nom.
    run(
        "data out; array a{3}; do i = 1 to 3; a{i} = i; end; t = a1 + a2 + a3; \
         a2 = 20; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "a1", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "a2", 0), Some(20.0));
    assert_eq!(num_at(&s, "out", "a3", 0), Some(3.0));
    assert_eq!(num_at(&s, "out", "t", 0), Some(6.0));
}
