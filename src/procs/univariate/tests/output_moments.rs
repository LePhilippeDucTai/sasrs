//! J02-P2 (issue #17) — SKEWNESS/KURTOSIS acceptés dans l'instruction
//! OUTPUT de PROC UNIVARIATE : parsing + dataset résultat. Oracle
//! conformance stat/univariate-moments-output (moments d'échantillon
//! recalculés indépendamment en Python 3 : n=12, mean=77.66666666666667,
//! std=12.108098967470518, skewness=-0.3089299119508112,
//! kurtosis=-1.044211980358745, min=57, max=95).

use super::*;

const SCORES: [f64; 12] = [
    78.0, 84.0, 91.0, 57.0, 66.0, 73.0, 88.0, 95.0, 62.0, 70.0, 81.0, 87.0,
];

fn moments_output_session(src: &str) -> (Session, UnivariateAst) {
    let mut session = make_session();
    let df = df!["score" => SCORES.to_vec()].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("score")],
    };
    write_dataset(&mut session, "SCORES", ds);
    let ast = parse_univ(src).unwrap();
    (session, ast)
}

#[test]
fn output_statement_parses_skewness_and_kurtosis() {
    let ast = parse_univ(
        "proc univariate data=work.scores noprint; var score; \
         output out=mom n=n mean=mean std=std skewness=skewness kurtosis=kurtosis \
         min=min max=max; run;",
    )
    .unwrap();
    let o = ast.output.expect("output statement parsed");
    assert_eq!(o.out.name.to_uppercase(), "MOM");
    let stats: Vec<&str> = o.specs.iter().map(|(s, _)| s.as_str()).collect();
    assert_eq!(
        stats,
        vec!["n", "mean", "std", "skewness", "kurtosis", "min", "max"]
    );
}

#[test]
fn output_moments_dataset_matches_oracle() {
    let (mut session, ast) = moments_output_session(
        "proc univariate data=work.scores noprint; var score; \
         output out=mom n=n mean=mean std=std skewness=skewness kurtosis=kurtosis \
         min=min max=max; run;",
    );
    execute(&ast, &mut session).unwrap();

    let val = |col: &str| match &read_num_col(&session, "MOM", col)[0] {
        Value::Num(x) => *x,
        v => panic!("MOM.{col} expected numeric, got {v:?}"),
    };

    assert_eq!(val("n"), 12.0);
    assert!((val("mean") - 77.666_666_666_666_67).abs() < 1e-12);
    assert!((val("std") - 12.108_098_967_470_518).abs() < 1e-12);
    assert!((val("skewness") - (-0.308_929_911_950_811_2)).abs() < 1e-12);
    assert!((val("kurtosis") - (-1.044_211_980_358_745)).abs() < 1e-12);
    assert_eq!(val("min"), 57.0);
    assert_eq!(val("max"), 95.0);

    let (ds, _) = session.libs.get("WORK").unwrap().read("MOM").unwrap();
    assert_eq!(ds.n_obs(), 1);
    let names: Vec<&str> = ds.vars.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["n", "mean", "std", "skewness", "kurtosis", "min", "max"]
    );
    let log = session.log.into_string();
    assert!(
        log.contains("The data set WORK.MOM has 1 observations and 7 variables."),
        "log: {log}"
    );
    assert!(!log.contains("ERROR:"), "log: {log}");
}

#[test]
fn output_skewness_kurtosis_aliases_accepted() {
    // SKEW=/KURT= sont des alias valides ; le rejet précédent
    // (« Unsupported statistic 'SKEWNESS' ») est levé.
    let (mut session, ast) = moments_output_session(
        "proc univariate data=work.scores noprint; var score; \
         output out=mom2 skew=sk kurt=ku; run;",
    );
    execute(&ast, &mut session).unwrap();
    let sk = match &read_num_col(&session, "MOM2", "sk")[0] {
        Value::Num(x) => *x,
        v => panic!("sk expected numeric, got {v:?}"),
    };
    let ku = match &read_num_col(&session, "MOM2", "ku")[0] {
        Value::Num(x) => *x,
        v => panic!("ku expected numeric, got {v:?}"),
    };
    assert!((sk - (-0.308_929_911_950_811_2)).abs() < 1e-12, "sk = {sk}");
    assert!((ku - (-1.044_211_980_358_745)).abs() < 1e-12, "ku = {ku}");
}
