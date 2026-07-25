use super::*;

// ── M16.6 : LINK / RETURN / GOTO / labels / RETAIN _ALL_ ─────────────

/// LINK/RETURN de base : appel d'une sous-routine étiquetée, retour après.
/// Structure SAS idiomatique : la ligne principale se termine par un RETURN
/// (output implicite), puis les sous-routines suivent.
#[test]
fn link_basic_call_and_return() {
    let mut s = session();
    run(
        "data out; x = 1; link sub; y = x; return; \
         sub: x = 10; return; \
         run;",
        &mut s,
    )
    .unwrap();
    // x=1, LINK sub → x=10, RETURN → reprise : y=x=10, RETURN principal →
    // output implicite (la sous-routine n'est pas exécutée en chute).
    assert_eq!(col(&s, "out", "x"), vec![Some(10.0)]);
    assert_eq!(col(&s, "out", "y"), vec![Some(10.0)]);
}

/// LINK imbriqué : la pile d'adresses de retour est correcte.
#[test]
fn link_nested_stack() {
    let mut s = session();
    run(
        "data out; a = 0; link one; a = a + 1; return; \
         one: a = a + 10; link two; a = a + 100; return; \
         two: a = a + 1000; return; \
         run;",
        &mut s,
    )
    .unwrap();
    // a: 0 → link one → +10 (10) → link two → +1000 (1010) → return one
    // → +100 (1110) → return main → +1 (1111). stop.
    assert_eq!(col(&s, "out", "a"), vec![Some(1111.0)]);
}

/// LINK vers une étiquette inexistante : erreur de compilation.
#[test]
fn link_undefined_label_compile_error() {
    let mut s = session();
    let e = run_err("data out; x = 1; link nowhere; run;", &mut s);
    assert!(
        e.contains("NOWHERE") && e.contains("not defined"),
        "got: {e}"
    );
}

/// LINK dans une boucle : l'itération de la boucle reprend après le retour.
#[test]
fn link_inside_do_loop_continues_iteration() {
    let mut s = session();
    run(
        "data out; total = 0; \
         do i = 1 to 4; link addit; end; \
         return; \
         addit: total = total + i; return; \
         run;",
        &mut s,
    )
    .unwrap();
    // total = 1+2+3+4 = 10 ; la boucle continue après chaque RETURN.
    assert_eq!(col(&s, "out", "total"), vec![Some(10.0)]);
    assert_eq!(col(&s, "out", "i"), vec![Some(5.0)]);
}

/// Modifications de variables dans le code LINKé : persistance (PDV partagé).
#[test]
fn link_modifications_persist() {
    let mut s = session();
    run(
        "data out; x = 5; y = 0; link doit; z = x + y; return; \
         doit: x = x * 2; y = 100; return; run;",
        &mut s,
    )
    .unwrap();
    // doit : x=10, y=100 (persistants) ; z = 10 + 100 = 110.
    assert_eq!(col(&s, "out", "x"), vec![Some(10.0)]);
    assert_eq!(col(&s, "out", "y"), vec![Some(100.0)]);
    assert_eq!(col(&s, "out", "z"), vec![Some(110.0)]);
}

/// LINK depuis une sous-routine LINK vers une troisième : pile à 2 niveaux,
/// chaque RETURN reprend au bon endroit.
#[test]
fn link_chain_returns_in_order() {
    let mut s = session();
    run(
        "data out; trace = 0; link a; trace = trace * 10 + 9; return; \
         a: trace = trace * 10 + 1; link b; trace = trace * 10 + 2; return; \
         b: trace = trace * 10 + 3; return; \
         run;",
        &mut s,
    )
    .unwrap();
    // 0 → a: *10+1 = 1 → b: *10+3 = 13 → ret a: *10+2 = 132 → ret main:
    // *10+9 = 1329.
    assert_eq!(col(&s, "out", "trace"), vec![Some(1329.0)]);
}

/// GOTO/LINK fonctionnent sur plusieurs itérations d'un SET (la pile de
/// retour est ré-initialisée à chaque itération).
#[test]
fn link_resets_per_iteration() {
    let mut s = session();
    write_num_ds(&s, "inp", &[("x", some(&[1.0, 2.0, 3.0]))]);
    run(
        "data out; set inp; link dbl; stop_marker: ; goto past; \
         dbl: d = x * 2; return; \
         past: ; run;",
        &mut s,
    )
    .unwrap();
    // Chaque itération : link dbl (d=2x), retour, goto past (saute rien),
    // output implicite. d = 2,4,6 sur les 3 obs.
    assert_eq!(
        col(&s, "out", "d"),
        vec![Some(2.0), Some(4.0), Some(6.0)]
    );
}

/// GOTO : saut inconditionnel (les statements entre le GOTO et la cible
/// sont ignorés).
#[test]
fn goto_unconditional_jump() {
    let mut s = session();
    run(
        "data out; x = 1; goto skip; x = 999; skip: y = x; run;",
        &mut s,
    )
    .unwrap();
    // x=1, GOTO skip → x=999 sauté, y=x=1 ; chute en fin → output implicite.
    assert_eq!(col(&s, "out", "x"), vec![Some(1.0)]);
    assert_eq!(col(&s, "out", "y"), vec![Some(1.0)]);
}

/// GOTO qui sort d'une boucle DO (termine la boucle prématurément).
#[test]
fn goto_breaks_out_of_do_loop() {
    let mut s = session();
    run(
        "data out; total = 0; \
         do i = 1 to 100; total = total + i; if i = 5 then goto done; end; \
         done: ; run;",
        &mut s,
    )
    .unwrap();
    // 1+2+3+4+5 = 15 ; la boucle est terminée par le GOTO à i=5.
    assert_eq!(col(&s, "out", "total"), vec![Some(15.0)]);
    assert_eq!(col(&s, "out", "i"), vec![Some(5.0)]);
}

/// GOTO avec plusieurs étiquettes : ciblage correct.
#[test]
fn goto_multiple_labels_targets_correctly() {
    let mut s = session();
    run(
        "data out; x = 1; goto third; \
         first: r = 1; goto fin; \
         second: r = 2; goto fin; \
         third: r = 3; goto fin; \
         fin: ; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "r"), vec![Some(3.0)]);
}

/// GOTO vers une étiquette inexistante : erreur de compilation.
#[test]
fn goto_undefined_label_compile_error() {
    let mut s = session();
    let e = run_err("data out; x = 1; goto nowhere; run;", &mut s);
    assert!(
        e.contains("NOWHERE") && e.contains("not defined"),
        "got: {e}"
    );
}

/// GOTO vers une étiquette imbriquée dans un bloc DO : non supporté (erreur).
#[test]
fn goto_into_nested_block_compile_error() {
    let mut s = session();
    let e = run_err(
        "data out; goto inner; do; inner: x = 1; end; stop; run;",
        &mut s,
    );
    assert!(e.contains("INNER") && e.contains("nested"), "got: {e}");
}

/// GOTO en arrière formant une boucle, terminée par une condition.
#[test]
fn goto_backward_forms_loop() {
    let mut s = session();
    run(
        "data out; n = 0; \
         loop: n = n + 1; if n < 5 then goto loop; \
         run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "n"), vec![Some(5.0)]);
}

/// Étiquette sur divers statements (ici un bloc DO et une assignation).
#[test]
fn label_on_various_statements() {
    let mut s = session();
    run(
        "data out; goto blk; a = 1; \
         blk: do; a = 7; b = 8; end; \
         after: c = 9; run;",
        &mut s,
    )
    .unwrap();
    // GOTO blk saute `a = 1` ; le bloc DO étiqueté pose a=7,b=8 ; puis c=9.
    assert_eq!(col(&s, "out", "a"), vec![Some(7.0)]);
    assert_eq!(col(&s, "out", "b"), vec![Some(8.0)]);
    assert_eq!(col(&s, "out", "c"), vec![Some(9.0)]);
}

/// RETURN sans LINK actif : termine l'itération (output implicite), pas une
/// erreur. La variable assignée APRÈS le RETURN n'est pas affectée.
#[test]
fn return_without_link_ends_iteration() {
    let mut s = session();
    let stats = run(
        "data out; x = 1; return; x = 2; run;",
        &mut s,
    )
    .unwrap();
    // RETURN sans LINK → fin d'itération avec output implicite : x=1.
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 1)]);
    assert_eq!(col(&s, "out", "x"), vec![Some(1.0)]);
}

/// Étiquette définie deux fois : erreur de compilation.
#[test]
fn duplicate_label_compile_error() {
    let mut s = session();
    let e = run_err("data out; lbl: x = 1; lbl: x = 2; goto lbl; run;", &mut s);
    assert!(e.contains("LBL") && e.contains("more than once"), "got: {e}");
}

/// Variable créée APRÈS RETAIN _ALL_ : NON retenue automatiquement (remise à
/// missing à chaque itération).
#[test]
fn variable_created_after_retain_all_not_retained() {
    let mut s = session();
    write_num_ds(&s, "inp", &[("x", some(&[5.0, 6.0, 7.0]))]);
    run(
        "data out; set inp; retain _all_; later = later + x; run;",
        &mut s,
    )
    .unwrap();
    // `later` n'existe PAS au point du RETAIN _ALL_ (créée par sa 1re
    // référence ensuite) → non retenue → remise à missing chaque itération
    // → later = . + x = . (missing propagé).
    assert_eq!(col(&s, "out", "later"), vec![None, None, None]);
}

/// `go to label;` (forme en deux mots) équivalente à `goto label;`.
#[test]
fn go_to_two_word_form() {
    let mut s = session();
    run(
        "data out; x = 1; go to skip; x = 999; skip: ; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "x"), vec![Some(1.0)]);
}
