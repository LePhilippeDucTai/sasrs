//! J08-P3 — ODS OUTPUT des objets REG : `ParameterEstimates`, `ANOVA`,
//! `FitStatistics`.

use super::*;
use crate::dataset::SasDataset;
use crate::procs::common::decode_column;
use crate::session::Session;
use crate::value::Value;

fn ods_targets(session: &mut Session, mappings: &[(&str, &str)]) {
    session.set_ods_output(
        mappings
            .iter()
            .map(|(t, n)| {
                (
                    t.to_string(),
                    DatasetRef {
                        libref: None,
                        name: n.to_string(),
                    },
                )
            })
            .collect::<Vec<_>>()
            .as_slice(),
    );
}

/// Une valeur de colonne absente : manquante, ou chaîne vide (décodage
/// Polars d'un null caractère).
fn is_missing_or_blank(v: &Value) -> bool {
    match v {
        Value::Missing(_) => true,
        Value::Char(s) => s.is_empty(),
        _ => false,
    }
}

fn read_col(session: &Session, table: &str, col: &str) -> Vec<crate::value::Value> {
    let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
    let idx = ds.vars.iter().position(|m| m.name == col).unwrap();
    decode_column(&ds, idx).unwrap()
}

/// Dataset y = 2x + 1 + bruit fixe (régression simple, R² < 1).
fn xy_session() -> Session {
    let mut session = make_session();
    let ds = SasDataset {
        df: df![
            "y" => [2.5_f64, 5.0, 6.5, 9.5, 10.5],
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0],
        ]
        .unwrap(),
        vars: vec![num_meta("y"), num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);
    session
}

fn run_reg(session: &mut Session) {
    let ast = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        basic_model("y", &["x"]),
    );
    execute(&ast, session).unwrap();
    session.flush_ods_output().unwrap();
}

/// `ods output ParameterEstimates=pe;` → colonnes SAS réelles Dependent,
/// Model, Variable, DF, Estimate, StdErr, tValue, Probt.
#[test]
fn ods_output_object_reg_parameter_estimates() {
    let mut session = xy_session();
    ods_targets(&mut session, &[("ParameterEstimates", "pe")]);
    run_reg(&mut session);

    let (out, _) = session.libs.get("WORK").unwrap().read("PE").unwrap();
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Dependent",
            "Model",
            "Variable",
            "DF",
            "Estimate",
            "StdErr",
            "tValue",
            "Probt",
        ]
    );
    assert_eq!(out.n_obs(), 2, "intercept + pente");

    let variable = read_col(&session, "PE", "Variable");
    assert_eq!(
        variable,
        vec![Value::Char("Intercept".into()), Value::Char("x".into())]
    );
    assert_eq!(
        read_col(&session, "PE", "Dependent")[0],
        Value::Char("y".into())
    );
    assert_eq!(
        read_col(&session, "PE", "Model")[0],
        Value::Char("MODEL1".into())
    );
    assert_eq!(
        read_col(&session, "PE", "DF"),
        vec![Value::Num(1.0), Value::Num(1.0)]
    );

    // OLS exact de y sur x (1..5) : pente = 2.05, intercept = 0.7.
    // Σ(x−3)(y−ȳ) = (−2)(−4.6)+(−1)(−2.1)+0+1·1.4+2·2.4 = 18.2 ; Σ(x−3)²=10
    // → pente 1.82 ; intercept ȳ − 1.82·3 = 6.8 − 5.46 = 1.34.
    let estimate = read_col(&session, "PE", "Estimate");
    match (&estimate[0], &estimate[1]) {
        (Value::Num(b0), Value::Num(b1)) => {
            assert!((b0 - 0.65).abs() < 1e-9, "intercept {b0}");
            assert!((b1 - 2.05).abs() < 1e-9, "pente {b1}");
        }
        v => panic!("Estimate doit être numérique, reçu {v:?}"),
    }
    // Probt dans [0,1], pleine précision numérique.
    match &read_col(&session, "PE", "Probt")[1] {
        Value::Num(p) => assert!((0.0..=1.0).contains(p), "Probt {p}"),
        v => panic!("Probt doit être numérique, reçu {v:?}"),
    }
}

/// `ods output ANOVA=av;` → colonnes SAS réelles Dependent, Model, Source,
/// DF, SumOfSquares, MeanSquare, FValue, ProbF — 3 lignes (Model, Error,
/// Corrected Total).
#[test]
fn ods_output_object_reg_anova() {
    let mut session = xy_session();
    ods_targets(&mut session, &[("ANOVA", "av")]);
    run_reg(&mut session);

    let (out, _) = session.libs.get("WORK").unwrap().read("AV").unwrap();
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Dependent",
            "Model",
            "Source",
            "DF",
            "SumOfSquares",
            "MeanSquare",
            "FValue",
            "ProbF",
        ]
    );
    assert_eq!(out.n_obs(), 3);

    let source = read_col(&session, "AV", "Source");
    assert_eq!(
        source,
        vec![
            Value::Char("Model".into()),
            Value::Char("Error".into()),
            Value::Char("Corrected Total".into()),
        ]
    );
    assert_eq!(
        read_col(&session, "AV", "DF"),
        vec![Value::Num(1.0), Value::Num(3.0), Value::Num(4.0)]
    );
    // SS Model + SS Error = SS Total (SST = Σ(y−ȳ)² = 33.18).
    let ss = read_col(&session, "AV", "SumOfSquares");
    match (&ss[0], &ss[1], &ss[2]) {
        (Value::Num(ssm), Value::Num(sse), Value::Num(sst)) => {
            assert!((sst - 42.8).abs() < 1e-9, "SST {sst}");
            assert!((ssm + sse - sst).abs() < 1e-9, "SSM+SSE != SST");
        }
        v => panic!("SumOfSquares doit être numérique, reçu {v:?}"),
    }
    // F et Prob portés par la ligne Model uniquement.
    assert!(matches!(
        read_col(&session, "AV", "FValue")[0],
        Value::Num(_)
    ));
    assert!(matches!(
        read_col(&session, "AV", "ProbF")[0],
        Value::Num(_)
    ));
    assert!(matches!(
        read_col(&session, "AV", "FValue")[1],
        Value::Missing(_)
    ));
}

/// `ods output FitStatistics=fs;` → colonnes SAS réelles Dependent, Model,
/// Label1, cValue1, nValue1, Label2, cValue2, nValue2 — 3 lignes.
#[test]
fn ods_output_object_reg_fit_statistics() {
    let mut session = xy_session();
    ods_targets(&mut session, &[("FitStatistics", "fs")]);
    run_reg(&mut session);

    let (out, _) = session.libs.get("WORK").unwrap().read("FS").unwrap();
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Dependent",
            "Model",
            "Label1",
            "cValue1",
            "nValue1",
            "Label2",
            "cValue2",
            "nValue2",
        ]
    );
    assert_eq!(out.n_obs(), 3);

    let label1 = read_col(&session, "FS", "Label1");
    assert_eq!(
        label1,
        vec![
            Value::Char("Root MSE".into()),
            Value::Char("Dependent Mean".into()),
            Value::Char("Coeff Var".into()),
        ]
    );
    let label2 = read_col(&session, "FS", "Label2");
    assert_eq!(label2[0], Value::Char("R-Square".into()));
    assert_eq!(label2[1], Value::Char("Adj R-Sq".into()));
    // 3e ligne sans seconde paire (manquante ou chaîne vide au décodage).
    assert!(is_missing_or_blank(&label2[2]));

    // Dependent Mean = 6.8 ; nValue1 pleine précision.
    match &read_col(&session, "FS", "nValue1")[1] {
        Value::Num(m) => assert!((m - 6.8).abs() < 1e-9, "Dependent Mean {m}"),
        v => panic!("nValue1 doit être numérique, reçu {v:?}"),
    }
    // R² = SSM/SST = 33.124/33.18 (numérique, pas la chaîne du listing).
    match &read_col(&session, "FS", "nValue2")[0] {
        Value::Num(r2) => assert!((r2 - 42.025 / 42.8).abs() < 1e-6, "R² {r2}"),
        v => panic!("nValue2 doit être numérique, reçu {v:?}"),
    }
}
