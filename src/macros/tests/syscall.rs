//! M41.3 — `%SYSCALL SORTN`/`SORTC` et `%SYSMACDELETE`.

use super::*;

/// Compare le texte produit à une liste de tokens séparés par des blancs,
/// insensible au détail exact de l'espacement autour des statements
/// consommés (convention déjà utilisée par les autres tests macro pour ne
/// pas coupler l'assertion aux règles de rognage de blancs des `consume_*`).
fn tokens(out: &str) -> Vec<&str> {
    out.split_whitespace().collect()
}

// ── %sysmacdelete ────────────────────────────────────────────────────────────

#[test]
fn sysmacdelete_makes_invocation_left_verbatim() {
    // Oracle PROGRESS.md M41.3 : %sysmacdelete puis appel → non trouvée
    // (le texte `%m` reste verbatim, comme toute macro jamais définie).
    let out = run("%macro m; hi %mend; %sysmacdelete m; [%m]");
    assert_eq!(out.trim(), "[%m]");
}

#[test]
fn sysmacdelete_emits_nothing() {
    // Le statement lui-même n'émet aucune NOTE ni résidu.
    let out = run("a %sysmacdelete nosuchmacro; b");
    assert_eq!(tokens(&out), vec!["a", "b"]);
}

#[test]
fn sysmacdelete_case_insensitive_name() {
    let out = run("%macro Foo; x %mend; %sysmacdelete FOO; [%foo]");
    assert_eq!(out.trim(), "[%foo]");
}

#[test]
fn sysmacdelete_undefined_name_is_silent_noop() {
    // Convention indulgente du projet : pas d'ERROR si le nom n'existe pas.
    let out = run("%sysmacdelete never_defined; ok");
    assert_eq!(out.trim(), "ok");
}

// ── %syscall sortn ───────────────────────────────────────────────────────────

#[test]
fn syscall_sortn_ascending() {
    let out = run("%let a=3; %let b=1; %let c=2; %syscall sortn(a,b,c); &a &b &c");
    assert_eq!(tokens(&out), vec!["1", "2", "3"]);
}

#[test]
fn syscall_sortn_reverse_arg_order_writes_back_in_call_order() {
    // Les arguments sont passés dans l'ordre (c,b,a) : les valeurs triées
    // ASCENDANTES sont écrites dans CET ordre d'appel (plus petite -> c).
    let out = run("%let a=1; %let b=2; %let c=3; %syscall sortn(c,b,a); &a &b &c");
    assert_eq!(tokens(&out), vec!["3", "2", "1"]);
}

#[test]
fn syscall_sortn_ties() {
    let out = run("%let a=5; %let b=5; %let c=1; %syscall sortn(a,b,c); &a &b &c");
    assert_eq!(tokens(&out), vec!["1", "5", "5"]);
}

#[test]
fn syscall_sortn_undefined_and_nonnumeric_fallback_to_zero() {
    // `b` indéfinie, `c` non numérique : les deux valent 0.0 au tri.
    let out = run("%let a=2; %let c=abc; %syscall sortn(a,b,c); &a &b &c");
    assert_eq!(tokens(&out), vec!["0", "0", "2"]);
}

#[test]
fn syscall_sortn_writes_no_trailing_zeros() {
    let out = run("%let a=1.500; %let b=0.25; %syscall sortn(a,b); &a &b");
    assert_eq!(tokens(&out), vec!["0.25", "1.5"]);
}

#[test]
fn syscall_sortn_respects_local_scope() {
    // La variable LOCALE de la macro en cours doit être triée là où elle
    // est déjà définie (scope local), pas recréée en global : après le
    // retour de la macro, %symexist(a) doit rendre 0 (portée disparue).
    let out = run(
        "%macro m; %local a b; %let a=9; %let b=1; %syscall sortn(a,b); &a.-&b %mend; [%m][%symexist(a)]",
    );
    assert_eq!(out.trim(), "[1-9][0]");
}

// ── %syscall sortc ───────────────────────────────────────────────────────────

#[test]
fn syscall_sortc_ascending() {
    let out = run("%let a=banana; %let b=apple; %let c=cherry; %syscall sortc(a,b,c); &a &b &c");
    assert_eq!(tokens(&out), vec!["apple", "banana", "cherry"]);
}

#[test]
fn syscall_sortc_ties() {
    let out = run("%let a=x; %let b=x; %let c=a; %syscall sortc(a,b,c); &a &b &c");
    assert_eq!(tokens(&out), vec!["a", "x", "x"]);
}

#[test]
fn syscall_sortc_undefined_sorts_as_empty() {
    let out = run("%let a=b; %syscall sortc(a,z); [&a][&z]");
    assert_eq!(out.trim(), "[][b]");
}

// ── %syscall autre routine : NOTE inchangée ─────────────────────────────────

#[test]
fn syscall_other_routine_still_noted() {
    let out = run("p %syscall set(x); q");
    assert!(
        out.contains("%SYSCALL") && out.contains("not supported"),
        "got: {out}"
    );
    assert!(out.contains('p') && out.contains('q'), "got: {out}");
}
