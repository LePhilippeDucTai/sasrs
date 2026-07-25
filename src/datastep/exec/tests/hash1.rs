use super::*;

// ── M17.1 : DECLARE HASH + defineKey/defineData/defineDone ───────────

/// DECLARE HASH sans option crée l'objet (options par défaut).
#[test]
fn hash_declare_no_options() {
    let mut s = session();
    run("data _null_; declare hash h(); run;", &mut s).unwrap();
    let h = s.debug_hashes.get("H").expect("hash H exists");
    assert!(h.ordered.is_none());
    assert!(h.duplicate.is_none());
    assert!(!h.multidata);
    assert!(h.dataset.is_none());
    assert!(h.keys.is_empty());
    assert!(h.data_vars.is_empty());
    assert!(!h.defined);
}

/// Options ordered/duplicate/multidata parsées et stockées (minuscules).
#[test]
fn hash_declare_options_parsed() {
    let mut s = session();
    run(
        "data _null_; declare hash h(ordered:'YES', duplicate:'replace', multidata:'yes'); run;",
        &mut s,
    )
    .unwrap();
    let h = s.debug_hashes.get("H").unwrap();
    assert_eq!(h.ordered.as_deref(), Some("yes"));
    assert_eq!(h.duplicate.as_deref(), Some("replace"));
    assert!(h.multidata);
}

/// multidata:'no' (ou absent) → false.
#[test]
fn hash_declare_multidata_no() {
    let mut s = session();
    run(
        "data _null_; declare hash h(multidata:'no'); run;",
        &mut s,
    )
    .unwrap();
    assert!(!s.debug_hashes.get("H").unwrap().multidata);
}

/// Option dataset: conservée et pré-lue à la compilation (M17.2).
#[test]
fn hash_declare_dataset_option() {
    let mut s = session();
    write_class(&s, "lookup");
    run(
        "data _null_; declare hash h(dataset:'work.lookup'); run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(
        s.debug_hashes.get("H").unwrap().dataset.as_deref(),
        Some("work.lookup")
    );
}

/// Option inconnue → erreur de compilation.
#[test]
fn hash_declare_unknown_option_errors() {
    let mut s = session();
    let e = run_err("data _null_; declare hash h(bogus:'x'); run;", &mut s);
    assert!(e.to_uppercase().contains("BOGUS"), "got: {e}");
}

/// defineKey avec une seule variable.
#[test]
fn hash_define_key_single() {
    let mut s = session();
    run(
        "data _null_; k = 1; declare hash h(); h.defineKey('k'); h.defineDone(); run;",
        &mut s,
    )
    .unwrap();
    let h = s.debug_hashes.get("H").unwrap();
    assert_eq!(h.keys, vec!["K".to_string()]);
    assert!(h.defined);
}

/// defineKey avec plusieurs variables (ordre préservé, UPPERCASE).
#[test]
fn hash_define_key_multiple() {
    let mut s = session();
    run(
        "data _null_; k1 = 1; k2 = 'a'; declare hash h(); h.defineKey('k1', 'k2'); run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(
        s.debug_hashes.get("H").unwrap().keys,
        vec!["K1".to_string(), "K2".to_string()]
    );
}

/// defineData simple et multiple.
#[test]
fn hash_define_data_single_and_multiple() {
    let mut s = session();
    run(
        "data _null_; k = 1; v1 = 2; v2 = 3; declare hash h(); h.defineKey('k'); h.defineData('v1', 'v2'); run;",
        &mut s,
    )
    .unwrap();
    let h = s.debug_hashes.get("H").unwrap();
    assert_eq!(h.keys, vec!["K".to_string()]);
    assert_eq!(h.data_vars, vec!["V1".to_string(), "V2".to_string()]);
}

/// defineDone est idempotent (deux appels → toujours defined, pas d'erreur).
#[test]
fn hash_define_done_idempotent() {
    let mut s = session();
    run(
        "data _null_; k = 1; declare hash h(); h.defineKey('k'); h.defineDone(); h.defineDone(); run;",
        &mut s,
    )
    .unwrap();
    assert!(s.debug_hashes.get("H").unwrap().defined);
}

/// Plusieurs objets hash dans la même étape, indépendants.
#[test]
fn hash_multiple_objects() {
    let mut s = session();
    run(
        "data _null_; a = 1; b = 2; \
         declare hash h1(); h1.defineKey('a'); h1.defineDone(); \
         declare hash h2(multidata:'yes'); h2.defineKey('b'); h2.defineData('a'); h2.defineDone(); \
         run;",
        &mut s,
    )
    .unwrap();
    let h1 = s.debug_hashes.get("H1").unwrap();
    let h2 = s.debug_hashes.get("H2").unwrap();
    assert_eq!(h1.keys, vec!["A".to_string()]);
    assert!(h1.data_vars.is_empty());
    assert!(!h1.multidata);
    assert_eq!(h2.keys, vec!["B".to_string()]);
    assert_eq!(h2.data_vars, vec!["A".to_string()]);
    assert!(h2.multidata);
}

/// defineKey sur une variable encore inconnue : SAS la crée au PDV
/// (numérique par défaut). L'étape s'exécute sans erreur.
#[test]
fn hash_define_key_creates_variable() {
    let mut s = session();
    run(
        "data _null_; declare hash h(); h.defineKey('nosuchvar'); h.defineDone(); run;",
        &mut s,
    )
    .unwrap();
    let h = s.debug_hashes.get("H").unwrap();
    assert_eq!(h.keys, vec!["NOSUCHVAR".to_string()]);
}

/// Méthode sur un objet non déclaré → erreur de compilation.
#[test]
fn hash_method_on_undeclared_object_errors() {
    let mut s = session();
    let e = run_err(
        "data _null_; k = 1; ghost.defineKey('k'); run;",
        &mut s,
    );
    assert!(e.to_uppercase().contains("GHOST"), "got: {e}");
}

/// Méthode réellement inconnue → erreur runtime « not yet implemented ».
#[test]
fn hash_unimplemented_method_errors() {
    let mut s = session();
    let e = run_err(
        "data _null_; k = 1; declare hash h(); h.defineKey('k'); h.defineDone(); h.bogusmethod(); run;",
        &mut s,
    );
    assert!(
        e.to_uppercase().contains("NOT YET IMPLEMENTED")
            && e.to_uppercase().contains("BOGUSMETHOD"),
        "got: {e}"
    );
}

// ── M17.2 : méthodes de données + HITER + dataset/ordered/multidata ──

/// add puis find : round-trip clé→données via le PDV.
#[test]
fn hash_add_find_roundtrip() {
    let mut s = session();
    let stats = run(
        "data out; \
         if _n_ = 1 then do; \
           declare hash h(); h.defineKey('k'); h.defineData('v'); h.defineDone(); \
           k = 10; v = 100; h.add(); \
           k = 20; v = 200; h.add(); \
         end; \
         k = 20; v = .; rc = h.find(); output; \
         stop; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(stats.written[0].1, 1);
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("v").unwrap().f64().unwrap().get(0), Some(200.0));
    assert_eq!(ds.df.column("rc").unwrap().f64().unwrap().get(0), Some(0.0));
}

/// find copie les données dans le PDV ; check ne les copie pas.
#[test]
fn hash_check_does_not_copy() {
    let mut s = session();
    run(
        "data out; \
         declare hash h(); h.defineKey('k'); h.defineData('v'); h.defineDone(); \
         k = 1; v = 99; h.add(); \
         k = 1; v = -1; rc = h.check(); output; \
         stop; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    // check trouve (rc=0) mais NE copie PAS v (reste -1).
    assert_eq!(ds.df.column("rc").unwrap().f64().unwrap().get(0), Some(0.0));
    assert_eq!(ds.df.column("v").unwrap().f64().unwrap().get(0), Some(-1.0));
}

/// find sur clé absente → rc ≠ 0, données du PDV inchangées.
#[test]
fn hash_find_miss_nonzero() {
    let mut s = session();
    run(
        "data out; \
         declare hash h(); h.defineKey('k'); h.defineData('v'); h.defineDone(); \
         k = 1; v = 5; h.add(); \
         k = 999; v = 5; rc = h.find(); output; stop; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_ne!(ds.df.column("rc").unwrap().f64().unwrap().get(0), Some(0.0));
    assert_eq!(ds.df.column("v").unwrap().f64().unwrap().get(0), Some(5.0));
}

/// replace remplace les données d'une clé existante.
#[test]
fn hash_replace_updates() {
    let mut s = session();
    run(
        "data out; \
         declare hash h(); h.defineKey('k'); h.defineData('v'); h.defineDone(); \
         k = 1; v = 10; h.add(); \
         k = 1; v = 77; h.replace(); \
         k = 1; v = .; rc = h.find(); output; stop; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("v").unwrap().f64().unwrap().get(0), Some(77.0));
}

/// remove supprime l'entrée (find ensuite échoue) ; num_items diminue.
#[test]
fn hash_remove_and_num_items() {
    let mut s = session();
    run(
        "data out; \
         declare hash h(); h.defineKey('k'); h.defineData('v'); h.defineDone(); \
         k = 1; v = 1; h.add(); k = 2; v = 2; h.add(); \
         n1 = h.num_items; \
         k = 1; rc1 = h.remove(); \
         n2 = h.num_items; \
         k = 1; rc2 = h.find(); \
         output; stop; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("n1").unwrap().f64().unwrap().get(0), Some(2.0));
    assert_eq!(ds.df.column("n2").unwrap().f64().unwrap().get(0), Some(1.0));
    assert_eq!(ds.df.column("rc1").unwrap().f64().unwrap().get(0), Some(0.0));
    assert_ne!(ds.df.column("rc2").unwrap().f64().unwrap().get(0), Some(0.0));
}

/// clear vide le hash (num_items → 0).
#[test]
fn hash_clear_empties() {
    let mut s = session();
    run(
        "data out; \
         declare hash h(); h.defineKey('k'); h.defineData('v'); h.defineDone(); \
         k = 1; v = 1; h.add(); k = 2; v = 2; h.add(); \
         h.clear(); n = h.num_items; output; stop; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("n").unwrap().f64().unwrap().get(0), Some(0.0));
}

/// num_items en forme expression vs statement (rc ignoré en statement).
#[test]
fn hash_method_statement_and_expression() {
    let mut s = session();
    run(
        "data out; \
         declare hash h(); h.defineKey('k'); h.defineData('v'); h.defineDone(); \
         k = 1; v = 1; h.add(); \
         k = 1; v = .; h.find(); output; stop; run;",
        &mut s,
    )
    .unwrap();
    // h.find() en statement copie quand même les données.
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("v").unwrap().f64().unwrap().get(0), Some(1.0));
}

/// output écrit le contenu du hash dans un dataset.
#[test]
fn hash_output_to_dataset() {
    let mut s = session();
    run(
        "data _null_; \
         declare hash h(); h.defineKey('k'); h.defineData('v'); h.defineDone(); \
         k = 3; v = 30; h.add(); k = 1; v = 10; h.add(); k = 2; v = 20; h.add(); \
         h.output(dataset:'work.hout'); stop; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "hout");
    assert_eq!(ds.n_obs(), 3);
    // Pas d'option ordered: → ordre d'insertion 3,1,2.
    let k = ds.df.column("k").unwrap().f64().unwrap();
    assert_eq!(k.get(0), Some(3.0));
    assert_eq!(k.get(1), Some(1.0));
    assert_eq!(k.get(2), Some(2.0));
}

/// output respecte ordered:'ascending' (tri croissant par clé).
#[test]
fn hash_output_ordered_ascending() {
    let mut s = session();
    run(
        "data _null_; \
         declare hash h(ordered:'ascending'); h.defineKey('k'); h.defineData('v'); h.defineDone(); \
         k = 3; v = 30; h.add(); k = 1; v = 10; h.add(); k = 2; v = 20; h.add(); \
         h.output(dataset:'work.hout'); stop; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "hout");
    let k = ds.df.column("k").unwrap().f64().unwrap();
    assert_eq!(k.get(0), Some(1.0));
    assert_eq!(k.get(1), Some(2.0));
    assert_eq!(k.get(2), Some(3.0));
}

/// output respecte ordered:'descending'.
#[test]
fn hash_output_ordered_descending() {
    let mut s = session();
    run(
        "data _null_; \
         declare hash h(ordered:'descending'); h.defineKey('k'); h.defineData('v'); h.defineDone(); \
         k = 1; v = 10; h.add(); k = 3; v = 30; h.add(); k = 2; v = 20; h.add(); \
         h.output(dataset:'work.hout'); stop; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "hout");
    let k = ds.df.column("k").unwrap().f64().unwrap();
    assert_eq!(k.get(0), Some(3.0));
    assert_eq!(k.get(1), Some(2.0));
    assert_eq!(k.get(2), Some(1.0));
}

/// dataset: charge la table dans le hash au defineDone.
#[test]
fn hash_dataset_load() {
    let mut s = session();
    write_class(&s, "lk");
    // Age est la clé, Name la donnée. find(Age=13) → Name="Barbara".
    run(
        "data out; \
         declare hash h(dataset:'work.lk'); h.defineKey('Age'); h.defineData('Name'); h.defineDone(); \
         Age = 13; length Name $7; Name = ''; rc = h.find(); output; stop; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("rc").unwrap().f64().unwrap().get(0), Some(0.0));
    assert_eq!(
        ds.df.column("Name").unwrap().str().unwrap().get(0),
        Some("Barbara")
    );
}

/// multidata: plusieurs données par clé, parcourues par find + find_next.
#[test]
fn hash_multidata_find_next() {
    let mut s = session();
    run(
        "data out; \
         declare hash h(multidata:'yes'); h.defineKey('k'); h.defineData('v'); h.defineDone(); \
         k = 1; v = 100; h.add(); k = 1; v = 200; h.add(); k = 1; v = 300; h.add(); \
         k = 1; v = .; rc = h.find(); v1 = v; \
         rc2 = h.find_next(); v2 = v; \
         rc3 = h.find_next(); v3 = v; \
         rc4 = h.find_next(); \
         output; stop; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("v1").unwrap().f64().unwrap().get(0), Some(100.0));
    assert_eq!(ds.df.column("v2").unwrap().f64().unwrap().get(0), Some(200.0));
    assert_eq!(ds.df.column("v3").unwrap().f64().unwrap().get(0), Some(300.0));
    // find_next au-delà → rc ≠ 0.
    assert_ne!(ds.df.column("rc4").unwrap().f64().unwrap().get(0), Some(0.0));
}

/// duplicate:'replace' écrase la donnée d'une clé existante (sans multidata).
#[test]
fn hash_duplicate_replace() {
    let mut s = session();
    run(
        "data out; \
         declare hash h(duplicate:'replace'); h.defineKey('k'); h.defineData('v'); h.defineDone(); \
         k = 1; v = 1; h.add(); k = 1; v = 2; h.add(); \
         k = 1; v = .; h.find(); n = h.num_items; output; stop; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("v").unwrap().f64().unwrap().get(0), Some(2.0));
    assert_eq!(ds.df.column("n").unwrap().f64().unwrap().get(0), Some(1.0));
}
