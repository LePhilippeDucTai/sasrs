use super::super::*;
use super::*;
use crate::ast::DatasetRef;
use crate::dataset::SasDataset;
use polars::df;

// ── M34.5: Type I vs Type III on an UNBALANCED two-way design ───────────

#[test]
fn test_type1_vs_type3_unbalanced() {
    // Unbalanced 2x2 (cell counts differ) so Type I != Type III.
    // a in {A,B}, b in {X,Y}.
    // (A,X): 1 obs y=10; (A,Y): 2 obs y=12,14; (B,X): 3 obs y=20,22,24; (B,Y): 1 obs y=30.
    let mut session = make_session();
    let frame = df![
        "a" => ["A","A","A","B","B","B","B"],
        "b" => ["X","Y","Y","X","X","X","Y"],
        "y" => [10.0_f64, 12.0, 14.0, 20.0, 22.0, 24.0, 30.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![char_meta("a"), char_meta("b"), num_meta("y")],
    };
    session.libs.get("WORK").unwrap().write("UB", &ds).unwrap();

    // Build factors / engine directly to inspect the SS numerically.
    let class_cols: Vec<(String, Vec<Value>)> = vec![
        (
            "a".into(),
            ["A", "A", "A", "B", "B", "B", "B"]
                .iter()
                .map(|s| Value::Char((*s).into()))
                .collect(),
        ),
        (
            "b".into(),
            ["X", "Y", "Y", "X", "X", "X", "Y"]
                .iter()
                .map(|s| Value::Char((*s).into()))
                .collect(),
        ),
    ];
    let y = vec![10.0, 12.0, 14.0, 20.0, 22.0, 24.0, 30.0];
    let mut factors: Vec<Factor> = Vec::new();
    for (name, col) in &class_cols {
        let mut levels: Vec<Value> = Vec::new();
        for v in col {
            if !levels
                .iter()
                .any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
            {
                levels.push(v.clone());
            }
        }
        levels.sort_by(|a, b| a.sas_cmp(b));
        factors.push(Factor {
            name: name.clone(),
            levels,
        });
    }
    let term_factor_idxs = vec![vec![0usize], vec![1usize]]; // a, b main effects
    let col_specs = term_column_specs(&term_factor_idxs, &factors);
    let n = y.len();
    let row_levels: Vec<Vec<usize>> = (0..n)
        .map(|r| {
            class_cols
                .iter()
                .enumerate()
                .map(|(fi, (_, col))| factors[fi].level_of(&col[r]))
                .collect()
        })
        .collect();
    let dummy_cache: Vec<Vec<Vec<f64>>> = row_levels
        .iter()
        .map(|rl| row_dummies(&factors, rl))
        .collect();
    let col_value = |row: usize, spec: &[(usize, usize)]| -> f64 {
        spec.iter()
            .map(|&(fi, dj)| dummy_cache[row][fi][dj])
            .product()
    };
    let build = |subset: &[usize]| -> Vec<Vec<f64>> {
        let mut d: Vec<Vec<f64>> = vec![vec![1.0]; n];
        for &t in subset {
            for spec in &col_specs[t] {
                for (r, row) in d.iter_mut().enumerate() {
                    row.push(col_value(r, spec));
                }
            }
        }
        d
    };
    let ybar = y.iter().sum::<f64>() / n as f64;
    let sst: f64 = y.iter().map(|v| (v - ybar).powi(2)).sum();
    let sse_full = sse_of(&build(&[0, 1]), &y);
    let ssm = sst - sse_full;

    // Type I: a then b.
    let sse_int = sse_of(&vec![vec![1.0]; n], &y);
    let sse_a = sse_of(&build(&[0]), &y);
    let t1_a = sse_int - sse_a;
    let t1_b = sse_a - sse_full;
    // Type I sums to model SS.
    assert!((t1_a + t1_b - ssm).abs() < 1e-8, "Type I should sum to SSM");

    // Type III: drop each term from full.
    let sse_drop_a = sse_of(&build(&[1]), &y); // full minus a = {intercept,b}
    let sse_drop_b = sse_of(&build(&[0]), &y); // full minus b = {intercept,a}
    let t3_a = sse_drop_a - sse_full;
    let t3_b = sse_drop_b - sse_full;

    // Unbalanced ⇒ Type I and Type III differ for the FIRST entered term (a).
    assert!(
        (t1_a - t3_a).abs() > 1e-6,
        "Type I vs III for 'a' should differ on unbalanced data: t1_a={t1_a}, t3_a={t3_a}"
    );
    // The last-entered term's Type I equals its Type III (b is adjusted for a in both).
    assert!(
        (t1_b - t3_b).abs() < 1e-8,
        "Type I and III for last term should match: {t1_b} vs {t3_b}"
    );
    // Also exercise the full execute path produces both tables.
    let ast = GlmAst {
        data_options: GlmDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "UB".into(),
            }),
        },
        class_vars: vec!["a".into(), "b".into()],
        model: Some(GlmModel {
            dependents: vec!["y".into()],
            effects: vec!["a".into(), "b".into()],
            effect_terms: vec![vec!["a".into()], vec!["b".into()]],
            solution: false,
            noprint: false,
        }),
        lsmeans_vars: vec![],
        estimates: vec![],
        contrasts: vec![],
        means_vars: vec![],
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("Type I SS"), "missing Type I: {listing}");
    assert!(
        listing.contains("Type III SS"),
        "missing Type III: {listing}"
    );
}

// ── M34.5 fix: effect-coded Type III on an UNBALANCED 2×2 WITH interaction ─
//
// Model: y = a b a*b on an unbalanced 2×2. Checks:
//  1. The effect-coded full model SSE == reference-cell full model SSE (~1e-6).
//  2. The interaction term's Type III is the SAME under both codings (it is the
//     highest-order term, so dropping it is coding-invariant).
//  3. The MAIN-EFFECT Type III values CHANGE between reference-cell and effect
//     coding — effect coding gives the SAS-correct estimable-function SS.
#[test]
fn test_type3_effect_coding_2x2_interaction() {
    // (A,X): 1 obs y=10; (A,Y): 2 obs y=12,14;
    // (B,X): 3 obs y=20,22,24; (B,Y): 1 obs y=30.  (same unbalanced cells)
    let class_cols: Vec<(String, Vec<Value>)> = vec![
        (
            "a".into(),
            ["A", "A", "A", "B", "B", "B", "B"]
                .iter()
                .map(|s| Value::Char((*s).into()))
                .collect(),
        ),
        (
            "b".into(),
            ["X", "Y", "Y", "X", "X", "X", "Y"]
                .iter()
                .map(|s| Value::Char((*s).into()))
                .collect(),
        ),
    ];
    let y = vec![10.0, 12.0, 14.0, 20.0, 22.0, 24.0, 30.0];
    let n = y.len();

    let mut factors: Vec<Factor> = Vec::new();
    for (name, col) in &class_cols {
        let mut levels: Vec<Value> = Vec::new();
        for v in col {
            if !levels
                .iter()
                .any(|l| l.sas_cmp(v) == std::cmp::Ordering::Equal)
            {
                levels.push(v.clone());
            }
        }
        levels.sort_by(|a, b| a.sas_cmp(b));
        factors.push(Factor {
            name: name.clone(),
            levels,
        });
    }
    // Terms: a, b, a*b.
    let term_factor_idxs = vec![vec![0usize], vec![1usize], vec![0usize, 1usize]];
    let col_specs = term_column_specs(&term_factor_idxs, &factors);

    let row_levels: Vec<Vec<usize>> = (0..n)
        .map(|r| {
            class_cols
                .iter()
                .enumerate()
                .map(|(fi, (_, col))| factors[fi].level_of(&col[r]))
                .collect()
        })
        .collect();
    let dummy_cache: Vec<Vec<Vec<f64>>> = row_levels
        .iter()
        .map(|rl| row_dummies(&factors, rl))
        .collect();
    let effect_cache: Vec<Vec<Vec<f64>>> = row_levels
        .iter()
        .map(|rl| row_effects(&factors, rl))
        .collect();

    // Build a design (intercept + given terms) from a chosen coding cache.
    let build = |subset: &[usize], cache: &[Vec<Vec<f64>>]| -> Vec<Vec<f64>> {
        let mut d: Vec<Vec<f64>> = vec![vec![1.0]; n];
        for &t in subset {
            for spec in &col_specs[t] {
                for (r, row) in d.iter_mut().enumerate() {
                    let v: f64 = spec.iter().map(|&(fi, dj)| cache[r][fi][dj]).product();
                    row.push(v);
                }
            }
        }
        d
    };

    // (1) Effect-coded full SSE == reference-cell full SSE.
    let sse_full_ref = sse_of(&build(&[0, 1, 2], &dummy_cache), &y);
    let sse_full_eff = sse_of(&build(&[0, 1, 2], &effect_cache), &y);
    assert!(
        (sse_full_ref - sse_full_eff).abs() < 1e-6,
        "effect-coded full SSE must equal reference-cell full SSE: {sse_full_ref} vs {sse_full_eff}"
    );

    // Reference-cell Type III (the OLD, incorrect-for-main-effects approach).
    let t3_ref = |t: usize| -> f64 {
        let subset: Vec<usize> = (0..3).filter(|&x| x != t).collect();
        sse_of(&build(&subset, &dummy_cache), &y) - sse_full_ref
    };
    // Effect-coded Type III (the FIXED approach).
    let t3_eff = |t: usize| -> f64 {
        let subset: Vec<usize> = (0..3).filter(|&x| x != t).collect();
        sse_of(&build(&subset, &effect_cache), &y) - sse_full_eff
    };

    // (2) Interaction term (t=2) is highest-order → Type III coding-invariant.
    assert!(
        (t3_ref(2) - t3_eff(2)).abs() < 1e-6,
        "interaction Type III must be unchanged: ref={} eff={}",
        t3_ref(2),
        t3_eff(2)
    );

    // (3) Main-effect Type III values CHANGE between codings.
    assert!(
        (t3_ref(0) - t3_eff(0)).abs() > 1e-6,
        "main-effect 'a' Type III must change: ref={} eff={}",
        t3_ref(0),
        t3_eff(0)
    );
    assert!(
        (t3_ref(1) - t3_eff(1)).abs() > 1e-6,
        "main-effect 'b' Type III must change: ref={} eff={}",
        t3_ref(1),
        t3_eff(1)
    );

    // Type I (coding-invariant) still sums to Model SS, regardless of coding.
    let ybar = y.iter().sum::<f64>() / n as f64;
    let sst: f64 = y.iter().map(|v| (v - ybar).powi(2)).sum();
    let ssm = sst - sse_full_ref;
    let sse_int = sse_of(&vec![vec![1.0]; n], &y);
    let sse_a = sse_of(&build(&[0], &dummy_cache), &y);
    let sse_ab = sse_of(&build(&[0, 1], &dummy_cache), &y);
    let t1_a = sse_int - sse_a;
    let t1_b = sse_a - sse_ab;
    let t1_ab = sse_ab - sse_full_ref;
    assert!(
        (t1_a + t1_b + t1_ab - ssm).abs() < 1e-8,
        "Type I must sum to Model SS: {t1_a}+{t1_b}+{t1_ab} vs {ssm}"
    );

    // Report the corrected main-effect Type III values (effect coding).
    eprintln!(
        "effect-coded Type III: a={:.6} b={:.6} a*b={:.6} (sse_full={:.6})",
        t3_eff(0),
        t3_eff(1),
        t3_eff(2),
        sse_full_eff
    );
    eprintln!(
        "reference-cell Type III (old): a={:.6} b={:.6} a*b={:.6}",
        t3_ref(0),
        t3_ref(1),
        t3_ref(2)
    );
}
