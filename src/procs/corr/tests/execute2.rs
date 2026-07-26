use super::super::*;
use super::*;
use crate::dataset::SasDataset;
use polars::df;

#[test]
fn execute_hoeffding_block() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0],
        "y" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let mut ast = base_ast("T");
    ast.hoeffding = true;
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(
        listing.contains("Hoeffding Dependence Coefficients"),
        "{listing}"
    );
    assert!(listing.contains("Prob > D"), "{listing}");
    // Off-diagonal D = 1.00000 (perfect monotone).
    assert!(listing.contains("1.00000"), "{listing}");
    // No Pearson block when only hoeffding requested.
    assert!(
        !listing.contains("Pearson Correlation Coefficients"),
        "{listing}"
    );
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
    assert!(
        listing.contains("Spearman Correlation Coefficients"),
        "{listing}"
    );
    // Weighted r_s = 0.57895 (matches replicated Spearman).
    assert!(listing.contains("0.57895"), "{listing}");
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
    assert!(
        (ru.unwrap() - rw.unwrap()).abs() < 1e-12,
        "{:?} {:?}",
        ru,
        rw
    );
}

#[test]
fn weighted_excludes_nonpositive_and_missing() {
    // Row with w=0 and row with missing w are dropped → n=2.
    let x = vnum(&[1.0, 2.0, 3.0, 4.0]);
    let y = vnum(&[2.0, 4.0, 6.0, 8.0]);
    let w = vec![
        Value::Num(2.0),
        Value::Num(0.0),
        Value::missing(),
        Value::Num(3.0),
    ];
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
    assert!(
        (rw.unwrap() - 0.578_947_368_421_052_6).abs() < 1e-12,
        "rw={rw:?}"
    );
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
        69.0, 56.5, 65.3, 62.8, 63.5, 57.3, 59.8, 62.5, 62.5, 59.0, 51.3, 64.3, 56.3, 66.5, 72.0,
        64.8, 67.0, 57.5, 66.5,
    ]);
    let w = vnum(&[
        112.5, 84.0, 98.0, 102.5, 102.5, 83.0, 84.5, 112.5, 84.0, 99.5, 50.5, 90.0, 77.0, 112.0,
        150.0, 128.0, 133.0, 85.0, 112.0,
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
