use super::*;

/// Fit a tiny one-way reference-cell model y = μ + α (2 groups A,B) by the
/// same path GLM uses, and wrap it in a LinCombEngine.
///
/// Data: A=[1,2,3], B=[10,11,12]. sas_cmp order → ref = B (last).
/// Design cols: [intercept(=B mean), A-dummy].
fn engine_ab() -> LinCombEngine {
    // y, with usable rows in input order.
    let y = vec![1.0, 2.0, 3.0, 10.0, 11.0, 12.0];
    // reference-cell design: intercept + A-dummy (A=1 for first 3 rows).
    let design = vec![
        vec![1.0, 1.0],
        vec![1.0, 1.0],
        vec![1.0, 1.0],
        vec![1.0, 0.0],
        vec![1.0, 0.0],
        vec![1.0, 0.0],
    ];
    let beta = crate::stat::linalg::least_squares(&design, &y).unwrap();
    let xt = crate::stat::linalg::transpose(&design);
    let xtx = crate::stat::linalg::matrix_mult(&xt, &design);
    let cov = crate::stat::linalg::invert_matrix(&xtx).unwrap();
    // SSE / df / MSE.
    let mut sse = 0.0;
    for (i, row) in design.iter().enumerate() {
        let fitted: f64 = row.iter().zip(beta.iter()).map(|(a, b)| a * b).sum();
        sse += (y[i] - fitted).powi(2);
    }
    let n = y.len();
    let ncols = 2;
    let df_error = (n - ncols) as f64;
    let mse = sse / df_error;
    let coding = Coding {
        factors: vec![(
            "sex".into(),
            vec![Value::Char("A".into()), Value::Char("B".into())],
        )],
        term_factor_idxs: vec![vec![0]],
        // term sex → one column = A-dummy = (factor 0, dummy 0).
        col_specs: vec![vec![vec![(0usize, 0usize)]]],
        ncols,
    };
    LinCombEngine::new(beta, cov, coding, df_error, mse)
}

// ── estimate: oracle from glm.rs test_execute_estimate_correct ──────────
// ESTIMATE 'A vs B' = 1*ȳ_A + (-1)*ȳ_B in *cell-mean* space, but in
// parameter space (intercept=B mean, A-dummy=α_A) the same contrast is
// l = [0, 1] (β_A = ȳ_A − ȳ_B = −9), c = 0.  est = −9.
#[test]
fn test_estimate_minus_nine() {
    let eng = engine_ab();
    let r = eng.estimate(&[0.0, 1.0], 0.0);
    assert!((r.estimate - (-9.0)).abs() < 1e-9, "est={}", r.estimate);
    // SE = sqrt(MSE * cov[1][1]); MSE=1, t ≈ -11.02.
    assert!(r.t < -10.0 && r.t > -12.0, "t={}", r.t);
    assert!(r.se > 0.0);
}

// ── contrast: F == t² of the same estimate (glm test 7 oracle) ──────────
#[test]
fn test_contrast_f_eq_t_squared() {
    let eng = engine_ab();
    let est = eng.estimate(&[0.0, 1.0], 0.0);
    let con = eng.contrast(&[0.0, 1.0], 0.0);
    assert!(
        (con.f - est.t * est.t).abs() < 1e-6,
        "f={} t^2={}",
        con.f,
        est.t * est.t
    );
    assert!((con.f - 121.5).abs() < 0.5, "f={}", con.f);
    assert_eq!(con.df1, 1.0);
    assert_eq!(con.df2, 4.0);
}

// ── lsmeans: ȳ_A = 2, ȳ_B = 11 (glm test_execute_lsmeans oracle) ────────
#[test]
fn test_lsmeans_group_means() {
    let eng = engine_ab();
    let lsm = eng.lsmeans("sex");
    assert_eq!(lsm.len(), 2);
    // sas_cmp order: A then B.
    assert_eq!(lsm[0].level_label, "A");
    assert_eq!(lsm[1].level_label, "B");
    assert!(
        (lsm[0].estimate - 2.0).abs() < 1e-9,
        "A={}",
        lsm[0].estimate
    );
    assert!(
        (lsm[1].estimate - 11.0).abs() < 1e-9,
        "B={}",
        lsm[1].estimate
    );
    assert!(lsm[0].se > 0.0 && lsm[1].se > 0.0);
}

// ── unknown effect → empty ──────────────────────────────────────────────
#[test]
fn test_lsmeans_unknown_effect() {
    let eng = engine_ab();
    assert!(eng.lsmeans("nope").is_empty());
}

// ─────────────────────── class_coding ───────────────────────

fn levels(names: &[&str]) -> Vec<Value> {
    names.iter().map(|s| Value::Char((*s).into())).collect()
}

/// Ref coding must reproduce the manual reference-cell layout (last = ref).
#[test]
fn test_class_coding_ref_matches_manual() {
    let lv = levels(&["A", "B", "C"]); // already sas_cmp order
    let coding = class_coding(&lv, Param::Ref);
    // 3 rows, 2 columns each.
    assert_eq!(
        coding,
        vec![
            vec![1.0, 0.0], // A
            vec![0.0, 1.0], // B
            vec![0.0, 0.0], // C = reference
        ]
    );
    // Manual oracle: column j is the indicator of levels[j] (j < L−1).
    let l = lv.len();
    for (li, _) in lv.iter().enumerate() {
        for j in 0..l - 1 {
            let manual = if li == j { 1.0 } else { 0.0 };
            assert_eq!(coding[li][j], manual, "li={li} j={j}");
        }
    }
}

/// Effect coding: reference (last) = −1 everywhere ⇒ each column sums to 0.
#[test]
fn test_class_coding_effect_columns_sum_to_zero() {
    let lv = levels(&["A", "B", "C", "D"]);
    let coding = class_coding(&lv, Param::Effect);
    assert_eq!(coding.len(), 4);
    let ncol = lv.len() - 1;
    for row in &coding {
        assert_eq!(row.len(), ncol);
    }
    // Last row all −1.
    assert_eq!(coding[3], vec![-1.0, -1.0, -1.0]);
    // Each column sums to 0.
    for j in 0..ncol {
        let s: f64 = coding.iter().map(|r| r[j]).sum();
        assert!(s.abs() < 1e-12, "col {j} sum = {s}");
    }
}

/// Glm coding: L columns, exactly one 1 per row, rest 0.
#[test]
fn test_class_coding_glm_one_hot() {
    let lv = levels(&["A", "B", "C"]);
    let coding = class_coding(&lv, Param::Glm);
    assert_eq!(coding.len(), 3);
    for (i, row) in coding.iter().enumerate() {
        assert_eq!(row.len(), 3, "L columns");
        let ones: Vec<usize> = row
            .iter()
            .enumerate()
            .filter(|&(_, &v)| v == 1.0)
            .map(|(k, _)| k)
            .collect();
        assert_eq!(ones, vec![i], "row {i} must be one-hot at i");
        let sum: f64 = row.iter().sum();
        assert_eq!(sum, 1.0);
    }
}

/// Poly coding: L−1 columns, pairwise orthogonal, col 0 = linear trend.
#[test]
fn test_class_coding_poly_orthogonal() {
    let lv = levels(&["A", "B", "C", "D", "E"]); // L = 5
    let coding = class_coding(&lv, Param::Poly);
    let l = lv.len();
    let ncol = l - 1;
    assert_eq!(coding.len(), l);
    for row in &coding {
        assert_eq!(row.len(), ncol);
    }
    // Pairwise orthogonality (and orthogonality to the constant).
    for a in 0..ncol {
        let dot_const: f64 = (0..l).map(|k| coding[k][a]).sum();
        assert!(
            dot_const.abs() < 1e-9,
            "col {a} not orthogonal to constant: {dot_const}"
        );
        for b in (a + 1)..ncol {
            let dot: f64 = (0..l).map(|k| coding[k][a] * coding[k][b]).sum();
            assert!(dot.abs() < 1e-9, "cols {a},{b} not orthogonal: {dot}");
        }
        // Unit norm (orthonormal).
        let nn: f64 = (0..l).map(|k| coding[k][a] * coding[k][a]).sum();
        assert!((nn - 1.0).abs() < 1e-9, "col {a} not unit norm: {nn}");
    }
    // Column 0 is the linear trend: strictly monotone with equal spacing.
    let col0: Vec<f64> = (0..l).map(|k| coding[k][0]).collect();
    let step = col0[1] - col0[0];
    assert!(step > 0.0, "linear trend must increase: step={step}");
    for k in 1..l {
        assert!(
            (col0[k] - col0[k - 1] - step).abs() < 1e-9,
            "col0 not equally spaced at {k}"
        );
    }
}

/// Degenerate sizes.
#[test]
fn test_class_coding_edge_sizes() {
    assert!(class_coding(&[], Param::Ref).is_empty());
    assert!(class_coding(&[], Param::Glm).is_empty());
    let one = levels(&["X"]);
    assert_eq!(class_coding(&one, Param::Ref), vec![vec![] as Vec<f64>]);
    assert_eq!(class_coding(&one, Param::Effect), vec![vec![] as Vec<f64>]);
    assert_eq!(class_coding(&one, Param::Poly), vec![vec![] as Vec<f64>]);
    assert_eq!(class_coding(&one, Param::Glm), vec![vec![1.0]]);
}

// ─────────────────────── score_test (Rao) ───────────────────────

fn identity(k: usize) -> Vec<Vec<f64>> {
    (0..k)
        .map(|i| (0..k).map(|j| if i == j { 1.0 } else { 0.0 }).collect())
        .collect()
}

/// I = identity ⇒ χ² = Σ uᵢ², df = k.
#[test]
fn test_score_identity() {
    let u = vec![1.0, -2.0, 3.0];
    let st = score_test(&u, &identity(3));
    assert!(
        (st.chi_square - (1.0 + 4.0 + 9.0)).abs() < 1e-12,
        "chi2={}",
        st.chi_square
    );
    assert_eq!(st.df, 3.0);
    let p = st.p.unwrap();
    assert!((0.0..=1.0).contains(&p));
    // χ²=14 on 3 df → p ≈ 0.0029.
    assert!((p - 0.0029074).abs() < 1e-4, "p={p}");
}

/// Hand-checked 2×2: I = [[2,1],[1,2]] (det=3), I⁻¹ = (1/3)[[2,-1],[-1,2]].
/// U = [1, 0] ⇒ χ² = Uᵀ I⁻¹ U = inv[0][0] = 2/3.
#[test]
fn test_score_2x2() {
    let u = vec![1.0, 0.0];
    let info = vec![vec![2.0, 1.0], vec![1.0, 2.0]];
    let st = score_test(&u, &info);
    assert!(
        (st.chi_square - 2.0 / 3.0).abs() < 1e-10,
        "chi2={}",
        st.chi_square
    );
    assert_eq!(st.df, 2.0);
    // Full U = [1, 2]: Uᵀ I⁻¹ U = (1/3)(2·1 −1·2 −2·1 +2·4) = (1/3)(2−2−2+8)=2.
    let st2 = score_test(&[1.0, 2.0], &info);
    assert!(
        (st2.chi_square - 2.0).abs() < 1e-10,
        "chi2={}",
        st2.chi_square
    );
}

/// Singular information ⇒ NaN statistic, p = None.
#[test]
fn test_score_singular() {
    let u = vec![1.0, 1.0];
    let info = vec![vec![1.0, 1.0], vec![1.0, 1.0]]; // rank 1
    let st = score_test(&u, &info);
    assert!(st.chi_square.is_nan());
    assert!(st.p.is_none());
    assert_eq!(st.df, 2.0);
}
