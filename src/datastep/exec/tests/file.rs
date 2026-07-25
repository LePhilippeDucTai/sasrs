use super::*;

#[test]
fn file_print_routes_to_listing() {
    let mut s = session();
    write_class(&s, "inp");
    run(
        "data _null_; set inp; if name='Alfred'; file print; put 'in listing' name; run;",
        &mut s,
    )
    .unwrap();
    let listing = s.listing.into_string();
    assert!(
        listing.contains("in listing Alfred"),
        "listing was: {listing}"
    );
    // Rien dans le log côté PUT.
    let log = s.log.into_string();
    assert!(!log.contains("in listing"), "log was: {log}");
}

#[test]
fn file_log_explicit_routes_to_log() {
    let mut s = session();
    run(
        "data _null_; x = 7; file log; put 'val' x; run;",
        &mut s,
    )
    .unwrap();
    let lines = put_log_lines(&s.log.into_string());
    assert_eq!(lines, vec!["val 7"]);
}

#[test]
fn file_path_writes_external_file() {
    let mut s = session();
    write_class(&s, "inp");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("report.txt");
    let path_str = path.to_str().unwrap();
    let src = format!(
        "data _null_; set inp; file '{path_str}'; put name age; run;"
    );
    run(&src, &mut s).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "Alfred 14\nAlice .\nBarbara 13\n");
}

// ---- CALL MISSING ---------------------------------------------------

#[test]
fn call_missing_sets_numeric_to_missing() {
    let mut s = session();
    run(
        "data out; x = 5; y = 10; call missing(x); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(num_col(&ds, "x"), vec![None]);
    assert_eq!(num_col(&ds, "y"), vec![Some(10.0)]);
}

#[test]
fn call_missing_sets_char_to_empty() {
    let mut s = session();
    run(
        "data out; length name $10; name = 'Alice'; call missing(name); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "name"), vec![String::new()]);
}

#[test]
fn call_missing_multiple_vars_mixed_types() {
    let mut s = session();
    run(
        "data out; length c $5; a = 1; b = 2; c = 'hi'; call missing(a, b, c); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(num_col(&ds, "a"), vec![None]);
    assert_eq!(num_col(&ds, "b"), vec![None]);
    assert_eq!(str_col(&ds, "c"), vec![String::new()]);
}

// ---- CALL EXECUTE ---------------------------------------------------

#[test]
fn call_execute_queues_literal_code() {
    let mut s = session();
    run(
        "data _null_; call execute('data q; v = 7; run;'); run;",
        &mut s,
    )
    .unwrap();
    // L'étape elle-même ne fait que mettre en file (rejeu = exécuteur).
    assert_eq!(
        s.call_execute_queue,
        vec!["data q; v = 7; run;".to_string()]
    );
}

#[test]
fn call_execute_queues_per_row_in_order() {
    let mut s = session();
    write_class(&s, "inp");
    run(
        "data _null_; set inp; call execute('proc print; '||name); run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(s.call_execute_queue.len(), 3);
    assert!(s.call_execute_queue[0].contains("Alfred"));
    assert!(s.call_execute_queue[2].contains("Barbara"));
}

#[test]
fn call_execute_requires_one_argument() {
    let mut s = session();
    let res = run("data _null_; call execute('a', 'b'); run;", &mut s);
    assert!(res.is_err());
}

// ---- CALL SORTN / SORTC --------------------------------------------

#[test]
fn call_sortn_sorts_array_ascending() {
    let mut s = session();
    run(
        "data out; array a{3} a1-a3; a1=3; a2=1; a3=2; call sortn(a); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(num_col(&ds, "a1"), vec![Some(1.0)]);
    assert_eq!(num_col(&ds, "a2"), vec![Some(2.0)]);
    assert_eq!(num_col(&ds, "a3"), vec![Some(3.0)]);
}

#[test]
fn call_sortn_missing_sorts_first() {
    let mut s = session();
    // SAS collation: missing (.) is smaller than any number.
    run(
        "data out; array a{3} a1-a3; a1=5; a2=.; a3=1; call sortn(a); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(num_col(&ds, "a1"), vec![None]);
    assert_eq!(num_col(&ds, "a2"), vec![Some(1.0)]);
    assert_eq!(num_col(&ds, "a3"), vec![Some(5.0)]);
}

#[test]
fn call_sortc_sorts_char_array_ascending() {
    let mut s = session();
    run(
        "data out; array c{3} $5 c1-c3; c1='pear'; c2='apple'; c3='kiwi'; \
         call sortc(c); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "c1"), vec!["apple".to_string()]);
    assert_eq!(str_col(&ds, "c2"), vec!["kiwi".to_string()]);
    assert_eq!(str_col(&ds, "c3"), vec!["pear".to_string()]);
}

#[test]
fn call_sortn_explicit_var_list() {
    let mut s = session();
    run(
        "data out; x=9; y=2; z=5; call sortn(x, y, z); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(num_col(&ds, "x"), vec![Some(2.0)]);
    assert_eq!(num_col(&ds, "y"), vec![Some(5.0)]);
    assert_eq!(num_col(&ds, "z"), vec![Some(9.0)]);
}

// ---- CALL SYMPUTX ---------------------------------------------------

#[test]
fn call_symputx_trims_value() {
    let mut s = session();
    run(
        "data _null_; length v $20; v = '   hi   '; call symputx('a', v); run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(s.macro_engine.get_symbol("a").as_deref(), Some("hi"));
}

#[test]
fn call_symputx_numeric_no_blanks() {
    let mut s = session();
    run("data _null_; call symputx('n', 42); run;", &mut s).unwrap();
    assert_eq!(s.macro_engine.get_symbol("n").as_deref(), Some("42"));
}

#[test]
fn call_symput_vs_symputx_value_trimming() {
    // SYMPUT keeps leading blanks of a char value; SYMPUTX trims them.
    let mut s = session();
    run(
        "data _null_; length v $10; v = '  x'; call symput('a', v); call symputx('b', v); run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(s.macro_engine.get_symbol("a").as_deref(), Some("  x"));
    assert_eq!(s.macro_engine.get_symbol("b").as_deref(), Some("x"));
}

// ---- CALL CATS ------------------------------------------------------

#[test]
fn call_cats_concatenates_stripped() {
    let mut s = session();
    run(
        "data out; length r $20; a='  foo '; b=' bar'; call cats(r, a, b); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "r"), vec!["foobar".to_string()]);
}

#[test]
fn call_cats_mixed_num_and_char() {
    let mut s = session();
    run(
        "data out; length r $20; call cats(r, 'x', 12, 'y'); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "r"), vec!["x12y".to_string()]);
}

#[test]
fn call_cats_truncates_to_result_length() {
    let mut s = session();
    run(
        "data out; length r $3; call cats(r, 'abcdef'); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "r"), vec!["abc".to_string()]);
}

// ---- CALL SCAN ------------------------------------------------------

#[test]
fn call_scan_extracts_nth_word() {
    let mut s = session();
    run(
        "data out; length w $10; call scan('alpha beta gamma', 2, w); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "w"), vec!["beta".to_string()]);
}

#[test]
fn call_scan_negative_index_from_end() {
    let mut s = session();
    run(
        "data out; length w $10; call scan('a b c', -1, w); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "w"), vec!["c".to_string()]);
}

#[test]
fn call_scan_custom_delimiter() {
    let mut s = session();
    run(
        "data out; length w $10; call scan('a,b,c', 2, w, ','); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "w"), vec!["b".to_string()]);
}

// ---- CALL LABEL -----------------------------------------------------

#[test]
fn call_label_returns_label() {
    let mut s = session();
    run(
        "data out; length lbl $40; x = 1; label x = 'My X Variable'; \
         call label(x, lbl); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "lbl"), vec!["My X Variable".to_string()]);
}

#[test]
fn call_label_falls_back_to_name_when_no_label() {
    let mut s = session();
    run(
        "data out; length lbl $40; weight = 1; call label(weight, lbl); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    // No label declared → SAS returns the variable name.
    assert_eq!(str_col(&ds, "lbl"), vec!["weight".to_string()]);
}

// ---- CALL VNAME -----------------------------------------------------

#[test]
fn call_vname_returns_variable_name() {
    let mut s = session();
    run(
        "data out; length nm $32; Height = 1; call vname(Height, nm); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    // Name preserved with first-reference casing.
    assert_eq!(str_col(&ds, "nm"), vec!["Height".to_string()]);
}

#[test]
fn call_vname_on_array_element() {
    let mut s = session();
    run(
        "data out; length nm $32; array a{3} a1-a3; call vname(a{2}, nm); output; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "nm"), vec!["a2".to_string()]);
}

#[test]
fn unknown_call_routine_errors() {
    let mut s = session();
    let res = run("data _null_; call frobnicate(1); run;", &mut s);
    let err = res.err().expect("expected error for unknown CALL routine");
    assert!(err.to_string().contains("not yet implemented"), "got: {err}");
}
