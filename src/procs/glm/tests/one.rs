use super::super::*;
use super::*;
use crate::ast::DatasetRef;
use crate::dataset::SasDataset;
use polars::df;

// ── Test 1: one-way GLM parameter estimates ────────────────────────────

#[test]
fn test_one_way_glm_params() {
    // y=[1,2,3,10,11,12], groups=["A","B"]
    // ȳ_A = 2.0, ȳ_B = 11.0
    // Reference = last level = "B"
    // Intercept = ȳ_B = 11.0
    // Effect A = ȳ_A - ȳ_B = 2.0 - 11.0 = -9.0

    let a_group: Vec<f64> = vec![1.0, 2.0, 3.0];
    let b_group: Vec<f64> = vec![10.0, 11.0, 12.0];

    let y_bar_a = a_group.iter().sum::<f64>() / a_group.len() as f64;
    let y_bar_b = b_group.iter().sum::<f64>() / b_group.len() as f64;

    assert!((y_bar_a - 2.0).abs() < 1e-10, "y_bar_a={y_bar_a}");
    assert!((y_bar_b - 11.0).abs() < 1e-10, "y_bar_b={y_bar_b}");

    // reference = last in sas_cmp order = "B" (B > A alphabetically)
    let intercept = y_bar_b;
    let effect_a = y_bar_a - y_bar_b;

    assert!((intercept - 11.0).abs() < 1e-10, "intercept={intercept}");
    assert!((effect_a - (-9.0)).abs() < 1e-10, "effect_a={effect_a}");
}

// ── Test 2: parse model with /SOLUTION ───────────────────────────────

#[test]
fn test_parse_model_solution() {
    let ast = parse_glm(
        "proc glm; class sex; model height = sex / solution; run;",
    )
    .unwrap();
    let m = ast.model.unwrap();
    assert!(m.solution, "solution should be true");
    assert_eq!(m.dependents, vec!["height"]);
    assert_eq!(m.effects, vec!["sex"]);
}

// ── Test 3: parse ESTIMATE statement ─────────────────────────────────

#[test]
fn test_parse_estimate() {
    let ast = parse_glm(
        "proc glm; class sex; model y = sex; estimate 'F vs M' sex 1 -1; run;",
    )
    .unwrap();
    assert_eq!(ast.estimates.len(), 1);
    let e = &ast.estimates[0];
    assert_eq!(e.label, "F vs M");
    assert_eq!(e.effect, "sex");
    assert_eq!(e.coefficients.len(), 2);
    assert!((e.coefficients[0] - 1.0).abs() < 1e-10);
    assert!((e.coefficients[1] - (-1.0)).abs() < 1e-10);
}

// ── Test 4: parse CONTRAST statement ─────────────────────────────────

#[test]
fn test_parse_contrast() {
    let ast = parse_glm(
        "proc glm; class sex; model y = sex; contrast 'F vs M' sex 1 -1; run;",
    )
    .unwrap();
    assert_eq!(ast.contrasts.len(), 1);
    let c = &ast.contrasts[0];
    assert_eq!(c.label, "F vs M");
    assert_eq!(c.effect, "sex");
    assert_eq!(c.coefficients, vec![1.0, -1.0]);
}

// ── M34.5: effect-term parsing `a b a*b` ────────────────────────────────

#[test]
fn test_parse_interaction_terms() {
    let ast = parse_glm(
        "proc glm; class a b; model y = a b a*b / solution; run;",
    )
    .unwrap();
    let m = ast.model.unwrap();
    // Legacy flat list keeps `a*b` joined.
    assert_eq!(m.effects, vec!["a", "b", "a*b"]);
    // Structured terms: main effects 1 elt, interaction 2 elts.
    assert_eq!(
        m.effect_terms,
        vec![
            vec!["a".to_string()],
            vec!["b".to_string()],
            vec!["a".to_string(), "b".to_string()],
        ]
    );
    assert!(m.solution);
}

// ── Test 5: execute listing contains LSMEANS ─────────────────────────

#[test]
fn test_execute_lsmeans() {
    let mut session = make_session();
    let frame = df![
        "sex"    => ["F","F","F","M","M","M"],
        "height" => [62.0_f64, 63.0, 64.0, 69.0, 70.0, 71.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![char_meta("sex"), num_meta("height")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let ast = GlmAst {
        data_options: GlmDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
        },
        class_vars: vec!["sex".into()],
        model: Some(GlmModel {
            dependents: vec!["height".into()],
            effects: vec!["sex".into()],
            effect_terms: vec![vec!["sex".into()]],
            solution: false,
            noprint: false,
        }),
        lsmeans_vars: vec!["sex".into()],
        estimates: vec![],
        contrasts: vec![],
        means_vars: vec![],
    };

    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();

    assert!(listing.contains("Least Squares Means"), "listing={listing}");
}

// ── Test 6: ESTIMATE arithmetic ──────────────────────────────────────

#[test]
fn test_execute_estimate_correct() {
    // y=[1,2,3,10,11,12], groups=["A","B"]
    // ȳ_A=2, ȳ_B=11, ESTIMATE 'A vs B' sex 1 -1 (coefficient order = sas_cmp A,B)
    // Estimate = 1*2 + (-1)*11 = -9
    // SSE = 2+2 = 4, df_error = 4, MSE = 1
    // SE = √(1*(1/3+1/3)) = √(2/3) ≈ 0.8165
    // t = -9 / 0.8165 ≈ -11.02 (negative, A < B)

    let mut session = make_session();
    let frame = df![
        "sex"    => ["A","A","A","B","B","B"],
        "height" => [1.0_f64, 2.0, 3.0, 10.0, 11.0, 12.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![char_meta("sex"), num_meta("height")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let ast = GlmAst {
        data_options: GlmDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
        },
        class_vars: vec!["sex".into()],
        model: Some(GlmModel {
            dependents: vec!["height".into()],
            effects: vec!["sex".into()],
            effect_terms: vec![vec!["sex".into()]],
            solution: false,
            noprint: false,
        }),
        lsmeans_vars: vec![],
        estimates: vec![GlmEstimate {
            label: "A vs B".into(),
            effect: "sex".into(),
            coefficients: vec![1.0, -1.0],
        }],
        contrasts: vec![],
        means_vars: vec![],
    };

    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();

    // Estimate should be -9.0
    assert!(listing.contains("-9.000000"), "Expected -9.000000 in listing: {listing}");
    // t value should be negative
    assert!(listing.contains("-11"), "Expected negative t value: {listing}");
}

// ── Test 7: CONTRAST F = (ESTIMATE t)² ───────────────────────────────

#[test]
fn test_execute_contrast_f_eq_t_squared() {
    // Same data as test 6: A=[1,2,3], B=[10,11,12]
    // ESTIMATE t ≈ -11.02, CONTRAST F ≈ 121.5
    // t² = 121.5 ≈ F (within rounding)

    let mut session = make_session();
    let frame = df![
        "sex"    => ["A","A","A","B","B","B"],
        "height" => [1.0_f64, 2.0, 3.0, 10.0, 11.0, 12.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![char_meta("sex"), num_meta("height")],
    };
    session.libs.get("WORK").unwrap().write("T2", &ds).unwrap();

    let ast = GlmAst {
        data_options: GlmDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T2".into(),
            }),
        },
        class_vars: vec!["sex".into()],
        model: Some(GlmModel {
            dependents: vec!["height".into()],
            effects: vec!["sex".into()],
            effect_terms: vec![vec!["sex".into()]],
            solution: false,
            noprint: false,
        }),
        lsmeans_vars: vec![],
        estimates: vec![GlmEstimate {
            label: "A vs B".into(),
            effect: "sex".into(),
            coefficients: vec![1.0, -1.0],
        }],
        contrasts: vec![GlmContrast {
            label: "A vs B".into(),
            effect: "sex".into(),
            coefficients: vec![1.0, -1.0],
        }],
        means_vars: vec![],
    };

    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();

    // F should be 121.5 (= t² = 11.02² ≈ 121.5)
    // Check both sections are present
    assert!(listing.contains("Estimates"), "listing missing Estimates: {listing}");
    assert!(listing.contains("Contrasts"), "listing missing Contrasts: {listing}");
    // The ANOVA F is 121.5 (same as the contrast F)
    assert!(listing.contains("121.50"), "Expected F=121.50 in listing: {listing}");
}

// ── M34.5: two-way design matrix dimensions ─────────────────────────────

#[test]
fn test_two_way_design_dimensions() {
    // Factor a: 3 levels (A1,A2,A3), b: 2 levels (B1,B2). Balanced 2 obs/cell.
    // Reference-cell columns: intercept(1) + a(2) + b(1) + a*b(2) = 6.
    let fa = Factor {
        name: "a".into(),
        levels: vec![
            Value::Char("A1".into()),
            Value::Char("A2".into()),
            Value::Char("A3".into()),
        ],
    };
    let fb = Factor {
        name: "b".into(),
        levels: vec![Value::Char("B1".into()), Value::Char("B2".into())],
    };
    let factors = vec![fa, fb];
    assert_eq!(factors[0].n_dummies(), 2);
    assert_eq!(factors[1].n_dummies(), 1);

    // terms: a (factor 0), b (factor 1), a*b (0,1)
    let term_factor_idxs = vec![vec![0usize], vec![1usize], vec![0usize, 1usize]];
    let specs = term_column_specs(&term_factor_idxs, &factors);
    let ncols: usize = 1 + specs.iter().map(|s| s.len()).sum::<usize>();
    assert_eq!(specs[0].len(), 2); // a
    assert_eq!(specs[1].len(), 1); // b
    assert_eq!(specs[2].len(), 2); // a*b = 2*1
    assert_eq!(ncols, 6);
}

// ── M34.5: reference-cell betas on a balanced 2×2 design ────────────────

#[test]
fn test_reference_cell_betas_2x2() {
    // Balanced 2x2: cell means chosen, two obs per cell.
    // a in {A,B}, b in {X,Y}. Reference = last level: a=B, b=Y.
    // Cell means: (A,X)=10, (A,Y)=14, (B,X)=20, (B,Y)=30.
    // Reference-cell model y = mu + a_A + b_X + ab_AX:
    //   mu = mean(B,Y) = 30
    //   b_X = mean(B,X) - mean(B,Y) = 20 - 30 = -10
    //   a_A = mean(A,Y) - mean(B,Y) = 14 - 30 = -16
    //   ab_AX = (A,X) - (A,Y) - (B,X) + (B,Y) = 10 - 14 - 20 + 30 = 6
    let mut session = make_session();
    let frame = df![
        "a" => ["A","A","A","A","B","B","B","B"],
        "b" => ["X","X","Y","Y","X","X","Y","Y"],
        "y" => [10.0_f64,10.0, 14.0,14.0, 20.0,20.0, 30.0,30.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![char_meta("a"), char_meta("b"), num_meta("y")],
    };
    session.libs.get("WORK").unwrap().write("TW", &ds).unwrap();

    let ast = GlmAst {
        data_options: GlmDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "TW".into(),
            }),
        },
        class_vars: vec!["a".into(), "b".into()],
        model: Some(GlmModel {
            dependents: vec!["y".into()],
            effects: vec!["a".into(), "b".into(), "a*b".into()],
            effect_terms: vec![
                vec!["a".into()],
                vec!["b".into()],
                vec!["a".into(), "b".into()],
            ],
            solution: true,
            noprint: false,
        }),
        lsmeans_vars: vec!["a".into()],
        estimates: vec![],
        contrasts: vec![],
        means_vars: vec![],
    };

    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();

    // Intercept (mu) = 30.000000
    assert!(listing.contains("30.000000"), "expected intercept 30: {listing}");
    // a A = -16.000000
    assert!(listing.contains("-16.000000"), "expected a A=-16: {listing}");
    // b X = -10.000000
    assert!(listing.contains("-10.000000"), "expected b X=-10: {listing}");
    // interaction a A b X = 6.000000
    assert!(listing.contains("6.000000"), "expected ab=6: {listing}");
    // LSMEAN for a=A is mean of cell means over b = (10+14)/2 = 12; a=B = 25.
    assert!(listing.contains("12.000000"), "expected LSMEAN a=A 12: {listing}");
    assert!(listing.contains("25.000000"), "expected LSMEAN a=B 25: {listing}");
}
