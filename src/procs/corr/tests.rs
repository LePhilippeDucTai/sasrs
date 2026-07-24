use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::session::Session;
use crate::source::SourceFile;
use crate::value::VarType;
use polars::df;
use std::path::PathBuf;

fn make_session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

fn num_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}

fn char_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Char,
        length: 4,
        format: None,
        label: None,
    }
}

fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
    session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
    session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
}

fn parse_corr(src: &str) -> Result<CorrAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "corr"
    parse(&mut ts)
}

// ───────────── parse tests ─────────────

#[test]
fn parse_minimal() {
    let ast = parse_corr("proc corr data=a; run;").unwrap();
    assert_eq!(ast.data.as_ref().unwrap().name, "a");
    assert!(!ast.nosimple && !ast.noprob && !ast.nocorr);
    assert!(ast.var.is_empty() && ast.with.is_empty());
}

#[test]
fn parse_options_and_statements() {
    let ast =
        parse_corr("proc corr data=a nosimple noprob nocorr; var x y; with z; run;").unwrap();
    assert!(ast.nosimple && ast.noprob && ast.nocorr);
    assert_eq!(ast.var, vec!["x", "y"]);
    assert_eq!(ast.with, vec!["z"]);
}

#[test]
fn parse_unknown_option_errors() {
    let r = parse_corr("proc corr data=a bogus; run;");
    assert!(r.is_err());
    assert!(r.err().unwrap().to_string().contains("BOGUS"));
}

#[test]
fn parse_out_options() {
    let ast = parse_corr(
        "proc corr data=a out=p outs=s outk=k spearman kendall pearson; run;",
    )
    .unwrap();
    assert_eq!(ast.outp.as_ref().unwrap().name, "p");
    assert_eq!(ast.outs.as_ref().unwrap().name, "s");
    assert_eq!(ast.outk.as_ref().unwrap().name, "k");
    assert!(ast.pearson && ast.spearman && ast.kendall);
}

#[test]
fn parse_outp_and_weight() {
    let ast = parse_corr("proc corr data=a outp=b; var x y; weight wt; run;").unwrap();
    assert_eq!(ast.outp.as_ref().unwrap().name, "b");
    assert_eq!(ast.weight.as_deref(), Some("wt"));
    // Default method selection: pearson wanted, spearman/kendall not.
    assert!(ast.want_pearson() && !ast.spearman && !ast.kendall);
}

// ───────────── numeric core tests ─────────────

#[test]
fn pearson_perfect_positive() {
    let x: Vec<Value> = [1.0, 2.0, 3.0, 4.0].iter().map(|v| Value::Num(*v)).collect();
    let y: Vec<Value> = [2.0, 4.0, 6.0, 8.0].iter().map(|v| Value::Num(*v)).collect();
    let (r, n) = pearson(&x, &y);
    assert_eq!(n, 4);
    assert!((r.unwrap() - 1.0).abs() < 1e-12);
}

#[test]
fn pearson_perfect_negative() {
    let x: Vec<Value> = [1.0, 2.0, 3.0].iter().map(|v| Value::Num(*v)).collect();
    let y: Vec<Value> = [3.0, 2.0, 1.0].iter().map(|v| Value::Num(*v)).collect();
    let (r, _) = pearson(&x, &y);
    assert!((r.unwrap() + 1.0).abs() < 1e-12);
}

#[test]
fn pearson_hand_computed() {
    // x=[1,2,3,5], y=[2,1,4,3].
    let x: Vec<Value> = [1.0, 2.0, 3.0, 5.0].iter().map(|v| Value::Num(*v)).collect();
    let y: Vec<Value> = [2.0, 1.0, 4.0, 3.0].iter().map(|v| Value::Num(*v)).collect();
    let (r, n) = pearson(&x, &y);
    assert_eq!(n, 4);
    // mx=2.75, my=2.5; Sxy=3.5, Sxx=8.75, Syy=5 -> r = 3.5/sqrt(43.75) = 0.52915026
    assert!((r.unwrap() - 0.52915026).abs() < 1e-6, "r={:?}", r);
}

#[test]
fn pearson_constant_is_missing() {
    let x: Vec<Value> = [5.0, 5.0, 5.0].iter().map(|v| Value::Num(*v)).collect();
    let y: Vec<Value> = [1.0, 2.0, 3.0].iter().map(|v| Value::Num(*v)).collect();
    let (r, n) = pearson(&x, &y);
    assert_eq!(n, 3);
    assert!(r.is_none());
}

#[test]
fn partial_pearson_matches_single_control_formula() {
    // Oracle: for ONE control variable z, the partial correlation
    //   r_xy.z = (r_xy − r_xz·r_yz) / sqrt((1−r_xz²)(1−r_yz²))
    // must equal the residual-method result computed by
    // `partial_pearson_matrix`. Two independent derivations agreeing.
    let xf = [2.0, 4.0, 5.0, 4.0, 7.0, 8.0];
    let yf = [1.0, 3.0, 4.0, 6.0, 7.0, 9.0];
    let zf = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let r_xy = pearson_xy(&xf, &yf).unwrap();
    let r_xz = pearson_xy(&xf, &zf).unwrap();
    let r_yz = pearson_xy(&yf, &zf).unwrap();
    let expected =
        (r_xy - r_xz * r_yz) / ((1.0 - r_xz * r_xz) * (1.0 - r_yz * r_yz)).sqrt();

    let to_col = |v: &[f64]| -> Vec<Value> { v.iter().map(|f| Value::Num(*f)).collect() };
    let mut decoded: std::collections::HashMap<usize, Vec<Value>> =
        std::collections::HashMap::new();
    decoded.insert(0, to_col(&xf)); // X
    decoded.insert(1, to_col(&yf)); // Y
    decoded.insert(2, to_col(&zf)); // Z (control)

    let cells = partial_pearson_matrix(&[0], &[1], &[2], &decoded);
    let got = cells[0][0].r.unwrap();
    assert!(
        (got - expected).abs() < 1e-9,
        "partial r mismatch: got {got}, expected {expected}"
    );
    assert_eq!(cells[0][0].n, 6); // listwise-complete count
    // df = n − k − 2 = 6 − 1 − 2 = 3 → p-value present and in (0,1].
    let p = cells[0][0].p.unwrap();
    assert!(p > 0.0 && p <= 1.0, "p out of range: {p}");
}

#[test]
fn partial_pearson_diagonal_is_one() {
    let xf = [1.0, 2.0, 3.0, 4.0, 5.0];
    let zf = [2.0, 1.0, 4.0, 3.0, 6.0];
    let to_col = |v: &[f64]| -> Vec<Value> { v.iter().map(|f| Value::Num(*f)).collect() };
    let mut decoded: std::collections::HashMap<usize, Vec<Value>> =
        std::collections::HashMap::new();
    decoded.insert(0, to_col(&xf));
    decoded.insert(1, to_col(&zf));
    let cells = partial_pearson_matrix(&[0], &[0], &[1], &decoded);
    assert_eq!(cells[0][0].r, Some(1.0));
    assert!(cells[0][0].p.is_none());
}

#[test]
fn pearson_pairwise_complete_n() {
    // Drop rows where either is missing.
    let x = vec![
        Value::Num(1.0),
        Value::Num(2.0),
        Value::missing(),
        Value::Num(4.0),
    ];
    let y = vec![
        Value::Num(2.0),
        Value::missing(),
        Value::Num(3.0),
        Value::Num(8.0),
    ];
    // Complete pairs: rows 0 and 3 → n=2.
    let (_r, n) = pearson(&x, &y);
    assert_eq!(n, 2);
}

#[test]
fn pvalue_approx_matches_known() {
    // r=0, any n → p = 1.0.
    assert!((pearson_pvalue(0.0, 10).unwrap() - 1.0).abs() < 1e-9);
    // n=3 df=1: r small → p near 1.
    let p = pearson_pvalue(0.5, 12).unwrap();
    // For r=0.5, n=12, df=10: t=0.5*sqrt(10/0.75)=1.8257; p≈0.0978.
    assert!((p - 0.0978).abs() < 1e-3, "p={p}");
}

#[test]
fn betai_symmetry_and_bounds() {
    // I_0 = 0, I_1 = 1.
    assert!(betai(2.0, 3.0, 0.0).abs() < 1e-12);
    assert!((betai(2.0, 3.0, 1.0) - 1.0).abs() < 1e-12);
    // I_x(a,b) + I_{1-x}(b,a) = 1.
    let s = betai(2.5, 4.0, 0.3) + betai(4.0, 2.5, 0.7);
    assert!((s - 1.0).abs() < 1e-9, "sum={s}");
}

#[test]
fn fmt_helpers() {
    assert_eq!(fmt_r(Some(1.0)), "1.00000");
    assert_eq!(fmt_r(Some(0.9583)), "0.95830");
    assert_eq!(fmt_r(None), ".");
    assert_eq!(fmt_p(Some(0.00001)), "<.0001");
    assert_eq!(fmt_p(Some(0.1234)), "0.1234");
    assert_eq!(fmt_p(None), "");
}

// ───────────── execute tests ─────────────

#[test]
fn execute_perfect_correlation_listing() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0],
        "y" => [2.0_f64, 4.0, 6.0, 8.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = CorrAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        nosimple: false,
        noprob: false,
        nocorr: false,
        var: vec![],
        with: vec![],
        partial: vec![],
        pearson: false,
        spearman: false,
        kendall: false,
        hoeffding: false,
        weight: None,
        outp: None,
        outs: None,
        outk: None,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("The CORR Procedure"), "{listing}");
    assert!(listing.contains("Simple Statistics"), "{listing}");
    assert!(listing.contains("Pearson Correlation Coefficients"), "{listing}");
    // Diagonal 1.00000 and off-diagonal 1.00000 (perfectly correlated).
    assert!(listing.contains("1.00000"), "{listing}");
    // Variable summary line.
    assert!(listing.contains("2 Variables:"), "{listing}");
}

#[test]
fn execute_nosimple_noprob_toggles() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0],
        "y" => [1.0_f64, 3.0, 2.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = CorrAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        nosimple: true,
        noprob: true,
        nocorr: false,
        var: vec!["x".into(), "y".into()],
        with: vec![],
        partial: vec![],
        pearson: false,
        spearman: false,
        kendall: false,
        hoeffding: false,
        weight: None,
        outp: None,
        outs: None,
        outk: None,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(!listing.contains("Simple Statistics"), "nosimple: {listing}");
    assert!(!listing.contains("Prob > |r|"), "noprob: {listing}");
    assert!(listing.contains("Pearson Correlation Coefficients"), "{listing}");
}

#[test]
fn execute_missing_pairwise_n_line() {
    let mut session = make_session();
    // x and y share 4 complete rows; x and z share only 3 (one missing),
    // so pairwise N differs and the N line should appear.
    let df = df![
        "x" => [Some(1.0_f64), Some(2.0), Some(3.0), Some(4.0)],
        "y" => [Some(2.0_f64), Some(1.0), Some(4.0), Some(3.0)],
        "z" => [Some(1.0_f64), None, Some(2.0), Some(5.0)]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y"), num_meta("z")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = CorrAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        nosimple: true,
        noprob: true,
        nocorr: false,
        var: vec!["x".into(), "y".into(), "z".into()],
        with: vec![],
        partial: vec![],
        pearson: false,
        spearman: false,
        kendall: false,
        hoeffding: false,
        weight: None,
        outp: None,
        outs: None,
        outk: None,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    // N line should show a "3" somewhere in the matrix region.
    assert!(listing.contains(" 3"), "expected N line with 3: {listing}");
    assert!(listing.contains(" 4"), "expected N line with 4: {listing}");
}

#[test]
fn execute_constant_variable_missing_r() {
    let mut session = make_session();
    let df = df![
        "x" => [5.0_f64, 5.0, 5.0, 5.0],
        "y" => [1.0_f64, 2.0, 3.0, 4.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = CorrAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        nosimple: true,
        noprob: true,
        nocorr: false,
        var: vec!["x".into(), "y".into()],
        with: vec![],
        partial: vec![],
        pearson: false,
        spearman: false,
        kendall: false,
        hoeffding: false,
        weight: None,
        outp: None,
        outs: None,
        outk: None,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    // Off-diagonal r between constant x and y is missing → ".".
    assert!(listing.contains(" ."), "expected missing r '.': {listing}");
}

#[test]
fn execute_with_statement_shapes_matrix() {
    let mut session = make_session();
    let df = df![
        "a" => [1.0_f64, 2.0, 3.0, 4.0],
        "b" => [4.0_f64, 3.0, 2.0, 1.0],
        "w" => [1.0_f64, 2.0, 3.0, 4.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("a"), num_meta("b"), num_meta("w")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = CorrAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        nosimple: true,
        noprob: true,
        nocorr: false,
        var: vec!["a".into(), "b".into()],
        with: vec!["w".into()],
        partial: vec![],
        pearson: false,
        spearman: false,
        kendall: false,
        hoeffding: false,
        weight: None,
        outp: None,
        outs: None,
        outk: None,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("1 With Variables:"), "{listing}");
    assert!(listing.contains("2 Variables:"), "{listing}");
    // w perfectly correlates with a (1.00000) and anti with b (-1.00000).
    assert!(listing.contains("1.00000"), "{listing}");
    assert!(listing.contains("-1.00000"), "{listing}");
}

#[test]
fn execute_default_var_all_numeric() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0],
        "g" => ["a", "b", "c"],
        "y" => [3.0_f64, 2.0, 1.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), char_meta("g"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = CorrAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        nosimple: false,
        noprob: true,
        nocorr: false,
        var: vec![],
        with: vec![],
        partial: vec![],
        pearson: false,
        spearman: false,
        kendall: false,
        hoeffding: false,
        weight: None,
        outp: None,
        outs: None,
        outk: None,
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    // Only x and y (numeric) are analyzed; char g excluded.
    assert!(listing.contains("2 Variables:"), "{listing}");
    assert!(listing.contains("x y"), "{listing}");
}

// ───────────── M21.5: Spearman / Kendall / WEIGHT / OUT= ─────────────

fn base_ast(table: &str) -> CorrAst {
    CorrAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: table.into() }),
        nosimple: true,
        noprob: false,
        nocorr: false,
        pearson: false,
        spearman: false,
        kendall: false,
        hoeffding: false,
        var: vec![],
        with: vec![],
        partial: vec![],
        weight: None,
        outp: None,
        outs: None,
        outk: None,
    }
}

fn vnum(vals: &[f64]) -> Vec<Value> {
    vals.iter().map(|v| Value::Num(*v)).collect()
}

// --- numeric core: Spearman ---

#[test]
fn spearman_perfect_monotone() {
    // Monotone but non-linear: ranks are perfectly correlated → r_s = 1.
    let x = vnum(&[1.0, 2.0, 3.0, 4.0]);
    let y = vnum(&[1.0, 4.0, 9.0, 16.0]);
    let (r, n) = spearman(&x, &y);
    assert_eq!(n, 4);
    assert!((r.unwrap() - 1.0).abs() < 1e-12, "r={r:?}");
}

#[test]
fn spearman_hand_example() {
    // [(1,1),(2,3),(3,2),(4,4)]: ranks x=[1,2,3,4], y=[1,3,2,4],
    // Pearson on ranks → r_s = 0.8.
    let x = vnum(&[1.0, 2.0, 3.0, 4.0]);
    let y = vnum(&[1.0, 3.0, 2.0, 4.0]);
    let (r, n) = spearman(&x, &y);
    assert_eq!(n, 4);
    assert!((r.unwrap() - 0.8).abs() < 1e-12, "r={r:?}");
}

#[test]
fn spearman_with_ties_uses_midranks() {
    // x=[1,1,2,3] → ranks [1.5,1.5,3,4]; y=[10,20,20,30] → [1,2.5,2.5,4].
    let x = vnum(&[1.0, 1.0, 2.0, 3.0]);
    let y = vnum(&[10.0, 20.0, 20.0, 30.0]);
    let rx = mean_ranks(&[1.0, 1.0, 2.0, 3.0]);
    assert_eq!(rx, vec![1.5, 1.5, 3.0, 4.0]);
    let (r, n) = spearman(&x, &y);
    assert_eq!(n, 4);
    // Pearson on [1.5,1.5,3,4] & [1,2.5,2.5,4]: hand → 0.9...
    assert!(r.unwrap() > 0.8 && r.unwrap() <= 1.0, "r={r:?}");
}

#[test]
fn spearman_constant_is_missing() {
    let x = vnum(&[5.0, 5.0, 5.0]);
    let y = vnum(&[1.0, 2.0, 3.0]);
    let (r, _) = spearman(&x, &y);
    assert!(r.is_none());
}

// --- numeric core: Kendall ---

#[test]
fn kendall_hand_example() {
    // [(1,1),(2,3),(3,2),(4,4)]: C=5, D=1, no ties → tau_b = (5-1)/6.
    let x = vnum(&[1.0, 2.0, 3.0, 4.0]);
    let y = vnum(&[1.0, 3.0, 2.0, 4.0]);
    let (t, n) = kendall_tau_b(&x, &y);
    assert_eq!(n, 4);
    assert!((t.unwrap() - (4.0 / 6.0)).abs() < 1e-12, "t={t:?}");
}

#[test]
fn kendall_perfect_and_anti() {
    let x = vnum(&[1.0, 2.0, 3.0, 4.0]);
    let yup = vnum(&[10.0, 20.0, 30.0, 40.0]);
    let ydn = vnum(&[40.0, 30.0, 20.0, 10.0]);
    assert!((kendall_tau_b(&x, &yup).0.unwrap() - 1.0).abs() < 1e-12);
    assert!((kendall_tau_b(&x, &ydn).0.unwrap() + 1.0).abs() < 1e-12);
}

#[test]
fn kendall_tie_b_correction() {
    // x has a tie: x=[1,1,2,3], y=[1,2,3,4]. n0=6, n1=1 (one x-tie pair),
    // n2=0. Pairs excluding tie: (concordant). C=5, D=0.
    // tau_b = (5-0)/sqrt((6-1)(6-0)) = 5/sqrt(30) ≈ 0.9128709.
    let x = vnum(&[1.0, 1.0, 2.0, 3.0]);
    let y = vnum(&[1.0, 2.0, 3.0, 4.0]);
    let (t, n) = kendall_tau_b(&x, &y);
    assert_eq!(n, 4);
    assert!((t.unwrap() - 5.0 / 30f64.sqrt()).abs() < 1e-9, "t={t:?}");
}

#[test]
fn kendall_all_tied_is_missing() {
    let x = vnum(&[2.0, 2.0, 2.0]);
    let y = vnum(&[1.0, 2.0, 3.0]);
    assert!(kendall_tau_b(&x, &y).0.is_none());
}

// --- weighted Pearson ---

#[test]
fn weighted_equals_unweighted_when_w1() {
    let x = vnum(&[1.0, 2.0, 3.0, 5.0]);
    let y = vnum(&[2.0, 1.0, 4.0, 3.0]);
    let w = vnum(&[1.0, 1.0, 1.0, 1.0]);
    let (ru, _) = pearson(&x, &y);
    let (rw, nw) = pearson_weighted(&x, &y, &w);
    assert_eq!(nw, 4);
    assert!((ru.unwrap() - rw.unwrap()).abs() < 1e-12, "{:?} {:?}", ru, rw);
}

#[test]
fn weighted_excludes_nonpositive_and_missing() {
    // Row with w=0 and row with missing w are dropped → n=2.
    let x = vnum(&[1.0, 2.0, 3.0, 4.0]);
    let y = vnum(&[2.0, 4.0, 6.0, 8.0]);
    let w = vec![Value::Num(2.0), Value::Num(0.0), Value::missing(), Value::Num(3.0)];
    let (r, n) = pearson_weighted(&x, &y, &w);
    assert_eq!(n, 2);
    // Remaining pairs perfectly correlated → r = 1.
    assert!((r.unwrap() - 1.0).abs() < 1e-12, "r={r:?}");
}

#[test]
fn weighted_changes_result() {
    // Up-weighting the well-aligned third point pulls the weighted r toward
    // 1, above the unweighted value. Hand-computed (weighted means
    // mx=2.75, my=7.75): sxy=19.25, sxx=4.25, syy=94.25 → r≈0.96183,
    // vs unweighted r=8/√76≈0.91766.
    let x = vnum(&[1.0, 2.0, 3.0]);
    let y = vnum(&[1.0, 2.0, 9.0]);
    let w = vnum(&[1.0, 1.0, 10.0]);
    let (ru, _) = pearson(&x, &y);
    let (rw, _) = pearson_weighted(&x, &y, &w);
    assert!((ru.unwrap() - 0.917663).abs() < 1e-5, "ru={ru:?}");
    assert!((rw.unwrap() - 0.961826).abs() < 1e-5, "rw={rw:?}");
    // Weighting materially changes the result (here, raises it).
    assert!(rw.unwrap() > ru.unwrap(), "ru={ru:?} rw={rw:?}");
}

// --- listing blocks ---

#[test]
fn execute_spearman_block() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0],
        "y" => [1.0_f64, 3.0, 2.0, 4.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("y")] };
    write_dataset(&mut session, "T", ds);

    let mut ast = base_ast("T");
    ast.spearman = true;
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("Spearman Correlation Coefficients"), "{listing}");
    // No Pearson block when only spearman requested.
    assert!(!listing.contains("Pearson Correlation Coefficients"), "{listing}");
    // r_s off-diagonal = 0.80000.
    assert!(listing.contains("0.80000"), "{listing}");
}

#[test]
fn execute_kendall_block() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0],
        "y" => [1.0_f64, 3.0, 2.0, 4.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("y")] };
    write_dataset(&mut session, "T", ds);

    let mut ast = base_ast("T");
    ast.kendall = true;
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("Kendall Tau b Coefficients"), "{listing}");
    assert!(listing.contains("Prob > |tau|"), "{listing}");
    // tau_b = 0.66667.
    assert!(listing.contains("0.66667"), "{listing}");
}

#[test]
fn execute_all_three_methods() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0],
        "y" => [2.0_f64, 1.0, 4.0, 3.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("y")] };
    write_dataset(&mut session, "T", ds);

    let mut ast = base_ast("T");
    ast.pearson = true;
    ast.spearman = true;
    ast.kendall = true;
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("Pearson Correlation Coefficients"), "{listing}");
    assert!(listing.contains("Spearman Correlation Coefficients"), "{listing}");
    assert!(listing.contains("Kendall Tau b Coefficients"), "{listing}");
}

// --- OUT= datasets ---

#[test]
fn execute_outp_dataset_structure() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0],
        "y" => [2.0_f64, 4.0, 6.0, 8.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("y")] };
    write_dataset(&mut session, "T", ds);

    let mut ast = base_ast("T");
    ast.outp = Some(DatasetRef { libref: Some("WORK".into()), name: "C".into() });
    execute(&ast, &mut session).unwrap();

    // Read back the produced TYPE=CORR dataset.
    let (out, _) = session.libs.get("WORK").unwrap().read("C").unwrap();
    // Columns: _TYPE_, _NAME_, x, y.
    let names: Vec<String> = out.vars.iter().map(|v| v.name.clone()).collect();
    assert_eq!(names, vec!["_TYPE_", "_NAME_", "x", "y"]);
    // 3 stats rows + 2 corr rows = 5.
    assert_eq!(out.n_obs(), 5);

    let type_col = decode_column(&out, 0).unwrap();
    let types: Vec<String> = type_col
        .iter()
        .map(|v| match v {
            Value::Char(s) => s.clone(),
            _ => String::new(),
        })
        .collect();
    assert_eq!(types, vec!["MEAN", "STD", "N", "CORR", "CORR"]);

    // _NAME_ on CORR rows = x then y; empty on stats rows.
    let name_col = decode_column(&out, 1).unwrap();
    match &name_col[3] {
        Value::Char(s) => assert_eq!(s, "x"),
        other => panic!("expected x, got {other:?}"),
    }

    // CORR row for x: r(x,x)=1, r(x,y)=1 (perfect). Column "x" idx 2.
    let xcorr = decode_column(&out, 2).unwrap();
    assert!((value_to_num(&xcorr[3]).unwrap() - 1.0).abs() < 1e-12);
    // N row value = 4.
    assert!((value_to_num(&xcorr[2]).unwrap() - 4.0).abs() < 1e-12);
    // MEAN of x = 2.5.
    assert!((value_to_num(&xcorr[0]).unwrap() - 2.5).abs() < 1e-12);

    // _LAST_ updated.
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.C"));
}

#[test]
fn execute_outs_outk_methods() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0],
        "y" => [1.0_f64, 3.0, 2.0, 4.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("y")] };
    write_dataset(&mut session, "T", ds);

    let mut ast = base_ast("T");
    ast.spearman = true;
    ast.kendall = true;
    ast.outs = Some(DatasetRef { libref: Some("WORK".into()), name: "S".into() });
    ast.outk = Some(DatasetRef { libref: Some("WORK".into()), name: "K".into() });
    execute(&ast, &mut session).unwrap();

    // Spearman OUTS: corr(x,y) row for x = 0.8.
    let (s, _) = session.libs.get("WORK").unwrap().read("S").unwrap();
    let sx = decode_column(&s, 2).unwrap(); // column x
    // row 4 (index 4) is CORR y; row 3 is CORR x → off-diag at col y.
    let sy = decode_column(&s, 3).unwrap(); // column y, CORR x row
    assert!((value_to_num(&sy[3]).unwrap() - 0.8).abs() < 1e-9, "{:?}", sy[3]);
    assert!((value_to_num(&sx[3]).unwrap() - 1.0).abs() < 1e-9);

    // Kendall OUTK: corr(x,y) = 0.6667.
    let (kd, _) = session.libs.get("WORK").unwrap().read("K").unwrap();
    let ky = decode_column(&kd, 3).unwrap();
    assert!((value_to_num(&ky[3]).unwrap() - 4.0 / 6.0).abs() < 1e-9, "{:?}", ky[3]);
}

#[test]
fn execute_weighted_listing_runs() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0],
        "y" => [2.0_f64, 4.0, 6.0, 8.0],
        "wt" => [1.0_f64, 1.0, 1.0, 1.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y"), num_meta("wt")],
    };
    write_dataset(&mut session, "T", ds);

    let mut ast = base_ast("T");
    ast.var = vec!["x".into(), "y".into()];
    ast.weight = Some("wt".into());
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    // With w=1 the weighted r equals the unweighted perfect correlation.
    assert!(listing.contains("1.00000"), "{listing}");
    assert!(listing.contains("Pearson Correlation Coefficients"), "{listing}");
}

// ───────────── M34.1: Hoeffding D + weighted Spearman / Kendall ─────────

/// Replicate `xs` according to integer weights `ws` (oracle helper).
fn replicate(xs: &[f64], ws: &[usize]) -> Vec<f64> {
    let mut out = Vec::new();
    for (&x, &w) in xs.iter().zip(ws) {
        for _ in 0..w {
            out.push(x);
        }
    }
    out
}

#[test]
fn hoeffding_perfect_monotone_n5() {
    // x=1..5, y=1..5: a strictly increasing pair. By hand, all ranks and
    // bivariate counts coincide: R_i = S_i = Q_i = i (1..5). Then
    //   D1 = Σ(Q-1)(Q-2) = 0+0+2+6+12 = 20
    //   D2 = Σ(R-1)(R-2)(S-1)(S-2) = 0+0+4+36+144 = 184
    //   D3 = Σ(R-2)(S-2)(Q-1) = 1·1·0 ... = 0+0+2+18+72 = 92  (wait: see below)
    // With n=5: den = 5·4·3·2·1 = 120; num = 3·2·20 + 184 − 2·3·92
    //   = 120 + 184 − 552 = -248? Recompute D3 carefully in the assertion.
    // We assert the closed value D = 1.0 (perfect monotone dependence, n=5).
    let x = vnum(&[1.0, 2.0, 3.0, 4.0, 5.0]);
    let y = vnum(&[1.0, 2.0, 3.0, 4.0, 5.0]);
    let (d, n) = hoeffding_d(&x, &y);
    assert_eq!(n, 5);
    assert!((d.unwrap() - 1.0).abs() < 1e-12, "D={d:?}");
}

#[test]
fn hoeffding_hand_arithmetic_n5() {
    // Spell out the arithmetic for x=1..5, y=1..5 to validate the formula.
    // R = S = Q = [1,2,3,4,5].
    // D1 = Σ(Q-1)(Q-2) = (0)(−1)+(1)(0)+(2)(1)+(3)(2)+(4)(3) = 0+0+2+6+12 = 20.
    // D2 = Σ(R-1)(R-2)(S-1)(S-2): for i, ((R-1)(R-2))² since R=S.
    //   R=1→0, R=2→0, R=3→(2·1)²=4, R=4→(3·2)²=36, R=5→(4·3)²=144 ⇒ 184.
    // D3 = Σ(R-2)(S-2)(Q-1) = Σ(R-2)²(R-1) (R=S=Q):
    //   R=1→(−1)²·0=0, R=2→0·1=0, R=3→1·2=2, R=4→4·3=12, R=5→9·4=36 ⇒ 50.
    // num = (n-2)(n-3)D1 + D2 − 2(n-2)D3 = 3·2·20 + 184 − 2·3·50
    //     = 120 + 184 − 300 = 4.  den = 5·4·3·2·1 = 120.
    // D = 30·4/120 = 1.0.
    let d1 = 20.0_f64;
    let d2 = 184.0_f64;
    let d3 = 50.0_f64;
    let num = 3.0 * 2.0 * d1 + d2 - 2.0 * 3.0 * d3;
    let den = 5.0 * 4.0 * 3.0 * 2.0 * 1.0;
    let d = 30.0 * num / den;
    assert!((d - 1.0).abs() < 1e-12, "hand D={d}");
    // And the implementation agrees.
    let x = vnum(&[1.0, 2.0, 3.0, 4.0, 5.0]);
    let y = vnum(&[1.0, 2.0, 3.0, 4.0, 5.0]);
    assert!((hoeffding_d(&x, &y).0.unwrap() - d).abs() < 1e-12);
}

#[test]
fn hoeffding_requires_n5() {
    let x = vnum(&[1.0, 2.0, 3.0, 4.0]);
    let y = vnum(&[1.0, 2.0, 3.0, 4.0]);
    let (d, n) = hoeffding_d(&x, &y);
    assert_eq!(n, 4);
    assert!(d.is_none());
}

#[test]
fn hoeffding_matches_sas_class_oracle() {
    // sashelp.class height × weight (19 obs) → SAS PROC CORR HOEFFDING
    // reports D = 0.31609 (exact to 5 decimals).
    let h = vnum(&[
        69.0, 56.5, 65.3, 62.8, 63.5, 57.3, 59.8, 62.5, 62.5, 59.0, 51.3, 64.3, 56.3, 66.5,
        72.0, 64.8, 67.0, 57.5, 66.5,
    ]);
    let w = vnum(&[
        112.5, 84.0, 98.0, 102.5, 102.5, 83.0, 84.5, 112.5, 84.0, 99.5, 50.5, 90.0, 77.0,
        112.0, 150.0, 128.0, 133.0, 85.0, 112.0,
    ]);
    let (d, n) = hoeffding_d(&h, &w);
    assert_eq!(n, 19);
    assert!((d.unwrap() - 0.31609).abs() < 5e-6, "D={d:?}");
}

#[test]
fn hoeffding_pvalue_in_range() {
    // Strong dependence → small Prob > D; independence-ish → larger.
    let p = hoeffding_pvalue(0.31609, 19).unwrap();
    assert!(p > 0.0 && p < 0.01, "p={p}");
    let p0 = hoeffding_pvalue(0.0, 19).unwrap();
    assert!(p0 > 0.5, "p0={p0}");
}

#[test]
fn weighted_spearman_equals_replicated() {
    // Oracle: weighted Spearman with integer weights {2,1,1,1} equals
    // ordinary Spearman on the {2,1,1,1}-replicated data.
    let xf = [1.0, 2.0, 3.0, 4.0];
    let yf = [2.0, 1.0, 4.0, 3.0];
    let ws = [2usize, 1, 1, 1];
    let x = vnum(&xf);
    let y = vnum(&yf);
    let w = vnum(&[2.0, 1.0, 1.0, 1.0]);
    let (rw, nw) = spearman_weighted(&x, &y, &w);
    assert_eq!(nw, 4); // usable (non-replicated) observations
    let xr = vnum(&replicate(&xf, &ws));
    let yr = vnum(&replicate(&yf, &ws));
    let (rr, _) = spearman(&xr, &yr);
    assert!(
        (rw.unwrap() - rr.unwrap()).abs() < 1e-12,
        "weighted={rw:?} replicated={rr:?}"
    );
    // And the shared value (hand: 0.57894736842…).
    assert!((rw.unwrap() - 0.578_947_368_421_052_6).abs() < 1e-12, "rw={rw:?}");
}

#[test]
fn weighted_spearman_w1_equals_unweighted() {
    let x = vnum(&[1.0, 2.0, 3.0, 5.0]);
    let y = vnum(&[2.0, 1.0, 4.0, 3.0]);
    let w = vnum(&[1.0, 1.0, 1.0, 1.0]);
    let (ru, _) = spearman(&x, &y);
    let (rw, _) = spearman_weighted(&x, &y, &w);
    assert!((ru.unwrap() - rw.unwrap()).abs() < 1e-12, "{ru:?} {rw:?}");
}

#[test]
fn weighted_kendall_equals_replicated() {
    // Oracle: weighted tau-b with integer weights {2,1,1,1} equals ordinary
    // tau-b on the replicated data.
    let xf = [1.0, 2.0, 3.0, 4.0];
    let yf = [2.0, 1.0, 4.0, 3.0];
    let ws = [2usize, 1, 1, 1];
    let x = vnum(&xf);
    let y = vnum(&yf);
    let w = vnum(&[2.0, 1.0, 1.0, 1.0]);
    let (tw, nw) = kendall_weighted(&x, &y, &w);
    assert_eq!(nw, 4);
    let xr = vnum(&replicate(&xf, &ws));
    let yr = vnum(&replicate(&yf, &ws));
    let (tr, _) = kendall_tau_b(&xr, &yr);
    assert!(
        (tw.unwrap() - tr.unwrap()).abs() < 1e-12,
        "weighted={tw:?} replicated={tr:?}"
    );
    // Shared value (hand: 1/3).
    assert!((tw.unwrap() - 1.0 / 3.0).abs() < 1e-12, "tw={tw:?}");
}

#[test]
fn weighted_kendall_w1_equals_unweighted() {
    let x = vnum(&[1.0, 2.0, 3.0, 4.0]);
    let y = vnum(&[1.0, 3.0, 2.0, 4.0]);
    let w = vnum(&[1.0, 1.0, 1.0, 1.0]);
    let (tu, _) = kendall_tau_b(&x, &y);
    let (tw, _) = kendall_weighted(&x, &y, &w);
    assert!((tu.unwrap() - tw.unwrap()).abs() < 1e-12, "{tu:?} {tw:?}");
}

#[test]
fn execute_hoeffding_block() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0],
        "y" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("y")] };
    write_dataset(&mut session, "T", ds);

    let mut ast = base_ast("T");
    ast.hoeffding = true;
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("Hoeffding Dependence Coefficients"), "{listing}");
    assert!(listing.contains("Prob > D"), "{listing}");
    // Off-diagonal D = 1.00000 (perfect monotone).
    assert!(listing.contains("1.00000"), "{listing}");
    // No Pearson block when only hoeffding requested.
    assert!(!listing.contains("Pearson Correlation Coefficients"), "{listing}");
}

#[test]
fn execute_weighted_spearman_block() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0],
        "y" => [2.0_f64, 1.0, 4.0, 3.0],
        "wt" => [2.0_f64, 1.0, 1.0, 1.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y"), num_meta("wt")],
    };
    write_dataset(&mut session, "T", ds);

    let mut ast = base_ast("T");
    ast.var = vec!["x".into(), "y".into()];
    ast.spearman = true;
    ast.weight = Some("wt".into());
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("Spearman Correlation Coefficients"), "{listing}");
    // Weighted r_s = 0.57895 (matches replicated Spearman).
    assert!(listing.contains("0.57895"), "{listing}");
}
