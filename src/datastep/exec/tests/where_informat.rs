use super::*;

// ── M40.3 : WHERE statement standalone ───────────────────────────────
//
// Sémantique SAS : même effet qu'un `WHERE=()` posé sur CHAQUE dataset
// d'entrée — filtre PRÉ-CHARGEMENT (les obs rejetées n'entrent jamais au
// PDV et ne comptent pas dans les NOTEs « observations read »).

/// ORACLE de la case : `where x > 2;` ≡ `set a(where=(x > 2))` — mêmes
/// obs de sortie ET mêmes comptes de lecture dans les deux runs.
#[test]
fn where_statement_equals_where_option_oracle() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0, 3.0, 4.0]))]);
    let stats_stmt = run("data b1; set a; where x > 2; run;", &mut s).unwrap();
    let stats_opt = run("data b2; set a(where=(x > 2)); run;", &mut s).unwrap();
    assert_eq!(col(&s, "b1", "x"), some(&[3.0, 4.0]));
    assert_eq!(col(&s, "b1", "x"), col(&s, "b2", "x"));
    assert_eq!(stats_stmt.read, stats_opt.read);
    assert_eq!(stats_stmt.read, vec![("WORK.A".to_string(), 2)]);
    // Même NOTE de lecture (format inchangé, cohérent avec WHERE=).
    let log = s.log.into_string();
    assert_eq!(
        log.matches("There were 2 observations read from the data set WORK.A.")
            .count(),
        2,
        "log was: {log}"
    );
}

/// _N_ ne s'incrémente que pour les obs RETENUES (filtre pré-chargement,
/// ≠ subsetting IF).
#[test]
fn where_statement_n_counts_only_kept_rows() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0, 3.0]))]);
    run("data out; set a; where x ne 2; n = _n_; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "x"), some(&[1.0, 3.0]));
    assert_eq!(col(&s, "out", "n"), some(&[1.0, 2.0]));
}

/// WHERE + BY : le filtre est pré-appliqué à CHAQUE dataset avant
/// l'interclassement — FIRST./LAST. sont calculés sur le flux filtré
/// (miroir en statement de `where_option_is_prefiltered_before_interleave`).
#[test]
fn where_statement_with_by_first_last_on_filtered_stream() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0, 3.0]))]);
    write_num_ds(&s, "b", &[("x", some(&[2.0, 2.0]))]);
    let stats = run(
        "data out; set a b; by x; where x ne 2; l = last.x; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "x"), some(&[1.0, 3.0]));
    assert_eq!(col(&s, "out", "l"), some(&[1.0, 1.0]));
    // Les obs rejetées ne comptent pas comme lues — sur les DEUX datasets.
    assert_eq!(
        stats.read,
        vec![("WORK.A".to_string(), 2), ("WORK.B".to_string(), 0)]
    );
}

/// Plusieurs WHERE statements : le DERNIER gagne, avec la NOTE SAS
/// « WHERE clause has been replaced. ».
#[test]
fn last_where_statement_wins_with_note() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0, 3.0]))]);
    run("data out; set a; where x > 10; where x ne 2; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "x"), some(&[1.0, 3.0]));
    let log = s.log.into_string();
    assert!(
        log.contains("NOTE: WHERE clause has been replaced."),
        "log was: {log}"
    );
}

/// L'option WHERE= d'un dataset REMPLACE le statement pour CE dataset
/// (pas de cumul) ; les datasets sans option reçoivent le statement.
#[test]
fn where_option_overrides_statement_for_its_dataset() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0]))]);
    write_num_ds(&s, "b", &[("x", some(&[1.0, 2.0]))]);
    let stats = run(
        "data out; set a(where=(x = 1)) b; where x = 2; run;",
        &mut s,
    )
    .unwrap();
    // a : l'option gagne (x=1) — le statement ne s'y cumule pas ;
    // b : le statement s'applique (x=2).
    assert_eq!(col(&s, "out", "x"), some(&[1.0, 2.0]));
    assert_eq!(
        stats.read,
        vec![("WORK.A".to_string(), 1), ("WORK.B".to_string(), 1)]
    );
}

/// WHERE sans SET/MERGE (étape INPUT/DATALINES) : ERROR SAS.
#[test]
fn where_statement_without_input_datasets_errors() {
    let mut s = session();
    let e = run_err(
        "data out; input x; where x > 2; datalines;\n1\n2\n;\nrun;",
        &mut s,
    );
    assert_eq!(e, "No input data sets available for WHERE statement.");
}

/// WHERE + SET multiples (M40.2) : le statement s'applique à TOUS les
/// sites (« chaque dataset d'entrée »), chaque site gardant ses comptes.
#[test]
fn where_statement_applies_to_all_set_sites() {
    let mut s = session();
    write_num_ds(
        &s,
        "a",
        &[
            ("k", some(&[1.0, 2.0, 3.0])),
            ("x", some(&[10.0, 20.0, 30.0])),
        ],
    );
    write_num_ds(
        &s,
        "b",
        &[
            ("k", some(&[1.0, 2.0, 3.0])),
            ("y", some(&[100.0, 200.0, 300.0])),
        ],
    );
    let stats = run("data out; set a; set b; where k ne 2; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "x"), some(&[10.0, 30.0]));
    assert_eq!(col(&s, "out", "y"), some(&[100.0, 300.0]));
    // Filtre appliqué site par site : 2 obs lues de chaque dataset.
    assert_eq!(
        stats.read,
        vec![("WORK.A".to_string(), 2), ("WORK.B".to_string(), 2)]
    );
}

/// Règle SAS : chaque variable du WHERE statement doit être une variable
/// DU dataset (« Variable x is not on file WORK.A. ») — une variable
/// calculée ou inconnue est rejetée.
#[test]
fn where_statement_variable_not_on_file_errors() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0]))]);
    let e = run_err("data out; set a; y = 1; where y > 0; run;", &mut s);
    assert_eq!(e, "Variable y is not on file WORK.A.");
}

/// WHERE + POINT= : l'accès direct n'a pas de sémantique de filtre
/// pré-chargement — refus propre.
#[test]
fn where_statement_with_point_errors() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0]))]);
    let e = run_err(
        "data out; p = 1; set a point=p; where x > 0; output; stop; run;",
        &mut s,
    );
    assert!(e.contains("POINT="), "got: {e}");
}

/// WHERE + MERGE : le statement filtre CHAQUE dataset du match-merge
/// avant l'appariement par BY (la clé id=2 disparaît des deux côtés).
#[test]
fn where_statement_applies_to_merge_datasets() {
    let mut s = session();
    write_num_ds(
        &s,
        "a",
        &[
            ("id", some(&[1.0, 2.0, 3.0])),
            ("x", some(&[10.0, 20.0, 30.0])),
        ],
    );
    write_num_ds(
        &s,
        "b",
        &[
            ("id", some(&[1.0, 2.0, 3.0])),
            ("y", some(&[100.0, 200.0, 300.0])),
        ],
    );
    let stats = run("data out; merge a b; by id; where id ne 2; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "id"), some(&[1.0, 3.0]));
    assert_eq!(col(&s, "out", "x"), some(&[10.0, 30.0]));
    assert_eq!(col(&s, "out", "y"), some(&[100.0, 300.0]));
    assert_eq!(
        stats.read,
        vec![("WORK.A".to_string(), 2), ("WORK.B".to_string(), 2)]
    );
}

/// WHERE + UPDATE : refus honnête (seul le WHERE= du maître est supporté).
#[test]
fn where_statement_with_update_errors() {
    let mut s = session();
    write_num_ds(&s, "mas", &[("id", some(&[1.0])), ("x", some(&[10.0]))]);
    write_num_ds(&s, "tra", &[("id", some(&[1.0])), ("x", some(&[99.0]))]);
    let e = run_err(
        "data mas; update mas tra key=id; where id > 0; run;",
        &mut s,
    );
    assert!(
        e.contains("WHERE statement is not supported with UPDATE"),
        "got: {e}"
    );
}

// ── M40.3 : INFORMAT statement ───────────────────────────────────────
//
// Déclaratif : associe un informat PAR DÉFAUT aux variables, utilisé par
// l'INPUT en mode liste quand l'item n'a pas d'informat explicite
// (lecture « modified list input », comme `:informat.`).

/// `informat d date9.; input d;` — l'INPUT liste convertit via l'informat
/// déclaré (epoch SAS : 01JAN1960 = 0).
#[test]
fn informat_statement_applies_to_list_input() {
    let mut s = session();
    run(
        "data out; informat d date9.; input d; datalines;\n01JAN1960\n02JAN1960\n;\nrun;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "d"), some(&[0.0, 1.0]));
}

/// L'ordre statement/INPUT n'importe pas (pré-passe déclarative, comme
/// SAS) : l'INFORMAT peut suivre l'INPUT.
#[test]
fn informat_statement_after_input_still_applies() {
    let mut s = session();
    run(
        "data out; input d; informat d date9.; datalines;\n02JAN1960\n;\nrun;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "d"), some(&[1.0]));
}

/// Informat caractère déclaré : la variable est créée CHAR avec la largeur
/// de l'informat comme longueur (même règle que l'informat explicite d'un
/// INPUT — choix documenté), et la valeur est tronquée à cette longueur.
#[test]
fn informat_statement_char_sets_type_and_length() {
    let mut s = session();
    run(
        "data out; informat name $10.; input name; datalines;\nAlexanderTheGreat\n;\nrun;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.vars[0].ty, VarType::Char);
    assert_eq!(ds.vars[0].length, 10);
    assert_eq!(
        ds.df.column("name").unwrap().str().unwrap().get(0),
        Some("AlexanderT")
    );
}

/// L'informat EXPLICITE d'un item INPUT l'emporte sur le statement :
/// `yymmdd10.` lit « 1960/01/02 » (date9. échouerait).
#[test]
fn explicit_input_informat_wins_over_statement() {
    let mut s = session();
    run(
        "data out; informat d date9.; input d yymmdd10.; datalines;\n1960/01/02\n;\nrun;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "d"), some(&[1.0]));
}

/// Un INFORMAT sur une variable jamais lue par INPUT est légal (déclaratif,
/// sans effet sur une étape SET).
#[test]
fn informat_statement_without_input_is_legal_noop() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0]))]);
    run("data out; set a; informat x comma9.; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "x"), some(&[1.0, 2.0]));
}

/// `attrib d informat=date9.;` — même canal que le statement INFORMAT.
#[test]
fn attrib_informat_applies_to_list_input() {
    let mut s = session();
    run(
        "data out; attrib d informat=date9.; input d; datalines;\n02JAN1960\n;\nrun;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "d"), some(&[1.0]));
}
