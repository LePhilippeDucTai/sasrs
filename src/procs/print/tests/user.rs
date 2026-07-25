use super::super::*;
use super::*;
use crate::dataset::{SasDataset, VarMeta};
use crate::value::VarType;

// ── End-to-end: PROC FORMAT → FORMAT statement → PROC PRINT ──────────────

/// Prove that a user-defined format registered via PROC FORMAT is resolved
/// by PROC PRINT through the session catalog.
///
/// Setup:
///   1. Define `SEXFMT` (1→Male, 2→Female, other→Unknown) in the session
///      format catalog (simulating `proc format; value sexfmt 1='Male' ...`).
///   2. Write a dataset with a `sex` column (format="SEXFMT.") holding
///      values 1, 2, and 3.
///   3. Execute PROC PRINT and check the listing shows "Male", "Female",
///      and "Unknown" (the `other` label).
#[test]
fn user_format_end_to_end_via_session_catalog() {
    use crate::formats::userdef::{Bound, Range, UserFormat};

    let mut session = make_session();

    // 1. Register a user-defined numeric format in the session catalog.
    let uf = UserFormat {
        is_char: false,
        ranges: vec![
            Range {
                from: Bound::Num(1.0),
                to: Bound::Num(1.0),
                from_exclusive: false,
                to_exclusive: false,
                label: "Male".to_string(),
            },
            Range {
                from: Bound::Num(2.0),
                to: Bound::Num(2.0),
                from_exclusive: false,
                to_exclusive: false,
                label: "Female".to_string(),
            },
        ],
        other: Some("Unknown".to_string()),
    };
    session.format_catalog.define("SEXFMT", uf);

    // 2. Write a dataset whose `sex` column has format="SEXFMT."
    let df = df!["sex" => [1.0_f64, 2.0, 3.0]].unwrap();
    let vars = vec![VarMeta {
        name: "sex".to_string(),
        ty: VarType::Num,
        length: 8,
        format: Some("SEXFMT.".to_string()),
        label: None,
    }];
    let ds = SasDataset { df, vars };
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("GENDER", &ds)
        .unwrap();

    // 3. PROC PRINT.
    let ast = PrintAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "GENDER".into(),
        }),
        vars: None,
        noobs: false,
        label: false,
        double: false,
        n: false,
        by: vec![],
        id: vec![],
        sum: vec![],
    };
    execute(&ast, &mut session).unwrap();

    let listing = session.listing.into_string();
    // The user-defined format labels must appear in the listing.
    // This proves the session catalog was used, not FormatCatalog::default().
    assert!(listing.contains("Male"), "listing: {listing}");
    assert!(listing.contains("Female"), "listing: {listing}");
    assert!(listing.contains("Unknown"), "listing: {listing}");
}
