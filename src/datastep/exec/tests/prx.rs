use super::*;

// ── PRX : fonctions et CALL routines (M40.1) ─────────────────────────
//
// Les valeurs attendues sont les résultats SAS 9.4 documentés (positions
// 1-based en caractères, 0 = pas de match).

#[test]
fn prxmatch_with_literal_patterns() {
    let mut s = session();
    run(
        "data out; p1 = prxmatch('/b+/', 'abbb'); p2 = prxmatch('/z/', 'abc'); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(num_col(&ds, "p1"), vec![Some(2.0)]);
    assert_eq!(num_col(&ds, "p2"), vec![Some(0.0)]);
}

#[test]
fn prxparse_once_idiom_with_retain() {
    // Idiome SAS classique : compilation à _N_=1 + RETAIN de l'id.
    let mut s = session();
    write_class(&s, "inp");
    run(
        "data out; set inp; retain re; if _n_ = 1 then re = prxparse('/a/i'); \
         p = prxmatch(re, name); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    // Alfred → 1, Alice → 1, Barbara → 2.
    assert_eq!(num_col(&ds, "p"), vec![Some(1.0), Some(1.0), Some(2.0)]);
}

// ---- CALL PRXCHANGE -------------------------------------------------

#[test]
fn call_prxchange_writes_result_with_backreferences() {
    let mut s = session();
    run(
        "data out; length r $20; re = prxparse('s/(\\w+), (\\w+)/$2 $1/'); \
         call prxchange(re, -1, 'Doe, John', r); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "r"), vec!["John Doe".to_string()]);
}

#[test]
fn call_prxchange_truncates_and_reports() {
    // Résultat complet "bXXnXXnXX" (9) tronqué à $4 ; res-length = longueur
    // stockée, trunc-value = 1, n-changes = 3.
    let mut s = session();
    run(
        "data out; length r $4; re = prxparse('s/a/XX/'); \
         call prxchange(re, -1, 'banana', r, rlen, tr, nc); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "r"), vec!["bXXn".to_string()]);
    assert_eq!(num_col(&ds, "rlen"), vec![Some(4.0)]);
    assert_eq!(num_col(&ds, "tr"), vec![Some(1.0)]);
    assert_eq!(num_col(&ds, "nc"), vec![Some(3.0)]);
}

#[test]
fn call_prxchange_in_place_without_result_arg() {
    // Sans 4e argument, la source (lvalue) est modifiée en place.
    let mut s = session();
    run(
        "data out; length s $12; s = 'banana'; re = prxparse('s/a/o/'); \
         call prxchange(re, 1, s); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "s"), vec!["bonana".to_string()]);
}

// ---- CALL PRXSUBSTR + PRXPOSN ---------------------------------------

#[test]
fn call_prxsubstr_then_call_prxposn_groups() {
    let mut s = session();
    run(
        "data out; re = prxparse('/(\\d+)-(\\d+)/'); \
         call prxsubstr(re, 'abc 12-345', p, l); \
         call prxposn(re, 2, p2, l2); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(num_col(&ds, "p"), vec![Some(5.0)]);
    assert_eq!(num_col(&ds, "l"), vec![Some(6.0)]);
    assert_eq!(num_col(&ds, "p2"), vec![Some(8.0)]);
    assert_eq!(num_col(&ds, "l2"), vec![Some(3.0)]);
}

#[test]
fn call_prxsubstr_no_match_gives_zeros() {
    let mut s = session();
    run(
        "data out; re = prxparse('/z+/'); call prxsubstr(re, 'abc', p, l); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(num_col(&ds, "p"), vec![Some(0.0)]);
    assert_eq!(num_col(&ds, "l"), vec![Some(0.0)]);
}

#[test]
fn call_prx_output_args_are_not_uninitialized() {
    // p/l (PRXSUBSTR), p2/l2 (PRXNEXT), r/rlen/tr/nc (PRXCHANGE) ne sont
    // écrites QUE par les routines : SAS ne les note pas "uninitialized".
    // `start` (PRXNEXT) est lu avant d'être écrit : assigné ici pour ne
    // tester QUE les positions de sortie.
    let mut s = session();
    run(
        "data out; length r $20; re = prxparse('/a/'); \
         call prxsubstr(re, 'banana', p, l); \
         start = 1; \
         call prxnext(re, start, -1, 'banana', p2, l2); \
         re2 = prxparse('s/a/o/'); \
         call prxchange(re2, -1, 'banana', r, rlen, tr, nc); \
         output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    // Les valeurs prouvent que les routines ont bien écrit.
    assert_eq!(num_col(&ds, "p"), vec![Some(2.0)]);
    assert_eq!(num_col(&ds, "l"), vec![Some(1.0)]);
    assert_eq!(num_col(&ds, "p2"), vec![Some(2.0)]);
    assert_eq!(num_col(&ds, "l2"), vec![Some(1.0)]);
    assert_eq!(str_col(&ds, "r"), vec!["bonono".to_string()]);
    let log = s.log.into_string();
    assert!(!log.contains("uninitialized"), "log was: {log}");
}

#[test]
fn prxposn_function_after_call_prxsubstr() {
    // La fonction PRXPOSN relit le groupe mémorisé par CALL PRXSUBSTR.
    let mut s = session();
    run(
        "data out; length g $8; re = prxparse('/(\\d+)-(\\d+)/'); \
         call prxsubstr(re, 'abc 12-345', p); \
         g = prxposn(re, 1, 'abc 12-345'); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "g"), vec!["12".to_string()]);
}

// ---- CALL PRXNEXT ---------------------------------------------------

#[test]
fn call_prxnext_iterates_all_matches() {
    // Boucle d'extraction : start avance à position + MAX(length, 1) après
    // chaque match ; position = 0 quand il n'y a plus de match.
    let mut s = session();
    run(
        "data out; re = prxparse('/\\d+/'); s = 'a1 bb22 c333'; \
         start = 1; lim = length(s); \
         call prxnext(re, start, lim, s, p, l); \
         do while (p > 0); output; call prxnext(re, start, lim, s, p, l); end; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(num_col(&ds, "p"), vec![Some(2.0), Some(6.0), Some(10.0)]);
    assert_eq!(num_col(&ds, "l"), vec![Some(1.0), Some(2.0), Some(3.0)]);
    // start déjà avancé au moment de chaque OUTPUT.
    assert_eq!(
        num_col(&ds, "start"),
        vec![Some(3.0), Some(8.0), Some(13.0)]
    );
}

// ---- CALL PRXFREE ---------------------------------------------------

#[test]
fn call_prxfree_invalidates_the_id() {
    let mut s = session();
    let err = run_err(
        "data _null_; re = prxparse('/a/'); call prxfree(re); x = prxmatch(re, 'abc'); run;",
        &mut s,
    );
    assert!(err.contains("Invalid regular expression id"), "got: {err}");
}

#[test]
fn call_prxdebug_is_a_silent_noop() {
    let mut s = session();
    run(
        "data _null_; call prxdebug(1); call prxdebug(0); run;",
        &mut s,
    )
    .unwrap();
}

// ---- Pattern invalide -----------------------------------------------

#[test]
fn invalid_pattern_yields_missing_and_logs_error() {
    let mut s = session();
    run(
        "data out; re = prxparse('/(/'); m = missing(re); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(num_col(&ds, "m"), vec![Some(1.0)]);
    let log = s.log.into_string();
    assert!(
        log.contains("ERROR:") && log.contains("/(/"),
        "log was: {log}"
    );
}
