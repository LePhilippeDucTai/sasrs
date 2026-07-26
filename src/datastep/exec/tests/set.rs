use super::*;
use crate::value::MissingKind;

#[test]
fn set_assign_implicit_output() {
    let mut s = session();
    write_class(&s, "inp");
    let stats = run("data out; set inp; x = age * 2; run;", &mut s).unwrap();
    assert_eq!(stats.read, vec![("WORK.INP".to_string(), 3)]);
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 3, 3)]);

    let ds = read_work(&s, "out");
    assert_eq!(ds.n_obs(), 3);
    let x = ds.df.column("x").unwrap().f64().unwrap();
    assert_eq!(x.get(0), Some(28.0));
    // age missing → x missing (propagation) + note.
    assert_eq!(x.get(1), None);
    assert_eq!(x.get(2), Some(26.0));

    let log = s.log.into_string();
    assert!(log.contains("There were 3 observations read from the data set WORK.INP."));
    assert!(log.contains("The data set WORK.OUT has 3 observations and 3 variables."));
    assert!(log.contains("Missing values were generated"));
    assert_eq!(s.last_dataset.as_deref(), Some("WORK.OUT"));
}

#[test]
fn set_exhausted_inside_do_ends_step() {
    let mut s = session();
    write_class(&s, "inp");
    // Le SET vit DANS la boucle : à l'épuisement de l'input (4e tour),
    // EndStep traverse le DO et termine l'étape. 3 outputs explicites.
    let stats = run(
        "data out; do i = 1 to 10; set inp; output; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(stats.read, vec![("WORK.INP".to_string(), 3)]);
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 3, 3)]);
}

#[test]
fn set_keep_outputs_only_kept_variables() {
    let mut s = session();
    write_class_full(&s, "class");
    let stats = run("data out; set class(keep=name age); run;", &mut s).unwrap();
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 4, 2)]);
    let ds = read_work(&s, "out");
    let cols: Vec<&str> = ds.df.get_column_names_str();
    assert_eq!(cols, vec!["Name", "Age"]);
}

#[test]
fn set_where_filters_rows_and_read_counter() {
    let mut s = session();
    write_class_full(&s, "class");
    let stats = run("data out; set class(where=(age > 13)); run;", &mut s).unwrap();
    // 14, ., 13, 15 : seuls 14 et 15 passent ; le compteur d'obs LUES
    // est réduit aux lignes qui passent (fidèle à la NOTE SAS).
    assert_eq!(stats.read, vec![("WORK.CLASS".to_string(), 2)]);
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 2, 3)]);
    let ds = read_work(&s, "out");
    let name = ds.df.column("Name").unwrap().str().unwrap();
    assert_eq!(name.get(0), Some("Alfred"));
    assert_eq!(name.get(1), Some("Henry"));
    let log = s.log.into_string();
    assert!(
        log.contains("There were 2 observations read from the data set WORK.CLASS."),
        "log was: {log}"
    );
}

#[test]
fn set_rename_exposes_new_name_only() {
    let mut s = session();
    write_class_full(&s, "class");
    run(
        "data out; set class(rename=(age=years)); next = years + 1; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    let cols: Vec<&str> = ds.df.get_column_names_str();
    assert!(cols.contains(&"years"), "columns were: {cols:?}");
    assert!(!cols.contains(&"Age"), "columns were: {cols:?}");
    let years = ds.df.column("years").unwrap().f64().unwrap();
    assert_eq!(years.get(0), Some(14.0));
    let next = ds.df.column("next").unwrap().f64().unwrap();
    assert_eq!(next.get(0), Some(15.0));
}

#[test]
fn set_two_datasets_without_by_concatenates() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 3.0, 5.0]))]);
    write_num_ds(&s, "b", &[("x", some(&[2.0, 3.0, 4.0]))]);
    let stats = run("data out; set a b; run;", &mut s).unwrap();
    // Tout a, puis tout b.
    assert_eq!(col(&s, "out", "x"), some(&[1.0, 3.0, 5.0, 2.0, 3.0, 4.0]));
    assert_eq!(
        stats.read,
        vec![("WORK.A".to_string(), 3), ("WORK.B".to_string(), 3)]
    );
    let log = s.log.into_string();
    assert!(log.contains("There were 3 observations read from the data set WORK.A."));
    assert!(log.contains("There were 3 observations read from the data set WORK.B."));
}

#[test]
fn set_by_interleaves_sorted_datasets_with_union_and_retained_vars() {
    let mut s = session();
    // u n'existe que dans a, v que dans b.
    write_num_ds(
        &s,
        "a",
        &[
            ("x", some(&[1.0, 3.0, 5.0])),
            ("u", some(&[10.0, 30.0, 50.0])),
        ],
    );
    write_num_ds(
        &s,
        "b",
        &[
            ("x", some(&[2.0, 3.0, 4.0])),
            ("v", some(&[200.0, 300.0, 400.0])),
        ],
    );
    let stats = run(
        "data out; set a b; by x; f = first.x; l = last.x; run;",
        &mut s,
    )
    .unwrap();
    // Interclassement par x croissant ; égalité (x=3) → a (premier du
    // SET) avant b.
    assert_eq!(col(&s, "out", "x"), some(&[1.0, 2.0, 3.0, 3.0, 4.0, 5.0]));
    // u/v : RETAIN implicite des variables de SET — une variable
    // absente du dataset de l'obs courante GARDE sa valeur précédente
    // (et reste missing avant sa première lecture).
    assert_eq!(
        col(&s, "out", "u"),
        vec![
            Some(10.0),
            Some(10.0),
            Some(30.0),
            Some(30.0),
            Some(30.0),
            Some(50.0)
        ]
    );
    assert_eq!(
        col(&s, "out", "v"),
        vec![
            None,
            Some(200.0),
            Some(200.0),
            Some(300.0),
            Some(400.0),
            Some(400.0)
        ]
    );
    // FIRST.x / LAST.x : le groupe x=3 a deux obs ; LAST. de la
    // dernière obs globale vaut 1.
    assert_eq!(col(&s, "out", "f"), some(&[1.0, 1.0, 1.0, 0.0, 1.0, 1.0]));
    assert_eq!(col(&s, "out", "l"), some(&[1.0, 1.0, 0.0, 1.0, 1.0, 1.0]));
    assert_eq!(
        stats.read,
        vec![("WORK.A".to_string(), 3), ("WORK.B".to_string(), 3)]
    );
}

#[test]
fn set_in_option_is_error() {
    let mut s = session();
    write_num_ds(&s, "a", &[("id", some(&[1.0]))]);
    let err = run("data out; set a(in=ina); run;", &mut s).err().unwrap();
    assert!(err.to_string().contains("IN="), "got: {err}");
}

/// SET de 3 datasets : concaténation dans l'ordre, comptes par dataset.
#[test]
fn set_three_datasets_concatenates_in_order() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0]))]);
    write_num_ds(&s, "b", &[("x", some(&[3.0]))]);
    write_num_ds(&s, "c", &[("x", some(&[4.0, 5.0]))]);
    let stats = run("data out; set a b c; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "x"), some(&[1.0, 2.0, 3.0, 4.0, 5.0]));
    assert_eq!(
        stats.read,
        vec![
            ("WORK.A".to_string(), 2),
            ("WORK.B".to_string(), 1),
            ("WORK.C".to_string(), 2),
        ]
    );
}

#[test]
fn subsetting_if_filters() {
    let mut s = session();
    write_class(&s, "inp");
    let stats = run("data out; set inp; if age > 13; run;", &mut s).unwrap();
    // age > 13 : 14 vrai, missing faux (. < 14), 13 faux.
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 2)]);
    let ds = read_work(&s, "out");
    let name = ds.df.column("Name").unwrap().str().unwrap();
    assert_eq!(name.get(0), Some("Alfred"));
}

#[test]
fn explicit_output_disables_implicit() {
    let mut s = session();
    write_class(&s, "inp");
    let stats = run("data out; set inp; output; output; run;", &mut s).unwrap();
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 6, 2)]);
}

#[test]
fn stop_ends_step_without_output() {
    let mut s = session();
    write_class(&s, "inp");
    let stats = run("data out; set inp; output; stop; run;", &mut s).unwrap();
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 2)]);
    // STOP au milieu : une seule ligne lue.
    assert_eq!(stats.read, vec![("WORK.INP".to_string(), 1)]);
}

#[test]
fn stop_inside_do_ends_step() {
    let mut s = session();
    write_class(&s, "inp");
    let stats = run("data out; set inp; do i = 1 to 10; stop; end; run;", &mut s).unwrap();
    // STOP au premier tour de la première itération : rien d'écrit,
    // une seule ligne lue.
    assert_eq!(stats.read, vec![("WORK.INP".to_string(), 1)]);
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 0, 3)]);
}

#[test]
fn no_input_runs_single_iteration() {
    let mut s = session();
    let stats = run("data out; x = 1; y = 'ab'; run;", &mut s).unwrap();
    assert_eq!(stats.read, vec![]);
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 2)]);
    let ds = read_work(&s, "out");
    assert_eq!(ds.n_obs(), 1);
    assert_eq!(ds.df.column("y").unwrap().str().unwrap().get(0), Some("ab"));
}

#[test]
fn data_null_writes_nothing() {
    let mut s = session();
    write_class(&s, "inp");
    let stats = run("data _null_; set inp; run;", &mut s).unwrap();
    assert!(stats.written.is_empty());
    assert!(!s.libs.get("WORK").unwrap().exists("_null_"));
    // _LAST_ inchangé.
    assert_eq!(s.last_dataset, None);
}

#[test]
fn if_then_else_branches() {
    let mut s = session();
    write_class(&s, "inp");
    run(
        "data out; set inp; if age >= 14 then grp = 'old'; else grp = 'yng'; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    let grp = ds.df.column("grp").unwrap().str().unwrap();
    assert_eq!(grp.get(0), Some("old"));
    // age missing : . >= 14 faux → else.
    assert_eq!(grp.get(1), Some("yng"));
    assert_eq!(grp.get(2), Some("yng"));
}

#[test]
fn uninitialized_note_and_missing_column() {
    let mut s = session();
    let stats = run("data out; y = x; run;", &mut s).unwrap();
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 2)]);
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("x").unwrap().f64().unwrap().get(0), None);
    let log = s.log.into_string();
    assert!(log.contains("Variable x is uninitialized."));
}

#[test]
fn assign_coercion_num_to_char_best12_right_justified() {
    let mut s = session();
    run("data out; c = 'init'; c = 7; run;", &mut s).unwrap();
    // c figée Char(4) par la 1re assignation ; 7 → BEST12 justifié
    // droite sur 12 ('           7') puis tronqué à 4 ('    ') → trim
    // stockage → "".
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("c").unwrap().str().unwrap().get(0), Some(""));
    let log = s.log.into_string();
    assert!(log.contains("Numeric values have been converted to character values."));
}

#[test]
fn special_missing_roundtrip_through_output() {
    let mut s = session();
    write_class(&s, "inp");
    // .a : missing spécial assigné puis écrit ; doit survivre au parquet.
    run("data out; set inp; m = .a; run;", &mut s).unwrap();
    let ds = read_work(&s, "out");
    // Relu : NaN payload → décodé par num_to_value au prochain SET ;
    // au niveau parquet brut c'est un NaN (pas un null).
    let m = ds.df.column("m").unwrap().f64().unwrap();
    let raw = m.get(0);
    assert!(raw.is_some_and(f64::is_nan));
    assert_eq!(
        crate::missing::num_to_value(raw),
        Value::Missing(MissingKind::Letter(0))
    );
}

// ── Missings spéciaux bout en bout + _ERROR_ + NOTEs (M2, lot 5) ─────

#[test]
fn special_missings_keep_identity_through_parquet_roundtrip() {
    use crate::missing::num_to_value;
    let mut s = session();
    // Étape 1 : assigne les trois familles de missing et écrit en
    // parquet (WORK est une DirLibrary : écriture/lecture RÉELLES).
    run("data a; x = .a; y = ._; z = .; run;", &mut s).unwrap();

    // Le parquet de A relu directement : x/y sont des NaN (PAS des
    // nulls), z est un null (`.` ordinaire ⇔ null Polars).
    let a = read_work(&s, "a");
    let at = |c: &str| a.df.column(c).unwrap().f64().unwrap().get(0);
    assert!(at("x").is_some_and(f64::is_nan));
    assert!(at("y").is_some_and(f64::is_nan));
    assert_eq!(at("z"), None);
    // Et chaque missing garde SON IDENTITÉ au décodage.
    assert_eq!(
        num_to_value(at("x")),
        Value::Missing(MissingKind::Letter(0))
    );
    assert_eq!(
        num_to_value(at("y")),
        Value::Missing(MissingKind::Underscore)
    );
    assert_eq!(num_to_value(at("z")), Value::missing());
    // x ≠ y ≠ z par sas_cmp (ordre total : ._ < . < .a).
    let (x, y, z) = (
        num_to_value(at("x")),
        num_to_value(at("y")),
        num_to_value(at("z")),
    );
    assert_ne!(x.sas_cmp(&y), std::cmp::Ordering::Equal);
    assert_ne!(x.sas_cmp(&z), std::cmp::Ordering::Equal);
    assert_ne!(y.sas_cmp(&z), std::cmp::Ordering::Equal);

    // Étape 2 : relecture via SET — `.a` relu == `.a`, distinct de
    // `.b` et de `.`.
    run(
        "data b; set a; xa = (x = .a); xb = (x = .b); xd = (x = .); \
         xy = (x = y); yu = (y = ._); run;",
        &mut s,
    )
    .unwrap();
    let b = read_work(&s, "b");
    let bt = |c: &str| b.df.column(c).unwrap().f64().unwrap().get(0);
    assert_eq!(bt("xa"), Some(1.0), ".a relu doit valoir .a");
    assert_eq!(bt("xb"), Some(0.0), ".a relu doit rester distinct de .b");
    assert_eq!(bt("xd"), Some(0.0), ".a relu doit rester distinct de .");
    assert_eq!(bt("xy"), Some(0.0), ".a et ._ doivent rester distincts");
    assert_eq!(bt("yu"), Some(1.0), "._ relu doit valoir ._");
}

// ── RETAIN / sum statement / LENGTH (M2) ─────────────────────────────

#[test]
fn sum_statement_counter_increments_per_obs() {
    let mut s = session();
    write_class(&s, "inp");
    run("data out; set inp; n + 1; run;", &mut s).unwrap();
    let ds = read_work(&s, "out");
    let n = ds.df.column("n").unwrap().f64().unwrap();
    assert_eq!(n.get(0), Some(1.0));
    assert_eq!(n.get(1), Some(2.0));
    assert_eq!(n.get(2), Some(3.0));
}

#[test]
fn sum_statement_ignores_missing_increment() {
    let mut s = session();
    write_class(&s, "inp");
    // age = 14, ., 13 : le missing du milieu ajoute 0 (PAS propagé).
    run("data out; set inp; total + age; run;", &mut s).unwrap();
    let ds = read_work(&s, "out");
    let total = ds.df.column("total").unwrap().f64().unwrap();
    assert_eq!(total.get(0), Some(14.0));
    assert_eq!(total.get(1), Some(14.0));
    assert_eq!(total.get(2), Some(27.0));
    // Aucun missing généré par le sum statement.
    let log = s.log.into_string();
    assert!(
        !log.contains("Missing values were generated"),
        "log was: {log}"
    );
}

#[test]
fn sum_statement_missing_accumulator_restarts_from_zero() {
    let mut s = session();
    write_class(&s, "inp");
    // total remis à `.` à chaque itération : total + age repart de 0.
    run("data out; set inp; total = .; total + age; run;", &mut s).unwrap();
    let ds = read_work(&s, "out");
    let total = ds.df.column("total").unwrap().f64().unwrap();
    assert_eq!(total.get(0), Some(14.0));
    assert_eq!(total.get(1), Some(0.0));
    assert_eq!(total.get(2), Some(13.0));
}
