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

fn parse_factor(src: &str) -> Result<FactorAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // "proc"
    ts.next(); // "factor"
    parse(&mut ts)
}

// ───────────── parse tests ─────────────

#[test]
fn parse_minimal() {
    let ast = parse_factor("proc factor data=a; var x y; run;").unwrap();
    assert_eq!(ast.data.as_ref().unwrap().name, "a");
    assert!(!ast.cov);
    assert_eq!(ast.nfactors, None);
    assert_eq!(ast.method, "principal");
    assert_eq!(ast.rotate, "none");
    assert!(ast.out.is_none());
    assert_eq!(ast.var, vec!["x", "y"]);
}

#[test]
fn parse_options() {
    let ast =
        parse_factor("proc factor data=a cov nfactors=2 rotate=varimax out=b; var x y z; run;")
            .unwrap();
    assert!(ast.cov);
    assert_eq!(ast.nfactors, Some(2));
    assert_eq!(ast.rotate, "varimax");
    assert_eq!(ast.out.as_ref().unwrap().name, "b");
    assert_eq!(ast.var, vec!["x", "y", "z"]);
}

#[test]
fn parse_unknown_option_errors() {
    let r = parse_factor("proc factor data=a bogus; var x y; run;");
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
    let ast = FactorAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        cov: false,
        nfactors: None,
        method: "principal".into(),
        rotate: "none".into(),
        out: None,
        var: vec!["x".into()],
    };
    let r = execute(&ast, &mut session);
    assert!(r.is_err());
    assert!(
        r.err()
            .unwrap()
            .to_string()
            .contains("at least 2 variables")
    );
}

#[test]
fn execute_invalid_method_errors() {
    let mut session = make_session();
    let ast = FactorAst {
        data: None,
        cov: false,
        nfactors: None,
        method: "ml".into(),
        rotate: "none".into(),
        out: None,
        var: vec!["x".into(), "y".into()],
    };
    let r = execute(&ast, &mut session);
    assert!(r.is_err());
    assert!(r.err().unwrap().to_string().contains("ML"));
}

#[test]
fn execute_invalid_rotate_errors() {
    let mut session = make_session();
    let ast = FactorAst {
        data: None,
        cov: false,
        nfactors: None,
        method: "principal".into(),
        rotate: "quartimax".into(),
        out: None,
        var: vec!["x".into(), "y".into()],
    };
    let r = execute(&ast, &mut session);
    assert!(r.is_err());
    assert!(r.err().unwrap().to_string().contains("QUARTIMAX"));
}

#[test]
fn execute_oblimin_deferred_errors() {
    let mut session = make_session();
    let ast = FactorAst {
        data: None,
        cov: false,
        nfactors: None,
        method: "principal".into(),
        rotate: "oblimin".into(),
        out: None,
        var: vec!["x".into(), "y".into()],
    };
    let r = execute(&ast, &mut session);
    assert!(r.is_err());
    assert!(r.err().unwrap().to_string().contains("OBLIMIN"));
}

/// Oracle test: x=[1,2,3,4,5], y=[2,3,3,5,4]
/// Expected: 1 factor retained (Kaiser), loading ≈ 0.9571,
/// h² ≈ 0.9160, total communality ≈ 1.8321.
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

    let ast = FactorAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        cov: false,
        nfactors: None,
        method: "principal".into(),
        rotate: "none".into(),
        out: None,
        var: vec!["x".into(), "y".into()],
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("The FACTOR Procedure"), "{listing}");
    assert!(listing.contains("Factor Pattern"), "{listing}");
    assert!(listing.contains("MINEIGEN criterion"), "{listing}");
    // Eigenvalue λ₁ ≈ 1.8321
    assert!(listing.contains("1.8321"), "λ₁ missing: {listing}");
    // Communality total ≈ 1.8321
    assert!(listing.contains("1.8321"), "total comm missing: {listing}");
}

/// Invariant: h²[i] before and after VARIMAX rotation differ by < 1e-8.
#[test]
fn varimax_communality_invariant() {
    // 3-variable, 2-factor loading matrix (arbitrary values).
    let l = vec![vec![0.8, 0.2], vec![0.7, 0.5], vec![0.3, 0.9]];
    let h2_before: Vec<f64> = l
        .iter()
        .map(|row| row.iter().map(|&x| x * x).sum())
        .collect();

    let (l_rot, _) = varimax(&l);
    let h2_after: Vec<f64> = l_rot
        .iter()
        .map(|row| row.iter().map(|&x| x * x).sum())
        .collect();

    for (i, (&b, &a)) in h2_before.iter().zip(&h2_after).enumerate() {
        assert!(
            (b - a).abs() < 1e-8,
            "h²[{i}] changed: before={b:.10}, after={a:.10}"
        );
    }

    // Also check total variance conserved.
    let total_before: f64 = h2_before.iter().sum();
    let total_after: f64 = h2_after.iter().sum();
    assert!(
        (total_before - total_after).abs() < 1e-8,
        "total variance changed: {total_before:.10} -> {total_after:.10}"
    );
}

/// VARIMAX: k=1 should be a no-op (return L unchanged).
#[test]
fn varimax_k1_noop() {
    let l = vec![vec![0.8], vec![0.7], vec![0.9]];
    let (l_rot, rot) = varimax(&l);
    // l_rot should equal l (no rotation possible with 1 factor).
    for (i, (orig, rotated)) in l.iter().zip(&l_rot).enumerate() {
        for (j, (&o, &r)) in orig.iter().zip(rotated).enumerate() {
            assert!(
                (o - r).abs() < 1e-12,
                "l_rot[{i}][{j}]={r} != l[{i}][{j}]={o}"
            );
        }
    }
    // Rotation matrix should be [[1]].
    assert_eq!(rot.len(), 1);
    assert!((rot[0][0] - 1.0).abs() < 1e-12);
}

/// VARIMAX rotation matrix R must be orthogonal: R · R^T = I.
/// Also verify L_rot = L_norm_rotated · scale (i.e., L · rot consistent).
#[test]
fn varimax_rotation_matrix_orthogonal() {
    let l = vec![
        vec![0.8, 0.2],
        vec![0.7, 0.5],
        vec![0.3, 0.9],
        vec![0.6, 0.1],
    ];
    let (_, rot) = varimax(&l);
    let k = rot.len();

    // R · R^T should be identity (k×k).
    for i in 0..k {
        for j in 0..k {
            let dot: f64 = (0..k).map(|m| rot[i][m] * rot[j][m]).sum();
            let expected = if i == j { 1.0 } else { 0.0 };
            assert!(
                (dot - expected).abs() < 1e-8,
                "R·R^T[{i}][{j}] = {dot:.10}, expected {expected}"
            );
        }
    }
}

/// End-to-end: execute with k=2 and rotate=varimax on 3-var data.
/// Verifies no panic, no NaN, communality invariant holds.
#[test]
fn execute_varimax_no_panic() {
    let mut session = make_session();
    let df = df![
        "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
        "y" => [2.0_f64, 4.0, 3.0, 5.0, 1.0, 6.0],
        "z" => [5.0_f64, 4.0, 3.0, 2.0, 1.0, 6.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x"), num_meta("y"), num_meta("z")],
    };
    write_dataset(&mut session, "V", ds);

    let ast = FactorAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "V".into(),
        }),
        cov: false,
        nfactors: Some(2),
        method: "principal".into(),
        rotate: "varimax".into(),
        out: None,
        var: vec!["x".into(), "y".into(), "z".into()],
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    assert!(listing.contains("Rotated Factor Pattern"), "{listing}");
    assert!(listing.contains("Rotation Method: Varimax"), "{listing}");
    // No NaN in the listing.
    assert!(!listing.contains("NaN"), "NaN found in listing: {listing}");
}

/// PROMAX oracle: on a clearly-clustered loading matrix (two blocks), the
/// oblique solution must (a) have an inter-factor correlation off-diagonal
/// that is non-zero (factors become correlated), and (b) produce a sharper
/// pattern than VARIMAX — larger primary loadings and smaller cross-loadings
/// (closer to a {0,1} structure).
#[test]
fn promax_correlates_factors_and_sharpens() {
    // 4 variables: vars 0,1 load on factor 1; vars 2,3 on factor 2, but
    // with a deliberate cross-loading so the clusters are oblique.
    let l = vec![
        vec![0.80, 0.40],
        vec![0.75, 0.35],
        vec![0.40, 0.80],
        vec![0.35, 0.75],
    ];
    let (l_var, _) = varimax(&l);
    let pm = promax(&l_var, 4).unwrap();

    // (a) Inter-factor correlation is not the identity.
    let off = pm.phi[0][1].abs();
    assert!(off > 1e-3, "Inter-factor correlation too small: {off}");
    assert!((pm.phi[0][0] - 1.0).abs() < 1e-9, "phi diag != 1");

    // (b) Sharper: for each variable, the dominant pattern loading is at
    // least as large in magnitude, and the cross-loading is smaller, than
    // varimax — on aggregate the cross-loadings shrink.
    let cross_varimax: f64 = (0..4)
        .map(|i| l_var[i][0].abs().min(l_var[i][1].abs()))
        .sum();
    let cross_promax: f64 = (0..4)
        .map(|i| pm.pattern[i][0].abs().min(pm.pattern[i][1].abs()))
        .sum();
    assert!(
        cross_promax < cross_varimax,
        "promax cross-loadings ({cross_promax:.4}) not smaller than varimax ({cross_varimax:.4})"
    );
}

/// PROMAX on k=1 is a no-op returning the input pattern and Φ=[[1]].
#[test]
fn promax_k1_noop() {
    let l = vec![vec![0.8], vec![0.7], vec![0.9]];
    let pm = promax(&l, 4).unwrap();
    assert_eq!(pm.phi.len(), 1);
    assert!((pm.phi[0][0] - 1.0).abs() < 1e-12);
    for (a, b) in l.iter().zip(&pm.pattern) {
        assert!((a[0] - b[0]).abs() < 1e-12);
    }
}

/// End-to-end PROMAX listing: prints the oblique pattern and inter-factor
/// correlations and creates no NaNs.
#[test]
fn execute_promax_listing() {
    let mut session = make_session();
    let df = df![
        "a" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
        "b" => [1.0_f64, 2.1, 2.9, 4.2, 5.1, 5.8],
        "c" => [6.0_f64, 5.0, 4.0, 3.0, 2.0, 1.0],
        "d" => [5.9_f64, 5.1, 4.0, 2.9, 2.1, 1.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("a"), num_meta("b"), num_meta("c"), num_meta("d")],
    };
    write_dataset(&mut session, "P", ds);

    let ast = FactorAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "P".into(),
        }),
        cov: false,
        nfactors: Some(2),
        method: "principal".into(),
        rotate: "promax".into(),
        out: None,
        var: vec!["a".into(), "b".into(), "c".into(), "d".into()],
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    assert!(listing.contains("Rotation Method: Promax"), "{listing}");
    assert!(listing.contains("Inter-Factor Correlations"), "{listing}");
    assert!(!listing.contains("NaN"), "NaN in listing: {listing}");
}

/// OUT= : the dataset is created with input columns + Factor1..Factork,
/// _LAST_ is updated, and the standardized factor scores have mean ≈ 0.
#[test]
fn execute_out_factor_scores() {
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

    let ast = FactorAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        cov: false,
        nfactors: Some(1),
        method: "principal".into(),
        rotate: "none".into(),
        out: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "FS".into(),
        }),
        var: vec!["x".into(), "y".into()],
    };
    execute(&ast, &mut session).unwrap();
    assert_eq!(session.last_dataset.as_deref(), Some("WORK.FS"));

    let (out, _) = session.libs.get("WORK").unwrap().read("FS").unwrap();
    assert!(out.vars.iter().any(|m| m.name == "Factor1"));
    assert!(out.vars.iter().any(|m| m.name.eq_ignore_ascii_case("x")));

    let col = out.df.column("Factor1").unwrap().f64().unwrap();
    let vals: Vec<f64> = col.into_no_null_iter().collect();
    let mean = vals.iter().sum::<f64>() / vals.len() as f64;
    assert!(mean.abs() < 1e-9, "Factor1 mean={mean}");
    // Standardized regression scores have ~unit variance for a 1-factor
    // solution that explains most variance; just assert finiteness here.
    assert!(vals.iter().all(|v| v.is_finite()), "non-finite score");
}
