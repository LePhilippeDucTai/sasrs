use super::super::*;
use super::*;

// ── Test 7: two-way design-matrix dimension check ─────────────────────

#[test]
fn test_two_way_design_dims() {
    // a has 2 levels, b has 3 levels, n observations.
    // Main effect a -> 1 dummy col; b -> 2 dummy cols; a*b -> 1*2 = 2 cols.
    // Full design = intercept(1) + 1 + 2 + 2 = 6 columns.
    let n = 6;
    let a_codes = vec![0usize, 0, 0, 1, 1, 1];
    let b_codes = vec![0usize, 1, 2, 0, 1, 2];
    let a_d = main_effect_dummies(&a_codes, 2, n);
    let b_d = main_effect_dummies(&b_codes, 3, n);
    assert_eq!(a_d.len(), 1, "a should give 1 dummy col");
    assert_eq!(b_d.len(), 2, "b should give 2 dummy cols");

    // Interaction columns: elementwise products of a's dummies × b's dummies.
    let mut inter: Vec<Vec<f64>> = Vec::new();
    for ac in &a_d {
        for bc in &b_d {
            inter.push(ac.iter().zip(bc).map(|(&x, &y)| x * y).collect());
        }
    }
    assert_eq!(inter.len(), 2, "a*b should give 2 interaction cols");
    // Full design columns count.
    let full_cols = 1 + a_d.len() + b_d.len() + inter.len();
    assert_eq!(full_cols, 6);
}

#[test]
fn test_two_way_balanced_type1_eq_type3() {
    // Balanced 2x2 with 2 reps per cell (n=8). Type I == Type III.
    // a: 0,0,0,0,1,1,1,1   b: 0,0,1,1,0,0,1,1
    let a = vec![0usize, 0, 0, 0, 1, 1, 1, 1];
    let b = vec![0usize, 0, 1, 1, 0, 0, 1, 1];
    let y = vec![10.0, 12.0, 20.0, 22.0, 30.0, 28.0, 44.0, 46.0];
    let (t1a, t1b, t3a, t3b, sst, sse_full) = two_way_ss(&a, 2, &b, 2, &y);

    assert!((t1a - t3a).abs() < 1e-7, "balanced: a t1={t1a} t3={t3a}");
    assert!((t1b - t3b).abs() < 1e-7, "balanced: b t1={t1b} t3={t3b}");
    // Type I should sum (with interaction omitted) to the model SS of the
    // two-main-effect model: SSM = SST - SSE_full.
    let ssm = sst - sse_full;
    assert!((t1a + t1b - ssm).abs() < 1e-7, "t1 sum {} != ssm {}", t1a + t1b, ssm);
}

#[test]
fn test_two_way_unbalanced_type1_ne_type3() {
    // UNBALANCED 2x2: cell counts differ so Type I != Type III for at least
    // one term, and the F/p use the fitted MSE.
    // a: 0,0,0,1,1   b: 0,1,1,0,1   (cell (0,0):1, (0,1):2, (1,0):1, (1,1):1)
    let a = vec![0usize, 0, 0, 1, 1];
    let b = vec![0usize, 1, 1, 0, 1];
    let y = vec![5.0, 9.0, 11.0, 20.0, 30.0];
    let (t1a, t1b, t3a, t3b, sst, sse_full) = two_way_ss(&a, 2, &b, 2, &y);

    // Type I of the LAST term equals its Type III (last sequential == partial
    // when it is the final term), but the FIRST term differs.
    assert!(
        (t1a - t3a).abs() > 1e-6,
        "unbalanced: expected a's TypeI({t1a}) != TypeIII({t3a})"
    );
    // Type I sums to model SS (2-main-effect model).
    let ssm = sst - sse_full;
    assert!(
        (t1a + t1b - ssm).abs() < 1e-7,
        "t1 sum {} != ssm {}",
        t1a + t1b,
        ssm
    );

    // F/p for term b against fitted MSE (df_error = n - cols = 5 - 3 = 2).
    let n = y.len();
    let full = build_two_way(&a, 2, &b, 2, false);
    let cols = full[0].len();
    let df_error = (n - cols) as f64;
    let mse = sse_full / df_error;
    let df_b = 1.0; // b has 2 levels -> 1 df
    let ms_b = t3b / df_b;
    let f_b = ms_b / mse;
    let p_b = (1.0 - f_cdf(f_b, df_b, df_error)).clamp(0.0, 1.0);
    assert!(f_b > 0.0 && f_b.is_finite(), "F_b={f_b}");
    assert!((0.0..=1.0).contains(&p_b), "p_b={p_b}");
}

#[test]
fn test_unbalanced_2x2_type3_effect_coding() {
    // UNBALANCED 2x2 with interaction. With reference-cell coding the
    // main-effect Type III SS are coding-dependent and wrong; sum-to-zero
    // effect coding gives the SAS Type III values. The highest-order
    // (interaction) Type III is already correct and must be unchanged.
    let a = vec![0usize, 0, 0, 1, 1, 1, 1];
    let b = vec![0usize, 1, 1, 0, 0, 1, 1];
    let y = vec![5.0, 9.0, 11.0, 20.0, 22.0, 30.0, 34.0];

    let (t3_a, t3_b, t3_ab, sse_ref, sse_eff) =
        two_way_full_type3_effect(&a, 2, &b, 2, &y);

    // SSE_full invariance: same fit, different basis.
    assert!(
        (sse_ref - sse_eff).abs() < 1e-6,
        "SSE_full not invariant: ref={sse_ref} eff={sse_eff}"
    );

    // Reference-cell Type III for the interaction (highest-order) term: this
    // is coding-invariant and must equal the effect-coded value.
    let n = y.len();
    let a_d = main_effect_dummies(&a, 2, n);
    let b_d = main_effect_dummies(&b, 2, n);
    let assemble_ref = |inc_ab: bool| -> Vec<Vec<f64>> {
        let mut rows = vec![vec![1.0]; n];
        for (i, row) in rows.iter_mut().enumerate() {
            for c in &a_d {
                row.push(c[i]);
            }
            for c in &b_d {
                row.push(c[i]);
            }
            if inc_ab {
                for ac in &a_d {
                    for bc in &b_d {
                        row.push(ac[i] * bc[i]);
                    }
                }
            }
        }
        rows
    };
    let sse_full_ref = fit_sse(&assemble_ref(true), &y);
    let sse_no_ab_ref = fit_sse(&assemble_ref(false), &y);
    let t3_ab_ref = sse_no_ab_ref - sse_full_ref;
    assert!(
        (t3_ab - t3_ab_ref).abs() < 1e-6,
        "interaction Type III must be coding-invariant: eff={t3_ab} ref={t3_ab_ref}"
    );

    // Main-effect Type III values are finite, positive, and (the point of
    // the fix) differ from the wrong reference-cell partial values.
    assert!(t3_a > 0.0 && t3_a.is_finite(), "t3_a={t3_a}");
    assert!(t3_b > 0.0 && t3_b.is_finite(), "t3_b={t3_b}");

    // Reference-cell "full minus a's dummy cols" partial (the OLD, wrong way).
    let assemble_ref_minus = |skip_a: bool, skip_b: bool| -> Vec<Vec<f64>> {
        let mut rows = vec![vec![1.0]; n];
        for (i, row) in rows.iter_mut().enumerate() {
            if !skip_a {
                for c in &a_d {
                    row.push(c[i]);
                }
            }
            if !skip_b {
                for c in &b_d {
                    row.push(c[i]);
                }
            }
            for ac in &a_d {
                for bc in &b_d {
                    row.push(ac[i] * bc[i]);
                }
            }
        }
        rows
    };
    let t3_a_ref_old = fit_sse(&assemble_ref_minus(true, false), &y) - sse_full_ref;
    // The main-effect Type III genuinely changed with the fix.
    assert!(
        (t3_a - t3_a_ref_old).abs() > 1e-6,
        "main-effect Type III should change: eff={t3_a} old-ref={t3_a_ref_old}"
    );

    eprintln!("unbalanced 2x2 Type III: a={t3_a} b={t3_b} a*b={t3_ab}");
}
