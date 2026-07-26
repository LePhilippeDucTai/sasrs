use super::super::*;
use super::*;

use crate::stat::dists::betai;

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
    let ast = parse_corr("proc corr data=a nosimple noprob nocorr; var x y; with z; run;").unwrap();
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
    let ast =
        parse_corr("proc corr data=a out=p outs=s outk=k spearman kendall pearson; run;").unwrap();
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
    let x: Vec<Value> = [1.0, 2.0, 3.0, 4.0]
        .iter()
        .map(|v| Value::Num(*v))
        .collect();
    let y: Vec<Value> = [2.0, 4.0, 6.0, 8.0]
        .iter()
        .map(|v| Value::Num(*v))
        .collect();
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
    let x: Vec<Value> = [1.0, 2.0, 3.0, 5.0]
        .iter()
        .map(|v| Value::Num(*v))
        .collect();
    let y: Vec<Value> = [2.0, 1.0, 4.0, 3.0]
        .iter()
        .map(|v| Value::Num(*v))
        .collect();
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
    let expected = (r_xy - r_xz * r_yz) / ((1.0 - r_xz * r_xz) * (1.0 - r_yz * r_yz)).sqrt();

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
