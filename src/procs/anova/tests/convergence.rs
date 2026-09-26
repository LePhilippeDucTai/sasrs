use super::*;
use crate::ast::DatasetRef;
use crate::dataset::SasDataset;
use crate::testkit::*;
use polars::df;

// ── J02-P6 : convergence_* — plan singulier ⇒ ERROR explicite ──────────
//
// Référence : SAS/STAT 9.4 User's Guide, The ANOVA Procedure, Details
// (construction du plan ; SAS gère le rang incomplet via l'inverse
// généralisée documentée dans The GLM Procedure, Parameterization of PROC
// GLM Models). Tant qu'aucune inverse généralisée n'est implémentée ici, un
// plan singulier (X'X non inversible) doit produire une ERROR explicite —
// pas des SS NaN silencieux.
// https://support.sas.com/documentation/cdl/en/statug/68162/HTML/default/statug_anova_details_toc.htm

/// CLASS variable `b` identical to `a`: main-effect dummies are collinear,
/// so the two-way design is rank deficient.
fn singular_anova_session() -> (crate::session::Session, AnovaAst) {
    let session = make_session();
    let frame = df![
        "y" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
        "a" => ["x", "x", "x", "y", "y", "y"],
        "b" => ["x", "x", "x", "y", "y", "y"]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), char_meta("a", 1), char_meta("b", 1)],
    };
    session.libs.get("WORK").unwrap().write("SGA", &ds).unwrap();
    let ast = AnovaAst {
        data_options: AnovaDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "SGA".into(),
            }),
        },
        class_vars: vec!["a".into(), "b".into()],
        model: Some(AnovaModel {
            dependents: vec!["y".into()],
            effects: vec!["a".into(), "b".into()],
            terms: vec![vec!["a".into()], vec!["b".into()]],
            noprint: false,
        }),
        means_vars: vec![],
    };
    (session, ast)
}

#[test]
fn convergence_anova_singular_design_is_explicit_error() {
    let (mut session, ast) = singular_anova_session();
    let err = execute(&ast, &mut session).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("rank deficient") && msg.contains("generalized inverse"),
        "err: {msg}"
    );
    let listing = session.listing.take_string();
    assert!(
        !listing.contains("NaN"),
        "no NaN SS may be printed on a singular design:\n{listing}"
    );
}

#[test]
fn convergence_anova_full_rank_fit_still_computes() {
    // Non-regression: one-way balanced design keeps fitting.
    let mut session = make_session();
    let frame = df![
        "y" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
        "a" => ["x", "x", "x", "y", "y", "y"]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), char_meta("a", 1)],
    };
    session.libs.get("WORK").unwrap().write("OKA", &ds).unwrap();
    let ast = AnovaAst {
        data_options: AnovaDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "OKA".into(),
            }),
        },
        class_vars: vec!["a".into()],
        model: Some(AnovaModel {
            dependents: vec!["y".into()],
            effects: vec!["a".into()],
            terms: vec![vec!["a".into()]],
            noprint: false,
        }),
        means_vars: vec![],
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(listing.contains("The ANOVA Procedure"), "{listing}");
    assert!(!listing.contains("NaN"), "{listing}");
}
