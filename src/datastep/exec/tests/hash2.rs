use super::*;

/// duplicate par défaut : la 1re valeur est conservée, l'ajout est ignoré.
#[test]
fn hash_duplicate_default_keeps_first() {
    let mut s = session();
    run(
        "data out; \
         declare hash h(); h.defineKey('k'); h.defineData('v'); h.defineDone(); \
         k = 1; v = 1; h.add(); k = 1; v = 2; rc = h.add(); \
         k = 1; v = .; h.find(); output; stop; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("v").unwrap().f64().unwrap().get(0), Some(1.0));
    // add d'un doublon → rc ≠ 0.
    assert_ne!(ds.df.column("rc").unwrap().f64().unwrap().get(0), Some(0.0));
}

/// HITER first/next : parcours avant dans l'ordre d'insertion.
#[test]
fn hash_iter_first_next() {
    let mut s = session();
    run(
        "data out; \
         declare hash h(); h.defineKey('k'); h.defineData('v'); h.defineDone(); \
         declare hiter hi('h'); \
         k = 5; v = 50; h.add(); k = 6; v = 60; h.add(); \
         rc = hi.first(); k1 = k; v1 = v; \
         rc2 = hi.next(); k2 = k; v2 = v; \
         rc3 = hi.next(); \
         output; stop; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("k1").unwrap().f64().unwrap().get(0), Some(5.0));
    assert_eq!(ds.df.column("v1").unwrap().f64().unwrap().get(0), Some(50.0));
    assert_eq!(ds.df.column("k2").unwrap().f64().unwrap().get(0), Some(6.0));
    assert_eq!(ds.df.column("v2").unwrap().f64().unwrap().get(0), Some(60.0));
    assert_ne!(ds.df.column("rc3").unwrap().f64().unwrap().get(0), Some(0.0));
}

/// HITER last/prev : parcours arrière.
#[test]
fn hash_iter_last_prev() {
    let mut s = session();
    run(
        "data out; \
         declare hash h(); h.defineKey('k'); h.defineData('v'); h.defineDone(); \
         declare hiter hi('h'); \
         k = 1; v = 10; h.add(); k = 2; v = 20; h.add(); k = 3; v = 30; h.add(); \
         rc = hi.last(); k1 = k; \
         rc2 = hi.prev(); k2 = k; \
         rc3 = hi.prev(); k3 = k; \
         rc4 = hi.prev(); \
         output; stop; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("k1").unwrap().f64().unwrap().get(0), Some(3.0));
    assert_eq!(ds.df.column("k2").unwrap().f64().unwrap().get(0), Some(2.0));
    assert_eq!(ds.df.column("k3").unwrap().f64().unwrap().get(0), Some(1.0));
    assert_ne!(ds.df.column("rc4").unwrap().f64().unwrap().get(0), Some(0.0));
}

/// HITER respecte ordered:'descending' (first = clé max).
#[test]
fn hash_iter_ordered_descending() {
    let mut s = session();
    run(
        "data out; \
         declare hash h(ordered:'descending'); h.defineKey('k'); h.defineData('v'); h.defineDone(); \
         declare hiter hi('h'); \
         k = 1; v = 10; h.add(); k = 3; v = 30; h.add(); k = 2; v = 20; h.add(); \
         rc = hi.first(); k1 = k; \
         rc2 = hi.next(); k2 = k; \
         output; stop; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("k1").unwrap().f64().unwrap().get(0), Some(3.0));
    assert_eq!(ds.df.column("k2").unwrap().f64().unwrap().get(0), Some(2.0));
}

/// HITER lié à un hash non déclaré → erreur de compilation.
#[test]
fn hash_iter_unknown_hash_errors() {
    let mut s = session();
    let e = run_err("data _null_; declare hiter hi('ghost'); run;", &mut s);
    assert!(e.to_uppercase().contains("GHOST"), "got: {e}");
}

/// add avec arguments nommés key:/data:.
#[test]
fn hash_add_named_args() {
    let mut s = session();
    run(
        "data out; \
         declare hash h(); h.defineKey('k'); h.defineData('v'); h.defineDone(); \
         k = .; v = .; h.add(key: 7, data: 70); \
         k = 7; v = .; h.find(); output; stop; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("v").unwrap().f64().unwrap().get(0), Some(70.0));
}

/// Clés/données caractère : round-trip add/find avec trim SAS.
#[test]
fn hash_char_key_data() {
    let mut s = session();
    run(
        "data out; \
         length name $10 city $10; \
         declare hash h(); h.defineKey('name'); h.defineData('city'); h.defineDone(); \
         name = 'alice'; city = 'paris'; h.add(); \
         name = 'bob'; city = 'rome'; h.add(); \
         name = 'bob'; city = ''; rc = h.find(); output; stop; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("rc").unwrap().f64().unwrap().get(0), Some(0.0));
    assert_eq!(
        ds.df.column("city").unwrap().str().unwrap().get(0).map(str::trim),
        Some("rome")
    );
}
