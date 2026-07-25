use super::*;

#[test]
fn invalid_numeric_data_sets_error_with_single_note() {
    let mut s = session();
    write_class(&s, "inp");
    // name + 0 : conversion char→num invalide à chaque ligne →
    // _ERROR_=1 partout, mais chaque NOTE n'apparaît qu'UNE fois.
    run("data out; set inp; v = name + 0; e = _error_; run;", &mut s).unwrap();
    let ds = read_work(&s, "out");
    let e = ds.df.column("e").unwrap().f64().unwrap();
    assert!(e.iter().all(|v| v == Some(1.0)));
    let log = s.log.into_string();
    assert_eq!(
        log.matches("NOTE: Invalid numeric data.").count(),
        1,
        "log was: {log}"
    );
    assert_eq!(
        log.matches("NOTE: Character values have been converted to numeric values.")
            .count(),
        1,
        "log was: {log}"
    );
}

// ── INFILE / INPUT / DATALINES (M14) ─────────────────────────────────

#[test]
fn input_list_mode_basic() {
    let mut s = session();
    let stats = run(
        "data out; input name $ age; datalines;\nAlice 14\nBob 16\n;\nrun;",
        &mut s,
    )
    .unwrap();
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 2, 2)]);
    let ds = read_work(&s, "out");
    let name = ds.df.column("name").unwrap().str().unwrap();
    let age = ds.df.column("age").unwrap().f64().unwrap();
    assert_eq!(name.get(0), Some("Alice"));
    assert_eq!(age.get(0), Some(14.0));
    assert_eq!(name.get(1), Some("Bob"));
    assert_eq!(age.get(1), Some(16.0));
    // Données instream : SAS n'émet PAS de NOTE "records were read from
    // the infile DATALINES" (réservée aux fichiers externes) — seule la
    // NOTE du data set apparaît.
    let log = s.log.into_string();
    assert!(
        !log.contains("records were read from the infile"),
        "instream DATALINES must not emit an infile-records NOTE; log was: {log}"
    );
    assert!(
        log.contains("The data set WORK.OUT has 2 observations and 2 variables."),
        "log was: {log}"
    );
}

#[test]
fn input_column_mode() {
    let mut s = session();
    // Colonnes fixes : name = 1-10, age = 11-12.
    run(
        "data out; input name $ 1-10 age 11-12; datalines;\nAlice     14\nBob       16\n;\nrun;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    let name = ds.df.column("name").unwrap().str().unwrap();
    let age = ds.df.column("age").unwrap().f64().unwrap();
    assert_eq!(name.get(0), Some("Alice"));
    assert_eq!(age.get(0), Some(14.0));
    assert_eq!(name.get(1), Some("Bob"));
    assert_eq!(age.get(1), Some(16.0));
}

#[test]
fn input_formatted_informat_decimal() {
    let mut s = session();
    // Informat 5.2 : sans point décimal dans le champ, divise par 100.
    run(
        "data out; input x 5.2; datalines;\n12345\n6.78\n;\nrun;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    let x = ds.df.column("x").unwrap().f64().unwrap();
    // "12345" sans point → 123.45 ; "6.78" avec point → 6.78 (d ignoré).
    assert_eq!(x.get(0), Some(123.45));
    assert_eq!(x.get(1), Some(6.78));
}

#[test]
fn input_char_truncation_at_pdv() {
    let mut s = session();
    // $char4. : la longueur du PDV est 4 → troncature à l'assignation.
    run(
        "data out; input name $char4.; datalines;\nAlexander\n;\nrun;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    let name = ds.df.column("name").unwrap().str().unwrap();
    assert_eq!(name.get(0), Some("Alex"));
}

#[test]
fn input_dsd_consecutive_delimiters_are_missing() {
    let mut s = session();
    run(
        "data out; infile datalines dsd; input a b c; datalines;\n1,,3\n;\nrun;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    let a = ds.df.column("a").unwrap().f64().unwrap();
    let b = ds.df.column("b").unwrap().f64().unwrap();
    let c = ds.df.column("c").unwrap().f64().unwrap();
    assert_eq!(a.get(0), Some(1.0));
    assert_eq!(b.get(0), None); // champ vide → missing
    assert_eq!(c.get(0), Some(3.0));
}

#[test]
fn input_dsd_quoted_field_with_comma() {
    let mut s = session();
    // `$20.` informat → longueur 20 (le défaut liste serait 8 et
    // tronquerait "Smith, John").
    run(
        "data out; infile datalines dsd; input name $20. x; datalines;\n\"Smith, John\",5\n;\nrun;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    let name = ds.df.column("name").unwrap().str().unwrap();
    let x = ds.df.column("x").unwrap().f64().unwrap();
    assert_eq!(name.get(0), Some("Smith, John"));
    assert_eq!(x.get(0), Some(5.0));
}

#[test]
fn input_delimiter_option() {
    let mut s = session();
    run(
        "data out; infile datalines dlm='|'; input a b; datalines;\n10|20\n;\nrun;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("a").unwrap().f64().unwrap().get(0), Some(10.0));
    assert_eq!(ds.df.column("b").unwrap().f64().unwrap().get(0), Some(20.0));
}

#[test]
fn input_missover_short_record() {
    let mut s = session();
    // MISSOVER : la 2e ligne n'a qu'une valeur → b reste missing.
    run(
        "data out; infile datalines missover; input a b; datalines;\n1 2\n3\n;\nrun;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    let b = ds.df.column("b").unwrap().f64().unwrap();
    assert_eq!(b.get(0), Some(2.0));
    assert_eq!(b.get(1), None);
    assert_eq!(ds.n_obs(), 2);
}

#[test]
fn input_truncover_partial_field() {
    let mut s = session();
    // TRUNCOVER : champ formaté partiel en fin de ligne lu tel quel.
    run(
        "data out; infile datalines truncover; input x 5.; datalines;\n12\n;\nrun;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("x").unwrap().f64().unwrap().get(0), Some(12.0));
}

#[test]
fn input_stopover_errors() {
    let mut s = session();
    let err = run(
        "data out; infile datalines stopover; input a b c; datalines;\n1 2\n;\nrun;",
        &mut s,
    );
    assert!(err.is_err(), "expected STOPOVER error");
}

#[test]
fn input_double_hold_multiple_obs_per_line() {
    let mut s = session();
    // `@@` : plusieurs observations par ligne.
    run(
        "data out; input x @@; datalines;\n1 2 3 4 5\n;\nrun;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.n_obs(), 5);
    let x = ds.df.column("x").unwrap().f64().unwrap();
    assert_eq!(x.get(0), Some(1.0));
    assert_eq!(x.get(4), Some(5.0));
}

#[test]
fn input_single_hold_then_release() {
    let mut s = session();
    // `@` : maintient l'enregistrement pour un second INPUT de la même
    // itération — ici un seul INPUT lit deux variables avec hold, l'autre
    // est relâché à l'itération suivante.
    run(
        "data out; input a @; input b; datalines;\n1 2\n3 4\n;\nrun;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.n_obs(), 2);
    let a = ds.df.column("a").unwrap().f64().unwrap();
    let b = ds.df.column("b").unwrap().f64().unwrap();
    assert_eq!(a.get(0), Some(1.0));
    assert_eq!(b.get(0), Some(2.0));
    assert_eq!(a.get(1), Some(3.0));
    assert_eq!(b.get(1), Some(4.0));
}

#[test]
fn input_column_pointer_at() {
    let mut s = session();
    run(
        "data out; input @3 x 2.; datalines;\nXX42\n;\nrun;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("x").unwrap().f64().unwrap().get(0), Some(42.0));
}

#[test]
fn input_firstobs_obs_options() {
    let mut s = session();
    // FIRSTOBS=2, OBS=3 : lignes 2 et 3 seulement.
    run(
        "data out; infile datalines firstobs=2 obs=3; input x; datalines;\n1\n2\n3\n4\n;\nrun;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.n_obs(), 2);
    let x = ds.df.column("x").unwrap().f64().unwrap();
    assert_eq!(x.get(0), Some(2.0));
    assert_eq!(x.get(1), Some(3.0));
}

#[test]
fn input_informat_date9() {
    let mut s = session();
    run(
        "data out; input d date9.; datalines;\n01JAN1960\n02JAN1960\n;\nrun;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    let d = ds.df.column("d").unwrap().f64().unwrap();
    // epoch SAS 1960-01-01 = 0.
    assert_eq!(d.get(0), Some(0.0));
    assert_eq!(d.get(1), Some(1.0));
}

#[test]
fn input_list_modifier_colon_informat() {
    let mut s = session();
    // `:date9.` lit un jeton délimité puis applique l'informat.
    run(
        "data out; infile datalines; input name $ x :date9.; datalines;\nAlice 01JAN1960\n;\nrun;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("x").unwrap().f64().unwrap().get(0), Some(0.0));
    assert_eq!(
        ds.df.column("name").unwrap().str().unwrap().get(0),
        Some("Alice")
    );
}

#[test]
fn datalines_without_infile_is_implicit_source() {
    let mut s = session();
    // Pas de `infile datalines;` : `input` utilise quand même le bloc.
    run(
        "data out; input x y; datalines;\n1 2\n3 4\n;\nrun;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.n_obs(), 2);
}

#[test]
fn put_list_mode_to_log() {
    let mut s = session();
    write_class(&s, "inp");
    // `data _null_` : sortie PUT seulement, aucun dataset écrit.
    run("data _null_; set inp; put name age; run;", &mut s).unwrap();
    let log = s.log.into_string();
    let lines = put_log_lines(&log);
    // Age missing (Alice) → "." ; format BEST par défaut.
    assert_eq!(lines, vec!["Alfred 14", "Alice .", "Barbara 13"]);
}

#[test]
fn put_named_form() {
    let mut s = session();
    write_class(&s, "inp");
    run(
        "data _null_; set inp; if name='Alfred'; put name= age=; run;",
        &mut s,
    )
    .unwrap();
    let lines = put_log_lines(&s.log.into_string());
    assert_eq!(lines, vec!["name=Alfred age=14"]);
}

#[test]
fn put_literal_and_var() {
    let mut s = session();
    write_class(&s, "inp");
    run(
        "data _null_; set inp; if name='Alfred'; put 'Report for' name; run;",
        &mut s,
    )
    .unwrap();
    let lines = put_log_lines(&s.log.into_string());
    assert_eq!(lines, vec!["Report for Alfred"]);
}

#[test]
fn put_formatted_numeric() {
    let mut s = session();
    run("data _null_; x = 3.14159; put x 8.2; run;", &mut s).unwrap();
    let lines = put_log_lines(&s.log.into_string());
    // 8.2 → "    3.14" justifié, puis trim de fin (les blancs de tête
    // restent mais sont rognés par render_put_slot via .trim()).
    assert_eq!(lines, vec!["3.14"]);
}

#[test]
fn put_formatted_date9() {
    let mut s = session();
    // 0 = 01JAN1960 (epoch SAS).
    run("data _null_; d = 0; put d date9.; run;", &mut s).unwrap();
    let lines = put_log_lines(&s.log.into_string());
    assert_eq!(lines, vec!["01JAN1960"]);
}

#[test]
fn put_column_pointer_and_skip() {
    let mut s = session();
    run(
        "data _null_; x = 1; y = 2; put @5 x +3 y; run;",
        &mut s,
    )
    .unwrap();
    let lines = put_log_lines(&s.log.into_string());
    // @5 → "1" en colonne 5 (index 4) ; le curseur passe à la colonne 6
    // (index 5), +3 l'avance à la colonne 9 (index 8) où s'écrit "2".
    assert_eq!(lines, vec!["    1    2"]);
}

#[test]
fn put_slash_newline_within_one_put() {
    let mut s = session();
    run(
        "data _null_; x = 1; y = 2; put x / y; run;",
        &mut s,
    )
    .unwrap();
    let lines = put_log_lines(&s.log.into_string());
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn put_single_hold_joins_one_line() {
    let mut s = session();
    write_class(&s, "inp");
    // `put name @;` maintient la ligne ; le PUT suivant (même itération)
    // la continue, puis la relâche.
    run(
        "data _null_; set inp; put name @; put age; run;",
        &mut s,
    )
    .unwrap();
    let lines = put_log_lines(&s.log.into_string());
    // Une ligne par observation (hold simple relâché en fin d'itération).
    assert_eq!(lines, vec!["Alfred 14", "Alice .", "Barbara 13"]);
}

#[test]
fn put_double_hold_joins_across_iterations() {
    let mut s = session();
    write_class(&s, "inp");
    // `put name @@;` maintient la ligne À TRAVERS les itérations : les
    // trois noms s'accumulent sur une seule ligne, relâchée en fin d'étape.
    run("data _null_; set inp; put name @@; run;", &mut s).unwrap();
    let lines = put_log_lines(&s.log.into_string());
    assert_eq!(lines, vec!["Alfred Alice Barbara"]);
}

#[test]
fn put_all_writes_every_pdv_var() {
    let mut s = session();
    write_class(&s, "inp");
    run(
        "data _null_; set inp; if name='Alfred'; put _all_; run;",
        &mut s,
    )
    .unwrap();
    let lines = put_log_lines(&s.log.into_string());
    // Ordre PDV : Age (num) puis Name (char) — l'ordre des colonnes de
    // l'input.
    assert_eq!(lines, vec!["Age=14 Name=Alfred"]);
}

#[test]
fn put_unknown_variable_errors() {
    let mut s = session();
    let res = run("data _null_; x = 1; put nosuchvar; run;", &mut s);
    let err = res.err().expect("expected an error for an unknown PUT variable");
    assert!(
        err.to_string().contains("nosuchvar is not on the PUT statement"),
        "got: {err}"
    );
}

#[test]
fn put_default_destination_is_log() {
    let mut s = session();
    // Sans FILE, un PUT écrit dans le LOG (défaut SAS).
    run("data _null_; x = 42; put x; run;", &mut s).unwrap();
    let lines = put_log_lines(&s.log.into_string());
    assert_eq!(lines, vec!["42"]);
    // Rien dans le listing.
    assert!(!s.listing.into_string().contains("42"));
}
