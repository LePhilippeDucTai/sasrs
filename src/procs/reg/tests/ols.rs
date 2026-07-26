use super::super::*;
use super::*;
use polars::df;

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
    assert!(
        listing.contains("0.8000") || listing.contains("R-Square"),
        "{listing}"
    );
}
