//! J02-P2 (issue #17) — non-régression du statement OUTPUT OUT= de PROC GLM :
//! parsing (`out=` + `p=`/`r=`), dataset résultat (variables d'entrée
//! recopiées + valeurs ajustées = moyennes de groupe + résidus) et refus
//! explicites (multi-voies, dépendantes multiples, mot-clés non supportés).
//! Oracle conformance stat/glm-oneway-predicted (moyennes de groupe
//! recalculées indépendamment en Python 3).

use super::*;
use crate::dataset::SasDataset;
use crate::testkit::*;
use crate::value::Value;
use polars::df;

fn drug_session() -> (Session, &'static str) {
    let mut session = make_session();
    // Oracle stat/glm-oneway-predicted : A=[26,24,27,25] (moyenne 25.5),
    // B=[30,31,29,30] (30.0), C=[34,33,35,34] (34.0).
    let df = df![
        "drug" => ["A", "A", "A", "A", "B", "B", "B", "B", "C", "C", "C", "C"],
        "absorb" => [26.0_f64, 24.0, 27.0, 25.0, 30.0, 31.0, 29.0, 30.0, 34.0, 33.0, 35.0, 34.0],
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("drug", 1), num_meta("absorb")],
    };
    write_dataset(&mut session, "DRUG", ds);
    (
        session,
        "proc glm data=work.drug; class drug; model absorb = drug; output out=pred p=phat r=resid; run;",
    )
}

fn read_col(session: &Session, table: &str, col: &str) -> Vec<Value> {
    let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
    let idx = ds.vars.iter().position(|m| m.name == col).unwrap();
    crate::procs::common::decode_column(&ds, idx).unwrap()
}

#[test]
fn output_statement_parsed_with_p_and_r() {
    let ast = parse_glm(
        "proc glm data=work.t; class g; model y = g; output out=lib.pred p=phat r=resid; run;",
    )
    .unwrap();
    let o = ast.output.expect("output statement parsed");
    assert_eq!(o.out.libref, Some("lib".to_string()));
    assert_eq!(o.out.name.to_uppercase(), "PRED");
    assert_eq!(o.p.as_deref(), Some("phat"));
    assert_eq!(o.r.as_deref(), Some("resid"));
}

#[test]
fn output_statement_requires_out_and_statistic() {
    let Err(e1) = parse_glm("proc glm; class g; model y = g; output p=phat; run;").map(|_| ())
    else {
        panic!("expected error for missing OUT=")
    };
    assert!(e1.to_string().contains("requires the OUT= option"), "{e1}");

    let Err(e2) = parse_glm("proc glm; class g; model y = g; output out=p; run;").map(|_| ())
    else {
        panic!("expected error for missing statistic keyword")
    };
    assert!(
        e2.to_string()
            .contains("requires at least one statistic keyword"),
        "{e2}"
    );
}

#[test]
fn output_statement_unsupported_statistic_rejected() {
    let Err(err) =
        parse_glm("proc glm; class g; model y = g; output out=p student=s; run;").map(|_| ())
    else {
        panic!("expected error")
    };
    assert!(
        err.to_string().contains("STUDENT") && err.to_string().contains("not supported in sasrs"),
        "{err}"
    );
}

#[test]
fn output_dataset_predicted_and_residuals_match_oracle() {
    let (mut session, src) = drug_session();
    let ast = parse_glm(src).unwrap();
    execute(&ast, &mut session).unwrap();

    let phat = read_col(&session, "PRED", "phat");
    let resid = read_col(&session, "PRED", "resid");
    let expected_p = [
        25.5, 25.5, 25.5, 25.5, 30.0, 30.0, 30.0, 30.0, 34.0, 34.0, 34.0, 34.0,
    ];
    let expected_r = [
        0.5, -1.5, 1.5, -0.5, 0.0, 1.0, -1.0, 0.0, 0.0, -1.0, 1.0, 0.0,
    ];
    for (i, (p, r)) in phat.iter().zip(resid.iter()).enumerate() {
        let (Value::Num(p), Value::Num(r)) = (p, r) else {
            panic!("row {i}: expected numeric p/r, got {p:?}/{r:?}")
        };
        assert!((p - expected_p[i]).abs() < 1e-12, "row {i}: phat = {p}");
        assert!((r - expected_r[i]).abs() < 1e-12, "row {i}: resid = {r}");
    }

    // Input variables are carried over (in order), statistics appended.
    let (ds, _) = session.libs.get("WORK").unwrap().read("PRED").unwrap();
    assert_eq!(ds.n_obs(), 12);
    let names: Vec<&str> = ds.vars.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, vec!["drug", "absorb", "phat", "resid"]);
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.PRED"));

    let log = session.log.current_text();
    assert!(
        log.contains("The data set WORK.PRED has 12 observations and 4 variables."),
        "log: {log}"
    );
    assert!(!log.contains("ERROR:"), "log: {log}");
}

#[test]
fn output_multiway_model_is_an_error() {
    let (mut session, _) = drug_session();
    let ast = parse_glm(
        "proc glm data=work.drug; class drug absorb; model absorb = drug; output out=p p=phat; run;",
    )
    .unwrap();
    let Err(err) = execute(&ast, &mut session) else {
        panic!("expected a runtime error")
    };
    assert!(
        err.to_string()
            .contains("only supported for one-way models"),
        "err: {err}"
    );
}
