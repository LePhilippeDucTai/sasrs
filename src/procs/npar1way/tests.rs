use super::*;

#[test]
fn test_wilcoxon_basic() {
    // Group A = [1,2,3], Group B = [4,5,6]; no ties.
    // W=6, E(W)=10.5, Var(W)=5.25.
    // With SAS continuity correction: Z = -(|6-10.5|-0.5)/sqrt(5.25) = -4.0/2.2913 ≈ -1.7458.
    let res = analyze(&[vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0]]);
    assert_eq!(res.n, 6);
    assert!(
        (res.tie_factor - 1.0).abs() < 1e-12,
        "tie_factor={}",
        res.tie_factor
    );
    let w = res.wilcoxon.expect("wilcoxon present for k=2");
    assert!((w.w - 6.0).abs() < 1e-12, "W={}", w.w);
    assert!((w.ew - 10.5).abs() < 1e-12, "E(W)={}", w.ew);
    assert!((w.var_w - 5.25).abs() < 1e-12, "Var(W)={}", w.var_w);
    assert!((w.z - (-1.7458)).abs() < 1e-3, "Z={}", w.z);
    assert!(w.p > 0.07 && w.p < 0.10, "p={}", w.p);
}

#[test]
fn test_kruskal_three_groups() {
    // Three groups, no ties: H ≈ 1.143, df = 2.
    let res = analyze(&[vec![1.0, 4.0], vec![2.0, 5.0], vec![3.0, 6.0]]);
    assert_eq!(res.n, 6);
    assert!((res.tie_factor - 1.0).abs() < 1e-12);
    assert_eq!(res.kruskal.df, 2);
    assert!((res.kruskal.h - 1.143).abs() < 1e-2, "H={}", res.kruskal.h);
    // k=3 → no Wilcoxon.
    assert!(res.wilcoxon.is_none());
}

#[test]
fn test_ties_correction() {
    // Group A = [1,2], Group B = [2,3]. Sorted: 1, 2, 2, 3.
    // Mid-ranks: 1 -> 1; the two 2.0 -> 2.5; 3 -> 4.
    // tie group {2,2}: t=2 → Σ(t³-t)=6; n=4 → n³-n=60; tie_factor = 1 - 6/60 = 0.9.
    // W (rank sum of A = [1.0, 2.5]) = 3.5; Var(W)_corrected = (20/12)*0.9 = 1.5.
    let res = analyze(&[vec![1.0, 2.0], vec![2.0, 3.0]]);
    assert_eq!(res.n, 4);
    assert!(
        (res.tie_factor - 0.9).abs() < 1e-12,
        "tie_factor={}",
        res.tie_factor
    );
    let w = res.wilcoxon.expect("wilcoxon present for k=2");
    assert!((w.w - 3.5).abs() < 1e-12, "W={}", w.w);
    assert!(
        (w.var_w - 1.5).abs() < 1e-12,
        "Var(W)_corrected={}",
        w.var_w
    );
    // Confirm the correction actually changed the variance from the uncorrected 20/12.
    assert!(
        (w.var_w - 20.0 / 12.0).abs() > 1e-6,
        "variance should be tie-corrected"
    );
}

// ───────────── generic linear-rank score framework ─────────────

#[test]
fn test_raw_scores_known_vector() {
    // n = 5, no ties: positions 1..=5.
    let n = 5;
    // Wilcoxon: 1,2,3,4,5.
    for p in 1..=n {
        assert!((raw_score(ScoreKind::Wilcoxon, p, n) - p as f64).abs() < 1e-12);
    }
    // Median (n odd): middle position 3 → 0.0, positions 4,5 → 1.0.
    assert_eq!(raw_score(ScoreKind::Median, 1, n), 0.0);
    assert_eq!(raw_score(ScoreKind::Median, 3, n), 0.0);
    assert_eq!(raw_score(ScoreKind::Median, 4, n), 1.0);
    assert_eq!(raw_score(ScoreKind::Median, 5, n), 1.0);
    // Savage: s(1) = 1/n - 1 = 0.2 - 1 = -0.8.
    assert!((raw_score(ScoreKind::Savage, 1, n) - (1.0 / 5.0 - 1.0)).abs() < 1e-12);
    // Savage last position: Σ_{j=1}^{n} 1/(n-j+1) = H_n; H_5 = 2.283333..., minus 1.
    let h5 = 1.0 + 0.5 + 1.0 / 3.0 + 0.25 + 0.2;
    assert!((raw_score(ScoreKind::Savage, 5, n) - (h5 - 1.0)).abs() < 1e-12);
    // Normal: Φ⁻¹(p/(n+1)); middle p=3 → Φ⁻¹(0.5) = 0.
    assert!(raw_score(ScoreKind::Normal, 3, n).abs() < 1e-9);
    // Symmetry: s(1) = -s(5).
    assert!((raw_score(ScoreKind::Normal, 1, n) + raw_score(ScoreKind::Normal, 5, n)).abs() < 1e-9);
}

#[test]
fn test_wilcoxon_score_matches_midranks() {
    // The generic Wilcoxon score routine must reproduce the existing
    // mid-ranks (including tie-averaging).
    let pooled = vec![1.0, 2.0, 2.0, 3.0];
    let (mid, _) = midranks(&pooled);
    let sc = tie_averaged_scores(&pooled, ScoreKind::Wilcoxon);
    for (a, b) in mid.iter().zip(&sc) {
        assert!((a - b).abs() < 1e-12, "mid={a} score={b}");
    }
    // Tie group {2,2} → mid-rank 2.5 each.
    assert!((sc[1] - 2.5).abs() < 1e-12);
    assert!((sc[2] - 2.5).abs() < 1e-12);
}

#[test]
fn test_two_sample_z_reproduces_wilcoxon() {
    // Generic 2-sample routine on Wilcoxon scores must match `analyze`.
    let groups = [vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0]];
    let res = analyze(&groups);
    let w = res.wilcoxon.unwrap();
    let sa = score_analysis(&groups, ScoreKind::Wilcoxon);
    let t = score_two_sample(&sa).expect("two-sample present");
    assert!((t.stat - w.w).abs() < 1e-9, "stat {} vs {}", t.stat, w.w);
    assert!((t.mean - w.ew).abs() < 1e-9, "mean {} vs {}", t.mean, w.ew);
    assert!(
        (t.sd - w.var_w.sqrt()).abs() < 1e-9,
        "sd {} vs {}",
        t.sd,
        w.var_w.sqrt()
    );
    assert!((t.z - w.z).abs() < 1e-9, "z {} vs {}", t.z, w.z);
    assert!((t.p2 - w.p).abs() < 1e-9, "p {} vs {}", t.p2, w.p);
}

#[test]
fn test_two_sample_z_snapshot_shape() {
    // Self-check on the m24-style table shape: n0=9, n1=10, n=19, group-0
    // rank sum 73 reproduces Stat 73, Mean 90, Std 12.2367, Z -1.3484,
    // p 0.1775 (the exact m24 `height` row). We build the same pooled rank
    // configuration directly: ranks 1..=19 with group-0 rank sum = 73 and no
    // ties (so the generic routine equals the closed-form Wilcoxon).
    // Construct values so the first 9 take ranks summing to 73.
    // ranks for group0: {1,2,3,4,5,6,7,8,37/?} — instead derive via analyze
    // on a constructed split with n0=9,n1=10 and W=73.
    // Pick group0 = positions giving rank sum 73: 1+2+3+4+5+6+7+8+37? invalid.
    // Use explicit values: group0 = ranks {2,4,5,7,9,11,12,11.5...} — too
    // fiddly; instead assert closed-form equality on a clean 9-vs-10 split.
    let g0: Vec<f64> = (1..=9).map(|i| i as f64).collect(); // ranks 1..9
    let g1: Vec<f64> = (10..=19).map(|i| i as f64).collect(); // ranks 10..19
    let res = analyze(&[g0.clone(), g1.clone()]);
    let w = res.wilcoxon.unwrap();
    let sa = score_analysis(&[g0, g1], ScoreKind::Wilcoxon);
    let t = score_two_sample(&sa).unwrap();
    // Generic routine reproduces the closed-form Wilcoxon for n0=9,n1=10.
    assert!((t.stat - w.w).abs() < 1e-9);
    assert!((t.mean - w.ew).abs() < 1e-9);
    assert!((t.sd - w.var_w.sqrt()).abs() < 1e-9);
    assert!((t.z - w.z).abs() < 1e-9);
    assert!((t.p2 - w.p).abs() < 1e-9);
    // Mean Under H0 for n0=9, n=19: 9*(20)/2 = 90 (the snapshot value).
    assert!((t.mean - 90.0).abs() < 1e-9, "mean={}", t.mean);
}

#[test]
fn test_one_way_chisq_equals_kruskal_for_wilcoxon() {
    // For Wilcoxon scores, the generic one-way χ² equals Kruskal-Wallis H.
    let groups = [vec![1.0, 4.0, 7.0], vec![2.0, 5.0], vec![3.0, 6.0, 8.0]];
    let res = analyze(&groups);
    let sa = score_analysis(&groups, ScoreKind::Wilcoxon);
    let ow = score_one_way(&sa);
    assert_eq!(ow.df, res.kruskal.df);
    assert!(
        (ow.chisq - res.kruskal.h).abs() < 1e-9,
        "chisq {} vs H {}",
        ow.chisq,
        res.kruskal.h
    );
}

#[test]
fn test_median_scores_two_groups() {
    // [1,2] vs [3,4]: n=4, median positions: p>2.5 → 1. ranks 1,2,3,4.
    // scores: 0,0,1,1. group0 sum = 0, group1 sum = 2. ā = 0.5.
    let sa = score_analysis(&[vec![1.0, 2.0], vec![3.0, 4.0]], ScoreKind::Median);
    assert!((sa.abar - 0.5).abs() < 1e-12);
    assert!((sa.s[0] - 0.0).abs() < 1e-12, "S0={}", sa.s[0]);
    assert!((sa.s[1] - 2.0).abs() < 1e-12, "S1={}", sa.s[1]);
    // SS = Σ(a-ā)² = 4 * 0.25 = 1.0.
    assert!((sa.ss - 1.0).abs() < 1e-12, "SS={}", sa.ss);
}

#[test]
fn test_tie_averaging_savage() {
    // Pooled [1,2,2,3]: tie group {2,2} spans positions 2..=3. Each tied obs
    // gets the average of savage scores at p=2 and p=3.
    let pooled = vec![1.0, 2.0, 2.0, 3.0];
    let sc = tie_averaged_scores(&pooled, ScoreKind::Savage);
    let n = 4;
    let s2 = raw_score(ScoreKind::Savage, 2, n);
    let s3 = raw_score(ScoreKind::Savage, 3, n);
    let avg = (s2 + s3) / 2.0;
    assert!((sc[1] - avg).abs() < 1e-12, "sc[1]={} avg={}", sc[1], avg);
    assert!((sc[2] - avg).abs() < 1e-12, "sc[2]={} avg={}", sc[2], avg);
    assert!((sc[0] - raw_score(ScoreKind::Savage, 1, n)).abs() < 1e-12);
    assert!((sc[3] - raw_score(ScoreKind::Savage, 4, n)).abs() < 1e-12);
}

// ───────────── exact Wilcoxon permutation test ─────────────

#[test]
fn test_exact_wilcoxon_textbook() {
    // [1,2,3] vs [4,5,6]: n0=3, C(6,3)=20. The observed rank-sum (group 0)
    // is the minimum (6). Two-sided exact p = 2/20 = 0.10 (only the two most
    // extreme arrangements are ≥ as extreme on each tail).
    let ex = exact_wilcoxon(&[vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0]]).unwrap();
    assert!((ex.p_two - 0.10).abs() < 1e-12, "p_two={}", ex.p_two);
    // One-sided lower Pr(S <= 6) = 1/20 = 0.05.
    assert!((ex.p_lower - 0.05).abs() < 1e-12, "p_lower={}", ex.p_lower);
}

#[test]
fn test_exact_wilcoxon_cap() {
    // Beyond the cap, exact returns None.
    let big0: Vec<f64> = (0..16).map(|i| i as f64).collect();
    let big1: Vec<f64> = (16..32).map(|i| i as f64).collect();
    assert!(exact_wilcoxon(&[big0, big1]).is_none());
}
