use super::super::*;
use super::*;
use crate::dataset::SasDataset;
use polars::df;

#[test]
fn crosstab_norow_drops_row_pct() {
    let l = crosstab_listing(|r| r.norow = true);
    assert!(!l.contains("Row Pct"), "{l}");
    assert!(l.contains("Col Pct"), "{l}");
}

#[test]
fn crosstab_nocol_drops_col_pct() {
    let l = crosstab_listing(|r| r.nocol = true);
    assert!(!l.contains("Col Pct"), "{l}");
    assert!(l.contains("Row Pct"), "{l}");
}

#[test]
fn crosstab_nofreq_keeps_others() {
    // NOFREQ drops the per-cell Frequency line; Percent/Row/Col remain.
    let l = crosstab_listing(|r| r.nofreq = true);
    assert!(l.contains("Percent"), "{l}");
    assert!(l.contains("Row Pct"), "{l}");
    assert!(l.contains("Col Pct"), "{l}");
    // The label row must still identify the row categories and Total.
    assert!(l.contains("Total"), "{l}");
}

// ───────────────────────── chisq block test ─────────────────────────

#[test]
fn crosstab_chisq_2x2_hand_computed() {
    // 2x2 table:
    //          c=1  c=2  | tot
    //   r=a :   10    20 |  30
    //   r=b :   30    40 |  70
    //   col :   40    60 | 100
    // Expected: e_a1=30*40/100=12, e_a2=18, e_b1=28, e_b2=42.
    // Pearson = (10-12)^2/12 + (20-18)^2/18 + (30-28)^2/28 + (40-42)^2/42
    //         = 4/12 + 4/18 + 4/28 + 4/42
    //         = 0.333333 + 0.222222 + 0.142857 + 0.095238 = 0.793651
    // DF = 1; p = chisq_sf(0.793651, 1) ~ 0.3730.
    let mut session = make_session();
    // Build column vectors that reproduce the table counts.
    let mut r: Vec<&str> = Vec::new();
    let mut c: Vec<f64> = Vec::new();
    for _ in 0..10 { r.push("a"); c.push(1.0); }
    for _ in 0..20 { r.push("a"); c.push(2.0); }
    for _ in 0..30 { r.push("b"); c.push(1.0); }
    for _ in 0..40 { r.push("b"); c.push(2.0); }
    let df = df!["r" => r, "c" => c].unwrap();
    let ds = SasDataset { df, vars: vec![char_meta("r"), num_meta("c")] };
    write_dataset(&mut session, "T", ds);

    let mut req = tr(&["r", "c"], false, None);
    req.chisq = true;
    let ast = FreqAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        tables: vec![req],
        weight: None,
        by: Vec::new(),
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("Statistics for Table of r by c"), "{listing}");
    assert!(listing.contains("Chi-Square"), "{listing}");
    assert!(listing.contains("Likelihood Ratio Chi-Square"), "{listing}");
    // Pearson value formatted to 4 decimals.
    assert!(listing.contains("0.7937"), "{listing}");

    // Cross-check the numeric pieces directly.
    let pearson: f64 = 4.0 / 12.0 + 4.0 / 18.0 + 4.0 / 28.0 + 4.0 / 42.0;
    assert!((pearson - 0.793651).abs() < 1e-4, "{pearson}");
    let p = chisq_sf(pearson, 1.0);
    assert!((p - 0.3730).abs() < 1e-3, "p={p}");
}

// ---- Fisher exact ----

#[test]
fn fisher_2x2_symmetric_classic() {
    // [[3,1],[1,3]] : documented SAS two-sided p ~ 0.4857.
    let freq = vec![vec![3, 1], vec![1, 3]];
    let (rt, ct, g) = margins(&freq);
    let out = run_block(|s| fisher_block(s, &freq, &rt, &ct, g));
    assert!(out.contains("Fisher's Exact Test"), "{out}");
    assert!(out.contains("Two-sided Pr <= P"), "{out}");
    assert!(out.contains("0.4857"), "two-sided 0.4857 expected:\n{out}");
}

#[test]
fn fisher_2x2_numeric_values() {
    // Recompute the canonical case exactly: r1=r2=c1=c2=4, n=8.
    // p(a)=C(4,a)C(4,4-a)/C(8,4). C(8,4)=70.
    // a=0:1*1/70, a=1:4*4/70, a=2:6*6/70, a=3:4*4/70, a=4:1*1/70.
    let c84 = 70.0;
    let pa = |a: u64| -> f64 {
        let lc = ln_choose(4, a) + ln_choose(4, 4 - a);
        lc.exp() / c84
    };
    // observed a=3 -> p_obs = 16/70.
    let p_obs = pa(3);
    assert!((p_obs - 16.0 / 70.0).abs() < 1e-12);
    // two-sided = sum of probs <= p_obs = a in {0,1,3,4} (a=2 is 36/70 > 16/70).
    let two = pa(0) + pa(1) + pa(3) + pa(4);
    assert!((two - (1.0 + 16.0 + 16.0 + 1.0) / 70.0).abs() < 1e-12);
    assert!((two - 0.485714).abs() < 1e-5, "two={two}");
    // right-sided P(A>=3) = (16+1)/70 = 0.242857.
    let right = pa(3) + pa(4);
    assert!((right - 17.0 / 70.0).abs() < 1e-12);
}

#[test]
fn fisher_larger_than_2x2_deferred() {
    let freq = vec![vec![1, 2, 3], vec![4, 5, 6]];
    let (rt, ct, g) = margins(&freq);
    let out = run_block(|s| fisher_block(s, &freq, &rt, &ct, g));
    assert!(out.contains("larger than 2x2"), "{out}");
}

// ---- MEASURES (odds ratio / RR) ----

#[test]
fn measures_odds_ratio_exact() {
    // [[20,10],[5,25]] : OR = (20*25)/(10*5) = 500/50 = 10.
    let freq = vec![vec![20, 10], vec![5, 25]];
    let out = run_block(|s| measures_block(s, &freq));
    assert!(out.contains("Odds Ratio"), "{out}");
    assert!(out.contains("10.0000"), "OR=10 expected:\n{out}");
    // RR col1 = (20/30)/(5/30) = (0.6667)/(0.1667) = 4.
    assert!(out.contains("4.0000"), "RR col1 = 4 expected:\n{out}");
}

#[test]
fn measures_odds_ratio_ci() {
    // OR=10, SE = sqrt(1/20+1/10+1/5+1/25)=sqrt(0.39)=0.62450.
    // ln10=2.302585; CI = exp(2.302585 ∓ 1.96*0.62450) = [2.9405, 34.008].
    let se = (1.0 / 20.0 + 1.0 / 10.0 + 1.0 / 5.0 + 1.0 / 25.0_f64).sqrt();
    assert!((se - 0.624500).abs() < 1e-4, "se={se}");
    let or: f64 = 10.0;
    let lo = (or.ln() - 1.96 * se).exp();
    let hi = (or.ln() + 1.96 * se).exp();
    assert!((lo - 2.9405).abs() < 1e-3, "lo={lo}");
    assert!((hi - 34.008).abs() < 1e-2, "hi={hi}");
}

#[test]
fn measures_zero_cell_no_panic() {
    // b=0 -> OR undefined; must not panic and must print ".".
    let freq = vec![vec![5, 0], vec![3, 7]];
    let out = run_block(|s| measures_block(s, &freq));
    assert!(out.contains("Odds Ratio"), "{out}");
    // Odds ratio row carries "." because b=0.
    assert!(out.contains('.'), "{out}");
}

#[test]
fn measures_requires_2x2() {
    let freq = vec![vec![1, 2, 3], vec![4, 5, 6]];
    let out = run_block(|s| measures_block(s, &freq));
    assert!(out.contains("require a 2x2"), "{out}");
}

// ---- AGREE (kappa) ----

#[test]
fn agree_kappa_hand_computed() {
    // Diagonal-heavy 2x2 agreement table:
    // [[20,5],[10,15]] : N=50.
    // Po = (20+15)/50 = 0.70.
    // row tot = [25,25], col tot = [30,20].
    // Pe = (25/50)(30/50) + (25/50)(20/50) = 0.5*0.6 + 0.5*0.4 = 0.3+0.2 = 0.5.
    // kappa = (0.70-0.50)/(1-0.50) = 0.20/0.50 = 0.40.
    let freq = vec![vec![20, 5], vec![10, 15]];
    let (rt, ct, g) = margins(&freq);
    let out = run_block(|s| agree_block(s, &freq, &rt, &ct, g));
    assert!(out.contains("Simple Kappa Coefficient"), "{out}");
    assert!(out.contains("0.4000"), "kappa=0.40 expected:\n{out}");
}

#[test]
fn agree_perfect_agreement_kappa_one() {
    // Pure diagonal -> Po=1 -> kappa = 1.
    let freq = vec![vec![10, 0], vec![0, 10]];
    let (rt, ct, g) = margins(&freq);
    let out = run_block(|s| agree_block(s, &freq, &rt, &ct, g));
    assert!(out.contains("1.0000"), "kappa=1 expected:\n{out}");
}

#[test]
fn agree_requires_square() {
    let freq = vec![vec![1, 2, 3], vec![4, 5, 6]];
    let (rt, ct, g) = margins(&freq);
    let out = run_block(|s| agree_block(s, &freq, &rt, &ct, g));
    assert!(out.contains("requires a square table"), "{out}");
}

#[test]
fn agree_3x3_kappa() {
    // 3x3 with strong diagonal.
    // [[10,1,1],[1,10,1],[1,1,10]] N=36.
    // Po = 30/36 = 0.833333.
    // each row tot=12, col tot=12 -> Pe = 3*(12/36)(12/36) = 3*(1/9)=0.333333.
    // kappa = (0.833333-0.333333)/(1-0.333333) = 0.5/0.666667 = 0.75.
    let freq = vec![vec![10, 1, 1], vec![1, 10, 1], vec![1, 1, 10]];
    let (rt, ct, g) = margins(&freq);
    let out = run_block(|s| agree_block(s, &freq, &rt, &ct, g));
    assert!(out.contains("0.7500"), "kappa=0.75 expected:\n{out}");
}

// ---- TREND (Cochran-Armitage) ----

#[test]
fn trend_monotone_increasing() {
    // 2x3 table, ordinal columns 1..3, clear increasing trend in row 1.
    // row0 (cases): [5,10,20], row1 (controls): [20,10,5].
    // col tot = [25,20,25], N=70, r1=35, r2=35.
    let freq = vec![vec![5, 10, 20], vec![20, 10, 5]];
    let (rt, ct, g) = margins(&freq);
    let out = run_block(|s| trend_block(s, &freq, &rt, &ct, g));
    assert!(out.contains("Cochran-Armitage Trend Test"), "{out}");
    assert!(out.contains("Statistic (Z)"), "{out}");

    // Hand recompute: scores s=[1,2,3]. row0=[5,10,20], col tot=[25,20,25],
    // N=70, r1=35, r2=35.
    // T = Σ s_i (n_{1i} - r1*c_i/N)
    //   = 1*(5 - 35*25/70) + 2*(10 - 35*20/70) + 3*(20 - 35*25/70)
    //   = 1*(5-12.5) + 2*(10-10) + 3*(20-12.5)
    //   = -7.5 + 0 + 22.5 = 15.
    // Σ c_i s_i = 25*1+20*2+25*3 = 25+40+75 = 140.
    // Σ c_i s_i² = 25*1+20*4+25*9 = 25+80+225 = 330.
    // Var = (35*35/70)*(330 - 140²/70) = 17.5 * (330-280) = 17.5*50 = 875.
    // Z = 15/sqrt(875) = 15/29.58 = 0.5071.
    let t = 15.0_f64;
    let var: f64 = (35.0 * 35.0 / 70.0) * (330.0 - 140.0 * 140.0 / 70.0);
    let z = t / var.sqrt();
    assert!((z - 0.5071).abs() < 1e-3, "z={z}");
    assert!(out.contains("0.5071"), "Z=0.5071 expected:\n{out}");
    let p_two = (2.0 * (1.0 - probnorm(z.abs()))).min(1.0);
    assert!((p_two - 0.6121).abs() < 1e-3, "p_two={p_two}");
}

#[test]
fn trend_requires_binary_dimension() {
    let freq = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
    let (rt, ct, g) = margins(&freq);
    let out = run_block(|s| trend_block(s, &freq, &rt, &ct, g));
    assert!(out.contains("requires a 2xC or Rx2"), "{out}");
}

#[test]
fn trend_rx2_orientation() {
    // 3x2 (rows are ordinal). Should compute (transposed roles), no panic.
    let freq = vec![vec![5, 20], vec![10, 10], vec![20, 5]];
    let (rt, ct, g) = margins(&freq);
    let out = run_block(|s| trend_block(s, &freq, &rt, &ct, g));
    assert!(out.contains("Statistic (Z)"), "{out}");
}

// ---- common.rs numeric helpers ----

#[test]
fn probnorm_known_values() {
    assert!((probnorm(0.0) - 0.5).abs() < 1e-9);
    assert!((probnorm(1.96) - 0.975).abs() < 1e-4, "{}", probnorm(1.96));
    assert!((probnorm(-1.96) - 0.025).abs() < 1e-4);
}

#[test]
fn ln_choose_known_values() {
    assert!((ln_choose(8, 4).exp() - 70.0).abs() < 1e-6);
    assert!((ln_choose(5, 2).exp() - 10.0).abs() < 1e-6);
    assert!(ln_choose(3, 5) == f64::NEG_INFINITY);
}

// ───────────────────── M33.1 WEIGHT / BY / LIST / n-way ─────────────────────

/// WEIGHT one-way: the cell frequency is the SUM OF WEIGHTS, not the count.
/// x = [1, 1, 2]; w = [2, 3, 5].
///   cat 1 -> weight 2+3 = 5 ; cat 2 -> weight 5. denom = 10.
///   percent 1 -> 50.00 ; percent 2 -> 50.00. cumulative 5 then 10.
#[test]
fn weighted_one_way_sum_of_weights() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 1.0, 2.0],
        "w" => [2.0_f64, 3.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("w")] };
    write_dataset(&mut session, "T", ds);

    let mut ast = fast(
        DatasetRef { libref: Some("WORK".into()), name: "T".into() },
        vec![tr(&["x"], false, None)],
    );
    ast.weight = Some("w".to_string());
    execute(&ast, &mut session).unwrap();
    let l = session.listing.into_string();
    // Frequencies 5 and 5 (sum of weights), each 50.00%, cum 5 then 10.
    assert!(l.contains("50.00"), "{l}");
    // Integer-valued weighted freqs print as integers (no decimals).
    assert!(l.contains(" 5 ") || l.contains(" 5\n") || l.contains("5  "), "{l}");
    assert!(l.contains("100.00"), "{l}");
}

/// WEIGHT excludes observations whose weight is missing or non-positive.
/// x = [1, 1, 2, 2]; w = [4, ., -1, 6].
///   obs2 (w missing) dropped, obs3 (w=-1) dropped.
///   cat 1 -> 4 ; cat 2 -> 6. denom = 10. percents 40.00 / 60.00.
#[test]
fn weighted_excludes_missing_and_nonpositive() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 1.0, 2.0, 2.0],
        "w" => [Some(4.0_f64), None, Some(-1.0), Some(6.0)]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("w")] };
    write_dataset(&mut session, "T", ds);

    let mut ast = fast(
        DatasetRef { libref: Some("WORK".into()), name: "T".into() },
        vec![tr(&["x"], false, None)],
    );
    ast.weight = Some("w".to_string());
    execute(&ast, &mut session).unwrap();
    let l = session.listing.into_string();
    assert!(l.contains("40.00"), "cat1 = 4/10 = 40.00:\n{l}");
    assert!(l.contains("60.00"), "cat2 = 6/10 = 60.00:\n{l}");
}

/// WEIGHT feeds CHISQ. 2x2 with weighted counts == the classic
/// [[10,20],[30,40]] table built from unit cells with those weights.
/// Pearson chi-square = 0.7937 (DF=1), as in `crosstab_chisq_2x2_hand_computed`.
#[test]
fn weighted_two_way_chisq() {
    let mut session = make_session();
    // One observation per cell, weight = the desired count.
    let df = df![
        "r" => ["a", "a", "b", "b"],
        "c" => [1.0_f64, 2.0, 1.0, 2.0],
        "w" => [10.0_f64, 20.0, 30.0, 40.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("r"), num_meta("c"), num_meta("w")],
    };
    write_dataset(&mut session, "T", ds);

    let mut req = tr(&["r", "c"], false, None);
    req.chisq = true;
    let mut ast = fast(
        DatasetRef { libref: Some("WORK".into()), name: "T".into() },
        vec![req],
    );
    ast.weight = Some("w".to_string());
    execute(&ast, &mut session).unwrap();
    let l = session.listing.into_string();
    // Weighted grand total = 100 ; Pearson = 0.7937.
    assert!(l.contains("0.7937"), "weighted Pearson 0.7937:\n{l}");
    // Weighted cell freq for (a,1) is 10 (integer-printed).
    assert!(l.contains("10"), "{l}");
}

/// BY splits the analysis into one section per group. class-like toy:
/// g = [A,A,A,B,B] (sorted); x = [1,2,2,1,1].
///   Group A: x=1 freq 1 (33.33%), x=2 freq 2 (66.67%).
///   Group B: x=1 freq 2 (100.00%).
#[test]
fn by_groups_split_one_way() {
    let mut session = make_session();
    let df = df![
        "g" => ["A", "A", "A", "B", "B"],
        "x" => [1.0_f64, 2.0, 2.0, 1.0, 1.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x")] };
    write_dataset(&mut session, "T", ds);

    let mut ast = fast(
        DatasetRef { libref: Some("WORK".into()), name: "T".into() },
        vec![tr(&["x"], false, None)],
    );
    ast.by = vec![("g".to_string(), false)];
    execute(&ast, &mut session).unwrap();
    let l = session.listing.into_string();
    assert!(l.contains("g=A"), "BY header for A:\n{l}");
    assert!(l.contains("g=B"), "BY header for B:\n{l}");
    // Group A percents 33.33 / 66.67 ; Group B 100.00.
    assert!(l.contains("33.33"), "{l}");
    assert!(l.contains("66.67"), "{l}");
    assert!(l.contains("100.00"), "{l}");
}

/// BY requires the input sorted by the BY var; otherwise the SAS error.
#[test]
fn by_unsorted_errors() {
    let mut session = make_session();
    let df = df![
        "g" => ["B", "A"],
        "x" => [1.0_f64, 1.0]
    ]
    .unwrap();
    let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x")] };
    write_dataset(&mut session, "T", ds);
    let mut ast = fast(
        DatasetRef { libref: Some("WORK".into()), name: "T".into() },
        vec![tr(&["x"], false, None)],
    );
    ast.by = vec![("g".to_string(), false)];
    let err = execute(&ast, &mut session).unwrap_err();
    assert!(
        err.to_string().contains("not sorted in ascending sequence"),
        "{err}"
    );
}
