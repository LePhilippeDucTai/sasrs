use super::*;
use crate::datastep::compile;
use crate::parser::StatementStream;
use crate::source::SourceFile;
use crate::value::MissingKind;
use polars::df;
use std::path::PathBuf;

fn session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

fn write_class(session: &Session, table: &str) {
    let df = df!(
        "Age" => [Some(14.0), None, Some(13.0)],
        "Name" => ["Alfred", "Alice", "Barbara"],
    )
    .unwrap();
    let vars = vec![
        VarMeta {
            name: "Age".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
        VarMeta {
            name: "Name".into(),
            ty: VarType::Char,
            length: 7,
            format: None,
            label: None,
        },
    ];
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(table, &SasDataset { df, vars })
        .unwrap();
}

fn run(src: &str, session: &mut Session) -> Result<StepStats> {
    let file = SourceFile::new(src);
    let mut ts = StatementStream::new(&file).unwrap();
    assert!(ts.next().is_kw("data"));
    let ast = crate::parser::datastep::parse_data_step(&mut ts).unwrap();
    let prog = compile(&ast, session)?;
    execute(prog, session)
}

fn read_work(session: &Session, table: &str) -> SasDataset {
    session.libs.get("WORK").unwrap().read(table).unwrap().0
}

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
fn no_input_runs_single_iteration() {
    let mut s = session();
    let stats = run("data out; x = 1; y = 'ab'; run;", &mut s).unwrap();
    assert_eq!(stats.read, vec![]);
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 2)]);
    let ds = read_work(&s, "out");
    assert_eq!(ds.n_obs(), 1);
    assert_eq!(
        ds.df.column("y").unwrap().str().unwrap().get(0),
        Some("ab")
    );
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

#[test]
fn retain_with_init_accumulates_max() {
    let mut s = session();
    write_class(&s, "inp");
    run(
        "data out; set inp; retain maxage 0; if age > maxage then maxage = age; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    let maxage = ds.df.column("maxage").unwrap().f64().unwrap();
    // 14 dès la 1re obs, retenu ensuite (. > 14 faux, 13 > 14 faux).
    assert_eq!(maxage.get(0), Some(14.0));
    assert_eq!(maxage.get(1), Some(14.0));
    assert_eq!(maxage.get(2), Some(14.0));
}

#[test]
fn retain_initial_value_wins_over_sum_zero() {
    let mut s = session();
    write_class(&s, "inp");
    run("data out; set inp; n + 1; retain n 100; run;", &mut s).unwrap();
    let ds = read_work(&s, "out");
    let n = ds.df.column("n").unwrap().f64().unwrap();
    assert_eq!(n.get(0), Some(101.0));
    assert_eq!(n.get(2), Some(103.0));
}

#[test]
fn retain_without_init_keeps_value_across_iterations() {
    let mut s = session();
    write_class(&s, "inp");
    // prev : Name de l'observation précédente ('' à la 1re itération).
    run(
        "data out; set inp; retain prev; output; prev = name; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    let prev = ds.df.column("prev").unwrap().str().unwrap();
    assert_eq!(prev.get(0), Some(""));
    assert_eq!(prev.get(1), Some("Alfred"));
    assert_eq!(prev.get(2), Some("Alice"));
}

#[test]
fn length_truncates_longer_assignment() {
    let mut s = session();
    let stats = run("data out; length c $ 3; c = 'abcdef'; run;", &mut s).unwrap();
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 1)]);
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("c").unwrap().str().unwrap().get(0), Some("abc"));
    assert_eq!(ds.vars[0].length, 3);
}

#[test]
fn retain_char_init_truncated_to_fixed_length() {
    let mut s = session();
    // c figée Char(3) par LENGTH ; l'init RETAIN 'abcdef' est tronquée
    // par le pdv.set normal au moment de poser les valeurs initiales.
    run("data out; length c $ 3; retain c 'abcdef'; run;", &mut s).unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(ds.df.column("c").unwrap().str().unwrap().get(0), Some("abc"));
}

// ── DO itératif / DELETE (M2) ────────────────────────────────────────

/// Lit la valeur f64 de la colonne `col`, ligne 0, de WORK.`table`.
fn num_at(session: &Session, table: &str, col: &str, row: usize) -> Option<f64> {
    read_work(session, table)
        .df
        .column(col)
        .unwrap()
        .f64()
        .unwrap()
        .get(row)
}

#[test]
fn do_to_sums_one_to_ten() {
    let mut s = session();
    run("data out; s = 0; do i = 1 to 10; s = s + i; end; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "s", 0), Some(55.0));
}

#[test]
fn index_is_four_after_do_one_to_three() {
    let mut s = session();
    // Règle SAS célèbre : à la sortie par le test TO, i vaut la
    // PREMIÈRE valeur qui dépasse.
    run("data out; do i = 1 to 3; end; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "i", 0), Some(4.0));
}

#[test]
fn do_negative_by_runs_three_times() {
    let mut s = session();
    run("data out; do i = 3 to 1 by -1; n + 1; end; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(3.0));
    // Sortie par le test TO : i a dépassé vers le bas.
    assert_eq!(num_at(&s, "out", "i", 0), Some(0.0));
}

#[test]
fn do_fractional_by() {
    let mut s = session();
    // 1, 1.5, 2, 2.5, 3 → 5 tours ; i == 3.5 après.
    run("data out; do i = 1 to 3 by 0.5; n + 1; end; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(5.0));
    assert_eq!(num_at(&s, "out", "i", 0), Some(3.5));
}

#[test]
fn do_while_clause_cuts_iteration() {
    let mut s = session();
    // WHILE testé avant chaque tour : coupe à i = 4 (3 tours).
    run(
        "data out; do i = 1 to 10 while(i < 4); n + 1; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(3.0));
    assert_eq!(num_at(&s, "out", "i", 0), Some(4.0));
}

#[test]
fn do_until_runs_at_least_once() {
    let mut s = session();
    run("data out; do until(1); n + 1; end; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(1.0));
}

#[test]
fn do_while_false_runs_zero_times() {
    let mut s = session();
    run("data out; n = 0; do while(0); n + 1; end; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(0.0));
}

#[test]
fn pure_do_while_loops_until_condition_false() {
    let mut s = session();
    run("data out; x = 0; do while(x < 3); x = x + 1; end; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "x", 0), Some(3.0));
}

#[test]
fn delete_filters_missing_age() {
    let mut s = session();
    write_class(&s, "inp");
    // 3 obs dont 1 age missing → 2 obs en sortie.
    let stats = run("data out; set inp; if age = . then delete; run;", &mut s).unwrap();
    assert_eq!(stats.read, vec![("WORK.INP".to_string(), 3)]);
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 2, 2)]);
    let ds = read_work(&s, "out");
    let name = ds.df.column("Name").unwrap().str().unwrap();
    assert_eq!(name.get(0), Some("Alfred"));
    assert_eq!(name.get(1), Some("Barbara"));
}

#[test]
fn delete_inside_do_exits_loop_and_iteration() {
    let mut s = session();
    write_class(&s, "inp");
    // Chaque itération entre dans le DO et DELETE à i = 2 : le Flow
    // NextIter traverse la boucle → aucune obs en sortie, tout est lu.
    let stats = run(
        "data out; set inp; do i = 1 to 10; if i = 2 then delete; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(stats.read, vec![("WORK.INP".to_string(), 3)]);
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 0, 3)]);
}

#[test]
fn nested_do_loops() {
    let mut s = session();
    run(
        "data out; do i = 1 to 3; do j = 1 to 2; n + 1; end; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(6.0));
    assert_eq!(num_at(&s, "out", "i", 0), Some(4.0));
    assert_eq!(num_at(&s, "out", "j", 0), Some(3.0));
}

#[test]
fn do_bounds_are_evaluated_once_at_entry() {
    let mut s = session();
    // n modifié dans le corps : la borne TO reste celle de l'entrée
    // (3) — règle SAS, les bornes sont figées.
    run(
        "data out; n = 3; do i = 1 to n; n = 0; c + 1; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "c", 0), Some(3.0));
    assert_eq!(num_at(&s, "out", "i", 0), Some(4.0));
}

#[test]
fn stop_inside_do_ends_step() {
    let mut s = session();
    write_class(&s, "inp");
    let stats = run(
        "data out; set inp; do i = 1 to 10; stop; end; run;",
        &mut s,
    )
    .unwrap();
    // STOP au premier tour de la première itération : rien d'écrit,
    // une seule ligne lue.
    assert_eq!(stats.read, vec![("WORK.INP".to_string(), 1)]);
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 0, 3)]);
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
fn missing_do_bound_is_runtime_error() {
    let mut s = session();
    // m jamais assignée → missing : erreur de contrôle de boucle
    // (divergence documentée : stoppe l'étape).
    let err = run("data out; do i = 1 to m; end; run;", &mut s)
        .err()
        .unwrap();
    assert_eq!(err.to_string(), "Invalid DO loop control information.");
}

#[test]
fn modifying_index_in_body_affects_loop() {
    let mut s = session();
    // L'index est une variable normale du PDV : i = i + 1 dans le
    // corps saute une valeur sur deux → 5 tours (1,3,5,7,9).
    run(
        "data out; do i = 1 to 10; i = i + 1; n + 1; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(5.0));
    assert_eq!(num_at(&s, "out", "i", 0), Some(11.0));
}

#[test]
fn infinite_do_loop_guard_trips() {
    let mut s = session();
    let err = run("data out; do while(1); end; run;", &mut s)
        .err()
        .unwrap();
    assert_eq!(
        err.to_string(),
        "DO loop exceeded 10000000 iterations; stopping (possible infinite loop)."
    );
}

// ── ARRAY 1-D + indexation (M2, lot 3) ───────────────────────────────

#[test]
fn array_fill_via_do_loop_braces() {
    let mut s = session();
    run(
        "data out; array a{3} x y z; do i = 1 to 3; a{i} = i * 10; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "x", 0), Some(10.0));
    assert_eq!(num_at(&s, "out", "y", 0), Some(20.0));
    assert_eq!(num_at(&s, "out", "z", 0), Some(30.0));
}

#[test]
fn array_paren_form_lvalue_and_rvalue() {
    let mut s = session();
    // Lvalue `a(i) = ...` et rvalue `a(i)` (l'array masque la
    // fonction) ; lecture croisée via t = a(1) + a(3).
    run(
        "data out; array a(3) x y z; do i = 1 to 3; a(i) = i * 10; end; \
         t = a(1) + a(3); run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "x", 0), Some(10.0));
    assert_eq!(num_at(&s, "out", "y", 0), Some(20.0));
    assert_eq!(num_at(&s, "out", "z", 0), Some(30.0));
    assert_eq!(num_at(&s, "out", "t", 0), Some(40.0));
}

#[test]
fn array_sum_via_dim() {
    let mut s = session();
    run(
        "data out; array a{3} x y z; do i = 1 to 3; a{i} = i; end; \
         do i = 1 to dim(a); s + a{i}; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "s", 0), Some(6.0));
}

#[test]
fn array_index_rounds_to_nearest() {
    let mut s = session();
    // 1.4 → 1, 2.6 → 3 (arrondi au plus proche, comme SAS).
    run(
        "data out; array a{3} x y z; a{1.4} = 7; a{2.6} = 9; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "x", 0), Some(7.0));
    assert_eq!(num_at(&s, "out", "z", 0), Some(9.0));
    assert_eq!(num_at(&s, "out", "y", 0), None);
}

#[test]
fn char_array_with_truncation() {
    let mut s = session();
    run(
        "data out; array c{2} $ 3 u v; c{1} = 'abcdef'; c{2} = 'xy'; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    // Longueur fixe 3 : troncature silencieuse à l'assignation.
    assert_eq!(ds.df.column("u").unwrap().str().unwrap().get(0), Some("abc"));
    assert_eq!(ds.df.column("v").unwrap().str().unwrap().get(0), Some("xy"));
    assert_eq!(ds.vars[0].length, 3);
}

#[test]
fn char_array_default_length_is_8() {
    let mut s = session();
    run("data out; array c{1} $ u; c{1} = 'abcdefghij'; run;", &mut s).unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(
        ds.df.column("u").unwrap().str().unwrap().get(0),
        Some("abcdefgh")
    );
    assert_eq!(ds.vars[0].length, 8);
}

// ── M16.2 : arrays multi-dimensionnels, valeurs initiales, DIM/HBOUND/
//    LBOUND, _TEMPORARY_/_NUMERIC_/_CHARACTER_/_ALL_ ─────────────────

#[test]
fn array_2d_creation_and_access_row_major() {
    let mut s = session();
    // 2×3 array sur 6 variables ; remplissage row-major v(i,j) = i*10+j.
    run(
        "data out; array m{2,3} v1-v6; do i = 1 to 2; do j = 1 to 3; \
         m{i,j} = i*10 + j; end; end; run;",
        &mut s,
    )
    .unwrap();
    // Ordre row-major : v1=m(1,1), v2=m(1,2), v3=m(1,3), v4=m(2,1)...
    assert_eq!(num_at(&s, "out", "v1", 0), Some(11.0));
    assert_eq!(num_at(&s, "out", "v2", 0), Some(12.0));
    assert_eq!(num_at(&s, "out", "v3", 0), Some(13.0));
    assert_eq!(num_at(&s, "out", "v4", 0), Some(21.0));
    assert_eq!(num_at(&s, "out", "v5", 0), Some(22.0));
    assert_eq!(num_at(&s, "out", "v6", 0), Some(23.0));
}

#[test]
fn array_3d_creation_and_access() {
    let mut s = session();
    // 2×3×2 = 12 slots, éléments auto-nommés t1..t12.
    run(
        "data out; array t{2,3,2}; \
         t{1,1,1} = 1; t{1,1,2} = 2; t{2,3,2} = 99; \
         a = t{1,1,1}; b = t{1,1,2}; c = t{2,3,2}; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "a", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "b", 0), Some(2.0));
    assert_eq!(num_at(&s, "out", "c", 0), Some(99.0));
    // t1 = (1,1,1) ; t12 = (2,3,2).
    assert_eq!(num_at(&s, "out", "t1", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "t12", 0), Some(99.0));
}

#[test]
fn array_linear_index_on_multidim() {
    let mut s = session();
    // Accès linéaire `m{n}` sur un array 2-D (interprétation row-major).
    run(
        "data out; array m{2,3} v1-v6; do n = 1 to 6; m{n} = n*n; end; \
         a = m{1,1}; f = m{2,3}; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "v1", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "v6", 0), Some(36.0));
    assert_eq!(num_at(&s, "out", "a", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "f", 0), Some(36.0));
}

#[test]
fn array_initial_values_row_major() {
    let mut s = session();
    run(
        "data out; array a{2,2} (1, 2, 3, 4); \
         p = a{1,1}; q = a{1,2}; r = a{2,1}; t = a{2,2}; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "p", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "q", 0), Some(2.0));
    assert_eq!(num_at(&s, "out", "r", 0), Some(3.0));
    assert_eq!(num_at(&s, "out", "t", 0), Some(4.0));
}

#[test]
fn array_initial_values_space_separated_1d() {
    let mut s = session();
    run(
        "data out; array a{3} x y z (10 20 30); run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "x", 0), Some(10.0));
    assert_eq!(num_at(&s, "out", "y", 0), Some(20.0));
    assert_eq!(num_at(&s, "out", "z", 0), Some(30.0));
}

#[test]
fn array_dim_hbound_lbound_functions() {
    let mut s = session();
    run(
        "data out; array m{2,3} v1-v6; \
         nd = dim(m); n1 = dim(m, 1); n2 = dim(m, 2); \
         hb = hbound(m); hb2 = hbound(m, 2); \
         lb = lbound(m); lb2 = lbound(m, 2); run;",
        &mut s,
    )
    .unwrap();
    // dim(m) sans n = 1re dimension = 2 ; dim(m,2) = 3.
    assert_eq!(num_at(&s, "out", "nd", 0), Some(2.0));
    assert_eq!(num_at(&s, "out", "n1", 0), Some(2.0));
    assert_eq!(num_at(&s, "out", "n2", 0), Some(3.0));
    // hbound = borne supérieure (= dim, lbound=1).
    assert_eq!(num_at(&s, "out", "hb", 0), Some(2.0));
    assert_eq!(num_at(&s, "out", "hb2", 0), Some(3.0));
    // lbound toujours 1.
    assert_eq!(num_at(&s, "out", "lb", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "lb2", 0), Some(1.0));
}

#[test]
fn array_dim_on_1d_array() {
    let mut s = session();
    run(
        "data out; array a{5} a1-a5; d = dim(a); h = hbound(a); l = lbound(a); run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "d", 0), Some(5.0));
    assert_eq!(num_at(&s, "out", "h", 0), Some(5.0));
    assert_eq!(num_at(&s, "out", "l", 0), Some(1.0));
}

#[test]
fn array_temporary_elements_not_in_output() {
    let mut s = session();
    run(
        "data out; array t{3} _temporary_ (100 200 300); \
         total = t{1} + t{2} + t{3}; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "total", 0), Some(600.0));
    let ds = read_work(&s, "out");
    // Les éléments temporaires ne sont PAS des colonnes de sortie.
    let cols: Vec<&str> = ds.df.get_column_names().iter().map(|s| s.as_str()).collect();
    assert_eq!(cols, vec!["total"], "temporary elements must not be output");
}

#[test]
fn array_temporary_retained_across_iterations() {
    let mut s = session();
    write_class(&s, "inp");
    // Les éléments _TEMPORARY_ sont retenus : un compteur accumule
    // (valeur initiale 0, puis +1 par itération).
    run(
        "data out; set inp; array acc{1} _temporary_ (0); \
         acc{1} = acc{1} + 1; n = acc{1}; run;",
        &mut s,
    )
    .unwrap();
    // 3 observations → n vaut 1, 2, 3 (retenu, pas remis à missing).
    assert_eq!(num_at(&s, "out", "n", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "n", 1), Some(2.0));
    assert_eq!(num_at(&s, "out", "n", 2), Some(3.0));
}

#[test]
fn array_numeric_special_list() {
    let mut s = session();
    // _NUMERIC_ : toutes les variables numériques déjà connues.
    run(
        "data out; x = 1; y = 2; z = 3; array nums{*} _numeric_; \
         d = dim(nums); s = 0; do i = 1 to dim(nums); s = s + nums{i}; end; run;",
        &mut s,
    )
    .unwrap();
    // x, y, z sont les 3 numériques (i, d, s entrent APRÈS l'ARRAY).
    assert_eq!(num_at(&s, "out", "d", 0), Some(3.0));
    assert_eq!(num_at(&s, "out", "s", 0), Some(6.0));
}

#[test]
fn array_character_special_list() {
    let mut s = session();
    run(
        "data out; a = 'foo'; b = 'bar'; array chs{*} $ _character_; \
         d = dim(chs); chs{1} = 'NEW'; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "d", 0), Some(2.0));
    let ds = read_work(&s, "out");
    // chs{1} pointe sur la 1re variable char (a).
    assert_eq!(ds.df.column("a").unwrap().str().unwrap().get(0), Some("NEW"));
}

#[test]
fn array_mixing_1d_and_multidim() {
    let mut s = session();
    // Une étape avec un array 1-D et un array 2-D coexistants.
    run(
        "data out; array a{3} a1-a3; array m{2,2} m1-m4; \
         do i = 1 to 3; a{i} = i; end; \
         m{1,1} = 9; m{2,2} = 8; \
         da = dim(a); dm = dim(m); dm2 = dim(m,2); run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "a2", 0), Some(2.0));
    assert_eq!(num_at(&s, "out", "m1", 0), Some(9.0));
    assert_eq!(num_at(&s, "out", "m4", 0), Some(8.0));
    assert_eq!(num_at(&s, "out", "da", 0), Some(3.0));
    assert_eq!(num_at(&s, "out", "dm", 0), Some(2.0));
    assert_eq!(num_at(&s, "out", "dm2", 0), Some(2.0));
}

#[test]
fn array_2d_out_of_bounds_stops_step() {
    // Indice de dimension hors bornes : arrêt avec ERROR.
    let out = crate::run(
        "data out; array m{2,3} v1-v6; m{3,1} = 1; run;",
        crate::RunOptions {
            work_dir: None,
            base_dir: None,
            deterministic: true,
            vectorize: false,
        },
    );
    assert_eq!(out.exit_code, 2, "log was:\n{}", out.log);
    assert!(
        out.log.contains("ERROR: Array subscript out of range."),
        "log was:\n{}",
        out.log
    );
}

#[test]
fn array_2d_wrong_index_count_stops_step() {
    // 2 indices attendus, 3 fournis → hors bornes.
    let out = crate::run(
        "data out; array m{2,3} v1-v6; t = m{1,2,1}; run;",
        crate::RunOptions {
            work_dir: None,
            base_dir: None,
            deterministic: true,
            vectorize: false,
        },
    );
    assert_eq!(out.exit_code, 2, "log was:\n{}", out.log);
    assert!(
        out.log.contains("ERROR: Array subscript out of range."),
        "log was:\n{}",
        out.log
    );
}

#[test]
fn array_initial_too_many_values_errors() {
    let mut s = session();
    match run("data out; array a{2} x y (1 2 3); run;", &mut s) {
        Err(e) => assert!(
            e.to_string().contains("Too many initial values"),
            "wrong error message: {e}"
        ),
        Ok(_) => panic!("expected too-many-initial-values error"),
    }
}

#[test]
fn array_dim_count_mismatch_errors() {
    let mut s = session();
    // 2×3 = 6 attendus, 4 variables fournies.
    match run("data out; array m{2,3} a b c d; run;", &mut s) {
        Err(e) => assert!(
            e.to_string().contains("does not match"),
            "wrong error message: {e}"
        ),
        Ok(_) => panic!("expected dimension-mismatch error"),
    }
}

#[test]
fn out_of_range_subscript_stops_step_with_error() {
    // Lvalue hors bornes : l'étape s'arrête avec ERROR (exit code 2).
    let out = crate::run(
        "data out; array a{3} x y z; a{4} = 1; run;",
        crate::RunOptions {
            work_dir: None,
            base_dir: None,
            deterministic: true,
            vectorize: false,
        },
    );
    assert_eq!(out.exit_code, 2, "log was:\n{}", out.log);
    assert!(
        out.log.contains("ERROR: Array subscript out of range."),
        "log was:\n{}",
        out.log
    );
    assert!(out
        .log
        .contains("The SAS System stopped processing this step because of errors."));

    // Rvalue hors bornes (y compris indice 0) : même arrêt.
    let out = crate::run(
        "data out; array a{3} x y z; t = a{0}; run;",
        crate::RunOptions {
            work_dir: None,
            base_dir: None,
            deterministic: true,
            vectorize: false,
        },
    );
    assert_eq!(out.exit_code, 2, "log was:\n{}", out.log);
    assert!(
        out.log.contains("ERROR: Array subscript out of range."),
        "log was:\n{}",
        out.log
    );
}

#[test]
fn missing_subscript_stops_step_with_error() {
    let out = crate::run(
        "data out; array a{3} x y z; i = .; a{i} = 1; run;",
        crate::RunOptions {
            work_dir: None,
            base_dir: None,
            deterministic: true,
            vectorize: false,
        },
    );
    assert_eq!(out.exit_code, 2, "log was:\n{}", out.log);
    assert!(
        out.log.contains("ERROR: Array subscript out of range."),
        "log was:\n{}",
        out.log
    );
}

#[test]
fn auto_named_elements_are_usable_as_variables() {
    let mut s = session();
    // a1 a2 a3 auto-nommés : adressables par indice ET par nom.
    run(
        "data out; array a{3}; do i = 1 to 3; a{i} = i; end; t = a1 + a2 + a3; \
         a2 = 20; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "a1", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "a2", 0), Some(20.0));
    assert_eq!(num_at(&s, "out", "a3", 0), Some(3.0));
    assert_eq!(num_at(&s, "out", "t", 0), Some(6.0));
}

#[test]
fn array_over_input_variables_updates_them() {
    let mut s = session();
    write_class(&s, "inp");
    // Array sur une variable d'input : l'élément référence le slot
    // existant (type/longueur de l'input conservés).
    run(
        "data out; set inp; array nums{1} age; nums{1} = nums{1} * 2; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    let age = ds.df.column("Age").unwrap().f64().unwrap();
    assert_eq!(age.get(0), Some(28.0));
    assert_eq!(age.get(2), Some(26.0));
}

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

// ── Options de dataset + OUTPUT ciblé (M2, lot 4) ────────────────────

/// Mini-CLASS à trois variables (Name char, Sex char, Age num) pour les
/// tests d'options de dataset et de sorties multiples.
fn write_class_full(session: &Session, table: &str) {
    let df = df!(
        "Name" => ["Alfred", "Alice", "Barbara", "Henry"],
        "Sex" => ["M", "F", "F", "M"],
        "Age" => [Some(14.0), None, Some(13.0), Some(15.0)],
    )
    .unwrap();
    let vars = vec![
        VarMeta {
            name: "Name".into(),
            ty: VarType::Char,
            length: 7,
            format: None,
            label: None,
        },
        VarMeta {
            name: "Sex".into(),
            ty: VarType::Char,
            length: 1,
            format: None,
            label: None,
        },
        VarMeta {
            name: "Age".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
    ];
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(table, &SasDataset { df, vars })
        .unwrap();
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
        vec![
            ("WORK.M".to_string(), 2, 3),
            ("WORK.F".to_string(), 2, 3),
        ]
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

// ── SET multi-datasets + BY + FIRST./LAST. (M3) ──────────────────────

/// Écrit un dataset entièrement numérique : colonnes (nom, valeurs).
fn write_num_ds(session: &Session, table: &str, cols: &[(&str, Vec<Option<f64>>)]) {
    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();
    for (name, vals) in cols {
        columns.push(Series::new((*name).into(), vals.clone()).into());
        vars.push(VarMeta {
            name: (*name).to_string(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        });
    }
    let df = DataFrame::new(columns).unwrap();
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(table, &SasDataset { df, vars })
        .unwrap();
}

fn some(vals: &[f64]) -> Vec<Option<f64>> {
    vals.iter().copied().map(Some).collect()
}

/// Colonne f64 complète de WORK.`table`.
fn col(session: &Session, table: &str, col: &str) -> Vec<Option<f64>> {
    read_work(session, table)
        .df
        .column(col)
        .unwrap()
        .f64()
        .unwrap()
        .iter()
        .collect()
}

#[test]
fn set_two_datasets_without_by_concatenates() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 3.0, 5.0]))]);
    write_num_ds(&s, "b", &[("x", some(&[2.0, 3.0, 4.0]))]);
    let stats = run("data out; set a b; run;", &mut s).unwrap();
    // Tout a, puis tout b.
    assert_eq!(
        col(&s, "out", "x"),
        some(&[1.0, 3.0, 5.0, 2.0, 3.0, 4.0])
    );
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
        &[("x", some(&[1.0, 3.0, 5.0])), ("u", some(&[10.0, 30.0, 50.0]))],
    );
    write_num_ds(
        &s,
        "b",
        &[("x", some(&[2.0, 3.0, 4.0])), ("v", some(&[200.0, 300.0, 400.0]))],
    );
    let stats = run(
        "data out; set a b; by x; f = first.x; l = last.x; run;",
        &mut s,
    )
    .unwrap();
    // Interclassement par x croissant ; égalité (x=3) → a (premier du
    // SET) avant b.
    assert_eq!(
        col(&s, "out", "x"),
        some(&[1.0, 2.0, 3.0, 3.0, 4.0, 5.0])
    );
    // u/v : RETAIN implicite des variables de SET — une variable
    // absente du dataset de l'obs courante GARDE sa valeur précédente
    // (et reste missing avant sa première lecture).
    assert_eq!(
        col(&s, "out", "u"),
        vec![Some(10.0), Some(10.0), Some(30.0), Some(30.0), Some(30.0), Some(50.0)]
    );
    assert_eq!(
        col(&s, "out", "v"),
        vec![None, Some(200.0), Some(200.0), Some(300.0), Some(400.0), Some(400.0)]
    );
    // FIRST.x / LAST.x : le groupe x=3 a deux obs ; LAST. de la
    // dernière obs globale vaut 1.
    assert_eq!(
        col(&s, "out", "f"),
        some(&[1.0, 1.0, 1.0, 0.0, 1.0, 1.0])
    );
    assert_eq!(
        col(&s, "out", "l"),
        some(&[1.0, 1.0, 0.0, 1.0, 1.0, 1.0])
    );
    assert_eq!(
        stats.read,
        vec![("WORK.A".to_string(), 3), ("WORK.B".to_string(), 3)]
    );
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
        &[
            ("a", some(&[1.0, 1.0, 2.0])),
            ("b", some(&[7.0, 8.0, 8.0])),
        ],
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
fn missing_by_key_collates_first() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", vec![None, Some(2.0)])]);
    write_num_ds(&s, "b", &[("x", some(&[1.0]))]);
    run("data out; set a b; by x; f = first.x; run;", &mut s).unwrap();
    // `.` < 1 < 2 (les missings se collationnent en premier).
    assert_eq!(col(&s, "out", "x"), vec![None, Some(1.0), Some(2.0)]);
    assert_eq!(col(&s, "out", "f"), some(&[1.0, 1.0, 1.0]));
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

// ── MERGE avec BY : match-merge SAS (M3-3) ───────────────────────────
//
// Plusieurs sorties sont COMPARÉES À UNE SORTIE SAS CALCULÉE À LA MAIN
// (indiqué dans le commentaire de chaque test).

/// Colonne char complète de WORK.`table`.
fn col_str(session: &Session, table: &str, col: &str) -> Vec<Option<String>> {
    read_work(session, table)
        .df
        .column(col)
        .unwrap()
        .str()
        .unwrap()
        .iter()
        .map(|o| o.map(str::to_string))
        .collect()
}

#[test]
fn merge_one_to_one() {
    // Sortie SAS calculée à la main : a={(1,x=10),(2,x=20)},
    // b={(1,y=100),(2,y=200)} ; merge a b; by id; → (1,10,100),(2,20,200).
    let mut s = session();
    write_num_ds(&s, "a", &[("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))]);
    write_num_ds(&s, "b", &[("id", some(&[1.0, 2.0])), ("y", some(&[100.0, 200.0]))]);
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
    write_num_ds(&s, "a", &[("id", some(&[1.0, 1.0])), ("x", some(&[10.0, 20.0]))]);
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
    write_num_ds(&s, "a", &[("id", some(&[1.0, 3.0])), ("x", some(&[10.0, 30.0]))]);
    write_num_ds(&s, "b", &[("id", some(&[2.0, 3.0])), ("y", some(&[20.0, 33.0]))]);
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
    write_num_ds(&s, "a", &[("id", some(&[1.0, 3.0])), ("x", some(&[10.0, 30.0]))]);
    write_num_ds(&s, "b", &[("id", some(&[2.0, 3.0])), ("y", some(&[20.0, 33.0]))]);
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
        VarMeta { name: "id".into(), ty: VarType::Num, length: 8, format: None, label: None },
        VarMeta { name: "v".into(), ty: VarType::Char, length: 8, format: None, label: None },
    ];
    s.libs.get("WORK").unwrap().write("a", &SasDataset { df: df_a, vars: vars.clone() }).unwrap();
    let id_b = Series::new("id".into(), &[Some(1.0)]);
    let v_b = Series::new("v".into(), &["B"]);
    let df_b = DataFrame::new(vec![id_b.into(), v_b.into()]).unwrap();
    s.libs.get("WORK").unwrap().write("b", &SasDataset { df: df_b, vars }).unwrap();
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
    write_num_ds(&s, "a", &[("id", some(&[1.0, 1.0, 2.0])), ("x", some(&[10.0, 11.0, 20.0]))]);
    write_num_ds(&s, "b", &[("id", some(&[1.0, 2.0])), ("y", some(&[100.0, 200.0]))]);
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
    write_num_ds(&s, "a", &[("id", some(&[2.0, 1.0])), ("x", some(&[1.0, 2.0]))]);
    write_num_ds(&s, "b", &[("id", some(&[1.0, 2.0])), ("y", some(&[1.0, 2.0]))]);
    let err = run("data out; merge a b; by id; run;", &mut s).err().unwrap();
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
    let err = run("data out; set a; merge a b; by id; run;", &mut s).err().unwrap();
    assert!(err.to_string().contains("not allowed"), "got: {err}");
}

#[test]
fn set_in_option_is_error() {
    let mut s = session();
    write_num_ds(&s, "a", &[("id", some(&[1.0]))]);
    let err = run("data out; set a(in=ina); run;", &mut s).err().unwrap();
    assert!(err.to_string().contains("IN="), "got: {err}");
}

#[test]
fn output_in_option_is_error() {
    let mut s = session();
    write_num_ds(&s, "a", &[("id", some(&[1.0]))]);
    let err = run("data out(in=foo); set a; run;", &mut s).err().unwrap();
    assert!(err.to_string().to_uppercase().contains("IN"), "got: {err}");
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
    assert_eq!(num_to_value(at("x")), Value::Missing(MissingKind::Letter(0)));
    assert_eq!(num_to_value(at("y")), Value::Missing(MissingKind::Underscore));
    assert_eq!(num_to_value(at("z")), Value::missing());
    // x ≠ y ≠ z par sas_cmp (ordre total : ._ < . < .a).
    let (x, y, z) = (num_to_value(at("x")), num_to_value(at("y")), num_to_value(at("z")));
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
fn missing_over_zero_does_not_emit_division_note() {
    let mut s = session();
    run("data out; m = .; r = m / 0; e = _error_; run;", &mut s).unwrap();
    // missing/0 : propagation missing, PAS une division par zéro.
    assert_eq!(num_at(&s, "out", "e", 0), Some(0.0));
    assert_eq!(num_at(&s, "out", "r", 0), None);
    let log = s.log.into_string();
    assert!(!log.contains("Division by zero"), "log was: {log}");
    assert!(log.contains("Missing values were generated"), "log was: {log}");
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

// ── FILE / PUT (M14.2) ───────────────────────────────────────────────

/// Extrait les lignes PUT du log (celles qui ne sont ni vides, ni un
/// écho de source numéroté, ni une NOTE/WARNING/ERROR).
fn put_log_lines(log: &str) -> Vec<String> {
    // L'écho de source SAS est de la forme "<num>     <texte>" : un nombre
    // suivi d'AU MOINS deux espaces (padding à la colonne 6) puis du texte.
    // Une ligne PUT purement numérique ("42") n'a pas ce padding.
    fn is_source_echo(l: &str) -> bool {
        let mut it = l.char_indices();
        let mut end = 0;
        for (i, c) in it.by_ref() {
            if c.is_ascii_digit() {
                end = i + 1;
            } else {
                break;
            }
        }
        if end == 0 {
            return false;
        }
        // Au moins deux espaces après le nombre.
        l[end..].starts_with("  ")
    }
    log.lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.is_empty()
                && !t.starts_with("NOTE:")
                && !t.starts_with("WARNING:")
                && !t.starts_with("ERROR:")
                && !is_source_echo(l)
                // Les continuations de NOTE timing ("real time...").
                && !t.starts_with("real time")
                && !t.starts_with("cpu time")
        })
        .map(|l| l.to_string())
        .collect()
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

// =====================================================================
// M15.6 — CALL routines
// =====================================================================

fn num_col(ds: &SasDataset, name: &str) -> Vec<Option<f64>> {
    let c = ds.df.column(name).unwrap().f64().unwrap();
    (0..ds.n_obs()).map(|i| c.get(i)).collect()
}
fn str_col(ds: &SasDataset, name: &str) -> Vec<String> {
    let c = ds.df.column(name).unwrap().str().unwrap();
    (0..ds.n_obs()).map(|i| c.get(i).unwrap_or("").to_string()).collect()
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

// ── SELECT / WHEN / OTHERWISE (M16.1) ────────────────────────────────

#[test]
fn select_selector_form_matches_first_value() {
    // Sélecteur numérique : age 14 → "teen", . → autre, 13 → "kid".
    let mut s = session();
    write_class(&s, "inp");
    run(
        "data out; set inp; length grp $8; \
         select (age); \
           when (13) grp='kid'; \
           when (14, 15) grp='teen'; \
           otherwise grp='other'; \
         end; \
         run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    // age = 14, missing, 13.
    assert_eq!(
        str_col(&ds, "grp"),
        vec!["teen".to_string(), "other".to_string(), "kid".to_string()]
    );
}

#[test]
fn select_selector_multiple_values_in_one_when() {
    // Une seule clause liste plusieurs valeurs ; n'importe laquelle suffit.
    let mut s = session();
    run(
        "data out; \
         do x = 1 to 4; \
           select (x); \
             when (1, 3) flag = 1; \
             otherwise flag = 0; \
           end; \
           output; \
         end; \
         run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "flag", 0), Some(1.0)); // x=1
    assert_eq!(num_at(&s, "out", "flag", 1), Some(0.0)); // x=2
    assert_eq!(num_at(&s, "out", "flag", 2), Some(1.0)); // x=3
    assert_eq!(num_at(&s, "out", "flag", 3), Some(0.0)); // x=4
}

#[test]
fn select_selector_char_form() {
    // Sélecteur caractère ; comparaison ignore les blancs finaux (sas_cmp).
    let mut s = session();
    run(
        "data out; length sex $1 desc $8; \
         sex = 'F'; \
         select (sex); \
           when ('M') desc = 'male'; \
           when ('F') desc = 'female'; \
           otherwise desc = 'unknown'; \
         end; \
         output; \
         run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "desc"), vec!["female".to_string()]);
}

#[test]
fn select_boolean_form_first_true_wins() {
    // Forme booléenne : conditions évaluées dans l'ordre, première vraie.
    let mut s = session();
    run(
        "data out; length band $8; \
         do x = 5 to 25 by 10; \
           select; \
             when (x < 10) band = 'low'; \
             when (x < 20) band = 'mid'; \
             otherwise band = 'high'; \
           end; \
           output; \
         end; \
         run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    // x = 5, 15, 25.
    assert_eq!(
        str_col(&ds, "band"),
        vec!["low".to_string(), "mid".to_string(), "high".to_string()]
    );
}

#[test]
fn select_boolean_form_range_condition() {
    // Plage exprimée par une condition booléenne 1 <= x <= 10.
    let mut s = session();
    run(
        "data out; length r $8; \
         do x = 0 to 15 by 5; \
           select; \
             when (x >= 1 and x <= 10) r = 'in'; \
             otherwise r = 'out'; \
           end; \
           output; \
         end; \
         run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    // x = 0(out), 5(in), 10(in), 15(out).
    assert_eq!(
        str_col(&ds, "r"),
        vec![
            "out".to_string(),
            "in".to_string(),
            "in".to_string(),
            "out".to_string()
        ]
    );
}

#[test]
fn select_when_do_block_runs_all_statements() {
    // Le corps d'un WHEN peut être un do; ... end; (plusieurs statements).
    let mut s = session();
    run(
        "data out; x = 2; \
         select (x); \
           when (2) do; a = 10; b = 20; end; \
           otherwise do; a = 0; b = 0; end; \
         end; \
         output; \
         run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "a", 0), Some(10.0));
    assert_eq!(num_at(&s, "out", "b", 0), Some(20.0));
}

#[test]
fn select_no_fall_through() {
    // Pas de fall-through : seule la PREMIÈRE clause vraie s'exécute,
    // même si une clause suivante correspondrait aussi.
    let mut s = session();
    run(
        "data out; x = 1; n = 0; \
         select (x); \
           when (1) n = n + 1; \
           when (1) n = n + 100; \
           otherwise n = n + 1000; \
         end; \
         output; \
         run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(1.0));
}

#[test]
fn select_missing_value_matches_dot() {
    // `. = .` est vrai en SAS : un WHEN (.) capture le sélecteur missing.
    let mut s = session();
    write_class(&s, "inp");
    run(
        "data out; set inp; length tag $8; \
         select (age); \
           when (.) tag = 'na'; \
           otherwise tag = 'ok'; \
         end; \
         run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    // age = 14, missing, 13.
    assert_eq!(
        str_col(&ds, "tag"),
        vec!["ok".to_string(), "na".to_string(), "ok".to_string()]
    );
}

#[test]
fn select_no_otherwise_no_match_is_runtime_error() {
    // Sans OTHERWISE et sans WHEN correspondant : erreur runtime (SAS).
    let mut s = session();
    let err = run(
        "data out; x = 99; select (x); when (1) y = 1; end; run;",
        &mut s,
    )
    .err()
    .unwrap();
    assert!(
        err.to_string().contains("does not match any clause"),
        "got: {err}"
    );
}

#[test]
fn select_no_otherwise_with_match_is_ok() {
    // Sans OTHERWISE mais avec un WHEN correspondant : pas d'erreur.
    let mut s = session();
    run(
        "data out; x = 1; select (x); when (1) y = 7; end; output; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "y", 0), Some(7.0));
}

#[test]
fn select_empty_when_body_is_noop() {
    // `when (1) ;` corps vide : la clause est prise mais ne fait rien
    // (pas de fall-through vers OTHERWISE).
    let mut s = session();
    run(
        "data out; x = 1; y = 5; \
         select (x); \
           when (1) ; \
           otherwise y = 0; \
         end; \
         output; \
         run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "y", 0), Some(5.0));
}

#[test]
fn select_selector_evaluated_once_via_subsetting() {
    // Le sélecteur est une expression : 2*x. x=3 → 6 → "six".
    let mut s = session();
    run(
        "data out; length w $8; x = 3; \
         select (2 * x); \
           when (6) w = 'six'; \
           otherwise w = 'no'; \
         end; \
         output; \
         run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "w"), vec!["six".to_string()]);
}

// ── M16.3 : DO sur liste de valeurs, DO OVER, RETAIN littéraux date ───

#[test]
fn do_list_numeric_explicit_values() {
    // `do i = 1, 3, 5, 7;` — somme et dernière valeur.
    let mut s = session();
    run(
        "data out; s = 0; do i = 1, 3, 5, 7; s = s + i; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "s", 0), Some(16.0));
    // À la sortie d'une liste, l'index garde la DERNIÈRE valeur (≠ TO).
    assert_eq!(num_at(&s, "out", "i", 0), Some(7.0));
}

#[test]
fn do_list_unordered_values() {
    // Ordre quelconque honoré tel quel : 5, 1, 9.
    let mut s = session();
    run(
        "data out; n = 0; do i = 5, 1, 9; n + 1; last = i; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(3.0));
    assert_eq!(num_at(&s, "out", "last", 0), Some(9.0));
}

#[test]
fn do_list_single_value() {
    // `do i = 42;` — liste à un élément (boucle une fois).
    let mut s = session();
    run("data out; c = 0; do i = 42; c + 1; end; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "c", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "i", 0), Some(42.0));
}

#[test]
fn do_list_character_values() {
    // `do color = 'red', 'blue', 'green';` — char.
    let mut s = session();
    run(
        "data out; length color $5; n = 0; \
         do color = 'red', 'blue', 'green'; n + 1; end; run;",
        &mut s,
    )
    .unwrap();
    // Dernière valeur conservée ; n = 3.
    assert_eq!(num_at(&s, "out", "n", 0), Some(3.0));
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "color"), vec!["green".to_string()]);
}

#[test]
fn do_list_mixed_range_and_explicit() {
    // `do i = 1 to 5 by 2, 10, 20 to 22;` → 1,3,5,10,20,21,22 (7 valeurs).
    let mut s = session();
    run(
        "data out; n = 0; s = 0; \
         do i = 1 to 5 by 2, 10, 20 to 22; n + 1; s = s + i; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(7.0));
    // 1+3+5+10+20+21+22 = 82.
    assert_eq!(num_at(&s, "out", "s", 0), Some(82.0));
    // Dernière valeur = 22.
    assert_eq!(num_at(&s, "out", "i", 0), Some(22.0));
}

#[test]
fn do_list_range_first_then_values() {
    // `1 to 12 by 2, 0` : c'est une LISTE (à cause de la virgule) → le
    // range énumère 1,3,5,7,9,11 puis la valeur 0 ; 7 tours, index final 0.
    let mut s = session();
    run(
        "data out; n = 0; do month = 1 to 12 by 2, 0; n + 1; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "n", 0), Some(7.0));
    // En LISTE, l'index garde la dernière valeur (≠ TO classique).
    assert_eq!(num_at(&s, "out", "month", 0), Some(0.0));
}

#[test]
fn do_over_1d_iterates_all_elements() {
    // DO OVER 1-D : `arr` nu = élément courant ; on double chaque élément.
    let mut s = session();
    run(
        "data out; array a{5} v1-v5; \
         do i = 1 to 5; a{i} = i; end; \
         do over a; a = a * 10; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "v1", 0), Some(10.0));
    assert_eq!(num_at(&s, "out", "v2", 0), Some(20.0));
    assert_eq!(num_at(&s, "out", "v3", 0), Some(30.0));
    assert_eq!(num_at(&s, "out", "v4", 0), Some(40.0));
    assert_eq!(num_at(&s, "out", "v5", 0), Some(50.0));
}

#[test]
fn do_over_1d_reads_current_element_into_accumulator() {
    // `arr` en lecture nue dans une accumulation.
    let mut s = session();
    run(
        "data out; array a{4} v1-v4 (3 6 9 12); \
         tot = 0; do over a; tot = tot + a; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "tot", 0), Some(30.0));
}

#[test]
fn do_over_static_indexed_access_inside_loop() {
    // Accès indexé `a{1}` reste STATIQUE même dans DO OVER : on lit le
    // premier élément à chaque tour.
    let mut s = session();
    run(
        "data out; array a{3} v1-v3 (5 6 7); \
         firstsum = 0; do over a; firstsum = firstsum + a{1}; end; run;",
        &mut s,
    )
    .unwrap();
    // a{1}=5 lu 3 fois → 15.
    assert_eq!(num_at(&s, "out", "firstsum", 0), Some(15.0));
}

#[test]
fn do_over_multidim_row_major_order() {
    // DO OVER sur un array 2×3 : itération row-major (= ordre des slots).
    // On affecte des valeurs croissantes par tour pour vérifier l'ordre.
    let mut s = session();
    run(
        "data out; array m{2,3} v1-v6; \
         k = 0; do over m; k + 1; m = k; end; run;",
        &mut s,
    )
    .unwrap();
    // Row-major : v1=1, v2=2, ..., v6=6.
    assert_eq!(num_at(&s, "out", "v1", 0), Some(1.0));
    assert_eq!(num_at(&s, "out", "v2", 0), Some(2.0));
    assert_eq!(num_at(&s, "out", "v3", 0), Some(3.0));
    assert_eq!(num_at(&s, "out", "v4", 0), Some(4.0));
    assert_eq!(num_at(&s, "out", "v5", 0), Some(5.0));
    assert_eq!(num_at(&s, "out", "v6", 0), Some(6.0));
}

#[test]
fn do_over_char_array() {
    // DO OVER sur array caractère : uppercase de chaque élément.
    let mut s = session();
    run(
        "data out; array c{3} $3 a b cc; \
         a = 'foo'; b = 'bar'; cc = 'baz'; \
         do over c; c = upcase(c); end; run;",
        &mut s,
    )
    .unwrap();
    let ds = read_work(&s, "out");
    assert_eq!(str_col(&ds, "a"), vec!["FOO".to_string()]);
    assert_eq!(str_col(&ds, "b"), vec!["BAR".to_string()]);
    assert_eq!(str_col(&ds, "cc"), vec!["BAZ".to_string()]);
}

#[test]
fn retain_date_literal_bare_suffix() {
    // `retain d 21710d;` — 21710 est la valeur SAS date (2019-06-14).
    let mut s = session();
    run("data out; retain d 21710d; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "d", 0), Some(21710.0));
}

#[test]
fn retain_date_literal_quoted() {
    // `retain d '01JAN1960'd;` — l'époque SAS = 0.
    let mut s = session();
    run("data out; retain d '01JAN1960'd; run;", &mut s).unwrap();
    assert_eq!(num_at(&s, "out", "d", 0), Some(0.0));
    // '02JAN1960'd = 1.
    let mut s2 = session();
    run("data out; retain e '02JAN1960'd; run;", &mut s2).unwrap();
    assert_eq!(num_at(&s2, "out", "e", 0), Some(1.0));
}

#[test]
fn retain_datetime_literal() {
    // `retain dt '01JAN1960 00:00:00'dt;` = 0 secondes depuis l'époque.
    let mut s = session();
    run(
        "data out; retain dt '01JAN1960 00:00:00'dt; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "dt", 0), Some(0.0));
    // '01JAN1960 00:01:00'dt = 60 secondes.
    let mut s2 = session();
    run(
        "data out; retain dt '01JAN1960 00:01:00'dt; run;",
        &mut s2,
    )
    .unwrap();
    assert_eq!(num_at(&s2, "out", "dt", 0), Some(60.0));
}

#[test]
fn retain_date_literal_is_retained_across_iterations() {
    // La valeur initiale issue d'un littéral date est bien RETENUE :
    // on l'incrémente à chaque obs lue.
    let mut s = session();
    write_class(&s, "inp");
    run(
        "data out; set inp; retain d 100d; d = d + 1; run;",
        &mut s,
    )
    .unwrap();
    // 100 (initial) +1 par obs : 101, 102, 103.
    assert_eq!(num_at(&s, "out", "d", 0), Some(101.0));
    assert_eq!(num_at(&s, "out", "d", 1), Some(102.0));
    assert_eq!(num_at(&s, "out", "d", 2), Some(103.0));
}

#[test]
fn do_over_then_index_value_independent() {
    // Intégration M16.2 : DO OVER puis accès indexé hors boucle.
    let mut s = session();
    run(
        "data out; array a{3} x y z (1 2 3); \
         do over a; a = a + 100; end; \
         p = a{2}; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(num_at(&s, "out", "x", 0), Some(101.0));
    assert_eq!(num_at(&s, "out", "y", 0), Some(102.0));
    assert_eq!(num_at(&s, "out", "z", 0), Some(103.0));
    assert_eq!(num_at(&s, "out", "p", 0), Some(102.0));
}

// ── M16.4 : SET options END= / NOBS= / POINT= + multi-datasets ────────

fn run_err(src: &str, session: &mut Session) -> String {
    let file = SourceFile::new(src);
    let mut ts = StatementStream::new(&file).unwrap();
    assert!(ts.next().is_kw("data"));
    let ast = crate::parser::datastep::parse_data_step(&mut ts).unwrap();
    match compile(&ast, session).and_then(|p| execute(p, session)) {
        Ok(_) => panic!("expected an error, got Ok"),
        Err(e) => e.to_string(),
    }
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

/// END= sur un seul dataset : 0 sauf la dernière obs.
#[test]
fn end_option_single_dataset() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[10.0, 20.0, 30.0]))]);
    run("data out; set a end=eof; flag = eof; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "flag"), some(&[0.0, 0.0, 1.0]));
    // eof n'est PAS écrite en sortie (variable automatique).
    assert!(read_work(&s, "out").df.column("eof").is_err());
}

/// END= permet une logique « dernière observation » (totaux).
#[test]
fn end_option_drives_last_obs_logic() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0, 3.0, 4.0]))]);
    run(
        "data out; set a end=eof; retain total 0; total + x; \
         if eof then output; run;",
        &mut s,
    )
    .unwrap();
    // Une seule obs sortie : le total final.
    assert_eq!(read_work(&s, "out").n_obs(), 1);
    assert_eq!(num_at(&s, "out", "total", 0), Some(10.0));
}

/// END= avec plusieurs datasets : 1 seulement après la dernière obs du
/// DERNIER dataset.
#[test]
fn end_option_multiple_datasets() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0]))]);
    write_num_ds(&s, "b", &[("x", some(&[3.0]))]);
    run("data out; set a b end=eof; flag = eof; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "flag"), some(&[0.0, 0.0, 1.0]));
}

/// END= avec WHERE= : la dernière obs RETENUE porte eof=1.
#[test]
fn end_option_with_where() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0, 3.0, 4.0]))]);
    run(
        "data out; set a(where=(x <= 2)) end=eof; flag = eof; run;",
        &mut s,
    )
    .unwrap();
    // Seules x=1 et x=2 passent ; eof=1 sur x=2.
    assert_eq!(col(&s, "out", "x"), some(&[1.0, 2.0]));
    assert_eq!(col(&s, "out", "flag"), some(&[0.0, 1.0]));
}

/// END= avec BY (interclassement) : 1 sur la toute dernière obs servie.
#[test]
fn end_option_with_by() {
    let mut s = session();
    write_num_ds(&s, "a", &[("k", some(&[1.0, 3.0]))]);
    write_num_ds(&s, "b", &[("k", some(&[2.0, 4.0]))]);
    run(
        "data out; set a b end=eof; by k; flag = eof; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "k"), some(&[1.0, 2.0, 3.0, 4.0]));
    assert_eq!(col(&s, "out", "flag"), some(&[0.0, 0.0, 0.0, 1.0]));
}

/// NOBS= : disponible AVANT la boucle (somme d'observations).
#[test]
fn nobs_option_available_before_loop() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[10.0, 20.0, 30.0]))]);
    run("data out; set a nobs=n; cnt = n; run;", &mut s).unwrap();
    // n est constant = 3 pour chaque obs.
    assert_eq!(col(&s, "out", "cnt"), some(&[3.0, 3.0, 3.0]));
    assert_eq!(col(&s, "out", "n"), some(&[3.0, 3.0, 3.0]));
}

/// NOBS= total sur plusieurs datasets.
#[test]
fn nobs_option_total_across_datasets() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0, 2.0]))]);
    write_num_ds(&s, "b", &[("x", some(&[3.0, 4.0, 5.0]))]);
    run("data out; set a b nobs=n; cnt = n; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "cnt"), some(&[5.0, 5.0, 5.0, 5.0, 5.0]));
}

/// NOBS= utilisable pour une initialisation AVANT toute lecture (le test
/// le plus parlant : un `_N_ = 1` avec `if _n_ = 1` initialise un tableau
/// dimensionné par n). Ici, on vérifie juste l'accès dès la 1re itération.
#[test]
fn nobs_usable_for_initialization() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[7.0, 8.0]))]);
    run(
        "data out; set a nobs=n; if _n_ = 1 then half = n / 2; \
         retain half; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "half"), some(&[1.0, 1.0]));
}

/// POINT= : accès direct via une boucle DO 1..NOBS + OUTPUT explicite.
#[test]
fn point_option_direct_access_loop() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0, 33.0]))]);
    run(
        "data out; do i = 1 to n; set a point=i nobs=n; output; end; stop; run;",
        &mut s,
    )
    .unwrap();
    // Toutes les obs, dans l'ordre de l'index.
    assert_eq!(col(&s, "out", "x"), some(&[11.0, 22.0, 33.0]));
}

/// POINT= : lecture inverse (index décroissant).
#[test]
fn point_option_reverse_order() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0, 33.0]))]);
    run(
        "data out; do i = n to 1 by -1; set a point=i nobs=n; output; end; stop; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "x"), some(&[33.0, 22.0, 11.0]));
}

/// POINT= : accès à UNE obs précise (1-based).
#[test]
fn point_option_single_index() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0, 33.0]))]);
    run(
        "data out; p = 2; set a point=p; output; stop; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(read_work(&s, "out").n_obs(), 1);
    assert_eq!(num_at(&s, "out", "x", 0), Some(22.0));
}

/// POINT= désactive l'output implicite : sans OUTPUT, rien n'est écrit.
#[test]
fn point_option_disables_implicit_output() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0]))]);
    // OUTPUT absent → 0 obs écrite (et STOP évite la boucle infinie).
    run("data out; p = 1; set a point=p; stop; run;", &mut s).unwrap();
    assert_eq!(read_work(&s, "out").n_obs(), 0);
}

/// POINT= index missing → erreur runtime « Error in variable ».
#[test]
fn point_option_missing_index_errors() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0]))]);
    // p jamais affecté → missing.
    let e = run_err("data out; set a point=p; output; stop; run;", &mut s);
    assert!(e.contains("Error in variable"), "got: {e}");
}

/// POINT= index hors bornes (0) → erreur runtime.
#[test]
fn point_option_zero_index_errors() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0]))]);
    let e = run_err("data out; p = 0; set a point=p; output; stop; run;", &mut s);
    assert!(e.contains("Error in variable"), "got: {e}");
}

/// POINT= index trop grand → erreur runtime.
#[test]
fn point_option_out_of_bounds_errors() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0]))]);
    let e = run_err("data out; p = 99; set a point=p; output; stop; run;", &mut s);
    assert!(e.contains("Error in variable"), "got: {e}");
}

/// POINT= avec plusieurs datasets : index GLOBAL sur la concaténation.
#[test]
fn point_option_multiple_datasets_global_index() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0]))]);
    write_num_ds(&s, "b", &[("x", some(&[33.0, 44.0]))]);
    run(
        "data out; do i = 1 to n; set a b point=i nobs=n; output; end; stop; run;",
        &mut s,
    )
    .unwrap();
    // n = 4 (total), index 1..4 parcourt a puis b.
    assert_eq!(col(&s, "out", "x"), some(&[11.0, 22.0, 33.0, 44.0]));
}

/// POINT= incompatible avec BY → erreur de compilation/exécution.
#[test]
fn point_option_with_by_errors() {
    let mut s = session();
    write_num_ds(&s, "a", &[("k", some(&[1.0, 2.0]))]);
    let e = run_err(
        "data out; set a point=p; by k; output; stop; run;",
        &mut s,
    );
    assert!(e.contains("POINT="), "got: {e}");
}

/// POINT= + END= : eof=1 quand l'index pointe la dernière obs.
#[test]
fn point_option_with_end() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0, 33.0]))]);
    run(
        "data out; do i = 1 to n; set a point=i nobs=n end=eof; \
         flag = eof; output; end; stop; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "flag"), some(&[0.0, 0.0, 1.0]));
}

/// POINT= : re-lecture de la même obs (contrôle d'itération manuel).
#[test]
fn point_option_reread_same_obs() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[11.0, 22.0, 33.0]))]);
    run(
        "data out; do i = 1, 1, 3; set a point=i; output; end; stop; run;",
        &mut s,
    )
    .unwrap();
    // Index 1, 1, 3 → re-lecture autorisée.
    assert_eq!(col(&s, "out", "x"), some(&[11.0, 11.0, 33.0]));
}

/// Plusieurs SET *statements* restent refusés (hors périmètre M16.4).
#[test]
fn multiple_set_statements_still_error() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0]))]);
    write_num_ds(&s, "b", &[("x", some(&[2.0]))]);
    let e = run_err("data out; set a; set b; run;", &mut s);
    assert!(e.contains("Multiple SET statements"), "got: {e}");
}

/// END=/NOBS= combinés : compteur de fin + total.
#[test]
fn end_and_nobs_combined() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[5.0, 6.0, 7.0]))]);
    run(
        "data out; set a end=eof nobs=n; \
         if eof then last_total = n; retain last_total; run;",
        &mut s,
    )
    .unwrap();
    // n est connu partout ; last_total posé sur eof.
    assert_eq!(num_at(&s, "out", "last_total", 2), Some(3.0));
}

// ── M16.5 : UPDATE / MODIFY ──────────────────────────────────────────

/// Écrit un dataset avec une colonne char `key` et des colonnes num.
/// `keys` = valeurs de la clé char ; `cols` = (nom, valeurs num).
fn write_keyed_ds(
    session: &Session,
    table: &str,
    key: &str,
    keys: &[&str],
    cols: &[(&str, Vec<Option<f64>>)],
) {
    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();
    columns.push(Series::new(key.into(), keys.to_vec()).into());
    vars.push(VarMeta {
        name: key.to_string(),
        ty: VarType::Char,
        length: 8,
        format: None,
        label: None,
    });
    for (name, vals) in cols {
        columns.push(Series::new((*name).into(), vals.clone()).into());
        vars.push(VarMeta {
            name: (*name).to_string(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        });
    }
    let df = DataFrame::new(columns).unwrap();
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(table, &SasDataset { df, vars })
        .unwrap();
}

// ----- UPDATE -----

/// UPDATE de base : transaction superpose le maître par clé (match).
#[test]
fn update_basic_overlay() {
    let mut s = session();
    // maître : id=1,2,3 ; x=10,20,30
    write_num_ds(&s, "mas", &[("id", some(&[1.0, 2.0, 3.0])), ("x", some(&[10.0, 20.0, 30.0]))]);
    // transaction : id=2 ; x=99
    write_num_ds(&s, "tra", &[("id", some(&[2.0])), ("x", some(&[99.0]))]);
    let stats = run("data mas; update mas tra key=id; run;", &mut s).unwrap();
    assert_eq!(col(&s, "mas", "id"), some(&[1.0, 2.0, 3.0]));
    // id=2 mis à jour à 99 ; les autres inchangés.
    assert_eq!(col(&s, "mas", "x"), some(&[10.0, 99.0, 30.0]));
    assert_eq!(stats.written, vec![("WORK.MAS".to_string(), 3, 2)]);
    // Deux NOTEs de lecture (maître + transaction).
    assert_eq!(
        stats.read,
        vec![("WORK.MAS".to_string(), 3), ("WORK.TRA".to_string(), 1)]
    );
}

/// UPDATE : une clé maître sans transaction correspondante reste inchangée.
#[test]
fn update_no_match_unchanged() {
    let mut s = session();
    write_num_ds(&s, "mas", &[("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))]);
    write_num_ds(&s, "tra", &[("id", some(&[9.0])), ("x", some(&[99.0]))]);
    run("data mas; update mas tra key=id; run;", &mut s).unwrap();
    assert_eq!(col(&s, "mas", "x"), some(&[10.0, 20.0]));
}

/// UPDATE : une valeur transaction MANQUANTE ne superpose pas (no-update).
#[test]
fn update_missing_transaction_skips_overlay() {
    let mut s = session();
    write_num_ds(
        &s,
        "mas",
        &[("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0])), ("y", some(&[1.0, 2.0]))],
    );
    // transaction id=1 : x=. (manquant → pas de MAJ), y=77 (MAJ).
    write_num_ds(
        &s,
        "tra",
        &[("id", some(&[1.0])), ("x", vec![None]), ("y", some(&[77.0]))],
    );
    run("data mas; update mas tra key=id; run;", &mut s).unwrap();
    // x inchangé (transaction manquante) ; y mis à jour.
    assert_eq!(col(&s, "mas", "x"), some(&[10.0, 20.0]));
    assert_eq!(col(&s, "mas", "y"), some(&[77.0, 2.0]));
}

/// UPDATE : la variable clé n'est jamais écrasée par la transaction.
#[test]
fn update_key_not_overwritten() {
    let mut s = session();
    write_num_ds(&s, "mas", &[("id", some(&[5.0])), ("x", some(&[1.0]))]);
    // La transaction porte la même clé 5 ; x=42.
    write_num_ds(&s, "tra", &[("id", some(&[5.0])), ("x", some(&[42.0]))]);
    run("data mas; update mas tra key=id; run;", &mut s).unwrap();
    assert_eq!(col(&s, "mas", "id"), some(&[5.0]));
    assert_eq!(col(&s, "mas", "x"), some(&[42.0]));
}

/// UPDATE : plusieurs transactions pour une clé → seule la PREMIÈRE compte.
#[test]
fn update_multiple_transactions_first_wins() {
    let mut s = session();
    write_num_ds(&s, "mas", &[("id", some(&[1.0])), ("x", some(&[10.0]))]);
    write_num_ds(&s, "tra", &[("id", some(&[1.0, 1.0])), ("x", some(&[20.0, 30.0]))]);
    run("data mas; update mas tra key=id; run;", &mut s).unwrap();
    // Première transaction (20) appliquée, la seconde (30) ignorée.
    assert_eq!(col(&s, "mas", "x"), some(&[20.0]));
}

/// UPDATE : une transaction sans maître correspondant est IGNORÉE (v1).
#[test]
fn update_unmatched_transaction_ignored() {
    let mut s = session();
    write_num_ds(&s, "mas", &[("id", some(&[1.0])), ("x", some(&[10.0]))]);
    write_num_ds(&s, "tra", &[("id", some(&[1.0, 2.0])), ("x", some(&[11.0, 22.0]))]);
    let stats = run("data mas; update mas tra key=id; run;", &mut s).unwrap();
    // id=2 (sans maître) n'est PAS inséré : 1 obs en sortie.
    assert_eq!(col(&s, "mas", "id"), some(&[1.0]));
    assert_eq!(col(&s, "mas", "x"), some(&[11.0]));
    assert_eq!(stats.written, vec![("WORK.MAS".to_string(), 1, 2)]);
}

/// UPDATE avec WHERE= sur le maître : les obs filtrées ne sont ni mises à
/// jour ni sorties.
#[test]
fn update_master_where() {
    let mut s = session();
    write_num_ds(&s, "mas", &[("id", some(&[1.0, 2.0, 3.0])), ("x", some(&[10.0, 20.0, 30.0]))]);
    write_num_ds(&s, "tra", &[("id", some(&[2.0])), ("x", some(&[99.0]))]);
    let stats = run(
        "data out; update mas(where=(id>=2)) tra key=id; run;",
        &mut s,
    )
    .unwrap();
    // id=1 filtré ; id=2 mis à jour, id=3 inchangé.
    assert_eq!(col(&s, "out", "id"), some(&[2.0, 3.0]));
    assert_eq!(col(&s, "out", "x"), some(&[99.0, 30.0]));
    // 2 obs maître lues (id=1 rejeté).
    assert_eq!(stats.read[0], ("WORK.MAS".to_string(), 2));
}

/// UPDATE avec plusieurs variables clé.
#[test]
fn update_multiple_keys() {
    let mut s = session();
    write_num_ds(
        &s,
        "mas",
        &[("k1", some(&[1.0, 1.0])), ("k2", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))],
    );
    // Met à jour seulement (1,2).
    write_num_ds(
        &s,
        "tra",
        &[("k1", some(&[1.0])), ("k2", some(&[2.0])), ("x", some(&[99.0]))],
    );
    run("data mas; update mas tra key=k1 k2; run;", &mut s).unwrap();
    assert_eq!(col(&s, "mas", "x"), some(&[10.0, 99.0]));
}

/// UPDATE avec clé CARACTÈRE (insensible aux blancs finaux).
#[test]
fn update_char_key() {
    let mut s = session();
    write_keyed_ds(&s, "mas", "name", &["a", "b", "c"], &[("x", some(&[1.0, 2.0, 3.0]))]);
    write_keyed_ds(&s, "tra", "name", &["b"], &[("x", some(&[20.0]))]);
    run("data mas; update mas tra key=name; run;", &mut s).unwrap();
    assert_eq!(col(&s, "mas", "x"), some(&[1.0, 20.0, 3.0]));
}

/// UPDATE : la transaction apporte une NOUVELLE variable absente du maître.
#[test]
fn update_new_variable_from_transaction() {
    let mut s = session();
    write_num_ds(&s, "mas", &[("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))]);
    write_num_ds(&s, "tra", &[("id", some(&[1.0])), ("z", some(&[5.0]))]);
    run("data mas; update mas tra key=id; run;", &mut s).unwrap();
    // z existe (du maître absent → missing), posée pour id=1.
    assert_eq!(col(&s, "mas", "z"), vec![Some(5.0), None]);
}

/// UPDATE : KEY= absente d'un dataset → erreur de compilation.
#[test]
fn update_key_not_on_transaction_errors() {
    let mut s = session();
    write_num_ds(&s, "mas", &[("id", some(&[1.0])), ("x", some(&[10.0]))]);
    write_num_ds(&s, "tra", &[("other", some(&[1.0])), ("x", some(&[20.0]))]);
    let e = run_err("data mas; update mas tra key=id; run;", &mut s);
    assert!(e.contains("KEY variable id"), "got: {e}");
}

/// UPDATE : KEY= obligatoire (erreur de parsing si absente).
#[test]
fn update_requires_key_option() {
    // Parsing seul : KEY= absente → erreur de parsing (pas d'exécution).
    let file = SourceFile::new("data mas; update mas tra; run;");
    let mut ts = StatementStream::new(&file).unwrap();
    assert!(ts.next().is_kw("data"));
    let err = crate::parser::datastep::parse_data_step(&mut ts).unwrap_err();
    assert!(err.to_string().to_uppercase().contains("KEY"), "got: {err}");
}

/// UPDATE avec BY : FIRST./LAST. exposés sur les groupes BY du maître.
#[test]
fn update_with_by_first_last() {
    let mut s = session();
    // maître trié par g : g=1,1,2 ; x=10,20,30.
    write_num_ds(
        &s,
        "mas",
        &[("g", some(&[1.0, 1.0, 2.0])), ("id", some(&[1.0, 2.0, 3.0])), ("x", some(&[10.0, 20.0, 30.0]))],
    );
    write_num_ds(&s, "tra", &[("id", some(&[2.0])), ("x", some(&[99.0]))]);
    run(
        "data out; update mas tra key=id; by g; \
         f = first.g; l = last.g; run;",
        &mut s,
    )
    .unwrap();
    // id=2 mis à jour ; FIRST.g sur les 1res obs de chaque groupe g.
    assert_eq!(col(&s, "out", "x"), some(&[10.0, 99.0, 30.0]));
    assert_eq!(col(&s, "out", "f"), some(&[1.0, 0.0, 1.0]));
    assert_eq!(col(&s, "out", "l"), some(&[0.0, 1.0, 1.0]));
}

/// UPDATE avec BY : la mise à jour reste pilotée par KEY= au sein des
/// groupes BY (chaque obs maître conserve son comportement).
#[test]
fn update_with_by_groups_update() {
    let mut s = session();
    write_num_ds(
        &s,
        "mas",
        &[("g", some(&[1.0, 2.0])), ("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))],
    );
    write_num_ds(&s, "tra", &[("id", some(&[1.0, 2.0])), ("x", some(&[100.0, 200.0]))]);
    run("data out; update mas tra key=id; by g; run;", &mut s).unwrap();
    assert_eq!(col(&s, "out", "x"), some(&[100.0, 200.0]));
}

/// UPDATE : le corps peut calculer des variables dérivées.
#[test]
fn update_with_derived_body_statement() {
    let mut s = session();
    write_num_ds(&s, "mas", &[("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))]);
    write_num_ds(&s, "tra", &[("id", some(&[1.0])), ("x", some(&[100.0]))]);
    run("data out; update mas tra key=id; d = x * 2; run;", &mut s).unwrap();
    // x après MAJ : 100, 20 ; d = 200, 40.
    assert_eq!(col(&s, "out", "x"), some(&[100.0, 20.0]));
    assert_eq!(col(&s, "out", "d"), some(&[200.0, 40.0]));
}

// ----- MODIFY -----

/// MODIFY de base : une modification par assignation persiste en place.
#[test]
fn modify_basic_assign_persists() {
    let mut s = session();
    write_num_ds(&s, "d", &[("id", some(&[1.0, 2.0, 3.0])), ("x", some(&[10.0, 20.0, 30.0]))]);
    let stats = run("data d; modify d; x = x + 1; run;", &mut s).unwrap();
    assert_eq!(col(&s, "d", "x"), some(&[11.0, 21.0, 31.0]));
    // Réécriture en place : même nombre d'obs/variables.
    assert_eq!(stats.written, vec![("WORK.D".to_string(), 3, 2)]);
    assert_eq!(stats.read, vec![("WORK.D".to_string(), 3)]);
    assert_eq!(s.last_dataset.as_deref(), Some("WORK.D"));
}

/// MODIFY : conditionnel (modifie seulement certaines obs).
#[test]
fn modify_conditional_update() {
    let mut s = session();
    write_num_ds(&s, "d", &[("id", some(&[1.0, 2.0, 3.0])), ("x", some(&[10.0, 20.0, 30.0]))]);
    run("data d; modify d; if id = 2 then x = 999; run;", &mut s).unwrap();
    assert_eq!(col(&s, "d", "x"), some(&[10.0, 999.0, 30.0]));
}

/// MODIFY : OUTPUT explicite est INTERDIT (erreur de compilation).
#[test]
fn modify_output_not_allowed() {
    let mut s = session();
    write_num_ds(&s, "d", &[("x", some(&[1.0]))]);
    let e = run_err("data d; modify d; output; run;", &mut s);
    assert!(e.contains("OUTPUT statement is not allowed"), "got: {e}");
}

/// MODIFY avec KEY= (lecture séquentielle, clés présentes).
#[test]
fn modify_with_key() {
    let mut s = session();
    write_num_ds(&s, "d", &[("id", some(&[1.0, 2.0])), ("x", some(&[10.0, 20.0]))]);
    run("data d; modify d key=id; x = x * 10; run;", &mut s).unwrap();
    assert_eq!(col(&s, "d", "x"), some(&[100.0, 200.0]));
}

/// MODIFY + NOBS= : le total est disponible avant la boucle.
#[test]
fn modify_nobs_available() {
    let mut s = session();
    write_num_ds(&s, "d", &[("x", some(&[5.0, 6.0, 7.0]))]);
    run("data d; modify d nobs=n; x = n; run;", &mut s).unwrap();
    // Chaque obs reçoit le total = 3.
    assert_eq!(col(&s, "d", "x"), some(&[3.0, 3.0, 3.0]));
}

/// MODIFY + POINT= : accès direct piloté par un DO, modifie toutes les obs.
#[test]
fn modify_point_loop_all() {
    let mut s = session();
    write_num_ds(&s, "d", &[("x", some(&[10.0, 20.0, 30.0]))]);
    let stats = run(
        "data d; do p = 1 to 3; modify d point=p; x = x + 100; end; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "d", "x"), some(&[110.0, 120.0, 130.0]));
    // 3 obs traitées, 3 réécrites.
    assert_eq!(stats.read, vec![("WORK.D".to_string(), 3)]);
    assert_eq!(stats.written, vec![("WORK.D".to_string(), 3, 1)]);
}

/// MODIFY + POINT= : accès direct ciblé (une seule obs modifiée).
#[test]
fn modify_point_single_row() {
    let mut s = session();
    write_num_ds(&s, "d", &[("x", some(&[10.0, 20.0, 30.0]))]);
    run(
        "data d; do p = 2 to 2; modify d point=p; x = 999; end; run;",
        &mut s,
    )
    .unwrap();
    // Seule la 2e obs change.
    assert_eq!(col(&s, "d", "x"), some(&[10.0, 999.0, 30.0]));
}

/// MODIFY : char + num, modification d'une colonne char persiste.
#[test]
fn modify_char_column() {
    let mut s = session();
    write_keyed_ds(&s, "d", "grp", &["a", "a", "b"], &[("x", some(&[1.0, 2.0, 3.0]))]);
    run("data d; modify d; if x >= 2 then grp = 'z'; run;", &mut s).unwrap();
    assert_eq!(
        col_str(&s, "d", "grp"),
        vec![Some("a".into()), Some("z".into()), Some("z".into())]
    );
}

/// MODIFY : KEY= absente du dataset → erreur de compilation.
#[test]
fn modify_key_not_present_errors() {
    let mut s = session();
    write_num_ds(&s, "d", &[("x", some(&[1.0]))]);
    let e = run_err("data d; modify d key=nope; run;", &mut s);
    assert!(e.contains("KEY variable nope"), "got: {e}");
}

/// UPDATE/MODIFY exclusif : pas plus d'une source par étape.
#[test]
fn update_after_set_is_error() {
    let mut s = session();
    write_num_ds(&s, "a", &[("id", some(&[1.0]))]);
    write_num_ds(&s, "b", &[("id", some(&[1.0]))]);
    let e = run_err("data out; set a; update a b key=id; run;", &mut s);
    assert!(e.contains("Only one SET, MERGE, UPDATE, or MODIFY"), "got: {e}");
}

/// MODIFY après MODIFY → erreur.
#[test]
fn modify_twice_is_error() {
    let mut s = session();
    write_num_ds(&s, "a", &[("x", some(&[1.0]))]);
    write_num_ds(&s, "b", &[("x", some(&[1.0]))]);
    let e = run_err("data a; modify a; modify b; run;", &mut s);
    assert!(e.contains("Only one SET, MERGE, UPDATE, or MODIFY"), "got: {e}");
}

// ── M16.6 : LINK / RETURN / GOTO / labels / RETAIN _ALL_ ─────────────

/// LINK/RETURN de base : appel d'une sous-routine étiquetée, retour après.
/// Structure SAS idiomatique : la ligne principale se termine par un RETURN
/// (output implicite), puis les sous-routines suivent.
#[test]
fn link_basic_call_and_return() {
    let mut s = session();
    run(
        "data out; x = 1; link sub; y = x; return; \
         sub: x = 10; return; \
         run;",
        &mut s,
    )
    .unwrap();
    // x=1, LINK sub → x=10, RETURN → reprise : y=x=10, RETURN principal →
    // output implicite (la sous-routine n'est pas exécutée en chute).
    assert_eq!(col(&s, "out", "x"), vec![Some(10.0)]);
    assert_eq!(col(&s, "out", "y"), vec![Some(10.0)]);
}

/// LINK imbriqué : la pile d'adresses de retour est correcte.
#[test]
fn link_nested_stack() {
    let mut s = session();
    run(
        "data out; a = 0; link one; a = a + 1; return; \
         one: a = a + 10; link two; a = a + 100; return; \
         two: a = a + 1000; return; \
         run;",
        &mut s,
    )
    .unwrap();
    // a: 0 → link one → +10 (10) → link two → +1000 (1010) → return one
    // → +100 (1110) → return main → +1 (1111). stop.
    assert_eq!(col(&s, "out", "a"), vec![Some(1111.0)]);
}

/// GOTO : saut inconditionnel (les statements entre le GOTO et la cible
/// sont ignorés).
#[test]
fn goto_unconditional_jump() {
    let mut s = session();
    run(
        "data out; x = 1; goto skip; x = 999; skip: y = x; run;",
        &mut s,
    )
    .unwrap();
    // x=1, GOTO skip → x=999 sauté, y=x=1 ; chute en fin → output implicite.
    assert_eq!(col(&s, "out", "x"), vec![Some(1.0)]);
    assert_eq!(col(&s, "out", "y"), vec![Some(1.0)]);
}

/// GOTO qui sort d'une boucle DO (termine la boucle prématurément).
#[test]
fn goto_breaks_out_of_do_loop() {
    let mut s = session();
    run(
        "data out; total = 0; \
         do i = 1 to 100; total = total + i; if i = 5 then goto done; end; \
         done: ; run;",
        &mut s,
    )
    .unwrap();
    // 1+2+3+4+5 = 15 ; la boucle est terminée par le GOTO à i=5.
    assert_eq!(col(&s, "out", "total"), vec![Some(15.0)]);
    assert_eq!(col(&s, "out", "i"), vec![Some(5.0)]);
}

/// GOTO avec plusieurs étiquettes : ciblage correct.
#[test]
fn goto_multiple_labels_targets_correctly() {
    let mut s = session();
    run(
        "data out; x = 1; goto third; \
         first: r = 1; goto fin; \
         second: r = 2; goto fin; \
         third: r = 3; goto fin; \
         fin: ; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "r"), vec![Some(3.0)]);
}

/// Étiquette sur divers statements (ici un bloc DO et une assignation).
#[test]
fn label_on_various_statements() {
    let mut s = session();
    run(
        "data out; goto blk; a = 1; \
         blk: do; a = 7; b = 8; end; \
         after: c = 9; run;",
        &mut s,
    )
    .unwrap();
    // GOTO blk saute `a = 1` ; le bloc DO étiqueté pose a=7,b=8 ; puis c=9.
    assert_eq!(col(&s, "out", "a"), vec![Some(7.0)]);
    assert_eq!(col(&s, "out", "b"), vec![Some(8.0)]);
    assert_eq!(col(&s, "out", "c"), vec![Some(9.0)]);
}

/// RETURN sans LINK actif : termine l'itération (output implicite), pas une
/// erreur. La variable assignée APRÈS le RETURN n'est pas affectée.
#[test]
fn return_without_link_ends_iteration() {
    let mut s = session();
    let stats = run(
        "data out; x = 1; return; x = 2; run;",
        &mut s,
    )
    .unwrap();
    // RETURN sans LINK → fin d'itération avec output implicite : x=1.
    assert_eq!(stats.written, vec![("WORK.OUT".to_string(), 1, 1)]);
    assert_eq!(col(&s, "out", "x"), vec![Some(1.0)]);
}

/// GOTO vers une étiquette inexistante : erreur de compilation.
#[test]
fn goto_undefined_label_compile_error() {
    let mut s = session();
    let e = run_err("data out; x = 1; goto nowhere; run;", &mut s);
    assert!(
        e.contains("NOWHERE") && e.contains("not defined"),
        "got: {e}"
    );
}

/// LINK vers une étiquette inexistante : erreur de compilation.
#[test]
fn link_undefined_label_compile_error() {
    let mut s = session();
    let e = run_err("data out; x = 1; link nowhere; run;", &mut s);
    assert!(
        e.contains("NOWHERE") && e.contains("not defined"),
        "got: {e}"
    );
}

/// Étiquette définie deux fois : erreur de compilation.
#[test]
fn duplicate_label_compile_error() {
    let mut s = session();
    let e = run_err("data out; lbl: x = 1; lbl: x = 2; goto lbl; run;", &mut s);
    assert!(e.contains("LBL") && e.contains("more than once"), "got: {e}");
}

/// GOTO vers une étiquette imbriquée dans un bloc DO : non supporté (erreur).
#[test]
fn goto_into_nested_block_compile_error() {
    let mut s = session();
    let e = run_err(
        "data out; goto inner; do; inner: x = 1; end; stop; run;",
        &mut s,
    );
    assert!(e.contains("INNER") && e.contains("nested"), "got: {e}");
}

/// RETAIN _ALL_ : toutes les variables connues sont retenues à travers les
/// itérations.
#[test]
fn retain_all_retains_every_variable() {
    let mut s = session();
    write_num_ds(&s, "inp", &[("x", some(&[1.0, 2.0, 3.0]))]);
    run(
        "data out; set inp; retain a b; a = 0; b = 0; retain _all_; \
         a = a + x; b = b + 1; run;",
        &mut s,
    )
    .unwrap();
    // a et b retenus (cumul) : a = 1,3,6 ; b = 1,2,3. Mais `a=0;b=0;` les
    // remet à 0 AVANT le cumul, à CHAQUE itération → a=x, b=1. On teste donc
    // que la retenue n'empêche pas la ré-assignation explicite : a=1,2,3.
    assert_eq!(col(&s, "out", "a"), vec![Some(1.0), Some(2.0), Some(3.0)]);
    assert_eq!(col(&s, "out", "b"), vec![Some(1.0), Some(1.0), Some(1.0)]);
}

/// RETAIN _ALL_ : effet cumulatif réel (pas de remise à missing entre
/// itérations) pour une variable jamais ré-initialisée. `t` est initialisé
/// à 0 à la 1re itération seulement, puis RETAIN _ALL_ le préserve.
#[test]
fn retain_all_accumulates() {
    let mut s = session();
    write_num_ds(&s, "inp", &[("x", some(&[1.0, 2.0, 3.0, 4.0]))]);
    run(
        "data out; set inp; if _n_ = 1 then t = 0; retain _all_; t = t + x; run;",
        &mut s,
    )
    .unwrap();
    // t=0 à la 1re obs, retenu ensuite (jamais remis à missing) → cumul :
    // 1, 3, 6, 10.
    assert_eq!(
        col(&s, "out", "t"),
        vec![Some(1.0), Some(3.0), Some(6.0), Some(10.0)]
    );
}

/// RETAIN _ALL_ mélangé à un RETAIN explicite avec valeur initiale : la
/// valeur initiale du RETAIN explicite est honorée.
#[test]
fn retain_all_mixed_with_explicit_retain() {
    let mut s = session();
    write_num_ds(&s, "inp", &[("x", some(&[1.0, 2.0, 3.0]))]);
    run(
        "data out; set inp; retain base 100; if _n_ = 1 then sum = 0; \
         retain _all_; sum = sum + x; run;",
        &mut s,
    )
    .unwrap();
    // base retenu avec init 100 (jamais réassigné) ; sum cumulé.
    assert_eq!(
        col(&s, "out", "base"),
        vec![Some(100.0), Some(100.0), Some(100.0)]
    );
    assert_eq!(col(&s, "out", "sum"), vec![Some(1.0), Some(3.0), Some(6.0)]);
}

/// Variable créée APRÈS RETAIN _ALL_ : NON retenue automatiquement (remise à
/// missing à chaque itération).
#[test]
fn variable_created_after_retain_all_not_retained() {
    let mut s = session();
    write_num_ds(&s, "inp", &[("x", some(&[5.0, 6.0, 7.0]))]);
    run(
        "data out; set inp; retain _all_; later = later + x; run;",
        &mut s,
    )
    .unwrap();
    // `later` n'existe PAS au point du RETAIN _ALL_ (créée par sa 1re
    // référence ensuite) → non retenue → remise à missing chaque itération
    // → later = . + x = . (missing propagé).
    assert_eq!(col(&s, "out", "later"), vec![None, None, None]);
}

/// LINK dans une boucle : l'itération de la boucle reprend après le retour.
#[test]
fn link_inside_do_loop_continues_iteration() {
    let mut s = session();
    run(
        "data out; total = 0; \
         do i = 1 to 4; link addit; end; \
         return; \
         addit: total = total + i; return; \
         run;",
        &mut s,
    )
    .unwrap();
    // total = 1+2+3+4 = 10 ; la boucle continue après chaque RETURN.
    assert_eq!(col(&s, "out", "total"), vec![Some(10.0)]);
    assert_eq!(col(&s, "out", "i"), vec![Some(5.0)]);
}

/// Modifications de variables dans le code LINKé : persistance (PDV partagé).
#[test]
fn link_modifications_persist() {
    let mut s = session();
    run(
        "data out; x = 5; y = 0; link doit; z = x + y; return; \
         doit: x = x * 2; y = 100; return; run;",
        &mut s,
    )
    .unwrap();
    // doit : x=10, y=100 (persistants) ; z = 10 + 100 = 110.
    assert_eq!(col(&s, "out", "x"), vec![Some(10.0)]);
    assert_eq!(col(&s, "out", "y"), vec![Some(100.0)]);
    assert_eq!(col(&s, "out", "z"), vec![Some(110.0)]);
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

/// GOTO en arrière formant une boucle, terminée par une condition.
#[test]
fn goto_backward_forms_loop() {
    let mut s = session();
    run(
        "data out; n = 0; \
         loop: n = n + 1; if n < 5 then goto loop; \
         run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "n"), vec![Some(5.0)]);
}

/// LINK depuis une sous-routine LINK vers une troisième : pile à 2 niveaux,
/// chaque RETURN reprend au bon endroit.
#[test]
fn link_chain_returns_in_order() {
    let mut s = session();
    run(
        "data out; trace = 0; link a; trace = trace * 10 + 9; return; \
         a: trace = trace * 10 + 1; link b; trace = trace * 10 + 2; return; \
         b: trace = trace * 10 + 3; return; \
         run;",
        &mut s,
    )
    .unwrap();
    // 0 → a: *10+1 = 1 → b: *10+3 = 13 → ret a: *10+2 = 132 → ret main:
    // *10+9 = 1329.
    assert_eq!(col(&s, "out", "trace"), vec![Some(1329.0)]);
}

/// `go to label;` (forme en deux mots) équivalente à `goto label;`.
#[test]
fn go_to_two_word_form() {
    let mut s = session();
    run(
        "data out; x = 1; go to skip; x = 999; skip: ; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(col(&s, "out", "x"), vec![Some(1.0)]);
}

/// GOTO/LINK fonctionnent sur plusieurs itérations d'un SET (la pile de
/// retour est ré-initialisée à chaque itération).
#[test]
fn link_resets_per_iteration() {
    let mut s = session();
    write_num_ds(&s, "inp", &[("x", some(&[1.0, 2.0, 3.0]))]);
    run(
        "data out; set inp; link dbl; stop_marker: ; goto past; \
         dbl: d = x * 2; return; \
         past: ; run;",
        &mut s,
    )
    .unwrap();
    // Chaque itération : link dbl (d=2x), retour, goto past (saute rien),
    // output implicite. d = 2,4,6 sur les 3 obs.
    assert_eq!(
        col(&s, "out", "d"),
        vec![Some(2.0), Some(4.0), Some(6.0)]
    );
}

/// RETAIN _ALL_ n'autorise pas de valeur initiale.
#[test]
fn retain_all_rejects_initial_value() {
    let mut s = session();
    let e = run_err("data out; x = 1; retain _all_ 5; run;", &mut s);
    assert!(e.contains("initial value"), "got: {e}");
}

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
