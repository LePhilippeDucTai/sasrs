use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::session::Session;
use crate::source::SourceFile;
use crate::testkit::*;
use crate::value::VarType;
use polars::df;

fn parse_discrim(src: &str) -> Result<DiscrimAst> {
    let source = SourceFile::new(src);
    let mut ts = StatementStream::new(&source).unwrap();
    ts.next(); // proc
    ts.next(); // discrim
    parse(&mut ts)
}

fn make_oracle_session() -> (Session, DiscrimAst) {
    let session = make_session();
    let frame = df![
        "class" => ["A", "A", "A", "B", "B", "B"],
        "x" => [1.0_f64, 2.0, 3.0, 5.0, 6.0, 7.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![
            VarMeta {
                name: "class".into(),
                ty: VarType::Char,
                length: 1,
                format: None,
                label: None,
            },
            VarMeta {
                name: "x".into(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            },
        ],
    };
    session.libs.get("WORK").unwrap().write("LDA", &ds).unwrap();
    let ast = DiscrimAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "LDA".into(),
        }),
        out: None,
        outstat: None,
        method: None,
        pool: Pool::Yes,
        priors: Priors::Equal,
        noclassify: false,
        crossvalidate: false,
        short: false,
        class_var: Some("class".into()),
        var_vars: vec!["x".into()],
        id_var: None,
    };
    (session, ast)
}

fn fit_oracle() -> LdaModel {
    let classes = vec![Value::Char("A".into()), Value::Char("B".into())];
    let class_obs = vec![
        vec![vec![1.0], vec![2.0], vec![3.0]],
        vec![vec![5.0], vec![6.0], vec![7.0]],
    ];
    fit_lda(classes, &class_obs, &Priors::Equal, 1).unwrap()
}

// ── parse tests ──

#[test]
fn test_parse_basic() {
    let ast = parse_discrim("proc discrim; class g; var x y; run;").unwrap();
    assert_eq!(ast.class_var, Some("g".to_string()));
    assert_eq!(ast.var_vars, vec!["x", "y"]);
    assert_eq!(ast.priors, Priors::Equal);
    assert_eq!(ast.pool, Pool::Yes);
}

#[test]
fn test_parse_options() {
    let ast = parse_discrim(
        "proc discrim data=a out=b method=normal pool=no noclassify short; class g; var x; id name; priors proportional; run;",
    )
    .unwrap();
    assert_eq!(ast.data.as_ref().unwrap().name, "a");
    assert_eq!(ast.out.as_ref().unwrap().name, "b");
    assert_eq!(ast.method.as_deref(), Some("NORMAL"));
    assert_eq!(ast.pool, Pool::No);
    assert_eq!(ast.priors, Priors::Proportional);
    assert!(ast.noclassify);
    assert!(ast.short);
    assert_eq!(ast.id_var, Some("name".to_string()));
}

// ── invariant tests ──

#[test]
fn test_pooled_cov_and_inverse() {
    let m = fit_oracle();
    // Σ_pooled = 1.0, inverse = 1.0
    assert!((m.pooled[0][0] - 1.0).abs() < 1e-12);
    assert!((m.pooled_inv[0][0] - 1.0).abs() < 1e-12);
}

#[test]
fn test_constants_bake_in_prior() {
    let m = fit_oracle();
    // Constant_A = -2.0 + ln(0.5) = -2.6931
    assert!(
        (m.constants[0] - (-2.6931)).abs() < 1e-3,
        "got {}",
        m.constants[0]
    );
    // Constant_B = -18.0 + ln(0.5) = -18.6931
    assert!(
        (m.constants[1] - (-18.6931)).abs() < 1e-3,
        "got {}",
        m.constants[1]
    );
    // coefficients
    assert!((m.coefs[0][0] - 2.0).abs() < 1e-12);
    assert!((m.coefs[1][0] - 6.0).abs() < 1e-12);
}

#[test]
fn test_decision_boundary_at_4() {
    let m = fit_oracle();
    // Score_A(4) == Score_B(4) at the boundary.
    let sa = m.score(0, &[4.0]);
    let sb = m.score(1, &[4.0]);
    assert!((sa - sb).abs() < 1e-8, "sa={sa} sb={sb}");
}

#[test]
fn test_group_distance() {
    let m = fit_oracle();
    // D²(A,B) = 16.0
    assert!((m.group_distance(0, 1) - 16.0).abs() < 1e-8);
    assert!((m.group_distance(1, 0) - 16.0).abs() < 1e-8);
    assert!(m.group_distance(0, 0).abs() < 1e-8);
}

#[test]
fn test_posteriors_sum_to_one() {
    let m = fit_oracle();
    for x in [1.0, 2.0, 3.0, 5.0, 6.0, 7.0] {
        let post = m.posteriors(&[x]);
        let s: f64 = post.iter().sum();
        assert!((s - 1.0).abs() < 1e-8, "sum={s} for x={x}");
    }
}

#[test]
fn test_classification_all_correct() {
    let m = fit_oracle();
    for x in [1.0, 2.0, 3.0] {
        assert_eq!(m.classify(&[x]), 0, "x={x} should be A");
    }
    for x in [5.0, 6.0, 7.0] {
        assert_eq!(m.classify(&[x]), 1, "x={x} should be B");
    }
}

#[test]
fn test_posterior_x3() {
    let m = fit_oracle();
    let post = m.posteriors(&[3.0]);
    // P_A ≈ 0.9820
    assert!((post[0] - 0.9820).abs() < 1e-3, "P_A={}", post[0]);
    assert!((post[1] - 0.0180).abs() < 1e-3, "P_B={}", post[1]);
}

// ── execute / listing tests ──

#[test]
fn test_execute_oracle_listing() {
    let (mut session, ast) = make_oracle_session();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(listing.contains("-2.6931"), "Constant_A missing: {listing}");
    assert!(
        listing.contains("-18.6931"),
        "Constant_B missing: {listing}"
    );
    assert!(listing.contains("16.0000"), "D²(A,B) missing: {listing}");
    assert!(
        listing.contains("The DISCRIMINANT Procedure"),
        "title missing"
    );
}

#[test]
fn test_out_dataset() {
    let (mut session, mut ast) = make_oracle_session();
    ast.out = Some(DatasetRef {
        libref: Some("WORK".into()),
        name: "RESULT".into(),
    });
    execute(&ast, &mut session).unwrap();
    let (out, _) = session.libs.get("WORK").unwrap().read("RESULT").unwrap();
    assert!(out.vars.iter().any(|v| v.name == "_FROM_"));
    assert!(out.vars.iter().any(|v| v.name == "_INTO_"));
    assert!(out.vars.iter().any(|v| v.name == "_A"));
    assert!(out.vars.iter().any(|v| v.name == "_B"));
    // All 6 rows classified correctly: _FROM_ == _INTO_.
    let from = out.df.column("_FROM_").unwrap().str().unwrap();
    let into = out.df.column("_INTO_").unwrap().str().unwrap();
    for i in 0..6 {
        assert_eq!(from.get(i), into.get(i), "row {i} misclassified");
    }
}

#[test]
fn test_proportional_priors() {
    // Unequal group sizes change the prior term in the constant.
    let classes = vec![Value::Char("A".into()), Value::Char("B".into())];
    let class_obs = vec![
        vec![vec![1.0], vec![2.0], vec![3.0], vec![4.0]],
        vec![vec![6.0], vec![7.0]],
    ];
    let m = fit_lda(classes, &class_obs, &Priors::Proportional, 1).unwrap();
    // priors = 4/6, 2/6
    assert!((m.priors[0] - 4.0 / 6.0).abs() < 1e-12);
    assert!((m.priors[1] - 2.0 / 6.0).abs() < 1e-12);
}
