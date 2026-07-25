use super::super::*;
use super::*;
use crate::dataset::SasDataset;
use polars::df;

/// 2-level CLASS predictor must equal manual 0/1 dummy coding of the same
/// predictor (reference = last level → "b", design column flags level "a").
#[test]
fn test_class_two_level_equals_manual_dummy() {
    // CLASS version: group ∈ {a,a,a,b,b,b}, y Poisson counts.
    let session = make_session();
    let frame_c = df![
        "y" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
        "g" => ["a", "a", "a", "b", "b", "b"]
    ]
    .unwrap();
    let ds_c = SasDataset {
        df: frame_c,
        vars: vec![num_meta("y"), char_meta("g")],
    };
    session.libs.get("WORK").unwrap().write("CLS", &ds_c).unwrap();
    let ast_c = GenmodAst {
        data_options: GenmodDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "CLS".into(),
            }),
        },
        class_vars: vec!["g".into()],
        model: Some(GenmodModel {
            response: "y".into(),
            event: None,
            descending: false,
            predictors: vec!["g".into()],
            dist: Distribution::Poisson,
            link: LinkFunction::Log,
            noprint: false,
            scale: None,
            noscale: false,
        }),
        freq_var: None,
    };
    let mut s_c = session;
    execute(&ast_c, &mut s_c).unwrap();
    let listing_c = s_c.listing.into_string();

    // Manual dummy: d = 1 if g=="a" else 0 (ref = last level "b").
    let session2 = make_session();
    let frame_d = df![
        "y" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
        "d" => [1.0_f64, 1.0, 1.0, 0.0, 0.0, 0.0]
    ]
    .unwrap();
    let ds_d = SasDataset {
        df: frame_d,
        vars: vec![num_meta("y"), num_meta("d")],
    };
    session2.libs.get("WORK").unwrap().write("DUM", &ds_d).unwrap();
    let ast_d = GenmodAst {
        data_options: GenmodDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "DUM".into(),
            }),
        },
        class_vars: vec![],
        model: Some(GenmodModel {
            response: "y".into(),
            event: None,
            descending: false,
            predictors: vec!["d".into()],
            dist: Distribution::Poisson,
            link: LinkFunction::Log,
            noprint: false,
            scale: None,
            noscale: false,
        }),
        freq_var: None,
    };
    let mut s_d = session2;
    execute(&ast_d, &mut s_d).unwrap();
    let listing_d = s_d.listing.into_string();

    // Both fit β̂ for the "a vs b" contrast = ln(2) − ln(5) = ln(0.4).
    let contrast = (2.0_f64).ln() - (5.0_f64).ln();
    let contrast_str = format!("{contrast:.4}");
    assert!(
        listing_c.contains(&contrast_str),
        "CLASS contrast {contrast_str} missing:\n{listing_c}"
    );
    assert!(
        listing_d.contains(&contrast_str),
        "manual-dummy contrast {contrast_str} missing:\n{listing_d}"
    );
    // The Class Level Information table must appear for the CLASS run.
    assert!(listing_c.contains("Class Level Information"));
    // Reference level row "g b" with DF 0 must be present.
    assert!(
        listing_c.contains("g b"),
        "reference-level row 'g b' missing:\n{listing_c}"
    );
}

/// Design-matrix dimensionality: a 3-level CLASS contributes L−1=2 columns,
/// plus a continuous predictor and the intercept ⇒ p = 4.
#[test]
fn test_design_matrix_dimensions() {
    let three = DesignTerm::Class {
        name: "g".into(),
        col: 0,
        levels: vec![
            Value::Char("a".into()),
            Value::Char("b".into()),
            Value::Char("c".into()),
        ],
    };
    assert_eq!(three.n_cols(), 2);
    let cont = DesignTerm::Continuous {
        name: "x".into(),
        col: 1,
    };
    assert_eq!(cont.n_cols(), 1);
    // intercept + 2 (class) + 1 (continuous) = 4 parameters.
    let p_param = 1 + three.n_cols() + cont.n_cols();
    assert_eq!(p_param, 4);
}

#[test]
fn test_scale_fixed_normal_noscale() {
    // NOSCALE on Normal fixes σ at 1 ⇒ Scale row = 1.0000, DF 0.
    let (mut session, mut ast) = make_normal_session();
    ast.model.as_mut().unwrap().noscale = true;
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();
    let scale_line = listing
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .find(|toks| toks.first() == Some(&"Scale"))
        .expect("Scale row");
    // toks: ["Scale", DF, Estimate, ...]
    assert_eq!(scale_line[1], "0", "NOSCALE ⇒ DF 0: {scale_line:?}");
    assert_eq!(scale_line[2], "1.0000", "NOSCALE ⇒ Scale 1: {scale_line:?}");
}
