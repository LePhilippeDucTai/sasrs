use super::*;
use crate::ast::DatasetRef;
use crate::dataset::SasDataset;
use crate::testkit::*;
use polars::df;

// ── J02-P6 : convergence_* — plan singulier ⇒ ERROR explicite ──────────
//
// Référence : SAS/STAT 9.4 User's Guide, The GLM Procedure, Details:
// Parameterization of PROC GLM Models. SAS paramètre les modèles de rang
// incomplet par inverse généralisée (g2/g3) ; tant qu'aucune inverse
// généralisée n'est implémentée ici, un plan singulier (X'X non inversible)
// doit produire une ERROR explicite — pas des SSE/SE NaN silencieux.
// https://support.sas.com/documentation/cdl/en/statug/68162/HTML/default/statug_glm_details_toc.htm

/// Dataset where CLASS variable `b` is an exact copy of `a`: the main-effect
/// dummy columns of b are collinear with those of a, so X'X is singular.
fn singular_session() -> (crate::session::Session, GlmAst) {
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
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("SING", &ds)
        .unwrap();
    let ast = GlmAst {
        data_options: GlmDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "SING".into(),
            }),
        },
        class_vars: vec!["a".into(), "b".into()],
        model: Some(GlmModel {
            dependents: vec!["y".into()],
            effects: vec!["a".into(), "b".into()],
            effect_terms: vec![vec!["a".into()], vec!["b".into()]],
            solution: false,
            noprint: false,
        }),
        lsmeans_vars: vec![],
        estimates: vec![],
        contrasts: vec![],
        means_vars: vec![],
    };
    (session, ast)
}

#[test]
fn convergence_glm_singular_design_is_explicit_error() {
    let (mut session, ast) = singular_session();
    let err = execute(&ast, &mut session).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("rank deficient") && msg.contains("generalized inverse"),
        "err: {msg}"
    );
    // No silent NaN output: the listing must not contain a half-computed
    // ANOVA table (the proc stops before reporting).
    let listing = session.listing.take_string();
    assert!(
        !listing.contains("NaN"),
        "no NaN statistic may be printed on a singular fit:\n{listing}"
    );
}

#[test]
fn convergence_glm_full_rank_fit_still_computes() {
    // Non-regression: the same data with only factor `a` fits normally.
    let session = make_session();
    let frame = df![
        "y" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
        "a" => ["x", "x", "x", "y", "y", "y"]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), char_meta("a", 1)],
    };
    let mut session = session;
    session.libs.get("WORK").unwrap().write("OKG", &ds).unwrap();
    let ast = GlmAst {
        data_options: GlmDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "OKG".into(),
            }),
        },
        class_vars: vec!["a".into()],
        model: Some(GlmModel {
            dependents: vec!["y".into()],
            effects: vec!["a".into()],
            effect_terms: vec![vec!["a".into()]],
            solution: false,
            noprint: false,
        }),
        lsmeans_vars: vec![],
        estimates: vec![],
        contrasts: vec![],
        means_vars: vec![],
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(listing.contains("The GLM Procedure"), "{listing}");
    assert!(!listing.contains("NaN"), "{listing}");
}
