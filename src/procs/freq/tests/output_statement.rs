//! J02-P1 (issue #16) / J02-P2 (issue #17) — non-régression du statement
//! OUTPUT OUT= : parsing (`out=` + `chisq`/`fisher`, EXACT rejeté
//! explicitement) et datasets résultats (_PCHI_, _PCHI_DF_, P_PCHI ;
//! XP2_FISH, LXP2_FISH) pour une table 2x2 pondérée (oracles conformance
//! base/freq-chisq-output : chi² = 10/3, df = 1, p = 0.06788915486182903 —
//! décision 8be00f84 ; stat/freq-fisher-2x2 : p exacte = 0.16980233388902102,
//! ln(p) = -1.7731202602698983).

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
fn output_statement_fisher_parsed() {
    // J02-P2 (issue #17) — FISHER est désormais un mot-clé statistique
    // honoré du statement OUTPUT (XP2_FISH / LXP2_FISH) ; EXACT reste
    // rejeté explicitement (ce n'est pas un mot-clé OUTPUT).
    let ast = ast_of("proc freq data=work.t; tables a*b / fisher; output out=s chisq fisher; run;")
        .unwrap();
    let o = ast.output.expect("output statement parsed");
    assert!(o.chisq);
    assert!(o.fisher);

    let Err(err) = ast_of("proc freq data=work.t; tables a*b; output out=s exact; run;") else {
        panic!("expected a parse error")
    };
    assert!(
        err.to_string()
            .contains("EXACT is not an OUTPUT statement statistic keyword"),
        "msg: {err}"
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

// ───────────── J02-P2 (issue #17) — OUTPUT FISHER ─────────────

fn fisher_output_session() -> (Session, FreqAst) {
    let mut session = make_session();
    // Weighted 2x2 table [[8,2],[4,6]] (n = 20) — oracle conformance
    // stat/freq-fisher-2x2.
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
        "proc freq data=work.two; weight n; tables treatment*result / fisher; output out=fs fisher; run;",
    )
    .unwrap();
    (session, ast)
}

#[test]
fn fisher_output_dataset_values_match_oracle() {
    let (mut session, ast) = fisher_output_session();
    execute(&ast, &mut session).unwrap();

    let xp2 = read_col(&session, "FS", "XP2_FISH");
    let lxp2 = read_col(&session, "FS", "LXP2_FISH");
    let num = |v: &Value| match v {
        Value::Num(x) => Some(*x),
        _ => None,
    };
    let p = xp2.iter().filter_map(num).next().expect("XP2_FISH present");
    let lp = lxp2
        .iter()
        .filter_map(num)
        .next()
        .expect("LXP2_FISH present");

    // Oracle stat/freq-fisher-2x2 (hypergéométrique exacte, Python 3) ; le
    // calcul sasrs passe par ln-choose → écart ~1e-12, couvert par la
    // tolérance conformance du cas (1e-9).
    assert!(
        (p - 0.169_802_333_889_021_02).abs() < 1e-9,
        "XP2_FISH = {p}"
    );
    assert!(
        (lp - (-1.773_120_260_269_898_3)).abs() < 1e-9,
        "LXP2_FISH = {lp}"
    );
    assert!((lp - p.ln()).abs() < 1e-12, "LXP2_FISH = ln(XP2_FISH)");
}

#[test]
fn fisher_output_dataset_layout_and_note() {
    let (mut session, ast) = fisher_output_session();
    execute(&ast, &mut session).unwrap();

    let (ds, _) = session.libs.get("WORK").unwrap().read("FS").unwrap();
    assert_eq!(ds.n_obs(), 1);
    let names: Vec<&str> = ds.vars.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, vec!["XP2_FISH", "LXP2_FISH"]);
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.FS"));

    let log = session.log.current_text();
    assert!(
        log.contains("The data set WORK.FS has 1 observations and 2 variables."),
        "log: {log}"
    );
    assert!(!log.contains("ERROR:"), "log: {log}");
}

#[test]
fn fisher_output_one_way_is_an_error() {
    let (mut session, _) = fisher_output_session();
    let ast = ast_of("proc freq data=work.two; tables n; output out=fs fisher; run;").unwrap();
    let Err(err) = execute(&ast, &mut session) else {
        panic!("expected a runtime error")
    };
    assert!(
        err.to_string()
            .contains("FISHER statistic of the PROC FREQ OUTPUT statement requires a two-way"),
        "err: {err}"
    );
}

#[test]
fn fisher_output_listing_unchanged_by_output_statement() {
    let (mut session, ast) = fisher_output_session();
    execute(&ast, &mut session).unwrap();
    let with_output = session.listing.take_string();

    let (mut session, _) = fisher_output_session();
    let ast = ast_of("proc freq data=work.two; weight n; tables treatment*result / fisher; run;")
        .unwrap();
    execute(&ast, &mut session).unwrap();
    let without = session.listing.take_string();

    assert_eq!(with_output, without);
}
