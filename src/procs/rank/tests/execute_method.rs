use super::super::*;
use super::*;
use crate::dataset::SasDataset;
use polars::df;

// ───────────── execute tests ─────────────

#[test]
fn execute_replace_no_ranks() {
    let mut session = make_session();
    let df = df!["x" => [30.0_f64, 10.0, 20.0], "g" => ["a", "b", "c"]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), char_meta("g")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = RankAst {
        data: Some(dref("T")),
        out: None,
        descending: false,
        ties: Ties::Mean,
        groups: None,
        method: Method::Rank,
        by: vec![],
        var: vec!["x".into()],
        ranks: vec![],
    };
    execute(&ast, &mut session).unwrap();

    // x replaced with ranks; g unchanged; no new column.
    let (out, _) = session.libs.get("WORK").unwrap().read("T").unwrap();
    assert_eq!(out.vars.len(), 2);
    let x = read_num_col(&session, "T", "x");
    assert_eq!(x, nums(&[3.0, 1.0, 2.0]));
    let g: Vec<String> = out
        .df
        .column("g")
        .unwrap()
        .str()
        .unwrap()
        .iter()
        .map(|o| o.unwrap().to_string())
        .collect();
    assert_eq!(g, vec!["a", "b", "c"]);
}

#[test]
fn execute_ranks_appends_new_columns() {
    let mut session = make_session();
    let df = df!["x" => [30.0_f64, 10.0, 20.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = RankAst {
        data: Some(dref("T")),
        out: Some(dref("O")),
        descending: false,
        ties: Ties::Mean,
        groups: None,
        method: Method::Rank,
        by: vec![],
        var: vec!["x".into()],
        ranks: vec!["rx".into()],
    };
    execute(&ast, &mut session).unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    // Original x preserved + new rx appended.
    assert_eq!(out.vars.len(), 2);
    let x = read_num_col(&session, "O", "x");
    assert_eq!(x, nums(&[30.0, 10.0, 20.0]));
    let rx = read_num_col(&session, "O", "rx");
    assert_eq!(rx, nums(&[3.0, 1.0, 2.0]));
}

#[test]
fn execute_ranks_length_mismatch_errors() {
    let mut session = make_session();
    let df = df!["x" => [1.0_f64, 2.0], "y" => [3.0_f64, 4.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = RankAst {
        data: Some(dref("T")),
        out: None,
        descending: false,
        ties: Ties::Mean,
        groups: None,
        method: Method::Rank,
        by: vec![],
        var: vec!["x".into(), "y".into()],
        ranks: vec!["rx".into()], // only one name for two vars
    };
    let r = execute(&ast, &mut session);
    assert!(r.is_err());
    assert!(r.err().unwrap().to_string().contains("RANKS"));
}

#[test]
fn execute_default_var_all_numeric() {
    let mut session = make_session();
    let df = df![
        "x" => [30.0_f64, 10.0, 20.0],
        "g" => ["a", "b", "c"],
        "y" => [1.0_f64, 3.0, 2.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), char_meta("g"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = RankAst {
        data: Some(dref("T")),
        out: None,
        descending: false,
        ties: Ties::Mean,
        groups: None,
        method: Method::Rank,
        by: vec![],
        var: vec![], // default: all numerics (x, y), not g
        ranks: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let x = read_num_col(&session, "T", "x");
    assert_eq!(x, nums(&[3.0, 1.0, 2.0]));
    let y = read_num_col(&session, "T", "y");
    assert_eq!(y, nums(&[1.0, 3.0, 2.0]));
}

#[test]
fn execute_missing_rank_and_note() {
    let mut session = make_session();
    let df = df!["x" => [Some(10.0_f64), None, Some(30.0), Some(20.0)]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = RankAst {
        data: Some(dref("T")),
        out: None,
        descending: false,
        ties: Ties::Mean,
        groups: None,
        method: Method::Rank,
        by: vec![],
        var: vec!["x".into()],
        ranks: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let x = read_num_col(&session, "T", "x");
    assert_eq!(x[0], Value::Num(1.0));
    assert!(x[1].is_missing());
    assert_eq!(x[2], Value::Num(3.0));
    assert_eq!(x[3], Value::Num(2.0));

    let log = session.log.into_string();
    assert!(
        log.contains("The data set WORK.T has 4 observations and 1 variables."),
        "log: {log}"
    );
}

#[test]
fn execute_groups_output() {
    let mut session = make_session();
    let df = df!["x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = RankAst {
        data: Some(dref("T")),
        out: Some(dref("O")),
        descending: false,
        ties: Ties::Mean,
        groups: Some(4),
        method: Method::Rank,
        by: vec![],
        var: vec!["x".into()],
        ranks: vec!["grp".into()],
    };
    execute(&ast, &mut session).unwrap();

    let grp = read_num_col(&session, "O", "grp");
    let expected: Vec<f64> = (1..=10).map(|r| ((4 * r) / 11).min(3) as f64).collect();
    assert_eq!(grp, nums(&expected));
}

#[test]
fn execute_out_omitted_overwrites_input() {
    let mut session = make_session();
    let df = df!["x" => [30.0_f64, 10.0, 20.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = RankAst {
        data: Some(dref("T")),
        out: None,
        descending: false,
        ties: Ties::Mean,
        groups: None,
        method: Method::Rank,
        by: vec![],
        var: vec!["x".into()],
        ranks: vec![],
    };
    execute(&ast, &mut session).unwrap();

    // Input WORK.T overwritten in place; last_dataset points at it.
    let x = read_num_col(&session, "T", "x");
    assert_eq!(x, nums(&[3.0, 1.0, 2.0]));
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.T"));
}

// ───────────── BY execute tests (hand-verified) ─────────────

#[test]
fn execute_by_independent_groups() {
    let mut session = make_session();
    // Two BY groups (g=a: 10,30,20 ; g=b: 5,15). Ranks recomputed per group.
    let df = df![
        "g" => ["a", "a", "a", "b", "b"],
        "x" => [10.0_f64, 30.0, 20.0, 5.0, 15.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("g"), num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = RankAst {
        data: Some(dref("T")),
        out: Some(dref("O")),
        descending: false,
        ties: Ties::Mean,
        groups: None,
        method: Method::Rank,
        by: vec![("g".into(), false)],
        var: vec!["x".into()],
        ranks: vec!["rx".into()],
    };
    execute(&ast, &mut session).unwrap();

    let rx = read_num_col(&session, "O", "rx");
    // Group a: 10→1, 30→3, 20→2. Group b: 5→1, 15→2.
    assert_eq!(rx, nums(&[1.0, 3.0, 2.0, 1.0, 2.0]));
}

#[test]
fn execute_by_fraction_per_group() {
    let mut session = make_session();
    // Group a has 2 obs, group b has 3 obs → different denominators.
    let df = df![
        "g" => ["a", "a", "b", "b", "b"],
        "x" => [10.0_f64, 20.0, 10.0, 20.0, 30.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("g"), num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = RankAst {
        data: Some(dref("T")),
        out: Some(dref("O")),
        descending: false,
        ties: Ties::Mean,
        groups: None,
        method: Method::Fraction,
        by: vec![("g".into(), false)],
        var: vec!["x".into()],
        ranks: vec!["fx".into()],
    };
    execute(&ast, &mut session).unwrap();

    let fx = read_num_col(&session, "O", "fx");
    // a: 1/2, 2/2 ; b: 1/3, 2/3, 3/3.
    approx_eq(&fx, &[0.5, 1.0, 1.0 / 3.0, 2.0 / 3.0, 1.0]);
}

#[test]
fn execute_by_not_sorted_errors() {
    let mut session = make_session();
    // BY key not sorted (a, b, a) → by_groups must error.
    let df = df![
        "g" => ["a", "b", "a"],
        "x" => [10.0_f64, 20.0, 30.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("g"), num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = RankAst {
        data: Some(dref("T")),
        out: None,
        descending: false,
        ties: Ties::Mean,
        groups: None,
        method: Method::Rank,
        by: vec![("g".into(), false)],
        var: vec!["x".into()],
        ranks: vec![],
    };
    let r = execute(&ast, &mut session);
    assert!(r.is_err());
    assert!(r.err().unwrap().to_string().contains("not sorted"));
}

#[test]
fn method_fraction() {
    let data = nums(&[10.0, 20.0, 30.0, 40.0]);
    let out = rank_column(&data, false, Ties::Mean, None, Method::Fraction);
    approx_eq(&out, &[0.25, 0.50, 0.75, 1.00]);
}

#[test]
fn method_nplus1() {
    let data = nums(&[10.0, 20.0, 30.0, 40.0]);
    let out = rank_column(&data, false, Ties::Mean, None, Method::NPlus1);
    approx_eq(&out, &[0.2, 0.4, 0.6, 0.8]);
}

#[test]
fn method_percent() {
    let data = nums(&[10.0, 20.0, 30.0, 40.0]);
    let out = rank_column(&data, false, Ties::Mean, None, Method::Percent);
    approx_eq(&out, &[25.0, 50.0, 75.0, 100.0]);
}

#[test]
fn method_normal_blom() {
    // y = (r - 0.375)/4.25 for r=1..4, then Phi^-1.
    let data = nums(&[10.0, 20.0, 30.0, 40.0]);
    let out = rank_column(
        &data,
        false,
        Ties::Mean,
        None,
        Method::Normal(NormalScore::Blom),
    );
    let exp: Vec<f64> = (1..=4)
        .map(|r| phi_inv((r as f64 - 0.375) / 4.25))
        .collect();
    approx_eq(&out, &exp);
    // BLOM scores are antisymmetric for symmetric ranks: s1 = -s4, s2 = -s3.
    if let (Value::Num(a), Value::Num(d)) = (&out[0], &out[3]) {
        assert!((a + d).abs() < 1e-9);
    }
}

#[test]
fn method_normal_vw() {
    // van der Waerden: y = r/(n+1) = r/5.
    let data = nums(&[10.0, 20.0, 30.0, 40.0]);
    let out = rank_column(
        &data,
        false,
        Ties::Mean,
        None,
        Method::Normal(NormalScore::Vw),
    );
    let exp: Vec<f64> = (1..=4).map(|r| phi_inv(r as f64 / 5.0)).collect();
    approx_eq(&out, &exp);
}

#[test]
fn method_savage_no_ties() {
    // n=4: s_m = (sum_{j=n-m+1}^{n} 1/j) - 1.
    let data = nums(&[10.0, 20.0, 30.0, 40.0]);
    let out = rank_column(&data, false, Ties::Mean, None, Method::Savage);
    let s1 = 1.0 / 4.0 - 1.0;
    let s2 = 1.0 / 4.0 + 1.0 / 3.0 - 1.0;
    let s3 = 1.0 / 4.0 + 1.0 / 3.0 + 1.0 / 2.0 - 1.0;
    let s4 = 1.0 / 4.0 + 1.0 / 3.0 + 1.0 / 2.0 + 1.0 - 1.0;
    approx_eq(&out, &[s1, s2, s3, s4]);
    // Savage scores sum to ~0.
    let sum: f64 = out
        .iter()
        .map(|v| if let Value::Num(x) = v { *x } else { 0.0 })
        .sum();
    assert!(sum.abs() < 1e-9);
}

#[test]
fn method_savage_ties_mean() {
    // Two tied 20s occupy ordinals 2 and 3; MEAN → average of s2 and s3.
    let data = nums(&[10.0, 20.0, 20.0, 40.0]);
    let out = rank_column(&data, false, Ties::Mean, None, Method::Savage);
    let s2 = 1.0 / 4.0 + 1.0 / 3.0 - 1.0;
    let s3 = 1.0 / 4.0 + 1.0 / 3.0 + 1.0 / 2.0 - 1.0;
    let mid = (s2 + s3) / 2.0;
    if let (Value::Num(a), Value::Num(b)) = (&out[1], &out[2]) {
        assert!((a - mid).abs() < 1e-9 && (b - mid).abs() < 1e-9);
    } else {
        panic!("ties not numeric");
    }
}

#[test]
fn method_fraction_with_ties() {
    // Ties::Mean rank of two 20s is 2.5 → fraction 2.5/4 = 0.625.
    let data = nums(&[10.0, 20.0, 20.0, 40.0]);
    let out = rank_column(&data, false, Ties::Mean, None, Method::Fraction);
    approx_eq(&out, &[0.25, 0.625, 0.625, 1.0]);
}

#[test]
fn method_empty_column_no_panic() {
    let data = vec![Value::missing(), Value::missing()];
    let out = rank_column(&data, false, Ties::Mean, None, Method::Fraction);
    assert!(out.iter().all(|v| v.is_missing()));
    let out2 = rank_column(&data, false, Ties::Mean, None, Method::Savage);
    assert!(out2.iter().all(|v| v.is_missing()));
}
