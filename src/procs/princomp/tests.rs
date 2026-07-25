use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::session::Session;
use crate::source::SourceFile;
use crate::value::VarType;
use polars::df;
use std::path::PathBuf;

fn make_session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

fn num_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}

fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
    session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
    session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
}

fn parse_princomp(src: &str) -> Result<PrincompAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "princomp"
    parse(&mut ts)
}

// ───────────── parse tests ─────────────

#[test]
fn parse_minimal() {
    let ast = parse_princomp("proc princomp data=a; var x y; run;").unwrap();
    assert_eq!(ast.data.as_ref().unwrap().name, "a");
    assert!(!ast.cov);
    assert_eq!(ast.n, None);
    assert!(ast.out.is_none());
    assert_eq!(ast.var, vec!["x", "y"]);
}

#[test]
fn parse_options() {
    let ast =
        parse_princomp("proc princomp data=a cov n=2 out=b; var x y z; run;").unwrap();
    assert!(ast.cov);
    assert_eq!(ast.n, Some(2));
    assert_eq!(ast.out.as_ref().unwrap().name, "b");
    assert_eq!(ast.var, vec!["x", "y", "z"]);
}

#[test]
fn parse_unknown_option_errors() {
    let r = parse_princomp("proc princomp data=a bogus; var x y; run;");
    assert!(r.is_err());
    assert!(r.err().unwrap().to_string().contains("BOGUS"));
}

// ───────────── execute / invariant tests ─────────────

#[test]
fn execute_too_few_variables_errors() {
    let mut session = make_session();
    let df = df!["x" => [1.0_f64, 2.0, 3.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);
    let ast = PrincompAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        cov: false,
        n: None,
        out: None,
        var: vec!["x".into()],
    };
    let r = execute(&ast, &mut session);
    assert!(r.is_err());
    assert!(r.err().unwrap().to_string().contains("at least 2 variables"));
}

#[test]
fn execute_missing_variable_errors() {
    let mut session = make_session();
    let df = df!["x" => [1.0_f64, 2.0], "y" => [3.0_f64, 4.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);
    let ast = PrincompAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        cov: false,
        n: None,
        out: None,
        var: vec!["x".into(), "z".into()],
    };
    let r = execute(&ast, &mut session);
    assert!(r.is_err());
    let msg = r.err().unwrap().to_string();
    assert!(msg.contains("'z' not found in dataset"), "{msg}");
}

/// Critical invariant: for the CORRELATION matrix, Σλ == number of variables.
/// If the code mistakenly used the covariance matrix, the sum would differ.
#[test]
fn correlation_eigenvalues_sum_to_p() {
    // x=[1,2,3,4,5], y=[2,3,3,5,4] (the oracle fixture data).
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let ys = [2.0, 3.0, 3.0, 5.0, 4.0];
    let n = xs.len();
    let nf = n as f64;
    let mx = xs.iter().sum::<f64>() / nf;
    let my = ys.iter().sum::<f64>() / nf;
    let mut sxx = 0.0;
    let mut syy = 0.0;
    let mut sxy = 0.0;
    for i in 0..n {
        let dx = xs[i] - mx;
        let dy = ys[i] - my;
        sxx += dx * dx;
        syy += dy * dy;
        sxy += dx * dy;
    }
    let r = sxy / (sxx.sqrt() * syy.sqrt());
    let corr = vec![vec![1.0, r], vec![r, 1.0]];
    let (_, lambda) = eigenvectors_jacobi(&corr).unwrap();
    let sum: f64 = lambda.iter().sum();
    // For a 2-variable correlation matrix, Σλ must equal p = 2.
    assert!((sum - 2.0).abs() < 1e-10, "Σλ={sum}, expected 2.0");
    // And the eigenvalues are 1±r.
    assert!((lambda[0] - (1.0 + r)).abs() < 1e-10, "λ1={}", lambda[0]);
    assert!((lambda[1] - (1.0 - r)).abs() < 1e-10, "λ2={}", lambda[1]);
    // r should be ~0.8321.
    assert!((r - 0.8321).abs() < 1e-3, "r={r}");
}

#[test]
fn execute_oracle_listing() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0],
        "y" => [2.0_f64, 3.0, 3.0, 5.0, 4.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = PrincompAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        cov: false,
        n: None,
        out: None,
        var: vec!["x".into(), "y".into()],
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("The PRINCOMP Procedure"), "{listing}");
    assert!(listing.contains("Correlation Matrix"), "{listing}");
    // Eigenvalues 1.8321 and 0.1679.
    assert!(listing.contains("1.8321"), "{listing}");
    assert!(listing.contains("0.1679"), "{listing}");
    // Eigenvector elements 0.707107.
    assert!(listing.contains("0.707107"), "{listing}");
    // Means 3.0000 (x) and 3.4000 (y).
    assert!(listing.contains("3.0000"), "{listing}");
    assert!(listing.contains("3.4000"), "{listing}");
}

/// OUT= oracle: on the 2-var fixture, each component score must have
/// sample variance (n-1) equal to its eigenvalue (1+r and 1-r), and the
/// score columns must have mean ≈ 0.
#[test]
fn out_scores_variance_equals_eigenvalues() {
    use polars::prelude::*;
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0],
        "y" => [2.0_f64, 3.0, 3.0, 5.0, 4.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = PrincompAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        cov: false,
        n: None,
        out: Some(DatasetRef { libref: Some("WORK".into()), name: "SCORES".into() }),
        var: vec!["x".into(), "y".into()],
    };
    execute(&ast, &mut session).unwrap();

    // _LAST_ should now be the OUT= dataset.
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.SCORES"));

    let (out, _) = session.libs.get("WORK").unwrap().read("SCORES").unwrap();
    // Input columns + Prin1 + Prin2.
    assert!(out.vars.iter().any(|m| m.name == "Prin1"));
    assert!(out.vars.iter().any(|m| m.name == "Prin2"));
    assert!(out.vars.iter().any(|m| m.name.eq_ignore_ascii_case("x")));

    // Known correlation r = 0.8321 -> eigenvalues 1+r and 1-r.
    let r = 0.8320502943378437_f64;
    let expected = [1.0 + r, 1.0 - r];

    for (comp, &lam) in expected.iter().enumerate() {
        let col = out
            .df
            .column(&format!("Prin{}", comp + 1))
            .unwrap()
            .f64()
            .unwrap();
        let vals: Vec<f64> = col.into_no_null_iter().collect();
        let n = vals.len() as f64;
        let mean = vals.iter().sum::<f64>() / n;
        assert!(mean.abs() < 1e-9, "Prin{} mean={mean}", comp + 1);
        let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
        assert!(
            (var - lam).abs() < 1e-9,
            "Prin{} variance={var}, expected eigenvalue {lam}",
            comp + 1
        );
    }
}

/// COV scoring: scores are only centered, not standardized; their sample
/// variances equal the covariance-matrix eigenvalues (== column variances'
/// sum). Verify the score column means are ≈ 0.
#[test]
fn out_scores_cov_centered_mean_zero() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0],
        "y" => [2.0_f64, 3.0, 3.0, 5.0, 4.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y")],
    };
    write_dataset(&mut session, "T", ds);

    let ast = PrincompAst {
        data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
        cov: true,
        n: None,
        out: Some(DatasetRef { libref: Some("WORK".into()), name: "CS".into() }),
        var: vec!["x".into(), "y".into()],
    };
    execute(&ast, &mut session).unwrap();
    let (out, _) = session.libs.get("WORK").unwrap().read("CS").unwrap();
    for comp in 1..=2 {
        let col = out.df.column(&format!("Prin{comp}")).unwrap().f64().unwrap();
        let vals: Vec<f64> = col.into_no_null_iter().collect();
        let mean = vals.iter().sum::<f64>() / vals.len() as f64;
        assert!(mean.abs() < 1e-9, "Prin{comp} mean={mean}");
    }
}
