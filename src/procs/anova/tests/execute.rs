use super::super::*;
use super::*;
use crate::ast::DatasetRef;
use crate::dataset::SasDataset;
use polars::df;

// ── Test 4: execute listing checks ───────────────────────────────────

#[test]
fn test_execute_listing() {
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

    let ast = AnovaAst {
        data_options: AnovaDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
        },
        class_vars: vec!["sex".into()],
        model: Some(AnovaModel {
            dependents: vec!["height".into()],
            effects: vec!["sex".into()],
            terms: vec![vec!["sex".into()]],
            noprint: false,
        }),
        means_vars: vec![],
    };

    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();

    assert!(listing.contains("The ANOVA Procedure"), "{listing}");
    assert!(listing.contains("Class Level Information"), "{listing}");
    assert!(listing.contains("Dependent Variable"), "{listing}");
    assert!(listing.contains("Corrected Total"), "{listing}");
    assert!(listing.contains("Type I SS"), "{listing}");
    assert!(listing.contains("Type III SS"), "{listing}");
}

// ── Test 5: execute means section ────────────────────────────────────

#[test]
fn test_execute_means() {
    let mut session = make_session();
    let frame = df![
        "sex"    => ["F","F","F","M","M","M"],
        "weight" => [100.0_f64, 110.0, 120.0, 150.0, 160.0, 170.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![char_meta("sex"), num_meta("weight")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let ast = AnovaAst {
        data_options: AnovaDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
        },
        class_vars: vec!["sex".into()],
        model: Some(AnovaModel {
            dependents: vec!["weight".into()],
            effects: vec!["sex".into()],
            terms: vec![vec!["sex".into()]],
            noprint: false,
        }),
        means_vars: vec!["sex".into()],
    };

    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();

    // Should show class level in the means table
    assert!(
        listing.contains("Level of sex") || listing.contains("sex"),
        "{listing}"
    );
    assert!(listing.contains('F'), "{listing}");
    assert!(listing.contains('M'), "{listing}");
}

// ── Test 9: end-to-end multiway execute path ──────────────────────────

#[test]
fn test_execute_multiway_listing() {
    let mut session = make_session();
    // 2x2 design, two CLASS vars + interaction.
    let frame = df![
        "a" => ["L","L","L","L","H","H","H","H"],
        "b" => ["X","X","Y","Y","X","X","Y","Y"],
        "y" => [10.0_f64, 12.0, 20.0, 22.0, 30.0, 28.0, 44.0, 46.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![char_meta("a"), char_meta("b"), num_meta("y")],
    };
    session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

    let ast = AnovaAst {
        data_options: AnovaDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
        },
        class_vars: vec!["a".into(), "b".into()],
        model: Some(AnovaModel {
            dependents: vec!["y".into()],
            effects: vec!["a".into(), "b".into(), "a*b".into()],
            terms: vec![
                vec!["a".into()],
                vec!["b".into()],
                vec!["a".into(), "b".into()],
            ],
            noprint: false,
        }),
        means_vars: vec!["a".into()],
    };

    execute(&ast, &mut session).unwrap();
    let listing = session.listing.into_string();

    assert!(listing.contains("The ANOVA Procedure"), "{listing}");
    assert!(listing.contains("Class Level Information"), "{listing}");
    assert!(listing.contains("Dependent Variable: y"), "{listing}");
    assert!(listing.contains("Type I SS"), "{listing}");
    assert!(listing.contains("Type III SS"), "{listing}");
    // Interaction term label uses `*` join.
    assert!(listing.contains("a*b"), "{listing}");
    // MEANS main-effect table present.
    assert!(listing.contains("Level of a"), "{listing}");
}
