//! J02-P1 (issue #16) — non-régression du statement OUTPUT OUT= :
//! parsing (`out=` + `chisq`, FISHER/EXACT rejetés explicitement) et
//! dataset résultat (_PCHI_, _PCHI_DF_, P_PCHI) pour une table 2x2 pondérée
//! (oracle conformance base/freq-chisq-output : chi² = 10/3, df = 1,
//! p = 0.06788915486182903 — décision 8be00f84).

use super::*;

/// Parse a full PROC FREQ step and return its FreqAst.
fn ast_of(src: &str) -> Result<FreqAst> {
    parse_freq(src)
}

fn chisq_output_session() -> (Session, FreqAst) {
    let mut session = make_session();
    // Weighted 2x2 table [[8,2],[4,6]] (n = 20).
    let df = df![
        "treatment" => ["active", "active", "placebo", "placebo"],
        "result" => ["success", "failure", "success", "failure"],
        "n" => [8.0_f64, 2.0, 4.0, 6.0],
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![
            char_meta("treatment", 8),
            char_meta("result", 8),
            num_meta("n"),
        ],
    };
    write_dataset(&mut session, "TWO", ds);
    let ast = ast_of(
        "proc freq data=work.two; weight n; tables treatment*result / chisq; output out=stats chisq; run;",
    )
    .unwrap();
    (session, ast)
}

#[test]
fn output_statement_parsed_with_out_and_chisq() {
    let ast = ast_of("proc freq data=work.t; tables a*b / chisq; output out=lib.stats chisq; run;")
        .unwrap();
    let o = ast.output.expect("output statement parsed");
    assert_eq!(o.out.libref, Some("lib".to_string()));
    assert_eq!(o.out.name.to_uppercase(), "STATS");
    assert!(o.chisq);
}

#[test]
fn output_statement_fisher_rejected_explicitly() {
    let Err(err) = ast_of("proc freq data=work.t; tables a*b; output out=s chisq fisher; run;")
    else {
        panic!("expected a parse error")
    };
    let msg = err.to_string();
    assert!(
        msg.contains("FISHER/EXACT statistics are not available"),
        "msg: {msg}"
    );
}

#[test]
fn output_statement_requires_out() {
    let Err(err) = ast_of("proc freq data=work.t; tables a*b; output chisq; run;") else {
        panic!("expected a parse error")
    };
    assert!(
        err.to_string().contains("requires the OUT= option"),
        "err: {err}"
    );
}

#[test]
fn output_statement_requires_statistic_keyword() {
    let Err(err) = ast_of("proc freq data=work.t; tables a*b; output out=s; run;") else {
        panic!("expected a parse error")
    };
    assert!(
        err.to_string()
            .contains("requires at least one statistic keyword"),
        "err: {err}"
    );
}

#[test]
fn output_statement_without_tables_is_an_error() {
    let Err(err) = ast_of("proc freq data=work.t; output out=s chisq; run;") else {
        panic!("expected a parse error")
    };
    assert!(
        err.to_string()
            .contains("requires a preceding TABLES statement"),
        "err: {err}"
    );
}

#[test]
fn chisq_output_dataset_values_match_oracle() {
    let (mut session, ast) = chisq_output_session();
    execute(&ast, &mut session).unwrap();

    let pchi = read_col(&session, "STATS", "_PCHI_");
    let df_v = read_col(&session, "STATS", "_PCHI_DF_");
    let p = read_col(&session, "STATS", "P_PCHI");

    let num = |v: &Value| match v {
        Value::Num(x) => Some(*x),
        _ => None,
    };
    let x = pchi.iter().filter_map(num).next().expect("_PCHI_ present");
    let d = df_v
        .iter()
        .filter_map(num)
        .next()
        .expect("_PCHI_DF_ present");
    let pv = p.iter().filter_map(num).next().expect("P_PCHI present");

    assert!((x - 10.0_f64 / 3.0).abs() < 1e-12, "_PCHI_ = {x}");
    assert!((d - 1.0).abs() < 1e-12, "_PCHI_DF_ = {d}");
    assert!(
        (pv - 0.067_889_154_861_829_03).abs() < 1e-12,
        "P_PCHI = {pv}"
    );
}

#[test]
fn chisq_output_dataset_has_single_observation_and_note() {
    let (mut session, ast) = chisq_output_session();
    execute(&ast, &mut session).unwrap();

    let (ds, _) = session.libs.get("WORK").unwrap().read("STATS").unwrap();
    assert_eq!(ds.n_obs(), 1);
    let names: Vec<&str> = ds.vars.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, vec!["_PCHI_", "_PCHI_DF_", "P_PCHI"]);
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.STATS"));

    let log = session.log.current_text();
    assert!(
        log.contains("The data set WORK.STATS has 1 observations and 3 variables."),
        "log: {log}"
    );
    assert!(!log.contains("ERROR:"), "log: {log}");
}

#[test]
fn chisq_output_listing_unchanged_by_output_statement() {
    // The OUTPUT statement must not alter the listing (same CHISQ block as
    // without it).
    let (mut session, ast) = chisq_output_session();
    execute(&ast, &mut session).unwrap();
    let with_output = session.listing.take_string();

    let mut session = make_session();
    let df = df![
        "treatment" => ["active", "active", "placebo", "placebo"],
        "result" => ["success", "failure", "success", "failure"],
        "n" => [8.0_f64, 2.0, 4.0, 6.0],
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![
            char_meta("treatment", 8),
            char_meta("result", 8),
            num_meta("n"),
        ],
    };
    write_dataset(&mut session, "TWO", ds);
    let ast =
        ast_of("proc freq data=work.two; weight n; tables treatment*result / chisq; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let without = session.listing.take_string();

    assert_eq!(with_output, without);
}
