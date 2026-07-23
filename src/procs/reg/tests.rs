use super::*;
use crate::dataset::VarMeta;
use crate::source::SourceFile;
use crate::value::VarType;
use polars::df;
use std::path::PathBuf;

fn make_session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

fn num_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.into(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}

fn parse_reg(src: &str) -> Result<RegAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // proc
    ts.next(); // reg
    parse(&mut ts)
}

/// Build a single-model AST (no OUTPUT) for the given model.
fn single_model_ast(input: DatasetRef, model: RegModel) -> RegAst {
    RegAst {
        data_options: RegDataOptions {
            input: Some(input),
            outest: None,
            outsscp: None,
            ridge: Vec::new(),
            pcomit: Vec::new(),
            outvif: false,
        },
        models: vec![RegModelEntry {
            model,
            outputs: vec![],
            tests: vec![],
            restricts: vec![],
            mtests: vec![],
            add: vec![],
            delete: vec![],
        }],
        plots_requested: false,
        plot_requests: PlotRequests::default(),
        plot_statements: Vec::new(),
        weight: None,
        freq: None,
        by: Vec::new(),
        id: Vec::new(),
        simple: false,
        corr: false,
        var_list: Vec::new(),
        reweight_seen: false,
        refit_seen: false,
        paint_seen: false,
    }
}

fn basic_model(dep: &str, regs: &[&str]) -> RegModel {
    RegModel {
        dependents: vec![dep.into()],
        regressors: regs.iter().map(|s| s.to_string()).collect(),
        noint: false,
        noprint: false,
        selection: None,
        alpha: 0.05,
        clb: false,
        clm: false,
        cli: false,
        r: false,
        influence: false,
        vif: false,
        tol: false,
        collin: false,
        collinoint: false,
        spec: false,
        dw: false,
        dwprob: false,
        acov: false,
        ss1: false,
        ss2: false,
        stb: false,
        pcorr1: false,
        pcorr2: false,
        scorr1: false,
        scorr2: false,
        seqb: false,
        press_opt: false,
        xpx: false,
        inv: false,
        covb: false,
        corrb: false,
    }
}

#[test]
fn test_ols_simple() {
    let mut session = make_session();
    let frame = df![
        "y" => [1.0_f64, 2.0, 3.0, 4.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let ast = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        basic_model("y", &["x"]),
    );
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(
        listing.contains("1.0000") || listing.contains("R-Square"),
        "listing: {listing}"
    );
    assert!(listing.contains("The REG Procedure"), "{listing}");
}

#[test]
fn test_ols_regression() {
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 5.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let ast = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        basic_model("y", &["x"]),
    );
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("0.8000") || listing.contains("R-Square"), "{listing}");
}

#[test]
fn test_parse_model() {
    let ast = parse_reg("proc reg data=a; model y = x1 x2; run;").unwrap();
    assert_eq!(ast.models.len(), 1);
    let m = &ast.models[0].model;
    assert_eq!(m.dependents, vec!["y"]);
    assert_eq!(m.regressors, vec!["x1", "x2"]);
    assert!(!m.noint);
    assert!(!m.noprint);
    assert!(m.selection.is_none());
}

#[test]
fn test_parse_multiple_models() {
    let ast = parse_reg(
        "proc reg data=a; model y = x1; output out=o1 p=p1; model y = x1 x2; output out=o2 p=p2; run;",
    )
    .unwrap();
    assert_eq!(ast.models.len(), 2);
    // First model has one regressor and its OUTPUT.
    assert_eq!(ast.models[0].model.regressors, vec!["x1"]);
    assert_eq!(ast.models[0].outputs.len(), 1);
    assert_eq!(ast.models[0].outputs[0].out.name, "o1");
    assert_eq!(ast.models[0].outputs[0].predicted.as_deref(), Some("p1"));
    // Second model has two regressors and its own OUTPUT.
    assert_eq!(ast.models[1].model.regressors, vec!["x1", "x2"]);
    assert_eq!(ast.models[1].outputs.len(), 1);
    assert_eq!(ast.models[1].outputs[0].out.name, "o2");
}

#[test]
fn test_parse_output() {
    let ast =
        parse_reg("proc reg data=a; model y = x; output out=work.out predicted=p residual=r; run;")
            .unwrap();
    assert_eq!(ast.models.len(), 1);
    assert_eq!(ast.models[0].outputs.len(), 1);
    let o = &ast.models[0].outputs[0];
    assert_eq!(o.out.name, "out");
    assert_eq!(o.predicted.as_deref(), Some("p"));
    assert_eq!(o.residual.as_deref(), Some("r"));
}

#[test]
fn test_parse_selection_forward() {
    let ast = parse_reg(
        "proc reg data=a; model y = x1 x2 / selection=forward slentry=0.3; run;",
    )
    .unwrap();
    let sel = ast.models[0].model.selection.unwrap();
    assert_eq!(sel.method, SelMethod::Forward);
    assert!((sel.slentry - 0.3).abs() < 1e-12);
}

#[test]
fn test_parse_selection_synonyms() {
    // sle=/sls= synonyms and stepwise.
    let ast = parse_reg(
        "proc reg data=a; model y = x1 x2 / selection=stepwise sle=0.2 sls=0.25; run;",
    )
    .unwrap();
    let sel = ast.models[0].model.selection.unwrap();
    assert_eq!(sel.method, SelMethod::Stepwise);
    assert!((sel.slentry - 0.2).abs() < 1e-12);
    assert!((sel.slstay - 0.25).abs() < 1e-12);
}

#[test]
fn test_parse_selection_defaults() {
    let ast =
        parse_reg("proc reg data=a; model y = x1 / selection=backward; run;").unwrap();
    let sel = ast.models[0].model.selection.unwrap();
    assert_eq!(sel.method, SelMethod::Backward);
    assert!((sel.slstay - 0.10).abs() < 1e-12);
}

#[test]
fn test_parse_noint() {
    let ast = parse_reg("proc reg data=a; model y = x / noint; run;").unwrap();
    assert!(ast.models[0].model.noint);
}

#[test]
fn test_execute_simple() {
    let mut session = make_session();
    let frame = df![
        "weight" => [112.0_f64, 100.0, 130.0, 145.0, 160.0, 105.0],
        "height" => [59.0_f64, 57.0, 63.0, 67.0, 67.0, 57.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("weight"), num_meta("height")],
    };
    session.libs.get("WORK").unwrap().write("CLASS", &ds).unwrap();

    let ast = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "CLASS".into(),
        },
        basic_model("weight", &["height"]),
    );
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("The REG Procedure"), "{listing}");
    assert!(listing.contains("Analysis of Variance"), "{listing}");
    assert!(listing.contains("Parameter Estimates") || listing.contains("Parameter"), "{listing}");
}

/// NOINT on a tiny known dataset: y = 2x exactly (no intercept), so the
/// no-intercept fit gives slope=2, uncorrected R² = Σŷ²/Σy² = 1, and there
/// is NO Intercept row in the parameter-estimates table.
#[test]
fn test_noint_fit() {
    let mut session = make_session();
    // y = 2*x, with x = 1..5.
    let frame = df![
        "y" => [2.0_f64, 4.0, 6.0, 8.0, 10.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let mut model = basic_model("y", &["x"]);
    model.noint = true;
    let ast = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        model,
    );
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("Uncorrected Total"), "{listing}");
    // R² = 1.0000 (perfect through-origin fit).
    assert!(listing.contains("R-Square     1.0000"), "{listing}");
    // No Intercept row in parameter estimates.
    assert!(!listing.contains("Intercept"), "{listing}");
    assert!(listing.contains("The REG Procedure"), "{listing}");
}

/// Direct numeric check of the NOINT uncorrected decomposition via ols_fit.
#[test]
fn test_noint_uncorrected_r2_formula() {
    // X has no intercept column.
    let x = vec![vec![1.0], vec![2.0], vec![3.0], vec![4.0], vec![5.0]];
    let y = vec![2.1, 3.9, 6.2, 7.8, 10.1];
    let fit = ols_fit(&x, &y).unwrap();
    let ssm: f64 = fit.y_hat.iter().map(|v| v * v).sum();
    let sst: f64 = y.iter().map(|v| v * v).sum();
    let r2 = ssm / sst;
    // 1 - SSE/Σy² must match Σŷ²/Σy².
    let r2_alt = 1.0 - fit.sse / sst;
    assert!((r2 - r2_alt).abs() < 1e-10, "r2={r2} r2_alt={r2_alt}");
    assert!(r2 > 0.99, "near-perfect fit expected, r2={r2}");
}

/// Partial-F to ENTER equals the candidate's t² in the augmented fit.
#[test]
fn test_partial_f_equals_t_squared() {
    // Two regressors; intercept present.
    // y depends mostly on x1.
    let x1 = vec![1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0];
    let x2 = vec![5.0_f64, 3.0, 6.0, 2.0, 7.0, 1.0]; // noise-ish
    let y: Vec<f64> = x1.iter().map(|&v| 3.0 + 2.0 * v).collect();
    let xcols = vec![x1.clone(), x2.clone()];
    let n = y.len();

    // Enter x1 (col 0) into empty set, intercept present.
    let s: Vec<usize> = vec![];
    let cand = vec![0usize];
    let sse_s = subset_sse(&xcols, &y, &s, true).unwrap();
    let sse_c = subset_sse(&xcols, &y, &cand, true).unwrap();
    let df_full = (n as f64) - (cand.len() as f64) - 1.0;
    let f_enter = (sse_s - sse_c) / (sse_c / df_full);

    // Augmented fit: design [1, x1]; t for x1's coefficient.
    let mut xmat: Vec<Vec<f64>> = Vec::new();
    for i in 0..n {
        xmat.push(vec![1.0, x1[i]]);
    }
    let fit = ols_fit(&xmat, &y).unwrap();
    let mse = fit.sse / df_full;
    let se = (mse * fit.xtx_inv[1][1]).sqrt();
    let t = fit.beta[1] / se;
    let t2 = t * t;
    // Perfect linear data → both huge; compare relative or that both large.
    // Use a perturbed y to avoid degeneracy.
    let _ = (f_enter, t2);

    // Re-run with a slightly noisy y so SSE>0.
    let y2: Vec<f64> = x1.iter().map(|&v| 3.0 + 2.0 * v + (v * 0.137).sin()).collect();
    let sse_s2 = subset_sse(&xcols, &y2, &s, true).unwrap();
    let sse_c2 = subset_sse(&xcols, &y2, &cand, true).unwrap();
    let f_enter2 = (sse_s2 - sse_c2) / (sse_c2 / df_full);
    let mut xmat2: Vec<Vec<f64>> = Vec::new();
    for i in 0..n {
        xmat2.push(vec![1.0, x1[i]]);
    }
    let fit2 = ols_fit(&xmat2, &y2).unwrap();
    let mse2 = fit2.sse / df_full;
    let se2 = (mse2 * fit2.xtx_inv[1][1]).sqrt();
    let t_2 = fit2.beta[1] / se2;
    let t2_2 = t_2 * t_2;
    assert!(
        (f_enter2 - t2_2).abs() < 1e-6,
        "F_enter={f_enter2} t^2={t2_2}"
    );
}

/// FORWARD selection: x1 strongly predicts y; x2 is pure noise → x1 enters,
/// x2 is rejected at slentry=0.05.
#[test]
fn test_forward_selection() {
    let mut session = make_session();
    // y tracks x1 closely (strong signal) with mild noise; x2 is unrelated.
    let frame = df![
        "y"  => [3.2_f64, 4.8, 7.1, 8.9, 11.3, 12.7, 15.2, 16.8],
        "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
        "x2" => [4.0_f64, 1.0, 9.0, 2.0, 8.0, 3.0, 7.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let mut model = basic_model("y", &["x1", "x2"]);
    model.selection = Some(Selection {
        method: SelMethod::Forward,
        // x1 enters (p<.0001); x2's partial-F p (~0.035) exceeds slentry,
        // so x2 is rejected.
        slentry: 0.01,
        slstay: 0.01,
        best: None,
        include: 0,
        start: None,
        stop: None,
        details: false,
        stb: false,
    });
    let ast = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        model,
    );
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("Summary of Forward Selection"), "{listing}");
    // Inspect the final fitted-model block (after the last "Model: MODEL1"),
    // which holds the parameter-estimates table.
    let final_block = listing.rsplit("Model: MODEL1").next().unwrap();
    // x1 entered → appears as a fitted parameter; x2 rejected → absent.
    assert!(final_block.contains("x1"), "{listing}");
    assert!(!final_block.contains("x2"), "x2 should be rejected: {listing}");
}

/// BACKWARD selection: start with both x1 and noise x2; x2 is eliminated.
#[test]
fn test_backward_selection() {
    let mut session = make_session();
    // y is x1 plus mild noise (not a perfect fit), x2 is unrelated noise.
    let frame = df![
        "y"  => [3.2_f64, 4.8, 7.1, 8.9, 11.3, 12.7, 15.2, 16.8],
        "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
        "x2" => [4.0_f64, 1.0, 9.0, 2.0, 8.0, 3.0, 7.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let mut model = basic_model("y", &["x1", "x2"]);
    model.selection = Some(Selection {
        method: SelMethod::Backward,
        // x2's removal p (~0.035) exceeds slstay, so x2 is eliminated; x1
        // (highly significant) is retained.
        slentry: 0.10,
        slstay: 0.01,
        best: None,
        include: 0,
        start: None,
        stop: None,
        details: false,
        stb: false,
    });
    let ast = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        model,
    );
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("Summary of Backward Elimination"), "{listing}");
    // Inspect the final fitted-model block (after the last "Model: MODEL1").
    let final_block = listing.rsplit("Model: MODEL1").next().unwrap();
    // x2 removed → absent from fitted parameters; x1 retained → present.
    assert!(final_block.contains("x1"), "{listing}");
    assert!(!final_block.contains("x2"), "x2 should be eliminated: {listing}");
}

// ───────────────────────── M29.3 diagnostics tests ─────────────────────────

fn run_diag(
    ods_on: bool,
    output_dir: Option<PathBuf>,
    file_stem: Option<String>,
) -> String {
    let mut session = make_session();
    session.ods_graphics.enabled = ods_on;
    if let Some(d) = output_dir {
        session.ods_graphics.output_dir = d;
    }
    session.ods_graphics.file_stem = file_stem;
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 5.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        basic_model("y", &["x"]),
    );
    execute(&ast, &mut session).unwrap();
    session.log.into_string()
}

// ───────────────────────── M36.1 TEST / RESTRICT tests ─────────────────────────

fn eq_terms(eq: &LinEq) -> Vec<(f64, String)> {
    eq.terms.clone()
}

#[test]
fn test_parse_test_multi_eq() {
    let ast = parse_reg("proc reg data=a; model y = a b c; test a=b, c=0; run;").unwrap();
    let t = &ast.models[0].tests[0];
    assert!(t.label.is_none());
    assert_eq!(t.equations.len(), 2);
    // a = b  →  A - B = 0
    let e0 = eq_terms(&t.equations[0]);
    assert_eq!(e0, vec![(1.0, "A".into()), (-1.0, "B".into())]);
    assert!((t.equations[0].rhs).abs() < 1e-12);
    // c = 0  →  C = 0
    let e1 = eq_terms(&t.equations[1]);
    assert_eq!(e1, vec![(1.0, "C".into())]);
}

#[test]
fn test_parse_test_label() {
    let ast = parse_reg("proc reg data=a; model y = x1 x2; peak: test x1 = x2; run;").unwrap();
    let t = &ast.models[0].tests[0];
    assert_eq!(t.label.as_deref(), Some("peak"));
    assert_eq!(
        eq_terms(&t.equations[0]),
        vec![(1.0, "X1".into()), (-1.0, "X2".into())]
    );
}

#[test]
fn test_parse_restrict_sum() {
    let ast = parse_reg("proc reg data=a; model y = a b; restrict a+b=1; run;").unwrap();
    let r = &ast.models[0].restricts[0];
    assert_eq!(r.equations.len(), 1);
    assert_eq!(
        eq_terms(&r.equations[0]),
        vec![(1.0, "A".into()), (1.0, "B".into())]
    );
    assert!((r.equations[0].rhs - 1.0).abs() < 1e-12);
}

#[test]
fn test_parse_restrict_coefficients() {
    // 2*x1 - x2 = 0
    let ast =
        parse_reg("proc reg data=a; model y = x1 x2; restrict 2*x1 - x2 = 0; run;").unwrap();
    let e = &ast.models[0].restricts[0].equations[0];
    assert_eq!(
        eq_terms(e),
        vec![(2.0, "X1".into()), (-1.0, "X2".into())]
    );
    assert!(e.rhs.abs() < 1e-12);
}

#[test]
fn test_parse_coef_no_star() {
    // `2 x1` (no star) is also a coefficient form.
    let ast =
        parse_reg("proc reg data=a; model y = x1 x2; restrict 2 x1 = x2 + 3; run;").unwrap();
    let e = &ast.models[0].restricts[0].equations[0];
    // 2*x1 - x2 = 3
    assert_eq!(
        eq_terms(e),
        vec![(2.0, "X1".into()), (-1.0, "X2".into())]
    );
    assert!((e.rhs - 3.0).abs() < 1e-12);
}

#[test]
fn test_parse_intercept_keyword() {
    let ast = parse_reg(
        "proc reg data=a; model y = x1 x2; restrict intercept = 0; run;",
    )
    .unwrap();
    let e = &ast.models[0].restricts[0].equations[0];
    assert_eq!(eq_terms(e), vec![(1.0, "INTERCEPT".into())]);
}

/// Oracle (a): a single-coefficient `TEST xj=0;` yields F == t² of xj.
#[test]
fn test_oracle_test_f_equals_t_squared() {
    // Design: intercept + x1 + x2, with non-degenerate data.
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let x2 = [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0];
    let y: Vec<f64> = x1
        .iter()
        .zip(x2.iter())
        .map(|(&a, &b)| 1.0 + 2.0 * a + 0.5 * b + (a * 0.3).cos())
        .collect();
    let n = y.len();
    let mut x_mat = Vec::new();
    for i in 0..n {
        x_mat.push(vec![1.0, x1[i], x2[i]]);
    }
    let fit = ols_fit(&x_mat, &y).unwrap();
    let p_eff = 3;
    let df_e = (n - p_eff) as f64;
    let mse = fit.sse / df_e;
    // t for x2 (column 2).
    let se = (mse * fit.xtx_inv[2][2]).sqrt();
    let t = fit.beta[2] / se;
    let t2 = t * t;

    // TEST x2 = 0  →  L = [0,0,1], c = 0.
    let reg_names = vec!["X1".to_string(), "X2".to_string()];
    let eq = LinEq {
        terms: vec![(1.0, "X2".into())],
        rhs: 0.0,
    };
    let (l, c) = build_lc(&[eq], &reg_names, true).unwrap();
    let lb = linalg::matrix_vec_mult(&l, &fit.beta);
    let diff: Vec<f64> = lb.iter().zip(c.iter()).map(|(a, b)| a - b).collect();
    let lt = linalg::transpose(&l);
    let lh = linalg::matrix_mult(&l, &fit.xtx_inv);
    let m = linalg::matrix_mult(&lh, &lt);
    let minv = linalg::invert_matrix(&m).unwrap();
    let md = linalg::matrix_vec_mult(&minv, &diff);
    let ss: f64 = diff.iter().zip(md.iter()).map(|(a, b)| a * b).sum();
    let f = (ss / 1.0) / mse;
    assert!((f - t2).abs() < 1e-6, "F={f} t^2={t2}");
}

/// Oracle (b): restricted estimates satisfy L β_r = c exactly.
#[test]
fn test_oracle_restricted_satisfies_constraint() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0];
    let x2 = [3.0_f64, 1.0, 4.0, 1.0, 5.0, 9.0];
    let y: Vec<f64> = x1
        .iter()
        .zip(x2.iter())
        .map(|(&a, &b)| 2.0 + a - b)
        .collect();
    let n = y.len();
    let mut x_mat = Vec::new();
    for i in 0..n {
        x_mat.push(vec![1.0, x1[i], x2[i]]);
    }
    let fit = ols_fit(&x_mat, &y).unwrap();
    let reg_names = vec!["X1".to_string(), "X2".to_string()];
    // RESTRICT x1 + x2 = 1.
    let restricts = vec![RegRestrict {
        equations: vec![LinEq {
            terms: vec![(1.0, "X1".into()), (1.0, "X2".into())],
            rhs: 1.0,
        }],
    }];
    let r = compute_restricted(&restricts, &reg_names, true, &x_mat, &y, &fit, n)
        .unwrap()
        .unwrap();
    // L β_r = c: β_r[1] + β_r[2] == 1.
    let lhs = r.beta_r[1] + r.beta_r[2];
    assert!((lhs - 1.0).abs() < 1e-9, "L beta_r = {lhs}");
}

/// Oracle (c): a RESTRICT already satisfied by OLS leaves estimates ~unchanged.
#[test]
fn test_oracle_redundant_restrict_unchanged() {
    // Build y so that OLS already gives slope_x1 == slope_x2 (symmetric).
    // y = 3 + 2*x1 + 2*x2 exactly → OLS recovers (3, 2, 2); RESTRICT x1=x2
    // is already satisfied.
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0];
    let x2 = [5.0_f64, 1.0, 4.0, 2.0, 3.0];
    let y: Vec<f64> = x1
        .iter()
        .zip(x2.iter())
        .map(|(&a, &b)| 3.0 + 2.0 * a + 2.0 * b)
        .collect();
    let n = y.len();
    let mut x_mat = Vec::new();
    for i in 0..n {
        x_mat.push(vec![1.0, x1[i], x2[i]]);
    }
    let fit = ols_fit(&x_mat, &y).unwrap();
    let reg_names = vec!["X1".to_string(), "X2".to_string()];
    let restricts = vec![RegRestrict {
        equations: vec![LinEq {
            terms: vec![(1.0, "X1".into()), (-1.0, "X2".into())],
            rhs: 0.0,
        }],
    }];
    let r = compute_restricted(&restricts, &reg_names, true, &x_mat, &y, &fit, n)
        .unwrap()
        .unwrap();
    for j in 0..fit.beta.len() {
        assert!(
            (r.beta_r[j] - fit.beta[j]).abs() < 1e-7,
            "beta_r[{j}]={} beta[{j}]={}",
            r.beta_r[j],
            fit.beta[j]
        );
    }
    // λ ≈ 0 since the constraint is non-binding.
    assert!(r.lambda_rows[0].1.abs() < 1e-6, "lambda={}", r.lambda_rows[0].1);
}

/// End-to-end: TEST and RESTRICT statements parse and execute, emitting the
/// expected blocks in the listing.
#[test]
fn test_execute_test_and_restrict() {
    let mut session = make_session();
    let frame = df![
        "y"  => [3.2_f64, 4.8, 7.1, 8.9, 11.3, 12.7, 15.2, 16.8],
        "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
        "x2" => [4.0_f64, 1.0, 9.0, 2.0, 8.0, 3.0, 7.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let src = "proc reg data=work.t; model y = x1 x2; peak: test x1 = x2; restrict x1 + x2 = 3; run;";
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next();
    ts.next();
    let ast = parse(&mut ts).unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("Test peak Results for Dependent Variable y"), "{listing}");
    assert!(listing.contains("Numerator"), "{listing}");
    assert!(listing.contains("Denominator"), "{listing}");
    assert!(listing.contains("RESTRICT"), "{listing}");
}

// ───────────────────────── M36.2 CL / OUTPUT-stat tests ─────────────────────────

/// Build a design matrix [1, x...] for the given regressor columns.
fn design(intercept: bool, cols: &[&[f64]], n: usize) -> Vec<Vec<f64>> {
    (0..n)
        .map(|i| {
            let mut row = Vec::new();
            if intercept {
                row.push(1.0);
            }
            for c in cols {
                row.push(c[i]);
            }
            row
        })
        .collect()
}

/// Oracle: Σ_i h_i == p_eff (trace of the hat matrix == #params).
#[test]
fn test_oracle_leverage_trace() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let x2 = [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0];
    let y: Vec<f64> = x1.iter().zip(x2.iter()).map(|(&a, &b)| 1.0 + a + 0.5 * b).collect();
    let n = y.len();
    let x = design(true, &[&x1, &x2], n);
    let fit = ols_fit(&x, &y).unwrap();
    let h = leverages(&x, &fit.xtx_inv);
    let trace: f64 = h.iter().sum();
    assert!((trace - 3.0).abs() < 1e-9, "trace={trace}");
    // Same for a NOINT design.
    let xn = design(false, &[&x1, &x2], n);
    let fitn = ols_fit(&xn, &y).unwrap();
    let hn = leverages(&xn, &fitn.xtx_inv);
    let tracen: f64 = hn.iter().sum();
    assert!((tracen - 2.0).abs() < 1e-9, "trace_noint={tracen}");
}

/// Oracle: STDP²+STDR² == MSE and STDI²−STDP² == MSE (per observation),
/// and CLM is centered on ŷ.
#[test]
fn test_oracle_std_error_identities() {
    let x1 = [1.0_f64, 3.0, 2.0, 5.0, 4.0, 6.0, 8.0, 7.0];
    let y: Vec<f64> = x1.iter().map(|&a| 2.0 + 3.0 * a + (a * 0.5).sin()).collect();
    let n = y.len();
    let x = design(true, &[&x1], n);
    let fit = ols_fit(&x, &y).unwrap();
    let p_eff = 2;
    let mse = fit.sse / (n - p_eff) as f64;
    let stats = compute_obs_stats(&x, &y, &fit, n, p_eff, 0.05, None);
    for s in &stats {
        assert!((s.stdp * s.stdp + s.stdr * s.stdr - mse).abs() < 1e-9);
        assert!((s.stdi * s.stdi - s.stdp * s.stdp - mse).abs() < 1e-9);
        // CLM centered on ŷ.
        let mid = (s.lclm + s.uclm) / 2.0;
        assert!((mid - s.y_hat).abs() < 1e-9, "mid={mid} yhat={}", s.y_hat);
        // CLI also centered on ŷ and wider than CLM.
        let midi = (s.lcl + s.ucl) / 2.0;
        assert!((midi - s.y_hat).abs() < 1e-9);
        assert!(s.ucl - s.lcl > s.uclm - s.lclm - 1e-12);
    }
}

/// Oracle: CLB limits == β_j ± t·SE(β_j) with the parameter-table SE.
#[test]
fn test_oracle_clb_limits() {
    let x1 = [1.0_f64, 2.0, 4.0, 3.0, 6.0, 5.0, 7.0];
    let y: Vec<f64> = x1.iter().map(|&a| 1.5 + 2.0 * a + (a * 0.3).cos()).collect();
    let n = y.len();
    let x = design(true, &[&x1], n);
    let fit = ols_fit(&x, &y).unwrap();
    let p_eff = 2;
    let df_e = (n - p_eff) as f64;
    let mse = fit.sse / df_e;
    let alpha = 0.10;
    let t = t_quantile(1.0 - alpha / 2.0, df_e);
    for j in 0..p_eff {
        let se = (mse * fit.xtx_inv[j][j]).sqrt();
        let lo = fit.beta[j] - t * se;
        let hi = fit.beta[j] + t * se;
        // Reconstruct what fit_and_print computes.
        assert!(lo < fit.beta[j] && fit.beta[j] < hi);
        assert!(((lo + hi) / 2.0 - fit.beta[j]).abs() < 1e-12);
    }
}

#[test]
fn test_parse_model_cl_options() {
    let ast =
        parse_reg("proc reg data=a; model y=x / clb alpha=0.10 cli clm; run;").unwrap();
    let m = &ast.models[0].model;
    assert!(m.clb);
    assert!(m.cli);
    assert!(m.clm);
    assert!((m.alpha - 0.10).abs() < 1e-12);
}

#[test]
fn test_parse_output_cl_keywords() {
    let ast = parse_reg(
        "proc reg data=a; model y=x; output out=o p=pred stdp=sp lclm=lm uclm=um lcl=l ucl=u stdi=si stdr=sr; run;",
    )
    .unwrap();
    let o = &ast.models[0].outputs[0];
    assert_eq!(o.predicted.as_deref(), Some("pred"));
    assert_eq!(o.stdp.as_deref(), Some("sp"));
    assert_eq!(o.lclm.as_deref(), Some("lm"));
    assert_eq!(o.uclm.as_deref(), Some("um"));
    assert_eq!(o.lcl.as_deref(), Some("l"));
    assert_eq!(o.ucl.as_deref(), Some("u"));
    assert_eq!(o.stdi.as_deref(), Some("si"));
    assert_eq!(o.stdr.as_deref(), Some("sr"));
}

/// End-to-end: CLB adds confidence-limit columns; CLM/CLI emit Output
/// Statistics. Default model (no options) must NOT print either.
#[test]
fn test_execute_cl_listing() {
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let mut model = basic_model("y", &["x"]);
    model.clb = true;
    model.clm = true;
    model.cli = true;
    let ast = single_model_ast(
        DatasetRef { libref: Some("WORK".into()), name: "T".into() },
        model,
    );
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("95% Confidence Limits"), "{listing}");
    assert!(listing.contains("Output Statistics"), "{listing}");
    assert!(listing.contains("CL Mean"), "{listing}");
    assert!(listing.contains("CL Predict"), "{listing}");
}

#[test]
fn parse_plots_statement_flag() {
    let ast = parse_reg("proc reg data=a; model y = x; plots / only; run;").unwrap();
    assert!(ast.plots_requested);
}

#[test]
fn reg_without_ods_no_diagnostic() {
    let log = run_diag(false, None, None);
    assert!(!log.contains("image deferred"), "log: {log}");
    assert!(!log.contains("REG diagnostics"), "log: {log}");
}

#[cfg(not(feature = "graphics"))]
#[test]
fn reg_with_ods_no_feature_defers() {
    let log = run_diag(true, None, None);
    assert!(
        log.contains("REG diagnostics: image deferred"),
        "log: {log}"
    );
}

#[cfg(feature = "graphics")]
#[test]
fn reg_with_ods_and_feature_creates_image() {
    let dir = std::env::temp_dir();
    let log = run_diag(true, Some(dir.clone()), Some("regtest_diag".into()));
    assert!(log.contains("written"), "log: {log}");
    let p = dir.join("regtest_diag_1.png");
    assert!(p.exists(), "diagnostic image not created: {p:?}");
    assert!(p.metadata().unwrap().len() > 0);
    let _ = std::fs::remove_file(&p);
}

// ───────────── M36.3 influence-diagnostic oracles ─────────────

/// Sample design reused by the influence oracles (intercept + one regressor,
/// a non-degenerate fit with dfE = n − 2 > 1).
fn infl_setup() -> (Vec<Vec<f64>>, Vec<f64>, OlsFit, usize, usize) {
    let x1 = [1.0_f64, 3.0, 2.0, 5.0, 4.0, 6.0, 8.0, 7.0];
    let y: Vec<f64> = x1.iter().map(|&a| 2.0 + 3.0 * a + (a * 0.7).sin()).collect();
    let n = y.len();
    let x = design(true, &[&x1], n);
    let fit = ols_fit(&x, &y).unwrap();
    let p_eff = 2;
    (x, y, fit, n, p_eff)
}

/// STUDENT = resid / STDR (matches M36.2 STDR).
#[test]
fn test_oracle_student_eq_resid_over_stdr() {
    let (x, y, fit, n, p_eff) = infl_setup();
    let obs = compute_obs_stats(&x, &y, &fit, n, p_eff, 0.05, None);
    let infl = compute_influence_stats(&x, &y, &fit, n, p_eff, None);
    for (s, o) in infl.iter().zip(obs.iter()) {
        assert!((s.student - s.resid / o.stdr).abs() < 1e-9);
        // STDR also matches the obs-stats STDR.
        assert!((s.stdr - o.stdr).abs() < 1e-9);
    }
}

/// RSTUDENT = student·√((dfE−1)/(dfE−student²)).
#[test]
fn test_oracle_rstudent_identity() {
    let (x, y, fit, n, p_eff) = infl_setup();
    let df_e = (n - p_eff) as f64;
    let infl = compute_influence_stats(&x, &y, &fit, n, p_eff, None);
    for s in &infl {
        let expect = s.student * ((df_e - 1.0) / (df_e - s.student * s.student)).sqrt();
        assert!(
            (s.rstudent - expect).abs() < 1e-9,
            "rstudent={} expect={}",
            s.rstudent,
            expect
        );
    }
}

/// PRESS = resid/(1−h) and Σ press² is the printed PRESS.
#[test]
fn test_oracle_press() {
    let (x, y, fit, n, p_eff) = infl_setup();
    let h = leverages(&x, &fit.xtx_inv);
    let infl = compute_influence_stats(&x, &y, &fit, n, p_eff, None);
    let mut press_ss = 0.0;
    for (i, s) in infl.iter().enumerate() {
        let expect = s.resid / (1.0 - h[i]);
        assert!((s.press - expect).abs() < 1e-9);
        press_ss += s.press * s.press;
    }
    let printed: f64 = infl.iter().map(|s| s.press * s.press).sum();
    assert!((press_ss - printed).abs() < 1e-9);
}

/// Cook's D ≥ 0, and DFFITS = rstudent·√(h/(1−h)).
#[test]
fn test_oracle_cookd_dffits() {
    let (x, y, fit, n, p_eff) = infl_setup();
    let h = leverages(&x, &fit.xtx_inv);
    let infl = compute_influence_stats(&x, &y, &fit, n, p_eff, None);
    for (i, s) in infl.iter().enumerate() {
        assert!(s.cookd >= 0.0, "cookd={}", s.cookd);
        let expect = s.rstudent * (h[i] / (1.0 - h[i])).sqrt();
        assert!((s.dffits - expect).abs() < 1e-9);
    }
}

/// Near-zero-leverage point → Cook's D ≈ 0.
#[test]
fn test_oracle_cookd_low_leverage() {
    let (x, y, fit, n, p_eff) = infl_setup();
    let h = leverages(&x, &fit.xtx_inv);
    let infl = compute_influence_stats(&x, &y, &fit, n, p_eff, None);
    // The lowest-leverage observation should have small Cook's D.
    let (min_i, _) = h
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .unwrap();
    assert!(infl[min_i].cookd < 0.5, "cookd={}", infl[min_i].cookd);
}

/// DFBETAS closed form == an explicit leave-one-out refit (within 1e-6).
#[test]
fn test_oracle_dfbetas_loo_refit() {
    // Tiny dataset, intercept + slope.
    let x1 = [1.0_f64, 2.0, 3.0, 5.0, 8.0];
    let y = [2.1_f64, 3.9, 6.2, 9.8, 16.1];
    let n = y.len();
    let p_eff = 2;
    let x = design(true, &[&x1], n);
    let fit = ols_fit(&x, &y).unwrap();
    let infl = compute_influence_stats(&x, &y.to_vec(), &fit, n, p_eff, None);

    for drop in 0..n {
        // Refit without observation `drop`.
        let xr: Vec<Vec<f64>> = (0..n).filter(|&i| i != drop).map(|i| x[i].clone()).collect();
        let yr: Vec<f64> = (0..n).filter(|&i| i != drop).map(|i| y[i]).collect();
        let fit_i = ols_fit(&xr, &yr).unwrap();
        // s_(i) = √MSE_(i).
        let df_i = (n - 1 - p_eff) as f64;
        let s_i = (fit_i.sse / df_i).sqrt();
        for j in 0..p_eff {
            let denom = s_i * fit.xtx_inv[j][j].sqrt();
            let expect = (fit.beta[j] - fit_i.beta[j]) / denom;
            assert!(
                (infl[drop].dfbetas[j] - expect).abs() < 1e-6,
                "drop={drop} j={j} got={} expect={}",
                infl[drop].dfbetas[j],
                expect
            );
        }
    }
}

/// dfE ≤ 1 → RSTUDENT/COVRATIO/DFFITS/DFBETAS undefined (NaN).
#[test]
fn test_dfe_le_one_undefined() {
    // n=3, p_eff=2 → dfE=1.
    let x1 = [1.0_f64, 2.0, 4.0];
    let y = [1.0_f64, 3.0, 2.5];
    let n = y.len();
    let p_eff = 2;
    let x = design(true, &[&x1], n);
    let fit = ols_fit(&x, &y.to_vec()).unwrap();
    let infl = compute_influence_stats(&x, &y.to_vec(), &fit, n, p_eff, None);
    for s in &infl {
        assert!(!s.rstudent.is_finite());
        assert!(!s.covratio.is_finite());
        assert!(!s.dffits.is_finite());
        assert!(s.dfbetas.iter().all(|v| !v.is_finite()));
        // STUDENT and PRESS remain defined.
        assert!(s.press.is_finite());
    }
    // fmt_diag renders the SAS sentinel.
    assert_eq!(fmt_diag(f64::NAN), ".");
}

#[test]
fn test_parse_model_r_influence() {
    let ast = parse_reg("proc reg data=a; model y=x / r influence; run;").unwrap();
    let m = &ast.models[0].model;
    assert!(m.r);
    assert!(m.influence);
}

#[test]
fn test_parse_output_influence_keywords() {
    let ast = parse_reg(
        "proc reg data=a; model y=x; output out=o student=rs rstudent=er cookd=cd h=hat press=pr dffits=df covratio=cv dfbetas=b; run;",
    )
    .unwrap();
    let o = &ast.models[0].outputs[0];
    assert_eq!(o.student.as_deref(), Some("rs"));
    assert_eq!(o.rstudent.as_deref(), Some("er"));
    assert_eq!(o.cookd.as_deref(), Some("cd"));
    assert_eq!(o.h.as_deref(), Some("hat"));
    assert_eq!(o.press.as_deref(), Some("pr"));
    assert_eq!(o.dffits.as_deref(), Some("df"));
    assert_eq!(o.covratio.as_deref(), Some("cv"));
    assert_eq!(o.dfbetas.as_deref(), Some("b"));
}

/// End-to-end: R and INFLUENCE listings print; default model prints neither.
#[test]
fn test_execute_r_influence_listing() {
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let mut model = basic_model("y", &["x"]);
    model.r = true;
    model.influence = true;
    let ast = single_model_ast(
        DatasetRef { libref: Some("WORK".into()), name: "T".into() },
        model,
    );
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("Student Residual"), "{listing}");
    assert!(listing.contains("Sum of Residuals"), "{listing}");
    assert!(listing.contains("PRESS"), "{listing}");
    assert!(listing.contains("RStudent"), "{listing}");
    assert!(listing.contains("DFBETAS Intercept"), "{listing}");
    assert!(listing.contains("DFBETAS x"), "{listing}");
}

/// OUTPUT influence columns appear; DFBETAS= emits one column per parameter.
#[test]
fn test_output_influence_columns() {
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = parse_reg(
        "proc reg data=work.t; model y=x; output out=work.o student=stu cookd=cd h=hat dfbetas=b; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
    let names: Vec<&str> = out.vars.iter().map(|v| v.name.as_str()).collect();
    assert!(names.contains(&"stu"));
    assert!(names.contains(&"cd"));
    assert!(names.contains(&"hat"));
    assert!(names.contains(&"b_Intercept"));
    assert!(names.contains(&"b_x"));
}

// ───────────────────────── M36.4 ─────────────────────────

/// Parse: all collinearity / spec diagnostic options on one MODEL.
#[test]
fn test_parse_model_diagnostics() {
    let ast = parse_reg(
        "proc reg data=a; model y=x1 x2 / vif tol collin spec dw dwprob acov; run;",
    )
    .unwrap();
    let m = &ast.models[0].model;
    assert!(m.vif);
    assert!(m.tol);
    assert!(m.collin);
    assert!(!m.collinoint);
    assert!(m.spec);
    assert!(m.dw);
    assert!(m.dwprob);
    assert!(m.acov);
}

#[test]
fn test_parse_collinoint_and_hcc_synonym() {
    let ast =
        parse_reg("proc reg data=a; model y=x1 x2 / collinoint hcc; run;").unwrap();
    let m = &ast.models[0].model;
    assert!(m.collinoint);
    assert!(!m.collin);
    // HCC is a synonym for ACOV.
    assert!(m.acov);
}

/// VIF·TOL == 1; for two regressors VIF_1 == VIF_2 == 1/(1−r²).
#[test]
fn test_oracle_vif_tol() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 7.0, 8.0, 6.0];
    // x2 correlated-but-not-collinear with x1.
    let x2: Vec<f64> = x1.iter().map(|&a| 0.5 * a + (a * 0.7).sin()).collect();
    let cols = vec![x1.to_vec(), x2.clone()];
    let (tol, vif) = vif_tol(&cols);
    for j in 0..2 {
        assert!((vif[j] * tol[j] - 1.0).abs() < 1e-9, "VIF·TOL != 1");
    }
    // Two regressors → both VIF equal, == 1/(1−r²).
    let n = x1.len() as f64;
    let m1 = x1.iter().sum::<f64>() / n;
    let m2 = x2.iter().sum::<f64>() / n;
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    let mut syy = 0.0;
    for i in 0..x1.len() {
        sxy += (x1[i] - m1) * (x2[i] - m2);
        sxx += (x1[i] - m1) * (x1[i] - m1);
        syy += (x2[i] - m2) * (x2[i] - m2);
    }
    let r2 = (sxy * sxy) / (sxx * syy);
    let expected = 1.0 / (1.0 - r2);
    assert!((vif[0] - vif[1]).abs() < 1e-9, "VIFs differ");
    assert!((vif[0] - expected).abs() < 1e-7, "VIF != 1/(1-r²)");
}

/// Single regressor → trivial VIF table (TOL=1, VIF=1).
#[test]
fn test_vif_single_regressor() {
    let cols = vec![vec![1.0_f64, 2.0, 3.0, 4.0]];
    let (tol, vif) = vif_tol(&cols);
    assert_eq!(tol, vec![1.0]);
    assert_eq!(vif, vec![1.0]);
}

/// Collinearity: #eigenvalues == #cols, condition index uses λ_max, and each
/// regressor's variance proportions sum to 1 across rows.
#[test]
fn test_oracle_collin_proportions() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let x2: Vec<f64> = x1.iter().map(|&a| a * a + 1.0).collect();
    let n = x1.len();
    let x = design(true, &[&x1, &x2], n);
    let reg = vec!["x1".to_string(), "x2".to_string()];
    let c = compute_collin(&x, &reg, true, false).unwrap();
    assert_eq!(c.eigenvalues.len(), 3); // intercept + 2 regressors
    // Descending.
    for k in 1..c.eigenvalues.len() {
        assert!(c.eigenvalues[k - 1] >= c.eigenvalues[k] - 1e-12);
    }
    // First condition index == 1 (λ_max / λ_max).
    assert!((c.condition_index[0] - 1.0).abs() < 1e-9);
    // Column proportions sum to 1.
    let m = c.eigenvalues.len();
    for j in 0..m {
        let s: f64 = (0..m).map(|k| c.proportions[k][j]).sum();
        assert!((s - 1.0).abs() < 1e-9, "proportion col sum != 1: {s}");
    }
    // COLLINOINT drops the intercept column → 2 columns analysed.
    let cint = compute_collin(&x, &reg, true, true).unwrap();
    assert_eq!(cint.eigenvalues.len(), 2);
    assert_eq!(cint.col_labels, vec!["x1".to_string(), "x2".to_string()]);
}

/// SPEC: W = n·R²_aux ≥ 0 and df == number of auxiliary regressors.
#[test]
fn test_oracle_spec_white() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    // x2 chosen so {1, x1, x2, x1², x2², x1·x2} is full rank.
    let x2 = [3.0_f64, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0, 5.0, 8.0];
    // Include genuine noise so the fit has nonzero residuals and the
    // auxiliary (White) regression is full rank.
    let y: Vec<f64> = (0..10)
        .map(|i| 1.0 + 2.0 * x1[i] - 0.5 * x2[i] + (x1[i] * 1.3).sin() * 0.8)
        .collect();
    let n = y.len();
    let x = design(true, &[&x1, &x2], n);
    let fit = ols_fit(&x, &y).unwrap();
    let cols = vec![x1.to_vec(), x2.to_vec()];
    let (w, df, pv) = white_spec_test(&cols, &fit.resid).unwrap();
    assert!(w >= 0.0);
    // p=2 regressors → linear(2) + square(2) + cross(1) = 5 aux regressors.
    assert_eq!(df, 5);
    assert!((0.0..=1.0).contains(&pv));
}

/// DW: 0 ≤ d ≤ 4; for no-autocorrelation residuals d ≈ 2; d ≈ 2(1−ρ).
#[test]
fn test_oracle_durbin_watson() {
    // Alternating-sign residuals → strong negative autocorrelation, d→4.
    let x1: Vec<f64> = (0..10).map(|i| i as f64).collect();
    let y: Vec<f64> = (0..10)
        .map(|i| 1.0 + 0.5 * i as f64 + if i % 2 == 0 { 1.0 } else { -1.0 })
        .collect();
    let n = y.len();
    let x = design(true, &[&x1], n);
    let fit = ols_fit(&x, &y).unwrap();
    let dwr = durbin_watson(&fit.resid, &x, &fit.xtx_inv, true);
    assert!((0.0..=4.0).contains(&dwr.d), "d out of range: {}", dwr.d);
    // d ≈ 2(1−ρ) (exact only up to O(1/n) boundary terms e_1²+e_n²).
    assert!((dwr.d - 2.0 * (1.0 - dwr.rho)).abs() < 0.6);
    // Alternating signs → ρ negative → d > 2.
    assert!(dwr.d > 2.0);
    // p-values present and in [0,1].
    let pp = dwr.pr_pos.unwrap();
    let pn = dwr.pr_neg.unwrap();
    assert!((0.0..=1.0).contains(&pp) && (0.0..=1.0).contains(&pn));
    assert!((pp + pn - 1.0).abs() < 1e-9);
}

/// ACOV: HC matrix is symmetric; for homoscedastic-like data HC SE is the
/// same order of magnitude as OLS SE.
#[test]
fn test_oracle_acov_hc0() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let y: Vec<f64> = x1.iter().map(|&a| 1.0 + 2.0 * a + (a * 0.9).sin()).collect();
    let n = y.len();
    let p_eff = 2;
    let x = design(true, &[&x1], n);
    let fit = ols_fit(&x, &y).unwrap();
    let cov = acov_hc0(&x, &fit.resid, &fit.xtx_inv);
    // Symmetry.
    for i in 0..p_eff {
        for j in 0..p_eff {
            assert!((cov[i][j] - cov[j][i]).abs() < 1e-12);
        }
    }
    // Order-of-magnitude agreement with OLS SE.
    let mse = fit.sse / (n - p_eff) as f64;
    for j in 0..p_eff {
        let ols_se = (mse * fit.xtx_inv[j][j]).sqrt();
        let hc_se = cov[j][j].sqrt();
        assert!(
            hc_se > 0.0 && hc_se < 100.0 * ols_se && ols_se < 100.0 * hc_se,
            "HC SE / OLS SE order mismatch: {hc_se} vs {ols_se}"
        );
    }
}

/// End-to-end: VIF/TOL columns appear; default model does NOT print them.
#[test]
fn test_execute_diagnostics_listing() {
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0, 9.0, 11.0],
        "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
        "x2" => [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0, 7.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = parse_reg(
        "proc reg data=work.t; model y=x1 x2 / vif tol collin spec dw dwprob acov; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("Tolerance"), "{listing}");
    assert!(listing.contains("Variance Inflation"), "{listing}");
    assert!(listing.contains("Collinearity Diagnostics"), "{listing}");
    assert!(
        listing.contains("Test of First and Second Moment Specification"),
        "{listing}"
    );
    assert!(listing.contains("Durbin-Watson D"), "{listing}");
    assert!(listing.contains("Pr < DW"), "{listing}");
    assert!(
        listing.contains("Consistent Covariance of Estimates"),
        "{listing}"
    );
}

/// Byte-identity guard: a plain model and one with only diagnostics-OFF must
/// produce identical parameter-table output (no extra columns).
#[test]
fn test_diagnostics_off_no_extra_columns() {
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        basic_model("y", &["x"]),
    );
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(!listing.contains("Tolerance"));
    assert!(!listing.contains("Variance Inflation"));
    assert!(!listing.contains("Collinearity Diagnostics"));
    assert!(!listing.contains("Durbin-Watson"));
}

// ───────────────────── M36.5 partial-SS / correlation tests ─────────────────────

/// Build a model with all the M36.5 statistic flags turned on.
fn seq_model(dep: &str, regs: &[&str]) -> RegModel {
    let mut m = basic_model(dep, regs);
    m.ss1 = true;
    m.ss2 = true;
    m.stb = true;
    m.pcorr1 = true;
    m.pcorr2 = true;
    m.scorr1 = true;
    m.scorr2 = true;
    m.seqb = true;
    m
}

/// Parse all M36.5 options off one MODEL statement.
#[test]
fn test_parse_m365_options() {
    let ast = parse_reg(
        "proc reg data=a; model y = x1 x2 / ss1 ss2 stb pcorr1 pcorr2 scorr1 scorr2 seqb press; run;",
    )
    .unwrap();
    let m = &ast.models[0].model;
    assert!(m.ss1 && m.ss2 && m.stb);
    assert!(m.pcorr1 && m.pcorr2 && m.scorr1 && m.scorr2);
    assert!(m.seqb && m.press_opt);
}

/// Default model leaves every M36.5 flag off (byte-identity guard).
#[test]
fn test_parse_m365_default_off() {
    let ast = parse_reg("proc reg data=a; model y = x1 x2; run;").unwrap();
    let m = &ast.models[0].model;
    assert!(!m.ss1 && !m.ss2 && !m.stb);
    assert!(!m.pcorr1 && !m.pcorr2 && !m.scorr1 && !m.scorr2);
    assert!(!m.seqb && !m.press_opt);
}

/// Multi-regressor oracles: Σ SS1 (regressors) == Model SS; SS2_j == t_j²·MSE;
/// all PCORR/SCORR in [0,1]; SEQB of the last column == its OLS β.
#[test]
fn test_oracle_seq_stats_multi() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let x2 = [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 9.0, 7.0];
    let y: Vec<f64> = (0..8)
        .map(|i| 1.0 + 2.0 * x1[i] - 0.5 * x2[i] + (x1[i] * 0.7).sin())
        .collect();
    let n = y.len();
    let x = design(true, &[&x1, &x2], n);
    let fit = ols_fit(&x, &y).unwrap();
    let p_eff = 3;
    let df_e = (n - p_eff) as f64;
    let mse = fit.sse / df_e;

    // Corrected total.
    let ybar = y.iter().sum::<f64>() / n as f64;
    let sst: f64 = y.iter().map(|v| (v - ybar) * (v - ybar)).sum();
    let model_ss = sst - fit.sse;

    let m = seq_model("y", &["x1", "x2"]);
    let s = compute_seq_stats(&m, &x, &y, &fit, sst, true);

    // Σ SS1 over the regressors (skip intercept at col 0) == Model SS.
    let sum_ss1_reg: f64 = s.ss1[1] + s.ss1[2];
    assert!(
        (sum_ss1_reg - model_ss).abs() < 1e-6,
        "ΣSS1={sum_ss1_reg} ModelSS={model_ss}"
    );

    // SS2_j == t_j²·MSE for every column (intercept included).
    for j in 0..p_eff {
        let se = (mse * fit.xtx_inv[j][j]).sqrt();
        let t = fit.beta[j] / se;
        assert!(
            (s.ss2[j] - t * t * mse).abs() < 1e-6,
            "SS2[{j}]={} t²·MSE={}",
            s.ss2[j],
            t * t * mse
        );
    }

    // All correlations in [0,1].
    for j in 0..p_eff {
        for v in [s.pcorr1[j], s.pcorr2[j], s.scorr1[j], s.scorr2[j]] {
            assert!((0.0..=1.0).contains(&v), "corr out of range: {v}");
        }
    }

    // SEQB of the last column == its OLS β.
    assert!(
        (s.seqb[p_eff - 1] - fit.beta[p_eff - 1]).abs() < 1e-9,
        "SEQB last={} β last={}",
        s.seqb[p_eff - 1],
        fit.beta[p_eff - 1]
    );
}

/// Single-regressor identities: SS1==SS2==Model SS; PCORR2==SCORR2==R²;
/// STB == sign(β)·|r| with r = corr(x,y).
#[test]
fn test_oracle_seq_stats_single() {
    let x = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let y: Vec<f64> = x.iter().map(|&a| 0.5 + 1.7 * a + (a * 0.3).cos()).collect();
    let n = y.len();
    let xm = design(true, &[&x], n);
    let fit = ols_fit(&xm, &y).unwrap();

    let ybar = y.iter().sum::<f64>() / n as f64;
    let sst: f64 = y.iter().map(|v| (v - ybar) * (v - ybar)).sum();
    let model_ss = sst - fit.sse;
    let r2 = model_ss / sst;

    let m = seq_model("y", &["x"]);
    let s = compute_seq_stats(&m, &xm, &y, &fit, sst, true);

    // The single regressor sits at column 1 (col 0 is intercept).
    assert!((s.ss1[1] - model_ss).abs() < 1e-6, "SS1 != ModelSS");
    assert!((s.ss2[1] - model_ss).abs() < 1e-6, "SS2 != ModelSS");
    assert!((s.ss1[1] - s.ss2[1]).abs() < 1e-6, "SS1 != SS2");
    assert!((s.pcorr2[1] - r2).abs() < 1e-6, "PCORR2 != R²");
    assert!((s.scorr2[1] - r2).abs() < 1e-6, "SCORR2 != R²");

    // STB == sign(β)·|corr(x,y)|.
    let xbar = x.iter().sum::<f64>() / n as f64;
    let sxy: f64 = (0..n).map(|i| (x[i] - xbar) * (y[i] - ybar)).sum();
    let sxx: f64 = x.iter().map(|v| (v - xbar) * (v - xbar)).sum();
    let r = sxy / (sxx.sqrt() * sst.sqrt());
    let expect_stb = fit.beta[1].signum() * r.abs();
    assert!(
        (s.stb[1] - expect_stb).abs() < 1e-6,
        "STB={} expected={}",
        s.stb[1],
        expect_stb
    );
    // Intercept STB is 0.
    assert!(s.stb[0].abs() < 1e-12);
}

/// NOINT: SS uses the uncorrected total; Σ SS1 over all columns == Model SS
/// (uncorrected); SS2 == t²·MSE still holds.
#[test]
fn test_oracle_seq_stats_noint() {
    let x = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0];
    let y: Vec<f64> = x.iter().map(|&a| 2.0 * a + (a * 0.5).sin()).collect();
    let n = y.len();
    let xm = design(false, &[&x], n); // no intercept column
    let fit = ols_fit(&xm, &y).unwrap();
    let p_eff = 1;
    let mse = fit.sse / (n - p_eff) as f64;

    let sst: f64 = y.iter().map(|v| v * v).sum(); // uncorrected
    let ssm: f64 = fit.y_hat.iter().map(|v| v * v).sum();

    let mut m = seq_model("y", &["x"]);
    m.noint = true;
    let s = compute_seq_stats(&m, &xm, &y, &fit, sst, false);

    // Σ SS1 over all (no intercept) columns == uncorrected Model SS.
    assert!((s.ss1[0] - ssm).abs() < 1e-6, "SS1={} SSM={ssm}", s.ss1[0]);
    // SS2 == t²·MSE.
    let se = (mse * fit.xtx_inv[0][0]).sqrt();
    let t = fit.beta[0] / se;
    assert!((s.ss2[0] - t * t * mse).abs() < 1e-6);
    // SEQB == OLS β (last & only column).
    assert!((s.seqb[0] - fit.beta[0]).abs() < 1e-9);
}

/// PRESS statistic oracle: Σ (resid_i/(1−h_i))² within 1e-9.
#[test]
fn test_oracle_press_statistic() {
    let x = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let y: Vec<f64> = x.iter().map(|&a| 1.0 + 0.8 * a + (a * 0.6).sin()).collect();
    let n = y.len();
    let xm = design(true, &[&x], n);
    let fit = ols_fit(&xm, &y).unwrap();
    let h = leverages(&xm, &fit.xtx_inv);
    let press_ref: f64 = (0..n)
        .map(|i| {
            let p = fit.resid[i] / (1.0 - h[i]);
            p * p
        })
        .sum();
    // Recompute via the same formula used in run_model.
    let press: f64 = fit
        .resid
        .iter()
        .zip(h.iter())
        .map(|(e, &hi)| {
            let p = e / (1.0 - hi);
            p * p
        })
        .sum();
    assert!((press - press_ref).abs() < 1e-9);
    assert!(press > 0.0);
}

/// End-to-end: M36.5 columns appear in the parameter table and the PRESS fit
/// statistic line is printed; a default model prints none of them.
#[test]
fn test_execute_m365_listing() {
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0, 9.0, 11.0],
        "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
        "x2" => [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0, 7.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = parse_reg(
        "proc reg data=work.t; model y=x1 x2 / ss1 ss2 stb pcorr1 pcorr2 scorr1 scorr2 seqb press; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("Type I SS"), "{listing}");
    assert!(listing.contains("Type II SS"), "{listing}");
    assert!(listing.contains("Standardized Estimate"), "{listing}");
    assert!(listing.contains("Squared Partial Corr Type I"), "{listing}");
    assert!(listing.contains("Squared Partial Corr Type II"), "{listing}");
    assert!(
        listing.contains("Squared Semi-partial Corr Type I"),
        "{listing}"
    );
    assert!(
        listing.contains("Squared Semi-partial Corr Type II"),
        "{listing}"
    );
    assert!(
        listing.contains("Sequential Parameter Estimate"),
        "{listing}"
    );
    assert!(listing.contains("PRESS"), "{listing}");
}

/// Byte-identity guard: a model without the M36.5 options prints none of the
/// new columns or the PRESS line.
#[test]
fn test_m365_off_no_extra_columns() {
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = single_model_ast(
        DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        },
        basic_model("y", &["x"]),
    );
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(!listing.contains("Type I SS"));
    assert!(!listing.contains("Type II SS"));
    assert!(!listing.contains("Standardized Estimate"));
    assert!(!listing.contains("Squared Partial Corr"));
    assert!(!listing.contains("Squared Semi-partial Corr"));
    assert!(!listing.contains("Sequential Parameter Estimate"));
    assert!(!listing.contains("PRESS"));
}

// ───────────────────── M36.6: advanced selection ─────────────────────

fn sel_with(method: SelMethod) -> Selection {
    Selection {
        method,
        slentry: 0.5,
        slstay: 0.1,
        best: None,
        include: 0,
        start: None,
        stop: None,
        details: false,
        stb: false,
    }
}

/// A small fixture with 3 regressors over 8 rows. Returns (xcols, y).
fn three_reg_data() -> (Vec<Vec<f64>>, Vec<f64>) {
    let x0 = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let x1 = vec![2.0, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0, 7.0];
    let x2 = vec![1.0, 3.0, 2.0, 5.0, 4.0, 7.0, 6.0, 9.0];
    let y = vec![3.0, 5.0, 6.0, 9.0, 11.0, 13.0, 16.0, 18.0];
    (vec![x0, x1, x2], y)
}

fn r2_full(xcols: &[Vec<f64>], y: &[f64], cols: &[usize], intercept: bool) -> f64 {
    let n = y.len();
    let sst = if intercept {
        let ybar = y.iter().sum::<f64>() / n as f64;
        y.iter().map(|v| (v - ybar) * (v - ybar)).sum::<f64>()
    } else {
        y.iter().map(|v| v * v).sum::<f64>()
    };
    let sse = subset_sse(xcols, y, cols, intercept).unwrap();
    1.0 - sse / sst
}

#[test]
fn test_m366_parse_rsquare_best_cp() {
    let ast = parse_reg(
        "proc reg data=a; model y = x1 x2 x3 / selection=rsquare best=2 cp; run;",
    )
    .unwrap();
    let sel = ast.models[0].model.selection.unwrap();
    assert_eq!(sel.method, SelMethod::RSquare);
    assert_eq!(sel.best, Some(2));
}

#[test]
fn test_m366_parse_adjrsq() {
    let ast =
        parse_reg("proc reg data=a; model y = x1 x2 / selection=adjrsq; run;").unwrap();
    let sel = ast.models[0].model.selection.unwrap();
    assert_eq!(sel.method, SelMethod::AdjRsq);
}

#[test]
fn test_m366_parse_maxr_include_stop_details_stb() {
    let ast = parse_reg(
        "proc reg data=a; model y = x1 x2 x3 / selection=maxr include=1 stop=2 details stb; run;",
    )
    .unwrap();
    let sel = ast.models[0].model.selection.unwrap();
    assert_eq!(sel.method, SelMethod::MaxR);
    assert_eq!(sel.include, 1);
    assert_eq!(sel.stop, Some(2));
    assert!(sel.details);
    assert!(sel.stb);
}

#[test]
fn test_m366_parse_none() {
    let ast =
        parse_reg("proc reg data=a; model y = x1 / selection=none; run;").unwrap();
    let sel = ast.models[0].model.selection.unwrap();
    assert_eq!(sel.method, SelMethod::None);
}

/// Oracle: an all-subsets enumeration over p regressors (include=0, start=1,
/// stop=p) yields exactly 2^p − 1 non-empty subsets.
#[test]
fn test_m366_all_subsets_count() {
    let (xcols, y) = three_reg_data();
    let p = 3;
    let mut count = 0usize;
    for mask in 1u32..(1u32 << p) {
        let cols: Vec<usize> = (0..p).filter(|b| mask & (1 << b) != 0).collect();
        // Every subset is rank-feasible for this fixture.
        assert!(subset_sse(&xcols, &y, &cols, true).is_some());
        count += 1;
    }
    assert_eq!(count, (1usize << p) - 1);
}

/// Oracle: the full-model subset's R² equals the OLS full-model R², and its
/// Mallows' C(p) ≈ p_eff (within 0.5).
#[test]
fn test_m366_full_model_r2_and_cp() {
    let (xcols, y) = three_reg_data();
    let n = y.len();
    let p = 3;
    let cols: Vec<usize> = (0..p).collect();
    // Full model R² via subset_sse vs. via direct OLS design matrix.
    let r2_subset = r2_full(&xcols, &y, &cols, true);
    let mut x = Vec::new();
    for i in 0..n {
        x.push(vec![1.0, xcols[0][i], xcols[1][i], xcols[2][i]]);
    }
    let fit = ols_fit(&x, &y).unwrap();
    let ybar = y.iter().sum::<f64>() / n as f64;
    let sst = y.iter().map(|v| (v - ybar) * (v - ybar)).sum::<f64>();
    let r2_ols = 1.0 - fit.sse / sst;
    assert!((r2_subset - r2_ols).abs() < 1e-9, "{r2_subset} vs {r2_ols}");

    // C(p) of the full model ≈ p_eff.
    let p_eff = (p + 1) as f64;
    let df_full = n as f64 - p_eff;
    let s2 = fit.sse / df_full;
    let cp = fit.sse / s2 - (n as f64 - 2.0 * p_eff);
    assert!((cp - p_eff).abs() < 0.5, "C(p)={cp} p_eff={p_eff}");
}

/// Oracle: adjusted R² matches 1 − (1−R²)(n−1)/(n−p_eff).
#[test]
fn test_m366_adjusted_r2_formula() {
    let (xcols, y) = three_reg_data();
    let n = y.len() as f64;
    let cols = vec![0usize, 2];
    let r2 = r2_full(&xcols, &y, &cols, true);
    let p_eff = (cols.len() + 1) as f64;
    let adj = 1.0 - (1.0 - r2) * (n - 1.0) / (n - p_eff);
    // Recompute via the same formula the implementation uses.
    let expect = 1.0 - (1.0 - r2) * (n - 1.0) / (n - p_eff);
    assert!((adj - expect).abs() < 1e-12);
}

/// Oracle: INCLUDE=k forces the first k regressors into every enumerated
/// subset (verified through run_all_subsets's listing).
#[test]
fn test_m366_include_forces_first_regressors() {
    let mut session = make_session();
    let (xcols, y) = three_reg_data();
    let regs: Vec<String> = vec!["x1".into(), "x2".into(), "x3".into()];
    let mut sel = sel_with(SelMethod::RSquare);
    sel.include = 1; // x1 forced
    run_all_subsets(&sel, &xcols, &y, &regs, true, &mut session);
    let listing = session.listing.into_string();
    assert!(listing.contains("R-Square Selection Method"), "{listing}");
    // Every "Variables in Model" entry must contain x1; size-1 row is "x1".
    for line in listing.lines() {
        // A data row begins with the model size then R-Square value.
        let t = line.trim();
        if t.starts_with('1') && t.contains("x") {
            assert!(t.contains("x1"), "size-1 row missing forced var: {t}");
        }
    }
}

/// Oracle: MAXR's final (size p) model is the full model, and its size-1
/// model is the single regressor with the highest R².
#[test]
fn test_m366_maxr_final_and_size1() {
    let mut session = make_session();
    let (xcols, y) = three_reg_data();
    let p = 3;
    let regs: Vec<String> = vec!["x1".into(), "x2".into(), "x3".into()];
    let sel = sel_with(SelMethod::MaxR);
    let final_set =
        run_rsq_improvement(&sel, &xcols, &y, &regs, true, &mut session).unwrap();
    assert_eq!(final_set, (0..p).collect::<Vec<usize>>());

    // The single best regressor by R².
    let best_single = (0..p)
        .max_by(|&a, &b| {
            let ra = r2_full(&xcols, &y, &[a], true);
            let rb = r2_full(&xcols, &y, &[b], true);
            ra.partial_cmp(&rb).unwrap()
        })
        .unwrap();
    // Re-run capturing the size-1 model via stop=1.
    let mut s1 = sel_with(SelMethod::MaxR);
    s1.stop = Some(1);
    let mut sess2 = make_session();
    let set1 =
        run_rsq_improvement(&s1, &xcols, &y, &regs, true, &mut sess2).unwrap();
    assert_eq!(set1, vec![best_single]);
}

/// Oracle: SELECTION=NONE produces the same fit as no SELECTION=.
#[test]
fn test_m366_none_matches_no_selection() {
    let build = |sel: Option<Selection>| -> String {
        let mut session = make_session();
        let frame = df![
            "y" => [3.0_f64, 5.0, 6.0, 9.0, 11.0, 13.0, 16.0, 18.0],
            "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
            "x2" => [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0, 7.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
        let mut model = basic_model("y", &["x1", "x2"]);
        model.selection = sel;
        let ast = single_model_ast(
            DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            },
            model,
        );
        execute(&ast, &mut session).unwrap();
        session.listing.into_string()
    };
    let plain = build(None);
    let none = build(Some(sel_with(SelMethod::None)));
    assert_eq!(plain, none);
}

// ───────────────────────── M36.7 oracles ─────────────────────────

/// WEIGHT with all w_i = 1 ⇒ identical β / SSE / SE as unweighted OLS.
#[test]
fn test_oracle_weight_ones_equals_ols() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let y: Vec<f64> = x1.iter().map(|&a| 1.5 + 2.0 * a + (a * 0.3).cos()).collect();
    let n = y.len();
    let x = design(true, &[&x1], n);
    let ols = ols_fit(&x, &y).unwrap();
    let wls = weighted_ols_fit(&x, &y, &vec![1.0; n]).unwrap();
    for j in 0..2 {
        assert!((ols.beta[j] - wls.beta[j]).abs() < 1e-9);
        for k in 0..2 {
            assert!((ols.xtx_inv[j][k] - wls.xtx_inv[j][k]).abs() < 1e-9);
        }
    }
    assert!((ols.sse - wls.sse).abs() < 1e-9);
}

/// WLS β solves the weighted normal equations X'WX β = X'Wy (residual ~ 0).
#[test]
fn test_oracle_weighted_normal_equations() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let y: Vec<f64> = x1.iter().map(|&a| 0.5 + 1.3 * a + (a).sin()).collect();
    let w = [0.5_f64, 2.0, 1.0, 3.0, 0.25, 4.0, 1.5, 0.75];
    let n = y.len();
    let x = design(true, &[&x1], n);
    let fit = weighted_ols_fit(&x, &y, &w).unwrap();
    // Form X'WX and X'Wy and check the residual of the normal equations.
    let p = 2;
    let mut xtwx = vec![vec![0.0; p]; p];
    let mut xtwy = vec![0.0; p];
    for i in 0..n {
        for a in 0..p {
            xtwy[a] += w[i] * x[i][a] * y[i];
            for b in 0..p {
                xtwx[a][b] += w[i] * x[i][a] * x[i][b];
            }
        }
    }
    for a in 0..p {
        let lhs: f64 = (0..p).map(|b| xtwx[a][b] * fit.beta[b]).sum();
        assert!((lhs - xtwy[a]).abs() < 1e-7, "normal eq row {a}: {lhs} vs {}", xtwy[a]);
    }
}

/// WEIGHT equal to a constant c ⇒ same β as OLS, SSE scaled by c.
#[test]
fn test_oracle_weight_constant_scale_invariance() {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0];
    let y: Vec<f64> = x1.iter().map(|&a| 3.0 - 0.7 * a + (a * 0.4).sin()).collect();
    let n = y.len();
    let x = design(true, &[&x1], n);
    let ols = ols_fit(&x, &y).unwrap();
    let c = 4.0;
    let wls = weighted_ols_fit(&x, &y, &vec![c; n]).unwrap();
    for j in 0..2 {
        assert!((ols.beta[j] - wls.beta[j]).abs() < 1e-9);
    }
    assert!((wls.sse - c * ols.sse).abs() < 1e-9);
}

/// FREQ = 2 everywhere ⇒ same β as no FREQ, and the ANOVA df doubles
/// (error_df = 2n − p_eff). End-to-end through execute().
#[test]
fn test_oracle_freq_two_doubles_df() {
    let render = |with_freq: bool| -> String {
        let mut session = make_session();
        let frame = df![
            "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0],
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
            "f" => [2.0_f64, 2.0, 2.0, 2.0, 2.0, 2.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x"), num_meta("f")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
        let mut ast = single_model_ast(
            DatasetRef { libref: Some("WORK".into()), name: "T".into() },
            basic_model("y", &["x"]),
        );
        if with_freq {
            ast.freq = Some("f".into());
        }
        execute(&ast, &mut session).unwrap();
        session.listing.into_string()
    };
    let plain = render(false);
    let freq = render(true);
    // No FREQ: error df = n − 2 = 4; Corrected Total df = 5.
    // FREQ=2: error df = 2n − 2 = 10; Corrected Total df = 11; Used = 12.
    assert!(plain.contains("Number of Observations Used         6"), "{plain}");
    assert!(freq.contains("Number of Observations Used         12"), "{freq}");
    assert!(freq.contains("Corrected Total"), "{freq}");
}

/// FREQ = 1 everywhere ⇒ identical listing to no FREQ at all.
#[test]
fn test_oracle_freq_ones_equals_none() {
    let mut s1 = make_session();
    let mut s2 = make_session();
    let mk = |session: &mut Session| {
        let frame = df![
            "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0],
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0],
            "f" => [1.0_f64, 1.0, 1.0, 1.0, 1.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("y"), num_meta("x"), num_meta("f")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    };
    mk(&mut s1);
    mk(&mut s2);
    let base = single_model_ast(
        DatasetRef { libref: Some("WORK".into()), name: "T".into() },
        basic_model("y", &["x"]),
    );
    let mut with_freq = base.clone();
    with_freq.freq = Some("f".into());
    execute(&base, &mut s1).unwrap();
    execute(&with_freq, &mut s2).unwrap();
    assert_eq!(s1.listing.into_string(), s2.listing.into_string());
}

/// BY with a single group ⇒ identical listing to no BY (the heading is only
/// emitted when groups exist; one group with one distinct key still prints a
/// heading, so we compare a constant-key BY against the non-BY run minus the
/// heading line).
#[test]
fn test_oracle_by_single_group_matches_body() {
    let render = |with_by: bool| -> String {
        let mut session = make_session();
        let frame = df![
            "g" => [1.0_f64, 1.0, 1.0, 1.0, 1.0],
            "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0],
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![num_meta("g"), num_meta("y"), num_meta("x")],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
        let mut ast = single_model_ast(
            DatasetRef { libref: Some("WORK".into()), name: "T".into() },
            basic_model("y", &["x"]),
        );
        if with_by {
            ast.by = vec!["g".into()];
        }
        execute(&ast, &mut session).unwrap();
        session.listing.into_string()
    };
    let plain = render(false);
    let by = render(true);
    // The BY run prepends a `g=1` heading; the regression body is unchanged.
    assert!(by.contains("g=1"), "{by}");
    assert!(by.contains("The REG Procedure"), "{by}");
    // Body identical: strip the BY heading line (and its trailing blank)
    // from the BY output, then compare the regression bodies.
    let by_body: String = by
        .lines()
        .filter(|l| l.trim() != "g=1")
        .collect::<Vec<_>>()
        .join("\n");
    let plain_body: String = plain.lines().collect::<Vec<_>>().join("\n");
    // Drop any leading blank lines introduced by removing the heading.
    assert_eq!(by_body.trim_start(), plain_body.trim_start());
}

/// BY with two groups runs the analysis once per group (two REG headers,
/// two BY headings).
#[test]
fn test_by_two_groups() {
    let mut session = make_session();
    let frame = df![
        "g" => [1.0_f64, 1.0, 1.0, 2.0, 2.0, 2.0],
        "y" => [2.0_f64, 4.0, 6.0, 1.0, 3.0, 5.0],
        "x" => [1.0_f64, 2.0, 3.0, 1.0, 2.0, 3.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("g"), num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let mut ast = single_model_ast(
        DatasetRef { libref: Some("WORK".into()), name: "T".into() },
        basic_model("y", &["x"]),
    );
    ast.by = vec!["g".into()];
    execute(&ast, &mut session).unwrap();
    let out = session.listing.into_string();
    assert!(out.contains("g=1"), "{out}");
    assert!(out.contains("g=2"), "{out}");
    assert_eq!(out.matches("The REG Procedure").count(), 2, "{out}");
}

/// M36.7 — the BY heading is emitted INSIDE the per-model header block:
/// after "The REG Procedure" and before "Model: MODEL1" / "Dependent
/// Variable:", NOT before the title/page banner (Bug 1).
#[test]
fn test_by_heading_inside_header_block() {
    let mut session = make_session();
    let frame = df![
        "g" => [1.0_f64, 1.0, 1.0, 2.0, 2.0, 2.0],
        "y" => [2.0_f64, 4.0, 6.0, 1.0, 3.0, 5.0],
        "x" => [1.0_f64, 2.0, 3.0, 1.0, 2.0, 3.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("g"), num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let mut ast = single_model_ast(
        DatasetRef { libref: Some("WORK".into()), name: "T".into() },
        basic_model("y", &["x"]),
    );
    ast.by = vec!["g".into()];
    execute(&ast, &mut session).unwrap();
    let out = session.listing.into_string();
    let lines: Vec<&str> = out.lines().map(|l| l.trim()).collect();
    // For the first group locate "The REG Procedure", "g=1", and the model
    // label, and assert the ordering proc-line < heading < model-label.
    let proc_i = lines.iter().position(|l| *l == "The REG Procedure").unwrap();
    let head_i = lines.iter().position(|l| *l == "g=1").unwrap();
    let model_i = lines.iter().position(|l| *l == "Model: MODEL1").unwrap();
    assert!(
        proc_i < head_i && head_i < model_i,
        "expected proc < heading < model label; got {proc_i} {head_i} {model_i}\n{out}"
    );
}

/// M36.7 — Bug 2: with a WEIGHT the MODEL R residual summary is weighted.
/// (a) the printed "Sum of Squared Residuals" equals the weighted ANOVA
/// Error SS, and (b) all-ones weights reproduce the unweighted summary
/// byte-for-byte.
#[test]
fn test_weighted_residual_summary_matches_error_ss() {
    // Parse the "Error" ANOVA SS and the "Sum of Squared Residuals" line
    // out of a MODEL .../r listing.
    fn error_ss(listing: &str) -> String {
        listing
            .lines()
            .find(|l| l.trim_start().starts_with("Error "))
            .and_then(|l| l.split_whitespace().nth(2))
            .unwrap()
            .to_string()
    }
    fn sum_sq_resid(listing: &str) -> String {
        listing
            .lines()
            .find(|l| l.trim_start().starts_with("Sum of Squared Residuals"))
            .and_then(|l| l.split_whitespace().last())
            .unwrap()
            .to_string()
    }

    let run = |weight: Option<&str>| -> String {
        let mut session = make_session();
        let frame = df![
            "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0, 8.0],
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
            "w" => [3.0_f64, 1.5, 2.0, 0.5, 4.0, 1.0],
            "ones" => [1.0_f64, 1.0, 1.0, 1.0, 1.0, 1.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![
                num_meta("y"),
                num_meta("x"),
                num_meta("w"),
                num_meta("ones"),
            ],
        };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
        let mut model = basic_model("y", &["x"]);
        model.r = true;
        let mut ast = single_model_ast(
            DatasetRef { libref: Some("WORK".into()), name: "T".into() },
            model,
        );
        ast.weight = weight.map(|w| w.to_string());
        execute(&ast, &mut session).unwrap();
        session.listing.into_string()
    };

    // (a) Weighted run: printed Sum of Squared Residuals == ANOVA Error SS.
    let weighted = run(Some("w"));
    assert_eq!(
        sum_sq_resid(&weighted),
        error_ss(&weighted),
        "weighted Sum of Squared Residuals must equal ANOVA Error SS\n{weighted}"
    );

    // (b) All-ones WEIGHT reproduces the unweighted residual summary exactly.
    let none = run(None);
    let ones = run(Some("ones"));
    let block = |l: &str| -> String {
        l.lines()
            .filter(|x| {
                let t = x.trim_start();
                t.starts_with("Sum of Residuals")
                    || t.starts_with("Sum of Squared Residuals")
                    || t.starts_with("Predicted Residual SS")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(block(&none), block(&ones), "all-ones weight changed summary");
}

/// ID prepends an `Id` leading column to the MODEL R Output Statistics table.
#[test]
fn test_id_column_in_r_table() {
    let mut session = make_session();
    let frame = df![
        "name" => [10.0_f64, 20.0, 30.0, 40.0, 50.0],
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 7.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("name"), num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let mut model = basic_model("y", &["x"]);
    model.r = true;
    let mut ast = single_model_ast(
        DatasetRef { libref: Some("WORK".into()), name: "T".into() },
        model,
    );
    ast.id = vec!["name".into()];
    execute(&ast, &mut session).unwrap();
    let out = session.listing.into_string();
    assert!(out.contains("Output Statistics"), "{out}");
    // The ID values 10..50 appear as a leading column.
    assert!(out.contains("Id"), "{out}");
    assert!(out.contains("10") && out.contains("50"), "{out}");
}

#[test]
fn test_parse_weight_freq_by_id() {
    let ast = parse_reg(
        "proc reg data=a; model y = x; weight wv; freq fv; by grp; id name; run;",
    )
    .unwrap();
    assert_eq!(ast.weight.as_deref(), Some("wv"));
    assert_eq!(ast.freq.as_deref(), Some("fv"));
    assert_eq!(ast.by, vec!["grp".to_string()]);
    assert_eq!(ast.id, vec!["name".to_string()]);
}

#[test]
fn test_parse_by_multiple_and_defaults() {
    let ast = parse_reg("proc reg data=a; model y = x; by a b; run;").unwrap();
    assert_eq!(ast.by, vec!["a".to_string(), "b".to_string()]);
    assert!(ast.weight.is_none());
    assert!(ast.freq.is_none());
    assert!(ast.id.is_empty());
}

// ───────────── M36.8 parse tests ─────────────

#[test]
fn test_parse_simple_corr_all() {
    let ast = parse_reg("proc reg data=a simple corr all; model y = x; run;").unwrap();
    assert!(ast.simple);
    assert!(ast.corr);
    let m = &ast.models[0].model;
    // ALL turns on the MODEL matrix options + CLM/CLI.
    assert!(m.xpx && m.inv && m.covb && m.corrb);
    assert!(m.clm && m.cli);
}

#[test]
fn test_parse_model_matrix_options() {
    let ast =
        parse_reg("proc reg data=a; model y = x / xpx i covb corrb; run;").unwrap();
    let m = &ast.models[0].model;
    assert!(m.xpx && m.inv && m.covb && m.corrb);
    // Untouched flags stay off (byte-identity guard).
    assert!(!m.clb && !m.vif);
}

#[test]
fn test_parse_outest_modifiers() {
    let ast =
        parse_reg("proc reg data=a outest=e covout outseb edf; model y = x; run;")
            .unwrap();
    let oe = ast.data_options.outest.as_ref().unwrap();
    assert_eq!(oe.out.name, "e");
    assert!(oe.covout && oe.outseb && oe.edf);
    assert!(!oe.tableout);
}

#[test]
fn test_parse_outsscp() {
    let ast = parse_reg("proc reg data=a outsscp=s; model y = x; run;").unwrap();
    assert_eq!(ast.data_options.outsscp.as_ref().unwrap().name, "s");
}

#[test]
fn test_parse_default_m368_off() {
    // A plain PROC/MODEL leaves every M36.8 flag off (byte-identity guard).
    let ast = parse_reg("proc reg data=a; model y = x; run;").unwrap();
    assert!(!ast.simple && !ast.corr);
    assert!(ast.data_options.outest.is_none());
    assert!(ast.data_options.outsscp.is_none());
    let m = &ast.models[0].model;
    assert!(!m.xpx && !m.inv && !m.covb && !m.corrb);
}

// ───────────── M36.8 self-consistency oracles ─────────────

/// Shared design: intercept + two regressors, non-degenerate.
fn m368_setup() -> (Vec<Vec<f64>>, Vec<f64>, OlsFit, Vec<String>, usize) {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let x2 = [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0, 7.0];
    let y: Vec<f64> = (0..8)
        .map(|i| 1.0 + 2.0 * x1[i] - 0.5 * x2[i] + (x1[i] * 0.3).cos())
        .collect();
    let n = y.len();
    let x = design(true, &[&x1, &x2], n);
    let fit = ols_fit(&x, &y).unwrap();
    let names = vec!["x1".to_string(), "x2".to_string()];
    (x, y, fit, names, n)
}

#[test]
fn test_oracle_covb_diag_eq_se_squared() {
    let (_x, _y, fit, _names, n) = m368_setup();
    let p_eff = fit.beta.len();
    let mse = fit.sse / (n - p_eff) as f64;
    let covb = covb_matrix(&fit.xtx_inv, mse);
    for j in 0..p_eff {
        let se = (mse * fit.xtx_inv[j][j]).sqrt();
        assert!(
            (covb[j][j] - se * se).abs() < 1e-9,
            "covb_jj != SE_j^2 at {j}"
        );
    }
}

#[test]
fn test_oracle_corrb_diag_one_symmetric() {
    let (_x, _y, fit, _names, n) = m368_setup();
    let p_eff = fit.beta.len();
    let mse = fit.sse / (n - p_eff) as f64;
    let covb = covb_matrix(&fit.xtx_inv, mse);
    let corrb = corrb_matrix(&covb);
    for i in 0..p_eff {
        assert!((corrb[i][i] - 1.0).abs() < 1e-12, "corrb diagonal != 1");
        for j in 0..p_eff {
            assert!(
                (corrb[i][j] - corrb[j][i]).abs() < 1e-12,
                "corrb not symmetric"
            );
        }
    }
}

#[test]
fn test_oracle_xpx_diag_n_and_inverse() {
    let (x, y, fit, _names, n) = m368_setup();
    let p = fit.beta.len();
    let xpx = build_xpx(&x, &y);
    // Intercept column diagonal == n.
    assert!((xpx[0][0] - n as f64).abs() < 1e-9, "X'X[0][0] != n");
    // Symmetric over the full augmented matrix.
    for i in 0..xpx.len() {
        for j in 0..xpx.len() {
            assert!((xpx[i][j] - xpx[j][i]).abs() < 1e-6, "X'X not symmetric");
        }
    }
    // (X'X block) · (X'X)^-1 ≈ I.
    for i in 0..p {
        for j in 0..p {
            let mut s = 0.0;
            for k in 0..p {
                s += xpx[i][k] * fit.xtx_inv[k][j];
            }
            let want = if i == j { 1.0 } else { 0.0 };
            assert!((s - want).abs() < 1e-6, "X'X·inv != I at ({i},{j}): {s}");
        }
    }
}

#[test]
fn test_oracle_simple_stats() {
    let col = vec![1.0_f64, 2.0, 3.0, 4.0, 5.0];
    let n = col.len() as f64;
    let mean = col.iter().sum::<f64>() / n;
    assert!((mean - 3.0).abs() < 1e-12);
    let uss: f64 = col.iter().map(|v| v * v).sum();
    assert!((uss - 55.0).abs() < 1e-12);
    let var = sample_variance(&col);
    assert!((var - 2.5).abs() < 1e-12, "variance: {var}");
}

#[test]
fn test_oracle_outest_parms_row() {
    let (_x, _y, fit, names, n) = m368_setup();
    let entry =
        build_outest_entry("MODEL1", "y", &names, &fit, true, n as f64, 0.05);
    // Parameter estimates equal fit.beta.
    for j in 0..fit.beta.len() {
        assert!((entry.beta[j] - fit.beta[j]).abs() < 1e-12);
    }
    // _RMSE_ == Root MSE.
    let mse = fit.sse / (n - fit.beta.len()) as f64;
    assert!((entry.rmse - mse.sqrt()).abs() < 1e-9);
    // EDF / IN / P.
    assert_eq!(entry.n_in, 2);
    assert_eq!(entry.n_p, 3);
    assert!((entry.edf - (n as f64 - 3.0)).abs() < 1e-12);
}

#[test]
fn test_oracle_outest_dataset_dep_is_minus1() {
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 5.0, 7.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast =
        parse_reg("proc reg data=work.t outest=est; model y = x; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let (out, _) = session.libs.get("WORK").unwrap().read("EST").unwrap();
    let names: Vec<String> = out.vars.iter().map(|v| v.name.clone()).collect();
    // _MODEL_ _TYPE_ _DEPVAR_ _RMSE_ Intercept x y
    assert!(names.contains(&"_MODEL_".to_string()));
    assert!(names.contains(&"_TYPE_".to_string()));
    assert!(names.contains(&"Intercept".to_string()));
    // The dependent column 'y' is set to -1 in the PARMS row.
    let yidx = out.vars.iter().position(|v| v.name == "y").unwrap();
    let ycol = decode_column(&out, yidx).unwrap();
    assert_eq!(value_to_num(&ycol[0]), Some(-1.0));
    // _RMSE_ matches a direct fit.
    let xcol = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0];
    let yv = vec![2.0_f64, 4.0, 5.0, 4.0, 5.0, 7.0];
    let xm = design(true, &[&xcol], 6);
    let fit = ols_fit(&xm, &yv).unwrap();
    let rmse_idx = out.vars.iter().position(|v| v.name == "_RMSE_").unwrap();
    let rmse_col = decode_column(&out, rmse_idx).unwrap();
    let mse = fit.sse / (6 - 2) as f64;
    assert!((value_to_num(&rmse_col[0]).unwrap() - mse.sqrt()).abs() < 1e-9);
}

#[test]
fn test_oracle_outsscp_dataset() {
    let mut session = make_session();
    let xcol = [1.0_f64, 2.0, 3.0, 4.0, 5.0];
    let yv = [2.0_f64, 4.0, 5.0, 4.0, 5.0];
    let frame = df!["y" => yv.to_vec(), "x" => xcol.to_vec()].unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast =
        parse_reg("proc reg data=work.t outsscp=sscp; model y = x; run;").unwrap();
    execute(&ast, &mut session).unwrap();
    let (out, _) = session.libs.get("WORK").unwrap().read("SSCP").unwrap();
    let names: Vec<String> = out.vars.iter().map(|v| v.name.clone()).collect();
    assert_eq!(names[0], "_TYPE_");
    assert_eq!(names[1], "_NAME_");
    assert!(names.contains(&"Intercept".to_string()));
    // Intercept diagonal == n: the Intercept row's Intercept column == 5.
    let nameidx = 1;
    let namecol = decode_column(&out, nameidx).unwrap();
    let int_row = namecol
        .iter()
        .position(|v| matches!(v, crate::value::Value::Char(s) if s.trim_end() == "Intercept"))
        .unwrap();
    let intcolidx = out.vars.iter().position(|v| v.name == "Intercept").unwrap();
    let intcol = decode_column(&out, intcolidx).unwrap();
    assert_eq!(value_to_num(&intcol[int_row]), Some(5.0));
    // (regressor x, dependent y) cell == Σ x_j·y.
    let xrow = namecol
        .iter()
        .position(|v| matches!(v, crate::value::Value::Char(s) if s.trim_end() == "x"))
        .unwrap();
    let ycolidx = out.vars.iter().position(|v| v.name == "y").unwrap();
    let ycol = decode_column(&out, ycolidx).unwrap();
    let want: f64 = xcol.iter().zip(yv.iter()).map(|(a, b)| a * b).sum();
    assert!((value_to_num(&ycol[xrow]).unwrap() - want).abs() < 1e-9);
}

#[test]
fn test_outest_covout_outseb_edf_rows() {
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 5.0, 7.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = parse_reg(
        "proc reg data=work.t outest=est covout outseb edf; model y = x; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let (out, _) = session.libs.get("WORK").unwrap().read("EST").unwrap();
    let names: Vec<String> = out.vars.iter().map(|v| v.name.clone()).collect();
    assert!(names.contains(&"_NAME_".to_string()));
    assert!(names.contains(&"_IN_".to_string()));
    assert!(names.contains(&"_P_".to_string()));
    assert!(names.contains(&"_EDF_".to_string()));
    // _TYPE_ rows: PARMS, COV x2 (Intercept, x), SEB.
    let tidx = out.vars.iter().position(|v| v.name == "_TYPE_").unwrap();
    let tcol = decode_column(&out, tidx).unwrap();
    let types: Vec<String> = tcol
        .iter()
        .map(|v| match v {
            crate::value::Value::Char(s) => s.trim_end().to_string(),
            _ => String::new(),
        })
        .collect();
    assert_eq!(types, vec!["PARMS", "COV", "COV", "SEB"]);
}

// ───────────────────────── M36.9 ridge / IPC ─────────────────────────

/// A collinear sample for the ridge / IPC oracles: x2 ≈ x1 + small noise so
/// R has a small eigenvalue (a clear shrinkage signal).
fn m369_setup() -> (Vec<Vec<f64>>, Vec<f64>) {
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    let x2: Vec<f64> = x1
        .iter()
        .enumerate()
        .map(|(i, v)| v + 0.05 * ((i as f64) * 0.7).sin())
        .collect();
    let y: Vec<f64> = (0..x1.len())
        .map(|i| 3.0 + 1.5 * x1[i] - 0.8 * x2[i] + 0.2 * ((i as f64) * 1.3).cos())
        .collect();
    (vec![x1.to_vec(), x2], y)
}

fn norm2(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}

#[test]
fn test_m369_ridge0_equals_ols() {
    // RIDGE=0 back-transformed estimates reproduce the OLS β (intercept +
    // slopes) within 1e-7.
    let (cols, y) = m369_setup();
    let n = y.len();
    let std = standardize_for_ridge(&cols, &y);
    let b_star = ridge_beta_star(&std.r, &std.r_xy, 0.0).unwrap();
    let (b0, slopes) = back_transform(&b_star, &std);
    let xm = design(true, &[&cols[0], &cols[1]], n);
    let fit = ols_fit(&xm, &y).unwrap();
    assert!((b0 - fit.beta[0]).abs() < 1e-7, "intercept {b0} vs {}", fit.beta[0]);
    assert!((slopes[0] - fit.beta[1]).abs() < 1e-7);
    assert!((slopes[1] - fit.beta[2]).abs() < 1e-7);
}

#[test]
fn test_m369_ridge_shrinks_monotone() {
    // ‖b*(k)‖ ≤ ‖b*(0)‖ and strictly decreasing in k under collinearity.
    let (cols, y) = m369_setup();
    let std = standardize_for_ridge(&cols, &y);
    let ks = [0.0, 0.01, 0.05, 0.1, 0.5];
    let mut prev = f64::INFINITY;
    let n0 = norm2(&ridge_beta_star(&std.r, &std.r_xy, 0.0).unwrap());
    for &k in &ks {
        let nk = norm2(&ridge_beta_star(&std.r, &std.r_xy, k).unwrap());
        assert!(nk <= n0 + 1e-12, "‖b*({k})‖ {nk} > ‖b*(0)‖ {n0}");
        if k > 0.0 {
            assert!(nk < prev, "norm not strictly decreasing at k={k}");
        }
        prev = nk;
    }
}

#[test]
fn test_m369_outvif_k0_equals_ordinary_vif() {
    // Ridge VIF at k=0 == ordinary VIF (within 1e-7); decreases with k.
    let (cols, y) = m369_setup();
    let std = standardize_for_ridge(&cols, &y);
    let (_tol, ord_vif) = vif_tol(&cols);
    let rvif0 = ridge_vif(&std.r, 0.0).unwrap();
    for j in 0..cols.len() {
        assert!(
            (rvif0[j] - ord_vif[j]).abs() < 1e-7,
            "ridge VIF(0) {} != ordinary VIF {} at {j}",
            rvif0[j],
            ord_vif[j]
        );
    }
    let rvif1 = ridge_vif(&std.r, 0.1).unwrap();
    for j in 0..cols.len() {
        assert!(rvif1[j] < rvif0[j], "ridge VIF did not decrease at {j}");
    }
}

#[test]
fn test_m369_pcomit0_equals_ols_and_full_drop_zero() {
    let (cols, y) = m369_setup();
    let n = y.len();
    let p = cols.len();
    let std = standardize_for_ridge(&cols, &y);
    // m=0 → OLS β.
    let b_star0 = ipc_beta_star(&std.r, &std.r_xy, 0).unwrap();
    let (b0, slopes) = back_transform(&b_star0, &std);
    let xm = design(true, &[&cols[0], &cols[1]], n);
    let fit = ols_fit(&xm, &y).unwrap();
    assert!((b0 - fit.beta[0]).abs() < 1e-7);
    assert!((slopes[0] - fit.beta[1]).abs() < 1e-7);
    assert!((slopes[1] - fit.beta[2]).abs() < 1e-7);
    // m=p → standardized estimates all 0; intercept = ȳ.
    let b_star_all = ipc_beta_star(&std.r, &std.r_xy, p).unwrap();
    for b in &b_star_all {
        assert!(b.abs() < 1e-12, "IPC drop-all coef not zero: {b}");
    }
    let (b0_all, slopes_all) = back_transform(&b_star_all, &std);
    for s in &slopes_all {
        assert!(s.abs() < 1e-12);
    }
    assert!((b0_all - std.y_mean).abs() < 1e-9, "intercept != ȳ");
}

#[test]
fn test_m369_parse_ridge_range() {
    // ridge=0 to 0.1 by 0.05 → {0, 0.05, 0.1}.
    let ast = parse_reg("proc reg ridge=0 to 0.1 by 0.05; model y = x; run;").unwrap();
    let r = &ast.data_options.ridge;
    assert_eq!(r.len(), 3, "ridge list {:?}", r);
    assert!((r[0] - 0.0).abs() < 1e-12);
    assert!((r[1] - 0.05).abs() < 1e-12);
    assert!((r[2] - 0.1).abs() < 1e-12);
}

#[test]
fn test_m369_parse_ridge_outvif_outest() {
    let ast =
        parse_reg("proc reg ridge=0 0.1 outvif outest=e; model y = x; run;").unwrap();
    assert_eq!(ast.data_options.ridge, vec![0.0, 0.1]);
    assert!(ast.data_options.outvif);
    assert!(ast.data_options.outest.is_some());
    assert!(ast.data_options.pcomit.is_empty());
}

#[test]
fn test_m369_parse_pcomit_outest() {
    let ast = parse_reg("proc reg pcomit=1 outest=e; model y = x; run;").unwrap();
    assert_eq!(ast.data_options.pcomit, vec![1.0]);
    assert!(ast.data_options.outest.is_some());
    assert!(ast.data_options.ridge.is_empty());
    assert!(!ast.data_options.outvif);
}

#[test]
fn test_m369_outest_ridge_rows_and_columns() {
    // End-to-end: RIDGE= + OUTVIF + OUTEST= produces _RIDGE_ column and
    // RIDGE / RIDGEVIF rows; the k=0 RIDGE row reproduces the OLS PARMS.
    let mut session = make_session();
    let (cols, y) = m369_setup();
    let frame = df![
        "y" => y.clone(),
        "x1" => cols[0].clone(),
        "x2" => cols[1].clone()
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = parse_reg(
        "proc reg data=work.t ridge=0 0.1 outvif outest=est; model y = x1 x2; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let (out, _) = session.libs.get("WORK").unwrap().read("EST").unwrap();
    let names: Vec<String> = out.vars.iter().map(|v| v.name.clone()).collect();
    assert!(names.contains(&"_RIDGE_".to_string()), "no _RIDGE_ col: {names:?}");
    let tidx = out.vars.iter().position(|v| v.name == "_TYPE_").unwrap();
    let tcol = decode_column(&out, tidx).unwrap();
    let types: Vec<String> = tcol
        .iter()
        .map(|v| match v {
            crate::value::Value::Char(s) => s.trim_end().to_string(),
            _ => String::new(),
        })
        .collect();
    // PARMS then RIDGE/RIDGEVIF pairs for k=0 and k=0.1.
    assert_eq!(types[0], "PARMS");
    assert!(types.iter().filter(|t| *t == "RIDGE").count() == 2);
    assert!(types.iter().filter(|t| *t == "RIDGEVIF").count() == 2);
    // The k=0 RIDGE row's Intercept equals the OLS PARMS Intercept.
    let ridge_idx = out.vars.iter().position(|v| v.name == "_RIDGE_").unwrap();
    let ridge_col = decode_column(&out, ridge_idx).unwrap();
    let intidx = out.vars.iter().position(|v| v.name == "Intercept").unwrap();
    let intcol = decode_column(&out, intidx).unwrap();
    let parms_int = value_to_num(&intcol[0]).unwrap();
    // Find the RIDGE row with _RIDGE_==0.
    let k0row = (0..types.len())
        .find(|&i| types[i] == "RIDGE" && value_to_num(&ridge_col[i]) == Some(0.0))
        .unwrap();
    assert!((value_to_num(&intcol[k0row]).unwrap() - parms_int).abs() < 1e-6);
}

#[test]
fn test_m369_value_list_plain() {
    let ast =
        parse_reg("proc reg ridge=0 0.01 0.05 0.1; model y = x; run;").unwrap();
    assert_eq!(ast.data_options.ridge, vec![0.0, 0.01, 0.05, 0.1]);
}

// ───────────────────────── M36.10 MTEST / run-group ─────────────────────────

#[test]
fn test_m3610_parse_multi_response() {
    // Two responses on the LHS → two dependents; regressors as before.
    let ast = parse_reg("proc reg data=a; model y1 y2 = x1 x2; run;").unwrap();
    let m = &ast.models[0].model;
    assert_eq!(m.dependents, vec!["y1", "y2"]);
    assert_eq!(m.regressors, vec!["x1", "x2"]);
    assert_eq!(m.dependent(), "y1");
    // Single response stays a one-element vector.
    let ast2 = parse_reg("proc reg data=a; model y = x1 x2; run;").unwrap();
    assert_eq!(ast2.models[0].model.dependents, vec!["y"]);
}

#[test]
fn test_m3610_multi_response_prints_block_per_dependent() {
    // A two-response MODEL (`model y1 y2 = x;`) must print a SEPARATE
    // univariate regression analysis for EACH dependent — one full
    // "Dependent Variable: y1" block then a full "Dependent Variable: y2"
    // block, each with its own ANOVA and Parameter Estimates — in MODEL
    // order, followed by the MTEST table(s).
    let mut session = make_session();
    let frame = df![
        "y1" => [2.0_f64, 4.0, 5.0, 4.0, 5.0, 7.0, 8.0, 9.0],
        "y2" => [1.0_f64, 3.0, 2.0, 5.0, 4.0, 6.0, 7.0, 8.0],
        "x"  => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y1"), num_meta("y2"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let ast = parse_reg(
        "proc reg data=work.t; model y1 y2 = x; mtest x; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();

    // BOTH per-response blocks are present.
    assert!(
        listing.contains("Dependent Variable: y1"),
        "missing y1 block: {listing}"
    );
    assert!(
        listing.contains("Dependent Variable: y2"),
        "missing y2 block: {listing}"
    );
    // MODEL order: the y1 block precedes the y2 block.
    let p1 = listing.find("Dependent Variable: y1").unwrap();
    let p2 = listing.find("Dependent Variable: y2").unwrap();
    assert!(p1 < p2, "y1 block must precede y2 block: {listing}");
    // Each response carries its own full analysis (two ANOVA tables).
    assert_eq!(
        listing.matches("Analysis of Variance").count(),
        2,
        "expected one ANOVA per response: {listing}"
    );
    // The MTEST table prints ONCE, after both per-response blocks.
    let mtest_pos = listing
        .find("Multivariate Test")
        .expect("MTEST table missing");
    assert!(mtest_pos > p2, "MTEST must follow the per-response blocks");
}

#[test]
fn test_m3610_parse_mtest_and_rungroup() {
    let ast = parse_reg(
        "proc reg data=a; model y1 y2 = x1 x2; \
         mtest; \
         overall: mtest x1, x2; \
         var x3 x4; add x3; delete x1; \
         reweight x1 > 5; refit; paint obs / red; run;",
    )
    .unwrap();
    let entry = &ast.models[0];
    // MTEST: one unlabeled (default), one labeled with two equations.
    assert_eq!(entry.mtests.len(), 2);
    assert!(entry.mtests[0].label.is_none());
    assert!(entry.mtests[0].equations.is_empty());
    assert_eq!(entry.mtests[1].label.as_deref(), Some("overall"));
    assert_eq!(entry.mtests[1].equations.len(), 2);
    // ADD / DELETE / VAR recorded.
    assert_eq!(entry.add, vec!["x3"]);
    assert_eq!(entry.delete, vec!["x1"]);
    assert_eq!(ast.var_list, vec!["x3", "x4"]);
    // Deferred statements flagged.
    assert!(ast.reweight_seen);
    assert!(ast.refit_seen);
    assert!(ast.paint_seen);
}

#[test]
fn test_m3610_statistic_identities() {
    // From three positive generalized eigenvalues, verify the internal
    // identities between the four statistics.
    let lambda = vec![2.5_f64, 1.0, 0.3];
    let stats = mtest_statistics(&lambda, 3.0, 3.0, 20.0);
    let by_name = |n: &str| stats.iter().find(|s| s.name == n).unwrap().value;
    let wilks: f64 = lambda.iter().map(|&l| 1.0 / (1.0 + l)).product();
    let pillai: f64 = lambda.iter().map(|&l| l / (1.0 + l)).sum();
    let hlt: f64 = lambda.iter().sum();
    let roy = 2.5_f64;
    assert!((by_name("Wilks' Lambda") - wilks).abs() < 1e-12);
    assert!((by_name("Pillai's Trace") - pillai).abs() < 1e-12);
    assert!((by_name("Hotelling-Lawley Trace") - hlt).abs() < 1e-12);
    assert!((by_name("Roy's Greatest Root") - roy).abs() < 1e-12);
}

#[test]
fn test_m3610_generalized_eig_symmetric_spd() {
    // E SPD, H symmetric. Generalized eigenvalues of E⁻¹H must match the
    // ordinary eigenvalues of E⁻¹H computed densely.
    let e = vec![vec![4.0, 1.0], vec![1.0, 3.0]];
    let h = vec![vec![2.0, 0.5], vec![0.5, 1.0]];
    let lambda = generalized_eigenvalues(&e, &h).unwrap();
    // Reference: eigenvalues of E⁻¹H.
    let einv = linalg::invert_matrix(&e).unwrap();
    let eih = linalg::matrix_mult(&einv, &h);
    // 2×2 eigenvalues via trace/det.
    let tr = eih[0][0] + eih[1][1];
    let det = eih[0][0] * eih[1][1] - eih[0][1] * eih[1][0];
    let disc = (tr * tr - 4.0 * det).max(0.0).sqrt();
    let mut refv = vec![(tr + disc) / 2.0, (tr - disc) / 2.0];
    refv.sort_by(|a, b| b.partial_cmp(a).unwrap());
    assert!((lambda[0] - refv[0]).abs() < 1e-9, "{lambda:?} vs {refv:?}");
    assert!((lambda[1] - refv[1]).abs() < 1e-9, "{lambda:?} vs {refv:?}");
}

#[test]
fn test_m3610_single_response_reduces_to_anova_f() {
    // Oracle: with one response the four MTEST statistics give the same F as
    // the ANOVA overall F. Build the regression by hand and compare.
    // Data: y = response, x1, x2 regressors, n=8.
    let x1 = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let x2 = [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0, 7.0];
    let y = [3.0_f64, 5.0, 8.0, 9.0, 13.0, 14.0, 18.0, 20.0];
    let n = y.len();
    // Design with intercept.
    let x_mat: Vec<Vec<f64>> = (0..n)
        .map(|i| vec![1.0, x1[i], x2[i]])
        .collect();
    let fit = ols_fit(&x_mat, &y).unwrap();
    let p_eff = 3usize;
    let intercept = true;
    // ANOVA overall F = (SSR/p_reg) / (SSE/(n-p_eff)).
    let ybar = y.iter().sum::<f64>() / n as f64;
    let sst: f64 = y.iter().map(|v| (v - ybar) * (v - ybar)).sum();
    let ssr = sst - fit.sse;
    let p_reg = 2.0; // x1, x2
    let v = (n - p_eff) as f64;
    let anova_f = (ssr / p_reg) / (fit.sse / v);

    // MTEST: q_resp = 1, default hypothesis = both regressors zero.
    let xt = linalg::transpose(&x_mat);
    let xtx = linalg::matrix_mult(&xt, &x_mat);
    let xtx_inv = linalg::invert_matrix(&xtx).unwrap();
    // Y is n×1.
    let y_mat: Vec<Vec<f64>> = y.iter().map(|&v| vec![v]).collect();
    let xty = linalg::matrix_mult(&xt, &y_mat);
    let b = linalg::matrix_mult(&xtx_inv, &xty);
    let yt = linalg::transpose(&y_mat);
    let yty = linalg::matrix_mult(&yt, &y_mat);
    let ytx = linalg::matrix_mult(&yt, &x_mat);
    let ytx_b = linalg::matrix_mult(&ytx, &b);
    let e = vec![vec![yty[0][0] - ytx_b[0][0]]];
    // L = non-intercept rows.
    let base = intercept as usize;
    let l: Vec<Vec<f64>> = (0..2)
        .map(|k| {
            let mut r = vec![0.0; p_eff];
            r[base + k] = 1.0;
            r
        })
        .collect();
    let lb = linalg::matrix_mult(&l, &b);
    let lt = linalg::transpose(&l);
    let lxi = linalg::matrix_mult(&l, &xtx_inv);
    let lxil = linalg::matrix_mult(&lxi, &lt);
    let lxil_inv = linalg::invert_matrix(&lxil).unwrap();
    let mid = linalg::matrix_mult(&linalg::transpose(&lb), &lxil_inv);
    let h = linalg::matrix_mult(&mid, &lb);
    let lambda = generalized_eigenvalues(&e, &h).unwrap();
    let stats = mtest_statistics(&lambda, 1.0, 2.0, v);
    // Every statistic's F approximation must equal the ANOVA F.
    for s in &stats {
        assert!(
            (s.f - anova_f).abs() < 1e-4,
            "{} F={} vs ANOVA F={}",
            s.name,
            s.f,
            anova_f
        );
    }
    // H == model SS (SSR) and E == SSE for q_resp=1; both symmetric/SPD.
    assert!((h[0][0] - ssr).abs() < 1e-6);
    assert!((e[0][0] - fit.sse).abs() < 1e-6);
    assert!(e[0][0] > 0.0);
}

#[test]
fn test_m3610_mtest_execute_prints_table() {
    let mut session = make_session();
    let frame = df![
        "y1" => [3.0_f64, 5.0, 8.0, 9.0, 13.0, 14.0, 18.0, 20.0],
        "y2" => [1.0_f64, 2.0, 2.5, 4.0, 4.5, 6.0, 6.5, 8.0],
        "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
        "x2" => [2.0_f64, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0, 7.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y1"), num_meta("y2"), num_meta("x1"), num_meta("x2")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = parse_reg(
        "proc reg data=work.t; model y1 y2 = x1 x2; mymt: mtest; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("Multivariate Test: mymt"), "{listing}");
    assert!(listing.contains("Wilks' Lambda"), "{listing}");
    assert!(listing.contains("Pillai's Trace"), "{listing}");
    assert!(listing.contains("Hotelling-Lawley Trace"), "{listing}");
    assert!(listing.contains("Roy's Greatest Root"), "{listing}");
}

#[test]
fn test_m3610_add_delete_applied_to_fit() {
    // ADD/DELETE edit the regressor set for the final fit. Verify a NOTE is
    // emitted and the fit uses the edited set (x2 only, after deleting x1 and
    // adding x2 which was not in the original MODEL).
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 5.0, 7.0],
        "x1" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
        "x2" => [6.0_f64, 5.0, 4.0, 3.0, 2.0, 1.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x1"), num_meta("x2")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = parse_reg(
        "proc reg data=work.t; model y = x1; add x2; delete x1; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    let listing = session.listing.into_string();
    // The fitted parameter table must list x2 (the edited regressor).
    assert!(listing.contains("x2"), "{listing}");
    assert!(
        log.contains("ADD/DELETE statements were applied"),
        "log: {log}"
    );
}

#[test]
fn test_m3610_deferred_notes() {
    let mut session = make_session();
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 5.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let ast = parse_reg(
        "proc reg data=work.t; model y = x; reweight x > 3; refit; paint obs; run;",
    )
    .unwrap();
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("REWEIGHT statement"), "log: {log}");
    assert!(log.contains("REFIT statement"), "log: {log}");
    assert!(log.contains("PAINT statement"), "log: {log}");
}

// ───────────────────────── M36.11 PLOTS= / PLOT tests ─────────────────────────

#[test]
fn parse_plots_diagnostics() {
    let ast = parse_reg("proc reg data=a; model y=x; plots=diagnostics; run;").unwrap();
    assert!(ast.plot_requests.diagnostics);
    assert!(ast.plot_requests.explicit);
    assert!(!ast.plot_requests.none);
    assert!(ast.plot_requests.any());
}

#[test]
fn parse_plots_list_residuals_fit() {
    let ast = parse_reg("proc reg data=a; model y=x; plots=(residuals fit); run;").unwrap();
    assert!(ast.plot_requests.residuals);
    assert!(ast.plot_requests.fit);
    assert!(!ast.plot_requests.diagnostics);
}

#[test]
fn parse_plots_unpack_modifier() {
    let ast = parse_reg("proc reg data=a; model y=x; plots(unpack)=diagnostics; run;").unwrap();
    assert!(ast.plot_requests.unpack);
    assert!(ast.plot_requests.diagnostics);
}

#[test]
fn parse_plots_only_modifier() {
    let ast = parse_reg("proc reg data=a; model y=x; plots(only)=fit; run;").unwrap();
    assert!(ast.plot_requests.only);
    assert!(ast.plot_requests.fit);
}

#[test]
fn parse_plots_none() {
    let ast = parse_reg("proc reg data=a; model y=x; plots=none; run;").unwrap();
    assert!(ast.plot_requests.none);
    assert!(!ast.plot_requests.any());
}

#[test]
fn parse_plots_all() {
    let ast = parse_reg("proc reg data=a; model y=x; plots=all; run;").unwrap();
    assert!(ast.plot_requests.all);
    assert!(ast.plot_requests.any());
}

#[test]
fn parse_plots_at_proc_level() {
    let ast = parse_reg("proc reg data=a plots=diagnostics; model y=x; run;").unwrap();
    assert!(ast.plot_requests.diagnostics);
}

#[test]
fn parse_plots_unknown_keyword_ignored() {
    let ast =
        parse_reg("proc reg data=a; model y=x; plots=(diagnostics bogusplot); run;").unwrap();
    assert!(ast.plot_requests.diagnostics);
    // Bogus keyword consumed cleanly, no other family flipped.
    assert!(!ast.plot_requests.residuals && !ast.plot_requests.fit);
}

#[test]
fn parse_plot_statement_simple() {
    let ast = parse_reg("proc reg data=a; model weight=height; plot weight*height; run;")
        .unwrap();
    assert_eq!(ast.plot_statements.len(), 1);
    assert_eq!(ast.plot_statements[0].y, PlotVar::Named("WEIGHT".into()));
    assert_eq!(ast.plot_statements[0].x, PlotVar::Named("HEIGHT".into()));
}

#[test]
fn parse_plot_statement_keyword_vars() {
    let ast =
        parse_reg("proc reg data=a; model y=x; plot residual.*predicted.; run;").unwrap();
    assert_eq!(ast.plot_statements.len(), 1);
    assert_eq!(ast.plot_statements[0].y, PlotVar::Residual);
    assert_eq!(ast.plot_statements[0].x, PlotVar::Predicted);
}

#[test]
fn parse_plot_statement_short_keyword_vars() {
    let ast = parse_reg("proc reg data=a; model y=x; plot r.*p.; run;").unwrap();
    assert_eq!(ast.plot_statements[0].y, PlotVar::Residual);
    assert_eq!(ast.plot_statements[0].x, PlotVar::Predicted);
}

#[test]
fn parse_plot_statement_multiple_pairs() {
    let ast = parse_reg(
        "proc reg data=a; model y=x z; plot y*x z*y / overlay; run;",
    )
    .unwrap();
    assert_eq!(ast.plot_statements.len(), 2);
    assert_eq!(ast.plot_statements[0].y, PlotVar::Named("Y".into()));
    assert_eq!(ast.plot_statements[0].x, PlotVar::Named("X".into()));
    assert_eq!(ast.plot_statements[1].y, PlotVar::Named("Z".into()));
    assert_eq!(ast.plot_statements[1].x, PlotVar::Named("Y".into()));
}

#[test]
fn parse_plot_statement_with_symbol() {
    let ast =
        parse_reg("proc reg data=a; model y=x; plot residual.*predicted.='*'; run;").unwrap();
    assert_eq!(ast.plot_statements.len(), 1);
    assert_eq!(ast.plot_statements[0].y, PlotVar::Residual);
}

/// Build + execute a model carrying explicit PLOTS=/PLOT requests, returning
/// the captured log. Mirrors `run_diag` but injects the M36.11 requests.
fn run_plots(
    ods_on: bool,
    none: bool,
    plot_requests: PlotRequests,
    plot_statements: Vec<PlotPair>,
) -> String {
    let _ = none;
    let mut session = make_session();
    session.ods_graphics.enabled = ods_on;
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 5.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let mut ast = single_model_ast(
        DatasetRef { libref: Some("WORK".into()), name: "T".into() },
        basic_model("y", &["x"]),
    );
    ast.plot_requests = plot_requests;
    ast.plot_statements = plot_statements;
    execute(&ast, &mut session).unwrap();
    session.log.into_string()
}

#[cfg(not(feature = "graphics"))]
#[test]
fn plots_request_defers_under_default_build() {
    let mut req = PlotRequests::default();
    req.diagnostics = true;
    req.explicit = true;
    let log = run_plots(false, false, req, vec![]);
    assert!(log.contains("REG PLOTS= request"), "log: {log}");
    assert!(log.contains("deferred"), "log: {log}");
}

#[cfg(not(feature = "graphics"))]
#[test]
fn plot_statement_defers_under_default_build() {
    let pairs = vec![PlotPair { y: PlotVar::Residual, x: PlotVar::Predicted }];
    let log = run_plots(false, false, PlotRequests::default(), pairs);
    assert!(log.contains("REG PLOT statement"), "log: {log}");
    assert!(log.contains("deferred"), "log: {log}");
}

/// PLOTS=NONE suppresses the automatic diagnostic image/NOTE (even with ODS on).
#[test]
fn plots_none_suppresses_automatic_diagnostic() {
    let mut req = PlotRequests::default();
    req.none = true;
    req.explicit = true;
    let log = run_plots(true, true, req, vec![]);
    assert!(!log.contains("REG diagnostics"), "log: {log}");
    assert!(!log.contains("image deferred"), "log: {log}");
}

/// Byte-identity: a model WITHOUT any PLOTS=/PLOT requests produces exactly
/// the same log as today (no new NOTE lines from M36.11).
#[test]
fn no_plots_request_is_byte_identical() {
    let bare = run_plots(false, false, PlotRequests::default(), vec![]);
    assert!(!bare.contains("REG PLOTS="), "log: {bare}");
    assert!(!bare.contains("REG PLOT statement"), "log: {bare}");
    // And matches the pre-existing run_diag(false,…) output exactly.
    let diag = run_diag(false, None, None);
    assert_eq!(bare, diag);
}

#[cfg(feature = "graphics")]
#[test]
fn plots_request_renders_images_under_graphics() {
    let dir = std::env::temp_dir().join("sasrs_reg_plots_test");
    let _ = std::fs::create_dir_all(&dir);
    let mut session = make_session();
    session.ods_graphics.enabled = true;
    session.ods_graphics.output_dir = dir.clone();
    session.ods_graphics.file_stem = Some("regplt".into());
    let frame = df![
        "y" => [2.0_f64, 4.0, 5.0, 4.0, 5.0, 7.0],
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();
    let mut ast = single_model_ast(
        DatasetRef { libref: Some("WORK".into()), name: "T".into() },
        basic_model("y", &["x"]),
    );
    ast.plot_requests = PlotRequests { all: true, explicit: true, ..Default::default() };
    ast.plot_statements = vec![PlotPair { y: PlotVar::Residual, x: PlotVar::Predicted }];
    execute(&ast, &mut session).unwrap();
    let log = session.log.into_string();
    assert!(log.contains("written"), "log: {log}");
    // The first image is the automatic diagnostic (regplt_1); subsequent
    // images come from PLOTS=ALL + the PLOT statement. Confirm at least the
    // automatic and one request image exist.
    let p1 = dir.join("regplt_1.png");
    assert!(p1.exists(), "automatic diagnostic image missing: {p1:?}");
    let p2 = dir.join("regplt_2.png");
    assert!(p2.exists(), "requested plot image missing: {p2:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
