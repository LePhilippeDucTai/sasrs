//! J08-P2 — BY généralisé pour PROC CORR (via `common::by`).
//!
//! Propriétés testées :
//! - parse de `BY v1 [DESCENDING v2] ;` (même grammaire que MEANS) ;
//! - données non triées par les variables BY → ERROR SAS « not sorted » ;
//! - un bloc listing par groupe (en-tête `var=valeur` façon SAS) et un OUT=
//!   TYPE=CORR par groupe, variables BY en tête ;
//! - invariant : UN SEUL groupe BY → sortie identique à la sortie sans BY
//!   (listing hormis la ligne d'en-tête de groupe, OUT= à colonnes égales).

use super::*;
use crate::dataset::SasDataset;
use crate::testkit::*;
use polars::df;

/// Exécute une source PROC CORR complète et renvoie le listing.
fn run_corr(session: &mut Session, src: &str) -> Result<String> {
    let ast = parse_corr(src)?;
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

/// g,x,y triés par g : A→(1,2)/(2,4), B→(3,6).
fn grouped_fixture(session: &mut Session) {
    let df = df![
        "g" => ["A", "A", "B"],
        "x" => [1.0_f64, 2.0, 3.0],
        "y" => [2.0_f64, 4.0, 6.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("g", 8), num_meta("x"), num_meta("y")],
    };
    write_dataset(session, "T", ds);
}

#[test]
fn by_generalized_corr_parses_by_with_descending() {
    let ast = parse_corr("proc corr data=t; by descending g v; var x y; run;").unwrap();
    assert_eq!(
        ast.by,
        vec![("g".to_string(), true), ("v".to_string(), false)]
    );
}

#[test]
fn by_generalized_corr_unsorted_errors() {
    let mut session = make_session();
    // NOT sorted by g: B,A.
    let df = df![
        "g" => ["B", "A"],
        "x" => [1.0_f64, 2.0],
        "y" => [2.0_f64, 4.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("g", 8), num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let r = run_corr(
        &mut session,
        "proc corr data=t nosimple; by g; var x y; run;",
    );
    assert!(r.is_err());
    let msg = r.err().unwrap().to_string();
    assert!(
        msg.contains("not sorted in ascending sequence")
            && msg.contains("g=B")
            && msg.contains("g=A"),
        "msg: {msg}"
    );
}

#[test]
fn by_generalized_corr_groups_listing_and_out() {
    let mut session = make_session();
    grouped_fixture(&mut session);

    let listing = run_corr(
        &mut session,
        "proc corr data=t outp=work.o; by g; var x y; run;",
    )
    .unwrap();

    // Un en-tête de groupe par groupe BY, dans l'ordre des groupes.
    let a = listing.find("\ng=A\n").expect("heading g=A");
    let b = listing.find("\ng=B\n").expect("heading g=B");
    assert!(a < b, "group headings out of order:\n{listing}");

    // OUT= : variables BY en tête, un bloc MEAN/STD/N/CORR (5 lignes) par
    // groupe → 10 observations.
    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    assert_eq!(out.n_obs(), 10);
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names[0], "g", "BY column must come first: {names:?}");
    assert_eq!(
        names[1..],
        ["_TYPE_", "_NAME_", "x", "y"],
        "unexpected OUT= layout: {names:?}"
    );

    let gcol = read_col(&session, "O", "g");
    assert!(gcol.iter().take(5).all(|v| *v == Value::Char("A".into())));
    assert!(gcol.iter().skip(5).all(|v| *v == Value::Char("B".into())));

    // Groupe A : x=(1,2), y=(2,4) → r=1 ; N=2.
    // Groupe B : une seule observation → r=1 (diagonale), N=1.
    let type_col = read_col(&session, "O", "_TYPE_");
    let name_col = read_col(&session, "O", "_NAME_");
    let xcol = read_col(&session, "O", "x");
    let ycol = read_col(&session, "O", "y");

    // Block layout per group: MEAN, STD, N, CORR x, CORR y.
    // Group A (rows 0..5): N row is index 2; CORR x row index 3.
    assert_eq!(type_col[2], Value::Char("N".into()));
    assert_eq!(xcol[2], Value::Num(2.0), "N for x in group A");
    assert_eq!(type_col[3], Value::Char("CORR".into()));
    assert_eq!(name_col[3], Value::Char("x".into()));
    assert_eq!(xcol[3], Value::Num(1.0), "r(x,x)=1 in group A");
    match &ycol[3] {
        Value::Num(r) => assert!(
            (r - 1.0).abs() < 1e-12,
            "r(x,y) must be 1 in group A (collinear), got {r}"
        ),
        other => panic!("r(x,y) missing in group A: {other:?}"),
    }
    assert_eq!(xcol[2 + 5], Value::Num(1.0), "N for x in group B");
    assert!(
        matches!(ycol[3 + 5], Value::Missing(_)),
        "r(x,y) undefined with n=1 in group B: {:?}",
        ycol[8]
    );
}

#[test]
fn by_generalized_corr_single_group_matches_no_by() {
    // UN SEUL groupe BY → sortie identique à la sortie sans BY.
    let mk = |session: &mut Session| {
        let df = df![
            "g" => ["A", "A", "A"],
            "x" => [1.0_f64, 2.0, 3.0],
            "y" => [2.0_f64, 4.0, 7.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("g", 8), num_meta("x"), num_meta("y")],
        };
        write_dataset(session, "T", ds);
    };

    let mut s0 = make_session();
    mk(&mut s0);
    let l0 = run_corr(&mut s0, "proc corr data=t outp=work.o0; var x y; run;").unwrap();
    let (o0, _) = s0.libs.get("WORK").unwrap().read("O0").unwrap();

    let mut s1 = make_session();
    mk(&mut s1);
    let l1 = run_corr(
        &mut s1,
        "proc corr data=t outp=work.o1; by g; var x y; run;",
    )
    .unwrap();
    let (o1, _) = s1.libs.get("WORK").unwrap().read("O1").unwrap();

    // Listing : identique hormis la ligne d'en-tête de groupe (et son blanc).
    let lines1: Vec<&str> = l1.lines().collect();
    let pos = lines1
        .iter()
        .position(|l| *l == "g=A")
        .expect("single-group heading");
    let mut stripped: Vec<&str> = lines1.clone();
    stripped.drain(pos..(pos + 2).min(lines1.len()));
    assert_eq!(
        stripped.join("\n"),
        l0.lines().collect::<Vec<_>>().join("\n"),
        "single BY group must render the same listing as no BY"
    );

    // OUT= : mêmes colonnes de valeurs (la colonne BY en plus, même ordre).
    assert_eq!(o0.n_obs(), o1.n_obs());
    let names0: Vec<String> = o0.vars.iter().map(|v| v.name.clone()).collect();
    let names1: Vec<String> = o1.vars.iter().map(|v| v.name.clone()).collect();
    assert_eq!(names0, names1[1..], "OUT= gains only the BY column");
    for name in ["_TYPE_", "_NAME_", "x", "y"] {
        let c0 = read_col(&s0, "O0", name);
        let c1 = read_col(&s1, "O1", name);
        assert_eq!(
            c0, c1,
            "column {name} differs between no-BY and single BY group"
        );
    }
}
