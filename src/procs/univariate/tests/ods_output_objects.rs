//! J08-P3 — ODS OUTPUT des objets UNIVARIATE : `Quantiles`, `ExtremeObs`,
//! `TestsForNormality`.
//!
//! Chaque test prouve une propriété de la capture (noms de colonnes SAS,
//! valeurs typées pleine précision) et échoue si la propriété est brisée.

use super::*;

fn ods_target(session: &mut Session, table: &str, name: &str) {
    session.set_ods_output(&[(
        table.to_string(),
        DatasetRef {
            libref: None,
            name: name.to_string(),
        },
    )]);
}

fn session_with(var: &str, xs: Vec<f64>) -> Session {
    let mut session = make_session();
    let ds = SasDataset {
        df: df![var => xs].unwrap(),
        vars: vec![num_meta(var)],
    };
    write_dataset(&mut session, "T", ds);
    session
}

fn run_univ(session: &mut Session, var: &str, normal: bool) {
    let ast = UnivariateAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        var: vec![var.into()],
        by: vec![],
        weight: None,
        vardef: VarDef::Df,
        exclnpwgt: false,
        output: None,
        normal,
        plots: vec![],
        noprint: false,
    };
    execute(&ast, session).unwrap();
    session.flush_ods_output().unwrap();
}

/// `ods output Quantiles=q;` → colonnes SAS réelles VarName, Quantile,
/// Estimate — 11 lignes, estimations pleine précision (Définition 5).
#[test]
fn ods_output_object_univariate_quantiles() {
    let mut session = session_with("x", vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    ods_target(&mut session, "Quantiles", "q");
    run_univ(&mut session, "x", false);

    let (out, _) = session.libs.get("WORK").unwrap().read("Q").unwrap();
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["VarName", "Quantile", "Estimate"]);
    assert_eq!(out.n_obs(), 11, "une ligne par niveau de quantile");

    let labels = read_num_col(&session, "Q", "Quantile");
    let median_row = labels
        .iter()
        .position(|v| *v == Value::Char("50% Median".into()))
        .expect("ligne 50% Median");
    let estimates = read_num_col(&session, "Q", "Estimate");
    assert_eq!(estimates[median_row], Value::Num(3.5));
    assert_eq!(estimates[0], Value::Num(6.0), "100% Max");
    assert_eq!(estimates[10], Value::Num(1.0), "0% Min");

    // VarName présent sur chaque ligne.
    let varname = read_num_col(&session, "Q", "VarName");
    assert_eq!(varname[0], Value::Char("x".into()));
}

/// `ods output ExtremeObs=e;` → colonnes SAS réelles VarName, Obs, Value —
/// 5 plus basses puis 5 plus hautes, chacune en ordre croissant.
#[test]
fn ods_output_object_univariate_extremeobs() {
    let xs: Vec<f64> = (1..=10).map(|i| i as f64).collect();
    let mut session = session_with("x", xs);
    ods_target(&mut session, "ExtremeObs", "e");
    run_univ(&mut session, "x", false);

    let (out, _) = session.libs.get("WORK").unwrap().read("E").unwrap();
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["VarName", "Obs", "Value"]);
    assert_eq!(out.n_obs(), 10, "5 plus basses + 5 plus hautes");

    let obs = read_num_col(&session, "E", "Obs");
    let values = read_num_col(&session, "E", "Value");
    // Ordre SAS : les 5 plus basses puis les 5 plus hautes, chacune
    // croissante. Les données étant déjà triées, obs = value = 1..10.
    for i in 0..10 {
        assert_eq!(obs[i], Value::Num((i + 1) as f64), "obs ligne {i}");
        assert_eq!(values[i], Value::Num((i + 1) as f64), "value ligne {i}");
    }
}

/// `ods output TestsForNormality=n;` (option NORMAL) → colonnes SAS réelles
/// VarName, Test, Stat, pType, pValue — une ligne par test, pValue numérique
/// pleine précision.
#[test]
fn ods_output_object_univariate_tests_for_normality() {
    let xs: Vec<f64> = vec![9.1, 10.2, 9.8, 10.5, 9.9, 10.1, 10.3, 9.7, 10.0, 10.4];
    let mut session = session_with("x", xs);
    ods_target(&mut session, "TestsForNormality", "n");
    run_univ(&mut session, "x", true);

    let (out, _) = session.libs.get("WORK").unwrap().read("N").unwrap();
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["VarName", "Test", "Stat", "pType", "pValue"]);
    assert_eq!(out.n_obs(), 4, "quatre tests de normalité");

    let tests = read_num_col(&session, "N", "Test");
    assert_eq!(
        tests,
        vec![
            Value::Char("Shapiro-Wilk".into()),
            Value::Char("Kolmogorov-Smirnov".into()),
            Value::Char("Cramer-von Mises".into()),
            Value::Char("Anderson-Darling".into()),
        ]
    );
    let stat = read_num_col(&session, "N", "Stat");
    assert_eq!(stat[0], Value::Char("W".into()));
    let ptype = read_num_col(&session, "N", "pType");
    assert_eq!(ptype[0], Value::Char("Pr < W".into()));
    // pValue numérique dans [0, 1] (pas la chaîne formatée du listing).
    match &read_num_col(&session, "N", "pValue")[0] {
        Value::Num(p) => assert!((0.0..=1.0).contains(p), "pValue {p} hors [0,1]"),
        v => panic!("pValue doit être numérique, reçu {v:?}"),
    }
}

/// Sans NORMAL la table TestsForNormality n'est pas produite : la demande
/// reste sans objet (WARNING de fin de step, géré par la session).
#[test]
fn ods_output_object_univariate_normality_requires_normal_option() {
    let mut session = session_with("x", vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    ods_target(&mut session, "TestsForNormality", "n");
    run_univ(&mut session, "x", false);
    assert!(
        session.libs.get("WORK").unwrap().read("N").is_err(),
        "aucune capture sans l'option NORMAL"
    );
}
