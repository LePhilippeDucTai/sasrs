use super::*;

#[test]
fn include_simple_relative_path() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "inc.sas", "%let x = 42;");
    let mut e = engine_in(dir.path());
    // Le %include charge inc.sas (pose &x), puis &x se résout.
    let out = e.expand_open_code("%include 'inc.sas'; &x");
    assert_eq!(out.trim(), "42");
}

#[test]
fn include_double_quotes() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "inc.sas", "data a;run;");
    let mut e = engine_in(dir.path());
    let out = e.expand_open_code("%include \"inc.sas\";");
    assert!(out.contains("data a;run;"), "got: {out}");
}

#[test]
fn include_absolute_path() {
    let dir = tempfile::tempdir().unwrap();
    let p = write_file(dir.path(), "abs.sas", "%let y = hi;");
    // Engine sans base : on utilise un chemin absolu.
    let mut e = MacroEngine::new(true);
    let stmt = format!("%include '{}'; &y", p.display());
    let out = e.expand_open_code(&stmt);
    assert_eq!(out.trim(), "hi");
}

#[test]
fn include_defines_macro_then_invoked() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "mac.sas", "%macro greet; hello %mend;");
    let mut e = engine_in(dir.path());
    // Le fichier inclus DÉFINIT %greet ; l'appel suivant l'expanse.
    let out = e.expand_open_code("%include 'mac.sas'; %greet");
    assert_eq!(out.trim(), "hello");
}

#[test]
fn include_nested() {
    let dir = tempfile::tempdir().unwrap();
    // a.sas inclut b.sas ; b.sas pose &z.
    write_file(dir.path(), "b.sas", "%let z = nested;");
    write_file(dir.path(), "a.sas", "%include 'b.sas';");
    let mut e = engine_in(dir.path());
    let out = e.expand_open_code("%include 'a.sas'; &z");
    assert_eq!(out.trim(), "nested");
}

#[test]
fn include_missing_file_emits_note_no_panic() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine_in(dir.path());
    let out = e.expand_open_code("%include 'does_not_exist.sas'; after");
    assert!(out.contains("cannot read"), "got: {out}");
    // Le scan se poursuit après le statement.
    assert!(out.contains("after"), "got: {out}");
}

#[test]
fn include_cycle_hits_depth_limit_no_panic() {
    let dir = tempfile::tempdir().unwrap();
    // self.sas s'inclut lui-même : la garde de profondeur arrête le cycle.
    write_file(dir.path(), "self.sas", "%include 'self.sas';");
    let mut e = engine_in(dir.path());
    let out = e.expand_open_code("%include 'self.sas';");
    assert!(out.contains("nesting limit"), "got: {out}");
}

#[test]
fn include_stdin_star_deferral_note() {
    // M35.2 — `%include *;` (clavier/stdin) reste non supporté : note claire.
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine_in(dir.path());
    let out = e.expand_open_code("%include *; tail");
    assert!(out.contains("keyboard/stdin"), "got: {out}");
    assert!(out.contains("tail"), "got: {out}");
}

#[test]
fn include_unknown_bare_token_cannot_read() {
    // M35.2 — un token nu qui n'est ni un fileref ni un fichier existant est
    // traité comme un chemin → erreur "cannot read", le scan se poursuit.
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine_in(dir.path());
    let out = e.expand_open_code("%include myref; tail");
    assert!(out.contains("cannot read"), "got: {out}");
    assert!(out.contains("tail"), "got: {out}");
}

#[test]
fn include_via_fileref() {
    // M35.2 — FILENAME enregistre un fileref ; `%include fileref;` inline le
    // fichier visé (ici un %let dont l'effet est visible ensuite).
    let dir = tempfile::tempdir().unwrap();
    let p = write_file(dir.path(), "incref.sas", "%let x = 99;");
    let mut e = engine_in(dir.path());
    e.set_fileref("INC", p.clone());
    let out = e.expand_open_code("%include inc; &x");
    assert_eq!(out.trim(), "99");
}

#[test]
fn include_fileref_case_insensitive() {
    // M35.2 — la recherche de fileref est insensible à la casse.
    let dir = tempfile::tempdir().unwrap();
    let p = write_file(dir.path(), "incref2.sas", "%let y = ok;");
    let mut e = engine_in(dir.path());
    e.set_fileref("myref", p);
    let out = e.expand_open_code("%include MYREF; &y");
    assert_eq!(out.trim(), "ok");
}

#[test]
fn include_bare_relative_path() {
    // M35.2 — un token nu non-fileref est résolu comme chemin relatif à la
    // base d'inclusion.
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).unwrap();
    write_file(&sub, "child.sas", "%let z = bare;");
    let mut e = engine_in(dir.path());
    let out = e.expand_open_code("%include sub/child.sas; &z");
    assert_eq!(out.trim(), "bare");
}

#[test]
fn set_fileref_round_trip() {
    // M35.2 — round-trip du registre fileref + insensibilité à la casse.
    let mut e = MacroEngine::new(true);
    let path = std::path::PathBuf::from("/tmp/some/where.sas");
    e.set_fileref("Abc", path.clone());
    assert_eq!(e.fileref_path("ABC"), Some(&path));
    assert_eq!(e.fileref_path("abc"), Some(&path));
    assert_eq!(e.fileref_path("nope"), None);
}

#[test]
fn autocall_basic() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "sayhi.sas", "%macro sayhi; HI %mend;");
    let mut e = MacroEngine::new(true);
    e.set_sasautos_path(vec![dir.path().to_path_buf()]);
    // %sayhi non défini : chargé paresseusement depuis sayhi.sas.
    let out = e.expand_open_code("%sayhi");
    assert_eq!(out.trim(), "HI");
}

#[test]
fn autocall_with_args() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "dbl.sas", "%macro dbl(x); &x&x %mend;");
    let mut e = MacroEngine::new(true);
    e.set_sasautos_path(vec![dir.path().to_path_buf()]);
    let out = e.expand_open_code("%dbl(ab)");
    assert_eq!(out.trim(), "abab");
}

#[test]
fn autocall_nested() {
    let dir = tempfile::tempdir().unwrap();
    // outer appelle inner ; les deux sont des fichiers autocall.
    write_file(dir.path(), "inner.sas", "%macro inner; IN %mend;");
    write_file(dir.path(), "outer.sas", "%macro outer; [%inner] %mend;");
    let mut e = MacroEngine::new(true);
    e.set_sasautos_path(vec![dir.path().to_path_buf()]);
    let out = e.expand_open_code("%outer");
    assert_eq!(out.trim(), "[IN]");
}

#[test]
fn autocall_first_dir_wins() {
    let d1 = tempfile::tempdir().unwrap();
    let d2 = tempfile::tempdir().unwrap();
    write_file(d1.path(), "pick.sas", "%macro pick; ONE %mend;");
    write_file(d2.path(), "pick.sas", "%macro pick; TWO %mend;");
    let mut e = MacroEngine::new(true);
    e.set_sasautos_path(vec![d1.path().to_path_buf(), d2.path().to_path_buf()]);
    let out = e.expand_open_code("%pick");
    assert_eq!(out.trim(), "ONE");
}

#[test]
fn autocall_not_found_left_verbatim() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = MacroEngine::new(true);
    e.set_sasautos_path(vec![dir.path().to_path_buf()]);
    // Macro introuvable : `%nope` laissé verbatim (comportement historique).
    let out = e.expand_open_code("%nope");
    assert_eq!(out, "%nope");
}

#[test]
fn autocall_tried_only_once() {
    // Même sans fichier, la deuxième invocation ne doit pas re-tenter le
    // disque ni paniquer ; le résultat reste verbatim.
    let dir = tempfile::tempdir().unwrap();
    let mut e = MacroEngine::new(true);
    e.set_sasautos_path(vec![dir.path().to_path_buf()]);
    let out = e.expand_open_code("%miss %miss");
    assert_eq!(out, "%miss %miss");
}

#[test]
fn defined_macro_takes_priority_over_autocall() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "m.sas", "%macro m; FROMDISK %mend;");
    let mut e = MacroEngine::new(true);
    e.set_sasautos_path(vec![dir.path().to_path_buf()]);
    // Définition inline : elle prime, autocall n'est pas consulté.
    let out = e.expand_open_code("%macro m; INLINE %mend; %m");
    assert_eq!(out.trim(), "INLINE");
}

// --- M19.3 : trace options + %put + %call execute ---

#[test]
fn put_simple_text() {
    let mut e = MacroEngine::new(true);
    let _out = e.expand_open_code("%put Hello world;");
    let logs = e.take_pending_log_lines();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0], "Hello world");
}

#[test]
fn put_with_symbol_resolution() {
    let mut e = MacroEngine::new(true);
    let _out = e.expand_open_code("%let name=Alice; %put Hello &name;");
    let logs = e.take_pending_log_lines();
    assert!(logs.iter().any(|l| l.contains("Hello Alice")));
}

#[test]
fn put_empty_line() {
    let mut e = MacroEngine::new(true);
    let _out = e.expand_open_code("%put;");
    let logs = e.take_pending_log_lines();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0], "");
}

#[test]
fn mprint_flag_echoes_macro_output() {
    let mut e = MacroEngine::new(true);
    e.set_mprint(true);
    let _out = e.expand_open_code("%macro m; DATA x; RUN; %mend; %m");
    let logs = e.take_pending_log_lines();
    assert!(logs.iter().any(|l| l.starts_with("MPRINT(M):")));
    assert!(logs.iter().any(|l| l.contains("DATA x")));
}

#[test]
fn mlogic_flag_echoes_macro_entry_exit() {
    let mut e = MacroEngine::new(true);
    e.set_mlogic(true);
    let _out = e.expand_open_code("%macro m(a=1); x=&a; %mend; %m(a=5)");
    let logs = e.take_pending_log_lines();
    assert!(logs.iter().any(|l| l.contains("Beginning execution")));
    assert!(logs.iter().any(|l| l.contains("Parameter A has value 5")));
    assert!(logs.iter().any(|l| l.contains("Ending execution")));
}

#[test]
fn mlogic_flag_echoes_if_condition() {
    let mut e = MacroEngine::new(true);
    e.set_mlogic(true);
    let _out = e.expand_open_code("%macro m; %if 1=1 %then YES; %else NO; %mend; %m");
    let logs = e.take_pending_log_lines();
    assert!(logs.iter().any(|l| l.contains("is TRUE")));
}

#[test]
fn symbolgen_flag_echoes_symbol_resolution() {
    let mut e = MacroEngine::new(true);
    e.set_symbolgen(true);
    // SYMBOLGEN traces when a symbol is USED in the expansion, not just defined
    let _out = e.expand_open_code("%let x=abc; data &x;");
    let logs = e.take_pending_log_lines();
    assert!(
        logs.iter()
            .any(|l| l.contains("Macro variable X resolves to abc")),
        "got logs: {:?}",
        logs
    );
}

#[test]
fn call_execute_queues_code() {
    let mut e = MacroEngine::new(true);
    let _out = e.expand_open_code("%macro m; %call execute(data step here;); %mend; %m");
    let queue = e.take_pending_call_execute();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0], "data step here;");
}

#[test]
fn call_execute_resolves_symbols() {
    let mut e = MacroEngine::new(true);
    let _out =
        e.expand_open_code("%let step=SET x; %macro m; %call execute(&step run;); %mend; %m");
    let queue = e.take_pending_call_execute();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0], "SET x run;");
}

#[test]
fn multiple_trace_flags_interact() {
    let mut e = MacroEngine::new(true);
    e.set_mprint(true);
    e.set_mlogic(true);
    e.set_symbolgen(true);
    let _out = e.expand_open_code("%let x=5; %macro m; %if &x > 3 %then YES; %mend; %m");
    let logs = e.take_pending_log_lines();
    // Should have logs from MLOGIC and MPRINT at minimum
    assert!(
        logs.iter().any(|l| l.contains("MLOGIC")),
        "got logs: {:?}",
        logs
    );
    assert!(
        logs.iter().any(|l| l.contains("is TRUE")),
        "got logs: {:?}",
        logs
    );
    assert!(
        logs.iter().any(|l| l.contains("MPRINT")),
        "got logs: {:?}",
        logs
    );
}

// --- M35.4 : %return / %goto / %abort / hors-périmètre ---

#[test]
fn return_stops_body_caller_continues() {
    // Le texte APRÈS `%return;` dans le corps n'est PAS émis ; l'appelant
    // (open code) poursuit normalement.
    let out = run("%macro m; A %return; B %mend; %m C");
    assert_eq!(out, "A  C");
}

#[test]
fn return_honours_if_branch() {
    // `%if ... %then %return;` saute le reste du corps quand pris.
    let out = run("%macro m(x); P %if &x %then %return; Q %mend; %m(1)|%m(0)");
    assert_eq!(out, "P |P  Q");
}

#[test]
fn return_in_open_code_notes_and_continues() {
    let out = run("X %return; Y");
    assert!(
        out.contains("NOTE: %RETURN is not valid in open code"),
        "got: {out}"
    );
    assert!(out.contains('X') && out.contains('Y'), "got: {out}");
}

#[test]
fn return_reentrancy_two_calls_identical() {
    // Ré-entrance : deux invocations se comportent à l'identique (le drapeau
    // ne fuit pas d'un appel à l'autre).
    let out = run("%macro m; A %return; B %mend; [%m][%m]");
    assert_eq!(out, "[A ][A ]");
}

#[test]
fn goto_skips_block_forward() {
    // `%goto skip;` saute le bloc intermédiaire jusqu'à `%skip:`.
    let out = run("%macro m; A %goto skip; B %skip: C %mend; %m");
    assert_eq!(out, "A  C");
}

#[test]
fn goto_bounded_loop() {
    // Idiome boucle bornée : `&i` incrémenté par `%let`, sortie par `%if`.
    let src = "%macro m; %let i=0; \
               %top: [&i] %let i=%eval(&i+1); %if &i < 3 %then %goto top; \
               done %mend; %m";
    let out = run(src);
    assert!(
        out.contains("[0]") && out.contains("[1]") && out.contains("[2]"),
        "got: {out}"
    );
    assert!(out.contains("done"), "got: {out}");
    assert!(!out.contains("[3]"), "got: {out}");
}

#[test]
fn goto_missing_label_notes() {
    let out = run("%macro m; A %goto nope; B %mend; %m");
    assert!(
        out.contains("NOTE: %GOTO target label %nope: not found"),
        "got: {out}"
    );
}

#[test]
fn goto_in_open_code_notes() {
    let out = run("%goto x;");
    assert!(
        out.contains("NOTE: %GOTO is not valid in open code"),
        "got: {out}"
    );
}

#[test]
fn label_marker_emits_nothing() {
    let out = run("%macro m; %lbl: hi %mend; %m");
    assert_eq!(out.trim(), "hi");
}

#[test]
fn abort_stops_body_and_notes() {
    let mut e = MacroEngine::new(true);
    let out = e.expand_open_code("%macro m; AAA %abort; ZZZ %mend; %m");
    assert!(out.contains("NOTE: %ABORT encountered"), "got: {out}");
    assert!(out.contains("AAA") && !out.contains("ZZZ"), "got: {out}");
    assert_eq!(e.take_abort_request(), Some(AbortKind::Plain));
}

#[test]
fn abort_variants_parsed() {
    let mut e = MacroEngine::new(true);
    let _ = e.expand_open_code("%macro m; %abort cancel; %mend; %m");
    assert_eq!(e.take_abort_request(), Some(AbortKind::Cancel));
    let _ = e.expand_open_code("%macro m; %abort return 8; %mend; %m");
    assert_eq!(e.take_abort_request(), Some(AbortKind::Return(Some(8))));
    let _ = e.expand_open_code("%macro m; %abort abend; %mend; %m");
    assert_eq!(e.take_abort_request(), Some(AbortKind::Abend(None)));
}

#[test]
fn abort_propagates_to_caller() {
    // `%abort` dans une macro interne stoppe AUSSI l'expansion de l'appelant.
    let out =
        run("%macro inner; III %abort; JJJ %mend; %macro outer; PPP %inner QQQ %mend; %outer RRR");
    assert!(out.contains("PPP") && out.contains("III"), "got: {out}");
    assert!(
        !out.contains("JJJ") && !out.contains("QQQ") && !out.contains("RRR"),
        "got: {out}"
    );
}

#[test]
fn abort_reentrancy_reset_between_segments() {
    // Après drainage, un 2ᵉ programme se comporte à l'identique.
    let mut e = MacroEngine::new(true);
    let out1 = e.expand_open_code("%macro m; AAA %abort; ZZZ %mend; %m");
    let _ = e.take_abort_request();
    let out2 = e.expand_open_code("%macro m2; AAA %abort; ZZZ %mend; %m2");
    assert!(out1.contains("AAA") && !out1.contains("ZZZ"));
    assert!(out2.contains("AAA") && !out2.contains("ZZZ"));
    assert_eq!(e.take_abort_request(), Some(AbortKind::Plain));
}

#[test]
fn sysexec_noted_and_consumed() {
    let out = run("before %sysexec(rm -rf x); after");
    assert!(
        out.contains("%SYSEXEC") && out.contains("not supported in this build"),
        "got: {out}"
    );
    assert!(
        out.contains("before") && out.contains("after"),
        "got: {out}"
    );
}

#[test]
fn sysexec_inner_semicolon_not_a_cutoff() {
    // Le `;` à l'intérieur des parenthèses ne doit pas couper prématurément.
    let out = run("%sysexec(echo a; echo b); tail");
    assert!(out.contains("not supported"), "got: {out}");
    assert!(
        out.contains("tail") && !out.contains("echo b"),
        "got: {out}"
    );
}
