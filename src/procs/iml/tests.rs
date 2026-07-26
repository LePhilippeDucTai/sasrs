use super::*;

fn test_session() -> Session {
    use std::path::PathBuf;
    Session::new(None, PathBuf::from("."), true).unwrap()
}

fn eval_one(src: &str) -> Matrix {
    // Enveloppe : assigne à `r` et renvoie sa valeur.
    let prog = parse_body(&format!("r = {src};")).unwrap();
    let mut env = Env::new();
    let mut ops = Vec::new();
    let mut session = test_session();
    exec_stmts(&prog.stmts, &mut env, &mut ops, &mut session).unwrap();
    env.vars.get("R").unwrap().clone()
}

fn eval_try(src: &str) -> Result<Matrix> {
    let prog = parse_body(&format!("r = {src};"))?;
    let mut env = Env::new();
    let mut ops = Vec::new();
    let mut session = test_session();
    exec_stmts(&prog.stmts, &mut env, &mut ops, &mut session)?;
    Ok(env.vars.get("R").unwrap().clone())
}

fn run_get(src: &str, var: &str) -> Matrix {
    let prog = parse_body(src).unwrap();
    let mut env = Env::new();
    let mut ops = Vec::new();
    let mut session = test_session();
    exec_stmts(&prog.stmts, &mut env, &mut ops, &mut session).unwrap();
    env.vars.get(&var.to_ascii_uppercase()).unwrap().clone()
}

#[test]
fn lit_row_vector() {
    assert_eq!(eval_one("{1 2 3}"), vec![vec![1.0, 2.0, 3.0]]);
}

#[test]
fn lit_col_vector() {
    assert_eq!(eval_one("{1, 2, 3}"), vec![vec![1.0], vec![2.0], vec![3.0]]);
}

#[test]
fn lit_2x2() {
    assert_eq!(eval_one("{1 2, 3 4}"), vec![vec![1.0, 2.0], vec![3.0, 4.0]]);
}

#[test]
fn matrix_product() {
    assert_eq!(
        eval_one("{1 2,3 4}*{5 6,7 8}"),
        vec![vec![19.0, 22.0], vec![43.0, 50.0]]
    );
}

#[test]
fn transpose_op() {
    assert_eq!(eval_one("{1 2,3 4}'"), vec![vec![1.0, 3.0], vec![2.0, 4.0]]);
}

#[test]
fn hadamard() {
    assert_eq!(
        eval_one("{1 2,3 4}#{5 6,7 8}"),
        vec![vec![5.0, 12.0], vec![21.0, 32.0]]
    );
}

#[test]
fn nrow_ncol() {
    assert_eq!(eval_one("nrow({1 2 3, 4 5 6})"), scalar(2.0));
    assert_eq!(eval_one("ncol({1 2 3, 4 5 6})"), scalar(3.0));
}

#[test]
fn sum_fn() {
    assert_eq!(eval_one("sum({2 4 6 8 10})"), scalar(30.0));
}

#[test]
fn std_fn() {
    let s = as_scalar(&eval_one("std({2 4 6 8 10})")).unwrap();
    assert!((s - 3.1622776601).abs() < 0.001, "std = {s}");
}

#[test]
fn do_loop_accumulates() {
    let m = run_get(
        "total = {0}; do i = 1 to 5; total = total + i; end;",
        "total",
    );
    assert_eq!(m, scalar(15.0));
}

#[test]
fn if_then_else() {
    let m = run_get("if 15 > {10} then big = {1}; else big = {0};", "big");
    assert_eq!(m, scalar(1.0));
}

#[test]
fn subscript_scalar() {
    assert_eq!(eval_one("{1 2,3 4}[2,1]"), scalar(3.0));
}

#[test]
fn subscript_row() {
    assert_eq!(eval_one("{1 2,3 4}[1,*]"), vec![vec![1.0, 2.0]]);
}

#[test]
fn subscript_col() {
    assert_eq!(eval_one("{1 2,3 4}[*,2]"), vec![vec![2.0], vec![4.0]]);
}

#[test]
fn quit_parsed_whole_block() {
    // parse_body reçoit le corps SANS le quit; (retiré par le lexer SAS).
    let prog = parse_body("a = {1};").unwrap();
    assert_eq!(prog.stmts.len(), 1);
}

#[test]
fn print_generates_listing_section() {
    use crate::session::Session;
    use std::path::PathBuf;
    let mut session = Session::new(None, PathBuf::from("."), true).unwrap();
    let prog = parse_body("a = {1 2, 3 4}; print a;").unwrap();
    execute(&prog, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("The IML Procedure"), "listing: {listing}");
    assert!(listing.contains("COL1"), "listing: {listing}");
    assert!(listing.contains("ROW1"), "listing: {listing}");
}

#[test]
fn kronecker_2x2() {
    // {1 0,0 1} @ {1 2,3 4} = block diag.
    let m = eval_one("{1 0,0 1}@{1 2,3 4}");
    assert_eq!(
        m,
        vec![
            vec![1.0, 2.0, 0.0, 0.0],
            vec![3.0, 4.0, 0.0, 0.0],
            vec![0.0, 0.0, 1.0, 2.0],
            vec![0.0, 0.0, 3.0, 4.0],
        ]
    );
}

#[test]
fn negative_and_decimal_literals() {
    assert_eq!(
        eval_one("{1.5 -2, 0 3.7}"),
        vec![vec![1.5, -2.0], vec![0.0, 3.7]]
    );
}

// ───────────────────── M28a.3 : algèbre linéaire ─────────────────────

fn approx(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn inv_2x2() {
    let m = eval_one("inv({4 7, 2 6})");
    // inv = {0.6 -0.7, -0.2 0.4}
    assert!(approx(m[0][0], 0.6, 1e-3), "m={m:?}");
    assert!(approx(m[0][1], -0.7, 1e-3), "m={m:?}");
    assert!(approx(m[1][0], -0.2, 1e-3), "m={m:?}");
    assert!(approx(m[1][1], 0.4, 1e-3), "m={m:?}");
}

#[test]
fn solve_diagonal() {
    let m = eval_one("solve({2 0, 0 3}, {6, 9})");
    // x = {3, 3} (column vector)
    assert_eq!(dims(&m), (2, 1));
    assert!(approx(m[0][0], 3.0, 1e-3), "m={m:?}");
    assert!(approx(m[1][0], 3.0, 1e-3), "m={m:?}");
}

#[test]
fn eigval_symmetric() {
    let m = eval_one("eigval({4 2, 2 1})");
    // {5, 0} descending, column vector
    assert_eq!(dims(&m), (2, 1));
    assert!(approx(m[0][0], 5.0, 1e-3), "m={m:?}");
    assert!(approx(m[1][0], 0.0, 1e-3), "m={m:?}");
}

#[test]
fn eigval_nonsymmetric_errors() {
    let e = eval_try("eigval({1 2, 3 4})");
    assert!(e.is_err());
    let msg = e.err().unwrap().to_string();
    assert!(msg.contains("symmetric"), "msg={msg}");
}

#[test]
fn chol_upper() {
    let m = eval_one("chol({4 2, 2 3})");
    // U = {2 1, 0 1.4142}
    assert!(approx(m[0][0], 2.0, 1e-3), "m={m:?}");
    assert!(approx(m[0][1], 1.0, 1e-3), "m={m:?}");
    assert!(approx(m[1][0], 0.0, 1e-3), "m={m:?}");
    assert!(approx(m[1][1], std::f64::consts::SQRT_2, 1e-3), "m={m:?}");
}

#[test]
fn chol_not_spd_errors() {
    // {1 2, 2 1} is indefinite (det = 1 - 4 = -3 < 0).
    let e = eval_try("chol({1 2, 2 1})");
    assert!(e.is_err(), "expected error for non-SPD matrix");
}

#[test]
fn call_qr_dimensions() {
    let prog = parse_body("call qr(q, r, {1 2, 3 4, 5 6});").unwrap();
    let mut env = Env::new();
    let mut ops = Vec::new();
    let mut session = test_session();
    exec_stmts(&prog.stmts, &mut env, &mut ops, &mut session).unwrap();
    let q = env.vars.get("Q").unwrap();
    let r = env.vars.get("R").unwrap();
    assert_eq!(dims(q), (3, 2), "Q dims");
    assert_eq!(dims(r), (2, 2), "R dims");
    // Q*R ≈ original.
    let qr = eval_binop(ImlOp::Mul, q, r).unwrap();
    assert!(
        approx(qr[0][0], 1.0, 1e-6) && approx(qr[2][1], 6.0, 1e-6),
        "qr={qr:?}"
    );
}

#[test]
fn call_svdcd_singular_values() {
    let prog = parse_body("call svdcd(u, d, v, {1 2, 3 4});").unwrap();
    let mut env = Env::new();
    let mut ops = Vec::new();
    let mut session = test_session();
    exec_stmts(&prog.stmts, &mut env, &mut ops, &mut session).unwrap();
    let d = env.vars.get("D").unwrap();
    assert_eq!(dims(d), (2, 1), "D should be a column vector");
    assert!(approx(d[0][0], 5.4651, 1e-2), "σ1={}", d[0][0]);
    assert!(approx(d[1][0], 0.3660, 1e-2), "σ2={}", d[1][0]);
    // Reconstruction: A = U diag(D) V'.
    let u = env.vars.get("U").unwrap().clone();
    let vmat = env.vars.get("V").unwrap().clone();
    let ud: Matrix = u
        .iter()
        .map(|row| row.iter().enumerate().map(|(j, &x)| x * d[j][0]).collect())
        .collect();
    let recon = eval_binop(ImlOp::Mul, &ud, &transpose(&vmat)).unwrap();
    assert!(
        approx(recon[0][0], 1.0, 1e-4) && approx(recon[1][1], 4.0, 1e-4),
        "recon={recon:?}"
    );
}

// ───────────────────── M34.10 : SHAPE / range / DET / EIGEN ─────────────

#[test]
fn shape_exact_reshape() {
    assert_eq!(
        eval_one("shape({1 2 3 4 5 6}, 2, 3)"),
        vec![vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0]]
    );
}

#[test]
fn shape_recycles() {
    assert_eq!(
        eval_one("shape({1 2}, 2, 2)"),
        vec![vec![1.0, 2.0], vec![1.0, 2.0]]
    );
}

#[test]
fn shape_infers_ncol() {
    // 6 elements into 2 rows → 3 columns inferred.
    assert_eq!(
        eval_one("shape({1 2 3 4 5 6}, 2)"),
        vec![vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0]]
    );
}

#[test]
fn range_subscript_2x2_block() {
    // Top-right 2×2 of a 3×3 (rows 1:2, cols 2:3).
    let m = run_get("a = {1 2 3, 4 5 6, 7 8 9}; r = a[1:2, 2:3];", "r");
    assert_eq!(m, vec![vec![2.0, 3.0], vec![5.0, 6.0]]);
}

#[test]
fn range_subscript_rows_all_cols() {
    let m = run_get("a = {1 2 3, 4 5 6, 7 8 9}; r = a[2:3, *];", "r");
    assert_eq!(m, vec![vec![4.0, 5.0, 6.0], vec![7.0, 8.0, 9.0]]);
}

#[test]
fn range_subscript_all_rows_cols() {
    let m = run_get("a = {1 2 3, 4 5 6, 7 8 9}; r = a[ , 1:2];", "r");
    assert_eq!(m, vec![vec![1.0, 2.0], vec![4.0, 5.0], vec![7.0, 8.0]]);
}

#[test]
fn det_2x2_oracle() {
    // DET({4 3, 6 3}) = 4*3 - 3*6 = -6.
    let d = as_scalar(&eval_one("det({4 3, 6 3})")).unwrap();
    assert!(approx(d, -6.0, 1e-9), "det = {d}");
}

#[test]
fn det_identity_is_one() {
    let d = as_scalar(&eval_one("det({1 0 0, 0 1 0, 0 0 1})")).unwrap();
    assert!(approx(d, 1.0, 1e-9), "det = {d}");
}

#[test]
fn det_singular_is_zero() {
    let d = as_scalar(&eval_one("det({1 2, 2 4})")).unwrap();
    assert!(approx(d, 0.0, 1e-9), "det = {d}");
}

#[test]
fn eigvec_orthonormal() {
    // V'V = I for a symmetric matrix.
    let v = eval_one("eigvec({2 0, 0 3})");
    let vtv = eval_binop(ImlOp::Mul, &transpose(&v), &v).unwrap();
    assert!(approx(vtv[0][0], 1.0, 1e-9), "vtv={vtv:?}");
    assert!(approx(vtv[1][1], 1.0, 1e-9), "vtv={vtv:?}");
    assert!(approx(vtv[0][1], 0.0, 1e-9), "vtv={vtv:?}");
    assert!(approx(vtv[1][0], 0.0, 1e-9), "vtv={vtv:?}");
}

#[test]
fn call_eigen_values_and_vectors() {
    // {2 0, 0 3}: eigenvalues 3,2 (descending) with axis-aligned vectors.
    let prog = parse_body("call eigen(val, vec, {2 0, 0 3});").unwrap();
    let mut env = Env::new();
    let mut ops = Vec::new();
    let mut session = test_session();
    exec_stmts(&prog.stmts, &mut env, &mut ops, &mut session).unwrap();
    let val = env.vars.get("VAL").unwrap();
    assert_eq!(dims(val), (2, 1), "values must be a column vector");
    assert!(approx(val[0][0], 3.0, 1e-9), "val={val:?}");
    assert!(approx(val[1][0], 2.0, 1e-9), "val={val:?}");
    // Vectors orthonormal: Vᵀ V = I.
    let vec = env.vars.get("VEC").unwrap().clone();
    let vtv = eval_binop(ImlOp::Mul, &transpose(&vec), &vec).unwrap();
    assert!(
        approx(vtv[0][0], 1.0, 1e-9) && approx(vtv[1][1], 1.0, 1e-9),
        "vtv={vtv:?}"
    );
    assert!(
        approx(vtv[0][1], 0.0, 1e-9) && approx(vtv[1][0], 0.0, 1e-9),
        "vtv={vtv:?}"
    );
    // Reconstruct A = V diag(λ) Vᵀ.
    let vd: Matrix = vec
        .iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .map(|(j, &x)| x * val[j][0])
                .collect()
        })
        .collect();
    let recon = eval_binop(ImlOp::Mul, &vd, &transpose(&vec)).unwrap();
    assert!(
        approx(recon[0][0], 2.0, 1e-9) && approx(recon[1][1], 3.0, 1e-9),
        "recon={recon:?}"
    );
}

#[test]
fn eigvec_nonsymmetric_errors() {
    let e = eval_try("eigvec({1 2, 3 4})");
    assert!(e.is_err());
    assert!(e.err().unwrap().to_string().contains("symmetric"));
}

// ───────────────────── M28a.4 : I/O datasets ─────────────────────

#[test]
fn create_append_close_writes_dataset() {
    let src = r#"
        mat_out = {1 10, 2 20, 3 30};
        cn = {"id" "val"};
        create work.iml_out from mat_out[colname=cn];
        append from mat_out;
        close work.iml_out;
    "#;
    let prog = parse_body(src).unwrap();
    let mut env = Env::new();
    let mut ops = Vec::new();
    let mut session = test_session();
    exec_stmts(&prog.stmts, &mut env, &mut ops, &mut session).unwrap();

    let (ds, _) = session.libs.get("WORK").unwrap().read("IML_OUT").unwrap();
    assert_eq!(ds.n_obs(), 3, "expected 3 rows");
    let names: Vec<String> = ds.vars.iter().map(|v| v.name.to_lowercase()).collect();
    assert_eq!(names, vec!["id".to_string(), "val".to_string()]);
}

#[test]
fn use_read_all_close_reads_dataset() {
    // First write a dataset, then USE/READ it back.
    let mut session = test_session();
    {
        use crate::dataset::{SasDataset, VarMeta};
        use crate::value::VarType;
        use polars::prelude::*;
        let df = df!["x" => [1.0_f64, 2.0, 3.0], "y" => [10.0_f64, 20.0, 30.0]].unwrap();
        let vars = vec![
            VarMeta {
                name: "x".into(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            },
            VarMeta {
                name: "y".into(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            },
        ];
        session
            .libs
            .get("WORK")
            .unwrap()
            .write("IML_IN", &SasDataset { df, vars })
            .unwrap();
    }
    let src = r#"
        use work.iml_in;
        read all var {"x" "y"} into m;
        close work.iml_in;
    "#;
    let prog = parse_body(src).unwrap();
    let mut env = Env::new();
    let mut ops = Vec::new();
    exec_stmts(&prog.stmts, &mut env, &mut ops, &mut session).unwrap();
    let m = env.vars.get("M").unwrap();
    assert_eq!(dims(m), (3, 2), "m={m:?}");
    assert!(
        approx(m[0][0], 1.0, 1e-9) && approx(m[2][1], 30.0, 1e-9),
        "m={m:?}"
    );
}

#[test]
fn read_next_deferred_error() {
    let prog = parse_body("read next into m;").unwrap();
    let mut env = Env::new();
    let mut ops = Vec::new();
    let mut session = test_session();
    let e = exec_stmts(&prog.stmts, &mut env, &mut ops, &mut session);
    assert!(e.is_err());
    assert!(e.err().unwrap().to_string().contains("READ NEXT"));
}
