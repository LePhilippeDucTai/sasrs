use super::*;
use crate::ast::DatasetRef;
use crate::dataset::SasDataset;
use polars::df;

// ── J02-P6 : convergence_* — convergence véridique ─────────────────────
//
// Référence : SAS/STAT 9.4 User's Guide, The GENMOD Procedure, Details:
// GENMOD Procedure — « Iteration History » / convergence status. Quand la
// convergence n'est pas atteinte, SAS émet un WARNING (« Convergence was not
// attained in 50 iterations. »), jamais une NOTE ni un silence, et le listing
// n'affiche PAS « Convergence criterion (GCONV=1E-8) satisfied. ».
// https://support.sas.com/documentation/cdl/en/statug/68162/HTML/default/statug_genmod_details_toc.htm

/// Session GENMOD binomial-logit avec séparation complète de l'échantillon :
/// y=0 pour x petit, y=1 pour x grand — le MLE n'existe pas (β → ∞).
fn separated_session() -> (Session, GenmodAst) {
    let session = make_session();
    let frame = df![
        "y" => [0.0_f64, 0.0, 0.0, 1.0, 1.0, 1.0],
        "x" => [1.0_f64, 2.0, 3.0, 9.0, 10.0, 11.0]
    ]
    .unwrap();
    let ds = SasDataset {
        df: frame,
        vars: vec![num_meta("y"), num_meta("x")],
    };
    session.libs.get("WORK").unwrap().write("SEP", &ds).unwrap();
    let ast = GenmodAst {
        data_options: GenmodDataOptions {
            input: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "SEP".into(),
            }),
        },
        class_vars: vec![],
        model: Some(GenmodModel {
            response: "y".into(),
            event: None,
            descending: false,
            predictors: vec!["x".into()],
            dist: Distribution::Binomial,
            link: LinkFunction::Logit,
            noprint: false,
            scale: None,
            noscale: false,
        }),
        freq_var: None,
    };
    (session, ast)
}

/// Séparation complète ⇒ ERROR propre + WARNING SAS dans le log, et le
/// listing n'affiche jamais « Convergence criterion … satisfied ».
#[test]
fn convergence_genmod_separation_is_warning_not_note() {
    let (mut session, ast) = separated_session();
    let res = execute(&ast, &mut session);
    assert!(res.is_err(), "separation must stop PROC GENMOD cleanly");
    let log = session.log.into_string();
    assert!(
        log.contains("WARNING") && log.contains("Convergence was not attained"),
        "SAS WARNING missing from log:\n{log}"
    );
    assert!(
        log.contains("maximum likelihood estimate may not exist"),
        "doc-quoted MLE warning missing:\n{log}"
    );
    assert!(
        !log.contains("NOTE: PROC GENMOD failed to converge"),
        "non-convergence must not be a NOTE anymore:\n{log}"
    );
    let listing = session.listing.take_string();
    assert!(
        !listing.contains("Convergence criterion (GCONV=1E-8) satisfied."),
        "listing must not claim convergence on failure:\n{listing}"
    );
}

/// Non-régression : un ajustement qui converge affiche bien la ligne
/// « Convergence criterion (GCONV=1E-8) satisfied. » après l'ajustement.
#[test]
fn convergence_genmod_converged_fit_still_claims_satisfied() {
    let (mut session, ast) = make_poisson_session();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(
        listing.contains("Convergence criterion (GCONV=1E-8) satisfied."),
        "converged fit must still print the status:\n{listing}"
    );
    // The status line keeps its historical listing position (after the
    // Response Profile block, before the goodness-of-fit table).
    let pos_status = listing
        .find("Convergence criterion (GCONV=1E-8) satisfied.")
        .unwrap();
    let pos_gof = listing
        .find("Criteria For Assessing Goodness Of Fit")
        .expect("GOF header");
    assert!(
        pos_status < pos_gof,
        "status must precede the GOF table:\n{listing}"
    );
}
