use super::*;

// ── Test 1: one-way ANOVA arithmetic ──────────────────────────────────

#[test]
fn test_one_way_anova_simple() {
    // y=[1,2,3,10,11,12], groups=["A","A","A","B","B","B"]
    // k=2, n=6, ȳ_A=2, ȳ_B=11, ȳ=6.5
    // SSModel = 3*(2-6.5)² + 3*(11-6.5)² = 60.75 + 60.75 = 121.5
    // SSE = (1-2)²+(2-2)²+(3-2)² + (10-11)²+(11-11)²+(12-11)² = 2 + 2 = 4
    // df_model=1, df_error=4, MSModel=121.5, MSE=1.0
    // F = 121.5

    let a_group: Vec<f64> = vec![1.0, 2.0, 3.0];
    let b_group: Vec<f64> = vec![10.0, 11.0, 12.0];
    let all_vals: Vec<f64> = a_group.iter().chain(b_group.iter()).cloned().collect();
    let n = 6usize;
    let k = 2usize;

    let y_bar = all_vals.iter().sum::<f64>() / n as f64;
    assert!((y_bar - 6.5).abs() < 1e-10, "y_bar={y_bar}");

    let y_bar_a = 2.0_f64;
    let y_bar_b = 11.0_f64;
    let ssm = 3.0 * (y_bar_a - y_bar).powi(2) + 3.0 * (y_bar_b - y_bar).powi(2);
    assert!((ssm - 121.5).abs() < 1e-9, "ssm={ssm}");

    let sse_a: f64 = a_group.iter().map(|&y| (y - y_bar_a).powi(2)).sum();
    let sse_b: f64 = b_group.iter().map(|&y| (y - y_bar_b).powi(2)).sum();
    let sse = sse_a + sse_b;
    assert!((sse - 4.0).abs() < 1e-9, "sse={sse}");

    let df_model = (k - 1) as f64;
    let df_error = (n - k) as f64;
    let msm = ssm / df_model;
    let mse = sse / df_error;
    let f_stat = msm / mse;

    assert!((f_stat - 121.5).abs() < 1e-9, "F={f_stat}");

    let p = (1.0 - f_cdf(f_stat, df_model, df_error)).clamp(0.0, 1.0);
    assert!(f_stat > 100.0, "F should be > 100, got {f_stat}");
    assert!(p < 0.001, "p should be very small, got {p}");
}

// ── Test 2: parse model with multiple dependents ───────────────────────

#[test]
fn test_parse_model_multi_dep() {
    let ast =
        parse_anova("proc anova; class sex; model height weight = sex; means sex; run;").unwrap();
    let m = ast.model.unwrap();
    assert_eq!(m.dependents, vec!["height", "weight"]);
    assert_eq!(m.effects, vec!["sex"]);
    assert_eq!(ast.means_vars, vec!["sex"]);
}

// ── Test 3: parse class with multiple vars ────────────────────────────

#[test]
fn test_parse_class() {
    let ast = parse_anova("proc anova data=x; class a b; model y = a; run;").unwrap();
    assert_eq!(ast.class_vars, vec!["a", "b"]);
}

// ── Test 6: effect-term parsing of `a b a*b` ──────────────────────────

#[test]
fn test_parse_interaction_terms() {
    let ast = parse_anova("proc anova; class a b; model y = a b a*b; run;").unwrap();
    let m = ast.model.unwrap();
    assert_eq!(m.effects, vec!["a", "b", "a*b"]);
    assert_eq!(
        m.terms,
        vec![
            vec!["a".to_string()],
            vec!["b".to_string()],
            vec!["a".to_string(), "b".to_string()],
        ]
    );
}

#[test]
fn test_parse_three_way_interaction() {
    let ast = parse_anova("proc anova; class a b c; model y = a*b*c; run;").unwrap();
    let m = ast.model.unwrap();
    assert_eq!(m.effects, vec!["a*b*c"]);
    assert_eq!(
        m.terms,
        vec![vec!["a".to_string(), "b".to_string(), "c".to_string()]]
    );
}
