use super::*;

#[test]
fn set_brings_input_vars_in_dataset_order() {
    let mut s = session();
    write_class(&s, "inp");
    let prog = compile_src("data out; set inp; x = age + 1; run;", &mut s).unwrap();

    let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["Age", "Name", "x"]);
    assert!(prog.pdv.vars()[0].from_input);
    assert!(prog.pdv.vars()[1].from_input);
    assert!(!prog.pdv.vars()[2].from_input);
    assert_eq!(prog.pdv.vars()[1].ty, VarType::Char);
    assert_eq!(prog.pdv.vars()[1].length, 7);

    let input = prog.input.as_ref().unwrap();
    assert!(input.by.is_empty());
    let ds0 = &input.datasets[0];
    assert_eq!(ds0.n_rows, 3);
    assert_eq!(ds0.display, "WORK.INP");
    assert_eq!(ds0.var_slots, vec![0, 1]);
    // Colonnes décodées en Value, missing `.` inclus.
    assert_eq!(ds0.columns[0][0], Value::Num(14.0));
    assert_eq!(ds0.columns[0][1], Value::missing());
    assert_eq!(ds0.columns[1][2], Value::Char("Barbara".into()));

    // Implicit output : pas de OUTPUT explicite.
    assert!(!prog.has_explicit_output);
    assert!(prog.uninitialized.is_empty());
    assert_eq!(prog.outputs.len(), 1);
    assert_eq!(prog.outputs[0].display, "WORK.OUT");
    assert_eq!(prog.outputs[0].kept_slots, vec![0, 1, 2]);
}

#[test]
fn set_missing_table_errors() {
    let mut s = session();
    let err = compile_src("data o; set nosuch; run;", &mut s).err().unwrap();
    assert_eq!(err.to_string(), "File WORK.NOSUCH.DATA does not exist.");
}

#[test]
fn set_two_datasets_union_of_variables_in_first_appearance_order() {
    let mut s = session();
    write_class(&s, "a"); // Age, Name
    write_weights(&s, "b"); // Age, Weight
    let prog = compile_src("data o; set a b; run;", &mut s).unwrap();
    let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
    // Variables de a, puis les NOUVELLES de b.
    assert_eq!(names, vec!["Age", "Name", "Weight"]);
    assert!(prog.pdv.vars().iter().all(|v| v.from_input));
    let input = prog.input.as_ref().unwrap();
    assert_eq!(input.datasets.len(), 2);
    assert!(input.by.is_empty());
    // Age de b pointe le slot partagé 0.
    assert_eq!(input.datasets[1].var_slots, vec![0, 2]);
}

#[test]
fn first_reference_order_without_set() {
    let mut s = session();
    let prog = compile_src("data o; x = y; z = 'abc'; run;", &mut s).unwrap();
    let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
    // Cible avant les variables de l'expression, ordre textuel.
    assert_eq!(names, vec!["x", "y", "z"]);
    // x inféré Num (y inconnue au moment de l'inférence).
    assert_eq!(prog.pdv.vars()[0].ty, VarType::Num);
    // z : Char(3) du littéral.
    assert_eq!(prog.pdv.vars()[2].ty, VarType::Char);
    assert_eq!(prog.pdv.vars()[2].length, 3);
    // y : référencée jamais assignée → uninitialized.
    assert_eq!(prog.uninitialized, vec!["y".to_string()]);
}

#[test]
fn first_assignment_freezes_type_and_length() {
    let mut s = session();
    let prog = compile_src("data o; s = 'ab'; s = 'abcdef'; run;", &mut s).unwrap();
    assert_eq!(prog.pdv.vars()[0].ty, VarType::Char);
    // La première assignation fige la longueur à 2.
    assert_eq!(prog.pdv.vars()[0].length, 2);
}

#[test]
fn first_last_have_no_pdv_slot_and_by_is_resolved() {
    let mut s = session();
    write_class(&s, "inp");
    let prog = compile_src(
        "data o; set inp; by age; f = first.age; l = last.age; run;",
        &mut s,
    )
    .unwrap();
    // Pas de slot PDV pour FIRST./LAST. (comme _N_/_ERROR_) : ni
    // colonne de sortie ni NOTE uninitialized.
    assert!(prog.pdv.slot("FIRST.AGE").is_none());
    assert!(prog.pdv.slot("LAST.AGE").is_none());
    let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["Age", "Name", "f", "l"]);
    assert!(prog.uninitialized.is_empty());
    let input = prog.input.as_ref().unwrap();
    assert_eq!(input.by.len(), 1);
    assert_eq!(input.by[0].name, "AGE");
    assert_eq!(input.by[0].slot, 0);
    assert!(!input.by[0].descending);
    assert_eq!(input.datasets[0].by_cols, vec![0]);
}

#[test]
fn first_last_on_non_by_variable_errors() {
    let mut s = session();
    write_class(&s, "inp");
    // Pas de BY du tout.
    let err = compile_src("data o; set inp; f = first.age; run;", &mut s)
        .err()
        .unwrap();
    assert!(
        err.to_string().contains("AGE is not a BY variable"),
        "got: {err}"
    );
    // BY présent mais sur une autre variable.
    let err = compile_src("data o; set inp; by age; f = last.name; run;", &mut s)
        .err()
        .unwrap();
    assert!(
        err.to_string().contains("NAME is not a BY variable"),
        "got: {err}"
    );
}

#[test]
fn concat_length_is_sum_with_num_as_12() {
    let mut s = session();
    let prog = compile_src("data o; c = 'ab' || 'cde'; d = c || x; run;", &mut s).unwrap();
    // c = 2 + 3.
    assert_eq!(prog.pdv.vars()[0].length, 5);
    assert_eq!(prog.pdv.vars()[0].ty, VarType::Char);
    // d = len(c) + 12 (x numérique).
    let d = &prog.pdv.vars()[1];
    assert_eq!(d.name, "d");
    assert_eq!(d.length, 5 + 12);
}

#[test]
fn call_inference_table() {
    let mut s = session();
    let prog = compile_src(
        "data o; a = 'xyz'; u = upcase(a); t = cats(a, a); n = sum(1, 2); run;",
        &mut s,
    )
    .unwrap();
    let var = |n: &str| {
        let slot = prog.pdv.slot(n).unwrap();
        &prog.pdv.vars()[slot]
    };
    assert_eq!((var("u").ty, var("u").length), (VarType::Char, 3));
    assert_eq!((var("t").ty, var("t").length), (VarType::Char, 200));
    assert_eq!(var("n").ty, VarType::Num);
}

#[test]
fn keep_drop_interaction() {
    let mut s = session();
    let prog = compile_src(
        "data o; x = 1; y = 2; z = 3; keep x y; drop y; run;",
        &mut s,
    )
    .unwrap();
    // keep {x,y} puis drop y → x seul.
    assert_eq!(prog.outputs[0].kept_slots, vec![0]);
    // Le WARNING de l'intersection est dans le log.
    let log = s.log.into_string();
    assert!(log.contains("WARNING"), "log was: {log}");
    assert!(log.contains("KEEP and DROP"), "log was: {log}");
}

#[test]
fn keep_unknown_variable_errors() {
    let mut s = session();
    let err = compile_src("data o; x = 1; keep nosuch; run;", &mut s).err().unwrap();
    assert!(
        err.to_string()
            .contains("in the DROP, KEEP, or RENAME list has never been referenced"),
        "got: {err}"
    );
}

#[test]
fn data_null_has_no_outputs() {
    let mut s = session();
    let prog = compile_src("data _null_; x = 1; run;", &mut s).unwrap();
    assert!(prog.outputs.is_empty());
}

#[test]
fn second_set_errors_m1() {
    let mut s = session();
    write_class(&s, "a");
    write_class(&s, "b");
    let err = compile_src("data o; set a; set b; run;", &mut s).err().unwrap();
    assert!(err.to_string().contains("not yet implemented"));
}

#[test]
fn explicit_output_detected_inside_if() {
    let mut s = session();
    let prog = compile_src(
        "data o; x = 1; if x then do; output; end; run;",
        &mut s,
    )
    .unwrap();
    assert!(prog.has_explicit_output);
}

#[test]
fn multiple_outputs_share_kept_slots() {
    let mut s = session();
    let prog = compile_src("data a b; x = 1; y = 2; drop y; run;", &mut s).unwrap();
    assert_eq!(prog.outputs.len(), 2);
    assert_eq!(prog.outputs[0].kept_slots, vec![0]);
    assert_eq!(prog.outputs[1].kept_slots, vec![0]);
    assert_eq!(prog.outputs[0].libref, "WORK");
    assert_eq!(prog.outputs[1].display, "WORK.B");
}

#[test]
fn assign_before_set_still_marks_from_input() {
    let mut s = session();
    write_class(&s, "inp");
    // `age` référencée avant le SET : elle doit malgré tout être
    // marquée from_input (pas de reset à chaque itération).
    let prog = compile_src("data o; age = 0; set inp; run;", &mut s).unwrap();
    let slot = prog.pdv.slot("age").unwrap();
    assert!(prog.pdv.vars()[slot].from_input);
    // Et l'ordre de première référence place age en tête.
    assert_eq!(prog.pdv.vars()[0].name, "age");
}

// ── RETAIN (M2) ──────────────────────────────────────────────────────

#[test]
fn retain_with_init_creates_retained_var_with_initial_value() {
    let mut s = session();
    let prog = compile_src("data o; retain x 5 s 'ab'; y = 1; run;", &mut s).unwrap();
    // Ordre de première référence : x et s entrent au RETAIN, avant y.
    let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["x", "s", "y"]);
    assert!(prog.pdv.vars()[0].retained);
    assert!(prog.pdv.vars()[1].retained);
    assert!(!prog.pdv.vars()[2].retained);
    // Types/longueurs des littéraux.
    assert_eq!(prog.pdv.vars()[0].ty, VarType::Num);
    assert_eq!(prog.pdv.vars()[1].ty, VarType::Char);
    assert_eq!(prog.pdv.vars()[1].length, 2);
    // Valeurs initiales.
    assert_eq!(
        prog.initial_values,
        vec![(0, Value::Num(5.0)), (1, Value::Char("ab".into()))]
    );
    // RETAIN avec init = initialisée : pas de NOTE uninitialized.
    assert!(prog.uninitialized.is_empty());
}

#[test]
fn retain_without_init_flags_later_reference_or_creates_num() {
    let mut s = session();
    // k : retenue sans init, type figé par l'assignation ultérieure.
    // j : retenue sans init, jamais référencée → Num + uninitialized,
    // créée en FIN d'ordre PDV (simplification M2 documentée).
    let prog = compile_src("data o; retain k j; x = 1; k = 'ab'; run;", &mut s).unwrap();
    let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["x", "k", "j"]);
    let var = |n: &str| &prog.pdv.vars()[prog.pdv.slot(n).unwrap()];
    assert!(var("k").retained);
    assert_eq!(var("k").ty, VarType::Char);
    assert_eq!(var("k").length, 2);
    assert!(var("j").retained);
    assert_eq!(var("j").ty, VarType::Num);
    assert!(!var("x").retained);
    assert_eq!(prog.uninitialized, vec!["j".to_string()]);
    // Pas de valeur initiale sans init.
    assert!(prog.initial_values.is_empty());
}

#[test]
fn retain_bare_retains_whole_pdv() {
    let mut s = session();
    write_class(&s, "inp");
    let prog = compile_src("data o; set inp; retain; x = 1; run;", &mut s).unwrap();
    assert!(prog.pdv.vars().iter().all(|v| v.retained));
}

#[test]
fn retain_init_wins_over_sum_zero_in_both_orders() {
    let mut s = session();
    // RETAIN d'abord : le sum statement ne pousse pas son 0.
    let prog = compile_src("data o; retain n 100; n + 1; run;", &mut s).unwrap();
    let slot = prog.pdv.slot("n").unwrap();
    assert_eq!(prog.initial_values, vec![(slot, Value::Num(100.0))]);
    // Sum d'abord : les deux entrées coexistent, le RETAIN (appliqué en
    // dernier par l'exécuteur) gagne.
    let prog = compile_src("data o; n + 1; retain n 100; run;", &mut s).unwrap();
    let slot = prog.pdv.slot("n").unwrap();
    assert_eq!(
        prog.initial_values,
        vec![(slot, Value::Num(0.0)), (slot, Value::Num(100.0))]
    );
}

// ── Sum statement (M2) ───────────────────────────────────────────────

#[test]
fn sum_statement_compiles_retained_num_with_initial_zero() {
    let mut s = session();
    write_class(&s, "inp");
    let prog = compile_src("data o; set inp; total + age; run;", &mut s).unwrap();
    let slot = prog.pdv.slot("total").unwrap();
    let v = &prog.pdv.vars()[slot];
    assert_eq!(v.ty, VarType::Num);
    assert_eq!(v.length, 8);
    assert!(v.retained);
    assert_eq!(prog.initial_values, vec![(slot, Value::Num(0.0))]);
    // La cible d'un sum statement compte comme initialisée.
    assert!(prog.uninitialized.is_empty());
}

// ── LENGTH (M2) ──────────────────────────────────────────────────────

#[test]
fn length_before_first_reference_fixes_type_and_length() {
    let mut s = session();
    let prog = compile_src(
        "data o; length c $ 3 n 4; c = 'abcdef'; n = 1; run;",
        &mut s,
    )
    .unwrap();
    let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["c", "n"]);
    let c = &prog.pdv.vars()[0];
    assert_eq!(c.ty, VarType::Char);
    // La longueur du LENGTH gagne sur celle du littéral assigné.
    assert_eq!(c.length, 3);
    let n = &prog.pdv.vars()[1];
    assert_eq!(n.ty, VarType::Num);
    // Pour une numérique, la longueur est une métadonnée (stockage f64).
    assert_eq!(n.length, 4);
    assert!(prog.uninitialized.is_empty());
}

#[test]
fn length_after_reference_warns_for_differing_char() {
    let mut s = session();
    let prog = compile_src("data o; c = 'ab'; length c $ 10; run;", &mut s).unwrap();
    // La longueur reste figée par la première référence.
    assert_eq!(prog.pdv.vars()[0].length, 2);
    let log = s.log.into_string();
    assert!(
        log.contains("WARNING: Length of character variable c has already been set."),
        "log was: {log}"
    );
}

#[test]
fn length_after_reference_is_silent_for_num_and_same_char_length() {
    let mut s = session();
    let prog = compile_src(
        "data o; x = 1; length x 5; c = 'ab'; length c $ 2; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(prog.pdv.vars()[0].length, 8);
    let log = s.log.into_string();
    assert!(!log.contains("WARNING"), "log was: {log}");
}

#[test]
fn length_out_of_range_errors() {
    let mut s = session();
    let err = compile_src("data o; length n 9; run;", &mut s).err().unwrap();
    assert!(err.to_string().contains("out of range (3-8)"), "got: {err}");
    let err = compile_src("data o; length c $ 40000; run;", &mut s)
        .err()
        .unwrap();
    assert!(
        err.to_string().contains("out of range (1-32767)"),
        "got: {err}"
    );
    let err = compile_src("data o; length n 2; run;", &mut s).err().unwrap();
    assert!(err.to_string().contains("out of range"), "got: {err}");
}

// ── DO itératif / DELETE (M2) ────────────────────────────────────────

#[test]
fn do_loop_index_enters_pdv_not_retained_and_assigned() {
    let mut s = session();
    let prog = compile_src("data o; do i = 1 to 3; x = i; end; run;", &mut s).unwrap();
    let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
    // L'index entre au point du DO, avant les variables du corps.
    assert_eq!(names, vec!["i", "x"]);
    let i = &prog.pdv.vars()[0];
    assert_eq!(i.ty, VarType::Num);
    assert_eq!(i.length, 8);
    assert!(!i.retained);
    // L'index compte comme assigné : pas de NOTE uninitialized.
    assert!(prog.uninitialized.is_empty());
}

#[test]
fn do_loop_bound_and_condition_vars_enter_pdv() {
    let mut s = session();
    let prog = compile_src(
        "data o; do i = a to b by c while(w) until(u); end; run;",
        &mut s,
    )
    .unwrap();
    let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
    // Index d'abord, puis from/to/by/while/until en ordre textuel.
    assert_eq!(names, vec!["i", "a", "b", "c", "w", "u"]);
    // Les bornes sont référencées jamais assignées → uninitialized.
    assert_eq!(
        prog.uninitialized,
        vec!["a", "b", "c", "w", "u"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
    );
}

#[test]
fn delete_compiles_and_output_in_do_body_is_detected() {
    let mut s = session();
    write_class(&s, "inp");
    let prog = compile_src(
        "data o; set inp; if age = . then delete; do i = 1 to 2; output; end; run;",
        &mut s,
    )
    .unwrap();
    assert!(prog.has_explicit_output);
    assert!(prog.pdv.slot("i").is_some());
}
