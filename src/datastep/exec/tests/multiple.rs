use super::super::*;
use super::*;

#[test]
fn multiple_outputs_written() {
    let mut s = session();
    write_class(&s, "inp");
    let stats = run("data a b; set inp; run;", &mut s).unwrap();
    assert_eq!(stats.written.len(), 2);
    assert!(s.libs.get("WORK").unwrap().exists("a"));
    assert!(s.libs.get("WORK").unwrap().exists("b"));
    // _LAST_ = dernière sortie.
    assert_eq!(s.last_dataset.as_deref(), Some("WORK.B"));
}

// ── M40.2 : SET multiples (un SITE de lecture par statement) ─────────
//
// Oracles raisonnés en SAS : chaque statement SET a son propre curseur
// séquentiel ; quand un SET s'exécute sur un flux épuisé, l'étape
// s'arrête IMMÉDIATEMENT (au point du statement — « fin au 1ᵉʳ EOF »).

/// Oracle 2 SET : a=3 obs, b=2 obs ⇒ 2 obs, chacune combinant l'obs i de
/// a et l'obs i de b (lecture parallèle). La 3ᵉ obs de a est LUE (comptée
/// dans la NOTE) avant que le SET de b ne stoppe l'étape.
#[test]
fn two_set_statements_read_in_parallel() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0, 3.0]))]);
    write_num_ds(&s, "b", &[("y", some(&[10.0, 20.0]))]);
    let stats = run("data c; set a; set b; run;", &mut s).unwrap();
    assert_eq!(col(&s, "c", "x"), some(&[1.0, 2.0]));
    assert_eq!(col(&s, "c", "y"), some(&[10.0, 20.0]));
    // Comptes PAR SITE : a a servi 3 obs (la 3ᵉ chargée avant l'EOF de b).
    assert_eq!(
        stats.read,
        vec![("WORK.A".to_string(), 3), ("WORK.B".to_string(), 2)]
    );
    // Une NOTE par dataset lu, avec les comptes exacts par site.
    let log = s.log.into_string();
    assert!(
        log.contains("There were 3 observations read from the data set WORK.A."),
        "log was: {log}"
    );
    assert!(
        log.contains("There were 2 observations read from the data set WORK.B."),
        "log was: {log}"
    );
}

/// Fin au 1ᵉʳ EOF AU POINT du statement : a=2, b=1. Itération 1 : output
/// après `set a` (y encore missing) puis output après `set b`. Itération
/// 2 : output après `set a` (y RETENU de l'itération 1), puis le `set b`
/// stoppe — le 2ᵉ output ne s'exécute pas. ⇒ 3 obs.
#[test]
fn first_eof_stops_at_statement_point() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0]))]);
    write_num_ds(&s, "b", &[("y", some(&[10.0]))]);
    run("data x; set a; output; set b; output; run;", &mut s).unwrap();
    assert_eq!(col(&s, "x", "x"), some(&[1.0, 1.0, 2.0]));
    // Obs 1 : y pas encore lu (missing) ; obs 3 : y auto-retenu.
    assert_eq!(col(&s, "x", "y"), vec![None, Some(10.0), Some(10.0)]);
}

/// Variables communes : le DERNIER SET exécuté (ordre du programme) gagne.
#[test]
fn common_variables_last_executed_set_wins() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0]))]);
    write_num_ds(&s, "b", &[("x", some(&[10.0, 20.0]))]);
    run("data out; set a; set b; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "x"), some(&[10.0, 20.0]));
}

/// END= PAR SITE : le flag du flux court (b) s'allume sur SA dernière obs
/// (itération 2), pendant que celui du flux long (a) reste à 0.
#[test]
fn end_flag_is_per_site() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0, 3.0]))]);
    write_num_ds(&s, "b", &[("y", some(&[10.0, 20.0]))]);
    run(
        "data out; set a end=ea; set b end=eb; fa = ea; fb = eb; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "fa"), some(&[0.0, 0.0]));
    assert_eq!(col(&s, "out", "fb"), some(&[0.0, 1.0]));
    // ea ET eb sont des automatiques temporaires (site 0 comme site
    // extra) : jamais écrites en sortie, pas de NOTE "uninitialized".
    let out_ds = read_work(&s, "out");
    let cols: Vec<&str> = out_ds.df.get_column_names_str();
    assert!(!cols.contains(&"ea"), "cols: {cols:?}");
    assert!(!cols.contains(&"eb"), "cols: {cols:?}");
    let log = s.log.into_string();
    assert!(!log.contains("uninitialized"), "log was: {log}");
}

/// NOBS= PAR SITE : chaque variable reçoit le total de SON flux, AVANT la
/// boucle (disponible dès la 1re itération).
#[test]
fn nobs_is_per_site() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0, 3.0]))]);
    write_num_ds(&s, "b", &[("y", some(&[10.0, 20.0]))]);
    run("data out; set a nobs=na; set b nobs=nb; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "na"), some(&[3.0, 3.0]));
    assert_eq!(col(&s, "out", "nb"), some(&[2.0, 2.0]));
}

/// Auto-retain PAR SITE : un SET conditionnel (`if _n_ = 1 then set a;`,
/// idiome SAS des totaux) ne lit qu'une fois — sa variable GARDE sa valeur
/// pendant que l'autre site continue de lire. Le SET imbriqué est bien un
/// site propre (stamping dans les branches IF).
#[test]
fn conditional_set_site_auto_retains() {
    let mut s = session();
    write_num_ds(&s, "b", &[("y", some(&[10.0, 20.0, 30.0]))]);
    write_num_ds(&s, "a", &[("total", some(&[99.0]))]);
    let stats = run("data out; set b; if _n_ = 1 then set a; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "y"), some(&[10.0, 20.0, 30.0]));
    // `total` lu à la 1re itération, retenu ensuite (from_input).
    assert_eq!(col(&s, "out", "total"), some(&[99.0, 99.0, 99.0]));
    assert_eq!(
        stats.read,
        vec![("WORK.B".to_string(), 3), ("WORK.A".to_string(), 1)]
    );
}

/// Deux SET du MÊME dataset = deux curseurs INDÉPENDANTS : chaque site
/// relit le flux depuis le début (un curseur partagé donnerait x=1,y=2 puis
/// x=3,y=... — faux). RENAME= sur le 2ᵉ site sépare les variables.
#[test]
fn two_sets_of_same_dataset_have_independent_cursors() {
    let mut s = session();
    write_num_ds(&s, "h", &[("x", some(&[1.0, 2.0, 3.0]))]);
    let stats = run("data out; set h; set h(rename=(x=y)); run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "x"), some(&[1.0, 2.0, 3.0]));
    assert_eq!(col(&s, "out", "y"), some(&[1.0, 2.0, 3.0]));
    // Une NOTE de lecture PAR SITE, même dataset.
    assert_eq!(
        stats.read,
        vec![("WORK.H".to_string(), 3), ("WORK.H".to_string(), 3)]
    );
}

/// WHERE= par site : le filtre d'un site ne touche pas l'autre.
#[test]
fn where_option_is_per_site() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0, 3.0]))]);
    write_num_ds(&s, "b", &[("y", some(&[10.0, 20.0, 30.0]))]);
    let stats = run("data out; set a(where=(x ne 2)); set b; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "x"), some(&[1.0, 3.0]));
    assert_eq!(col(&s, "out", "y"), some(&[10.0, 20.0]));
    // Les lignes rejetées par WHERE= ne comptent pas comme lues.
    assert_eq!(
        stats.read,
        vec![("WORK.A".to_string(), 2), ("WORK.B".to_string(), 2)]
    );
}

/// BY avec plusieurs statements SET : refus honnête (le match par site
/// n'est pas implémenté — cf. README).
#[test]
fn by_with_multiple_set_statements_errors() {
    let mut s = session();
    write_num_ds(&s, "a", &[("id", some(&[1.0]))]);
    write_num_ds(&s, "b", &[("id", some(&[1.0]))]);
    let e = run_err("data out; set a; set b; by id; run;", &mut s);
    assert!(
        e.contains("BY statement is not supported with multiple SET statements"),
        "got: {e}"
    );
}

/// POINT= avec plusieurs statements SET : refus honnête (accès direct sur
/// un site + séquentiel sur l'autre non supporté — cf. README).
#[test]
fn point_with_multiple_set_statements_errors() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0]))]);
    write_num_ds(&s, "b", &[("y", some(&[1.0]))]);
    let e = run_err("data out; set a point=p; set b; stop; run;", &mut s);
    assert!(
        e.contains("POINT= is not supported with multiple SET statements"),
        "got: {e}"
    );
}

// ── M40.2 : `set;` / `merge;` nus (re-référence _LAST_) ──────────────

/// `set;` nu lit `_LAST_` (le dataset le plus récemment créé).
#[test]
fn bare_set_reads_last_dataset() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0]))]);
    s.last_dataset = Some("WORK.A".to_string());
    let stats = run("data out; set; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "x"), some(&[1.0, 2.0]));
    assert_eq!(stats.read, vec![("WORK.A".to_string(), 2)]);
}

/// `set end=eof;` nu : _LAST_ + option de niveau statement.
#[test]
fn bare_set_with_end_option() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0]))]);
    s.last_dataset = Some("WORK.A".to_string());
    run("data out; set end=eof; e = eof; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "e"), some(&[0.0, 1.0]));
}

/// `set;` nu sans _LAST_ : ERROR claire (comme les PROC sans DATA=).
#[test]
fn bare_set_without_last_errors() {
    let mut s = session();
    let e = run_err("data out; set; run;", &mut s);
    assert!(e.contains("_LAST_"), "got: {e}");
}

/// `merge;` nu re-référence `_LAST_` (un seul flux — cf. PROGRESS M40.2).
#[test]
fn bare_merge_reads_last_dataset() {
    let mut s = session();
    write_num_ds(
        &s,
        "a",
        &[("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))],
    );
    s.last_dataset = Some("WORK.A".to_string());
    run("data out; merge; by id; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "id"), some(&[1.0, 2.0]));
    assert_eq!(col(&s, "out", "x"), some(&[10.0, 20.0]));
}

/// Entrées multiples vers la même étiquette via LINK (réutilisation).
#[test]
fn multiple_link_entries_same_label() {
    let mut s = session();
    run(
        "data out; c = 0; link bump; link bump; link bump; return; \
         bump: c = c + 1; return; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "c"), vec![Some(3.0)]);
}

#[test]
fn output_drop_option_removes_work_variable() {
    let mut s = session();
    write_class_full(&s, "class");
    run(
        "data out(drop=tmp); set class; tmp = age * 2; final = tmp + 1; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    let cols: Vec<&str> = ds.df.get_column_names_str();
    assert!(!cols.contains(&"tmp"), "columns were: {cols:?}");
    let final_ = ds.df.column("final").unwrap().f64().unwrap();
    assert_eq!(final_.get(0), Some(29.0));
}

#[test]
fn output_rename_option_writes_renamed_column() {
    let mut s = session();
    write_class_full(&s, "class");
    run("data out(rename=(age=years)); set class; run;", &mut s).unwrap();
    let ds = read_work(&s, "out");
    // La colonne du parquet (et son VarMeta) porte le nouveau nom.
    let cols: Vec<&str> = ds.df.get_column_names_str();
    assert_eq!(cols, vec!["Name", "Sex", "years"]);
    assert_eq!(ds.vars[2].name, "years");
    let years = ds.df.column("years").unwrap().f64().unwrap();
    assert_eq!(years.get(3), Some(15.0));
}

#[test]
fn output_in_option_is_error() {
    let mut s = session();
    write_num_ds(&s, "a", &[("id", some(&[1.0]))]);
    let err = run("data out(in=foo); set a; run;", &mut s).err().unwrap();
    assert!(err.to_string().to_uppercase().contains("IN"), "got: {err}");
}

#[test]
fn targeted_outputs_split_disjoint_datasets() {
    let mut s = session();
    write_class_full(&s, "class");
    let stats = run(
        "data m f; set class; if sex = 'M' then output m; else output f; run;",
        &mut s,
    )
    .unwrap();
    // Deux datasets disjoints, total = obs d'origine, comptes PAR
    // sortie indépendants.
    assert_eq!(
        stats.written,
        vec![("WORK.M".to_string(), 2, 3), ("WORK.F".to_string(), 2, 3),]
    );
    let m = read_work(&s, "m");
    let f = read_work(&s, "f");
    assert_eq!(m.n_obs() + f.n_obs(), 4);
    let m_names = m.df.column("Name").unwrap().str().unwrap();
    assert_eq!(m_names.get(0), Some("Alfred"));
    assert_eq!(m_names.get(1), Some("Henry"));
    let f_names = f.df.column("Name").unwrap().str().unwrap();
    assert_eq!(f_names.get(0), Some("Alice"));
    assert_eq!(f_names.get(1), Some("Barbara"));
    let log = s.log.into_string();
    assert!(log.contains("The data set WORK.M has 2 observations and 3 variables."));
    assert!(log.contains("The data set WORK.F has 2 observations and 3 variables."));
}

#[test]
fn targeted_output_two_names_writes_both() {
    let mut s = session();
    write_class(&s, "inp");
    let stats = run("data a b; set inp; output a b; run;", &mut s).unwrap();
    assert_eq!(
        stats.written,
        vec![("WORK.A".to_string(), 3, 2), ("WORK.B".to_string(), 3, 2)]
    );
}

#[test]
fn where_skip_does_not_run_rest_of_iteration() {
    let mut s = session();
    write_class_full(&s, "class");
    // n compte les itérations qui exécutent le corps : avec WHERE=,
    // seules les lignes qui passent y arrivent.
    run(
        "data out; set class(where=(sex = 'F')); n + 1; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    let n = ds.df.column("n").unwrap().f64().unwrap();
    assert_eq!(ds.n_obs(), 2);
    assert_eq!(n.get(0), Some(1.0));
    assert_eq!(n.get(1), Some(2.0));
}

#[test]
fn first_last_group_count_per_group() {
    let mut s = session();
    write_num_ds(
        &s,
        "g",
        &[
            ("grp", some(&[1.0, 1.0, 1.0, 2.0, 2.0])),
            ("val", some(&[5.0, 6.0, 7.0, 8.0, 9.0])),
        ],
    );
    // Idiome SAS canonique : compteur remis à zéro en tête de groupe,
    // une obs émise par groupe (subsetting IF sur last.grp).
    run(
        "data out; set g; by grp; if first.grp then n = 0; n + 1; if last.grp; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "grp"), some(&[1.0, 2.0]));
    assert_eq!(col(&s, "out", "n"), some(&[3.0, 2.0]));
}

#[test]
fn two_by_keys_first_last_prefix_rule() {
    let mut s = session();
    write_num_ds(
        &s,
        "g",
        &[("a", some(&[1.0, 1.0, 2.0])), ("b", some(&[7.0, 8.0, 8.0]))],
    );
    run(
        "data out; set g; by a b; fa = first.a; fb = first.b; la = last.a; lb = last.b; run;",
        &mut s,
    )
    .unwrap();
    // first.b = 1 dès que a OU b change (préfixe de clés).
    assert_eq!(col(&s, "out", "fa"), some(&[1.0, 0.0, 1.0]));
    assert_eq!(col(&s, "out", "fb"), some(&[1.0, 1.0, 1.0]));
    // last.b suit le même préfixe vers l'obs suivante ; b=8 ne forme
    // PAS un groupe à cheval sur a=1/a=2.
    assert_eq!(col(&s, "out", "la"), some(&[0.0, 1.0, 1.0]));
    assert_eq!(col(&s, "out", "lb"), some(&[1.0, 1.0, 1.0]));
}

#[test]
fn descending_by_interleaves_in_decreasing_order() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[3.0, 1.0]))]);
    write_num_ds(&s, "b", &[("x", some(&[2.0]))]);
    run("data out; set a b; by descending x; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "x"), some(&[3.0, 2.0, 1.0]));
}

#[test]
fn unsorted_by_data_stops_with_error() {
    let mut s = session();
    write_num_ds(&s, "d", &[("x", some(&[2.0, 1.0]))]);
    let err = run("data out; set d; by x; run;", &mut s).err().unwrap();
    assert_eq!(
        err.to_string(),
        "BY variables are not properly sorted on data set WORK.D."
    );
}

#[test]
fn where_option_is_prefiltered_before_interleave() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0, 3.0]))]);
    write_num_ds(&s, "b", &[("x", some(&[2.0]))]);
    // a filtré sur x ne 2 : l'interclassement voit 1,3 côté a.
    let stats = run(
        "data out; set a(where=(x ne 2)) b; by x; l = last.x; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "x"), some(&[1.0, 2.0, 3.0]));
    assert_eq!(col(&s, "out", "l"), some(&[1.0, 1.0, 1.0]));
    // Les lignes rejetées par WHERE= ne comptent pas comme lues.
    assert_eq!(
        stats.read,
        vec![("WORK.A".to_string(), 2), ("WORK.B".to_string(), 1)]
    );
}

#[test]
fn single_dataset_by_groups_match_simple_set() {
    let mut s = session();
    write_class(&s, "inp"); // Age = 14, ., 13 — PAS trié.
    // Un SET simple sans BY reste inchangé (chemin M1/M2 intact).
    let stats = run("data out; set inp; run;", &mut s).unwrap();
    assert_eq!(stats.read, vec![("WORK.INP".to_string(), 3)]);
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 3, 2)]);
}

#[test]
fn merge_one_to_one() {
    // Sortie SAS calculée à la main : a={(1,x=10),(2,x=20)},
    // b={(1,y=100),(2,y=200)} ; merge a b; by id; → (1,10,100),(2,20,200).
    let mut s = session();
    write_num_ds(
        &s,
        "a",
        &[("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))],
    );
    write_num_ds(
        &s,
        "b",
        &[("id", some(&[1.0, 2.0])), ("y", some(&[100.0, 200.0]))],
    );
    let stats = run("data out; merge a b; by id; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "id"), some(&[1.0, 2.0]));
    assert_eq!(col(&s, "out", "x"), some(&[10.0, 20.0]));
    assert_eq!(col(&s, "out", "y"), some(&[100.0, 200.0]));
    assert_eq!(
        stats.read,
        vec![("WORK.A".to_string(), 2), ("WORK.B".to_string(), 2)]
    );
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 2, 3)]);
}

#[test]
fn merge_one_to_many_short_side_persists() {
    // Sortie SAS calculée à la main : a={(1,x=10),(1,x=20)}, b={(1,y=100)}
    // ; merge a b; by id; → (1,10,100),(1,20,100). y PERSISTE à 100 sur
    // la 2e obs (persistance du côté court).
    let mut s = session();
    write_num_ds(
        &s,
        "a",
        &[("id", some(&[1.0, 1.0])), ("x", some(&[10.0, 20.0]))],
    );
    write_num_ds(&s, "b", &[("id", some(&[1.0])), ("y", some(&[100.0]))]);
    run("data out; merge a b; by id; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "id"), some(&[1.0, 1.0]));
    assert_eq!(col(&s, "out", "x"), some(&[10.0, 20.0]));
    // VÉRIFICATION EXPLICITE : y == 100 sur la 2e obs (persistance).
    assert_eq!(col(&s, "out", "y"), some(&[100.0, 100.0]));
}

#[test]
fn merge_unmatched_keys_with_in_and_missing() {
    // Sortie SAS calculée à la main : a={(1,x=10),(3,x=30)},
    // b={(2,y=20),(3,y=33)} ; merge a(in=ina) b(in=inb); by id; →
    //   id=1 : x=10, y=. , ina=1, inb=0
    //   id=2 : x=. , y=20, ina=0, inb=1
    //   id=3 : x=30, y=33, ina=1, inb=1
    let mut s = session();
    write_num_ds(
        &s,
        "a",
        &[("id", some(&[1.0, 3.0])), ("x", some(&[10.0, 30.0]))],
    );
    write_num_ds(
        &s,
        "b",
        &[("id", some(&[2.0, 3.0])), ("y", some(&[20.0, 33.0]))],
    );
    run(
        "data out; merge a(in=ina) b(in=inb); by id; a_in = ina; b_in = inb; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "id"), some(&[1.0, 2.0, 3.0]));
    assert_eq!(col(&s, "out", "x"), vec![Some(10.0), None, Some(30.0)]);
    assert_eq!(col(&s, "out", "y"), vec![None, Some(20.0), Some(33.0)]);
    assert_eq!(col(&s, "out", "a_in"), some(&[1.0, 0.0, 1.0]));
    assert_eq!(col(&s, "out", "b_in"), some(&[0.0, 1.0, 1.0]));
    // ina/inb sont des automatiques : jamais écrites en sortie.
    let out_ds = read_work(&s, "out");
    let cols: Vec<&str> = out_ds.df.get_column_names_str();
    assert!(!cols.contains(&"ina"), "cols: {cols:?}");
    assert!(!cols.contains(&"inb"), "cols: {cols:?}");
}

#[test]
fn merge_inner_join_via_in() {
    // Idiome SAS : `if ina and inb;` = inner join. Mêmes données que le
    // test précédent → 1 obs (id=3). Sortie calculée à la main.
    let mut s = session();
    write_num_ds(
        &s,
        "a",
        &[("id", some(&[1.0, 3.0])), ("x", some(&[10.0, 30.0]))],
    );
    write_num_ds(
        &s,
        "b",
        &[("id", some(&[2.0, 3.0])), ("y", some(&[20.0, 33.0]))],
    );
    run(
        "data out; merge a(in=ina) b(in=inb); by id; if ina and inb; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "id"), some(&[3.0]));
    assert_eq!(col(&s, "out", "x"), some(&[30.0]));
    assert_eq!(col(&s, "out", "y"), some(&[33.0]));
}

#[test]
fn merge_variable_overlap_rightmost_wins() {
    // a={(1,v='A')}, b={(1,v='B')} ; merge a b; by id; → v='B' (le dernier
    // dataset du MERGE écrase). Sortie calculée à la main.
    let mut s = session();
    let id_a = Series::new("id".into(), &[Some(1.0)]);
    let v_a = Series::new("v".into(), &["A"]);
    let df_a = DataFrame::new(vec![id_a.into(), v_a.into()]).unwrap();
    let vars = vec![
        VarMeta {
            name: "id".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
        VarMeta {
            name: "v".into(),
            ty: VarType::Char,
            length: 8,
            format: None,
            label: None,
        },
    ];
    s.libs
        .get("WORK")
        .unwrap()
        .write(
            "a",
            &SasDataset {
                df: df_a,
                vars: vars.clone(),
            },
        )
        .unwrap();
    let id_b = Series::new("id".into(), &[Some(1.0)]);
    let v_b = Series::new("v".into(), &["B"]);
    let df_b = DataFrame::new(vec![id_b.into(), v_b.into()]).unwrap();
    s.libs
        .get("WORK")
        .unwrap()
        .write("b", &SasDataset { df: df_b, vars })
        .unwrap();
    run("data out; merge a b; by id; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "id"), some(&[1.0]));
    assert_eq!(col_str(&s, "out", "v"), vec![Some("B".to_string())]);
}

#[test]
fn merge_first_last_on_one_to_many_group() {
    // FIRST./LAST. avec MERGE sur un groupe one-to-many. a a deux obs
    // id=1 et une id=2 ; b une obs id=1 et une id=2. Groupe id=1 → 2
    // obs : first=1/0, last=0/1 ; groupe id=2 → 1 obs : first=1, last=1.
    // Sortie calculée à la main.
    let mut s = session();
    write_num_ds(
        &s,
        "a",
        &[
            ("id", some(&[1.0, 1.0, 2.0])),
            ("x", some(&[10.0, 11.0, 20.0])),
        ],
    );
    write_num_ds(
        &s,
        "b",
        &[("id", some(&[1.0, 2.0])), ("y", some(&[100.0, 200.0]))],
    );
    run(
        "data out; merge a b; by id; f = first.id; l = last.id; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "id"), some(&[1.0, 1.0, 2.0]));
    assert_eq!(col(&s, "out", "f"), some(&[1.0, 0.0, 1.0]));
    assert_eq!(col(&s, "out", "l"), some(&[0.0, 1.0, 1.0]));
    // y persiste sur la 2e obs du groupe id=1.
    assert_eq!(col(&s, "out", "y"), some(&[100.0, 100.0, 200.0]));
}

#[test]
fn merge_unsorted_data_stops_with_error() {
    // Un dataset non trié selon le BY → ERROR de désordre.
    let mut s = session();
    write_num_ds(
        &s,
        "a",
        &[("id", some(&[2.0, 1.0])), ("x", some(&[1.0, 2.0]))],
    );
    write_num_ds(
        &s,
        "b",
        &[("id", some(&[1.0, 2.0])), ("y", some(&[1.0, 2.0]))],
    );
    let err = run("data out; merge a b; by id; run;", &mut s)
        .err()
        .unwrap();
    assert_eq!(
        err.to_string(),
        "BY variables are not properly sorted on data set WORK.A."
    );
}

#[test]
fn merge_without_by_is_compile_error() {
    let mut s = session();
    write_num_ds(&s, "a", &[("id", some(&[1.0]))]);
    write_num_ds(&s, "b", &[("id", some(&[1.0]))]);
    let err = run("data out; merge a b; run;", &mut s).err().unwrap();
    assert!(err.to_string().contains("BY"), "got: {err}");
}

#[test]
fn merge_after_set_is_error() {
    let mut s = session();
    write_num_ds(&s, "a", &[("id", some(&[1.0]))]);
    write_num_ds(&s, "b", &[("id", some(&[1.0]))]);
    let err = run("data out; set a; merge a b; by id; run;", &mut s)
        .err()
        .unwrap();
    assert!(err.to_string().contains("not allowed"), "got: {err}");
}

#[test]
fn division_by_zero_sets_error_only_for_nonmissing_numerator() {
    let mut s = session();
    write_class(&s, "inp"); // age = 14, ., 13
    run("data out; set inp; r = age / 0; e = _error_; run;", &mut s).unwrap();
    let ds = read_work(&s, "out");
    let e = ds.df.column("e").unwrap().f64().unwrap();
    // 14/0 → division par zéro → _ERROR_=1 ; ./0 → missing propagé
    // (opérande missing AVANT le test du diviseur) SANS _ERROR_ — ce
    // 0 prouve aussi le RESET de _ERROR_ entre itérations ; 13/0 → 1.
    assert_eq!(e.get(0), Some(1.0));
    assert_eq!(e.get(1), Some(0.0));
    assert_eq!(e.get(2), Some(1.0));
    // Le résultat est `.` ordinaire dans tous les cas → nulls.
    let r = ds.df.column("r").unwrap().f64().unwrap();
    assert_eq!(r.null_count(), 3);
    // NOTE émise UNE fois malgré deux divisions par zéro, plus la
    // NOTE de missing généré (./0).
    let log = s.log.into_string();
    assert_eq!(
        log.matches("NOTE: Division by zero detected.").count(),
        1,
        "log was: {log}"
    );
    assert_eq!(
        log.matches(
            "NOTE: Missing values were generated as a result of \
             performing an operation on missing values."
        )
        .count(),
        1,
        "log was: {log}"
    );
}

#[test]
fn automatic_variables_readable_but_never_output_columns() {
    let mut s = session();
    write_class(&s, "inp");
    let stats = run("data out; set inp; n = _n_; e = _error_; run;", &mut s).unwrap();
    // 4 colonnes seulement : ni _N_ ni _ERROR_ ne sont écrites.
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 3, 4)]);
    let ds = read_work(&s, "out");
    let cols: Vec<&str> = ds.df.get_column_names_str();
    assert_eq!(cols, vec!["Age", "Name", "n", "e"]);
    // _N_ == numéro d'observation avec un simple SET.
    let n = ds.df.column("n").unwrap().f64().unwrap();
    assert_eq!(n.get(0), Some(1.0));
    assert_eq!(n.get(1), Some(2.0));
    assert_eq!(n.get(2), Some(3.0));
    // Pas d'erreur : _ERROR_ = 0 partout.
    let e = ds.df.column("e").unwrap().f64().unwrap();
    assert!(e.iter().all(|v| v == Some(0.0)));
    // Et surtout pas de NOTE "uninitialized" parasite pour les
    // variables automatiques.
    let log = s.log.into_string();
    assert!(!log.contains("uninitialized"), "log was: {log}");
}
