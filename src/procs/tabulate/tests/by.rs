//! J08-P2 — BY généralisé pour PROC TABULATE (via `common::by`).
//!
//! Propriétés testées :
//! - parse de `BY v1 [DESCENDING v2] ;` (même grammaire que MEANS) ;
//! - données non triées par les variables BY → ERROR SAS « not sorted » ;
//! - une table par groupe (en-tête `var=valeur` façon SAS) et un OUT= dont
//!   les lignes portent les variables BY en tête ;
//! - invariant : UN SEUL groupe BY → sortie identique à la sortie sans BY
//!   (listing hormis la ligne d'en-tête de groupe, OUT= à colonnes égales).

use super::*;

/// Comme `run` (tests/mod.rs) mais emprunte la session : le test continue de
/// lire WORK.* après l'exécution.
fn run_keep(session: &mut Session, src: &str) -> Result<String> {
    let ast = parse_src(src)?;
    execute(&ast, session)?;
    Ok(session.listing.take_string())
}

fn read_col(session: &Session, table: &str, col: &str) -> Vec<Value> {
    let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
    let idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(col))
        .unwrap();
    decode_column(&ds, idx).unwrap()
}

/// sex,x triés par sex : F→(10,20), M→(30,).
fn grouped_fixture(session: &mut Session) {
    let df = df![
        "sex" => ["F", "F", "M"],
        "x"   => [10.0_f64, 20.0, 30.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("sex", 8), num_meta("x")],
    };
    write_dataset(session, "T", ds);
}

#[test]
fn by_generalized_tabulate_parses_by_with_descending() {
    let ast =
        parse_src("proc tabulate data=t; by descending sex g; class sex; table sex; run;").unwrap();
    assert_eq!(
        ast.by,
        vec![("sex".to_string(), true), ("g".to_string(), false)]
    );
}

#[test]
fn by_generalized_tabulate_unsorted_errors() {
    let mut session = make_session();
    // NOT sorted by sex: M,F.
    let df = df![
        "sex" => ["M", "F"],
        "x"   => [1.0_f64, 2.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("sex", 8), num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let r = run_keep(
        &mut session,
        "proc tabulate data=t; class sex; var x; by sex; table sex, x*mean; run;",
    );
    assert!(r.is_err());
    let msg = r.err().unwrap().to_string();
    assert!(
        msg.contains("not sorted in ascending sequence")
            && msg.contains("sex=M")
            && msg.contains("sex=F"),
        "msg: {msg}"
    );
}

#[test]
fn by_generalized_tabulate_groups_listing_and_out() {
    let mut session = make_session();
    grouped_fixture(&mut session);

    let listing = run_keep(
        &mut session,
        "proc tabulate data=t out=work.o; class sex; var x; by sex; table sex, x*mean; run;",
    )
    .unwrap();

    // Un en-tête de groupe par groupe BY, dans l'ordre des groupes, et une
    // table par groupe (une ligne « F » puis une ligne « M »).
    let a = listing.find("\nsex=F\n").expect("heading sex=F");
    let b = listing.find("\nsex=M\n").expect("heading sex=M");
    assert!(a < b, "group headings out of order:\n{listing}");

    // OUT= : variables BY en tête ; une ligne par cellule ET par groupe
    // (chaque groupe n'observe que son propre niveau de sex) → 2 lignes.
    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    assert_eq!(out.n_obs(), 2);
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names[0], "sex", "BY column must come first: {names:?}");
    assert_eq!(
        names[1..],
        ["_TYPE_", "_PAGE_", "_TABLE_", "x_Mean"],
        "unexpected OUT= layout: {names:?}"
    );

    let sex = read_col(&session, "O", "sex");
    assert_eq!(sex[0], Value::Char("F".into()));
    assert_eq!(sex[1], Value::Char("M".into()));
    let mean = read_col(&session, "O", "x_Mean");
    assert_eq!(mean[0], Value::Num(15.0), "mean of F group (10,20)");
    assert_eq!(mean[1], Value::Num(30.0), "mean of M group (30)");
}

#[test]
fn by_generalized_tabulate_single_group_matches_no_by() {
    // UN SEUL groupe BY → sortie identique à la sortie sans BY.
    let mk = |session: &mut Session| {
        let df = df![
            "sex" => ["F", "F", "F"],
            "x"   => [10.0_f64, 20.0, 30.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("sex", 8), num_meta("x")],
        };
        write_dataset(session, "T", ds);
    };

    let mut s0 = make_session();
    mk(&mut s0);
    let l0 = run_keep(
        &mut s0,
        "proc tabulate data=t out=work.o0; class sex; var x; table sex, x*mean; run;",
    )
    .unwrap();
    let (o0, _) = s0.libs.get("WORK").unwrap().read("O0").unwrap();

    let mut s1 = make_session();
    mk(&mut s1);
    let l1 = run_keep(
        &mut s1,
        "proc tabulate data=t out=work.o1; class sex; var x; by sex; table sex, x*mean; run;",
    )
    .unwrap();
    let (o1, _) = s1.libs.get("WORK").unwrap().read("O1").unwrap();

    // Listing : identique hormis la ligne d'en-tête de groupe (et son blanc).
    let lines1: Vec<&str> = l1.lines().collect();
    let pos = lines1
        .iter()
        .position(|l| *l == "sex=F")
        .expect("single-group heading");
    let mut stripped: Vec<&str> = lines1.clone();
    stripped.drain(pos..(pos + 2).min(lines1.len()));
    assert_eq!(
        stripped.join("\n"),
        l0.lines().collect::<Vec<_>>().join("\n"),
        "single BY group must render the same listing as no BY"
    );

    // OUT= : mêmes colonnes de valeurs. La variable CLASS sex coïncide avec
    // la variable BY → une seule colonne sex (la colonne BY), layout identique.
    assert_eq!(o0.n_obs(), o1.n_obs());
    let names0: Vec<String> = o0.vars.iter().map(|v| v.name.clone()).collect();
    let names1: Vec<String> = o1.vars.iter().map(|v| v.name.clone()).collect();
    assert_eq!(names0, names1, "OUT= layout must be unchanged by single-BY");
    for name in ["sex", "_TYPE_", "_PAGE_", "_TABLE_", "x_Mean"] {
        let c0 = read_col(&s0, "O0", name);
        let c1 = read_col(&s1, "O1", name);
        assert_eq!(
            c0, c1,
            "column {name} differs between no-BY and single BY group"
        );
    }
}
