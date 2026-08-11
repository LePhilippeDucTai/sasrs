//! M38.4 — ODS SELECT/EXCLUDE par section d'UNIVARIATE : chaque section porte
//! son nom d'objet ODS SAS (Moments, BasicMeasures, TestsForNormality,
//! Quantiles, ExtremeObs, MissingValues) et n'est émise que si la liste de
//! sélection la retient. Les captures ODS OUTPUT restent indépendantes de
//! l'affichage.

use super::super::*;
use super::*;
use crate::ast::DatasetRef;
use crate::dataset::SasDataset;
use polars::df;

fn x_session() -> Session {
    let mut session = make_session();
    let df = df!["x" => [Some(1.0_f64), Some(2.0), Some(3.0), Some(4.0), Some(5.0), None]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };
    write_dataset(&mut session, "T", ds);
    session
}

fn x_ast() -> UnivariateAst {
    UnivariateAst {
        data: Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "T".into(),
        }),
        var: vec!["x".into()],
        by: vec![],
        weight: None,
        output: None,
        normal: false,
        plots: vec![],
    }
}

/// `ods select Moments;` : seule la section Moments s'affiche — l'en-tête du
/// proc et « Variable: x » restent (au moins un objet s'affiche), les autres
/// sections disparaissent.
#[test]
fn select_moments_keeps_only_the_moments_section() {
    let mut session = x_session();
    session.set_ods_selection(false, &["Moments".to_string()]);
    execute(&x_ast(), &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(
        listing.contains("The UNIVARIATE Procedure"),
        "listing: {listing}"
    );
    assert!(listing.contains("Variable: x"), "listing: {listing}");
    assert!(listing.contains("Moments"), "listing: {listing}");
    assert!(listing.contains("Std Error Mean"), "listing: {listing}");
    for gone in [
        "Basic Statistical Measures",
        "Quantiles (Definition 5)",
        "Extreme Observations",
        "Missing Values",
    ] {
        assert!(!listing.contains(gone), "'{gone}' présent: {listing}");
    }
}

/// `ods exclude Moments Quantiles;` : les sections nommées disparaissent, les
/// autres restent.
#[test]
fn exclude_named_sections_drops_them_only() {
    let mut session = x_session();
    session.set_ods_selection(true, &["Moments".to_string(), "Quantiles".to_string()]);
    execute(&x_ast(), &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(
        listing.contains("Basic Statistical Measures"),
        "listing: {listing}"
    );
    assert!(
        listing.contains("Extreme Observations"),
        "listing: {listing}"
    );
    assert!(listing.contains("Missing Values"), "listing: {listing}");
    assert!(
        !listing.contains("Quantiles (Definition 5)"),
        "listing: {listing}"
    );
    assert!(!listing.contains("Std Error Mean"), "listing: {listing}");
}

/// Liste nominative qui ne retient AUCUNE section du proc : aucune sortie du
/// tout (pas d'en-tête « The UNIVARIATE Procedure », pas de page vide).
#[test]
fn select_foreign_object_suppresses_the_whole_listing() {
    let mut session = x_session();
    session.set_ods_selection(false, &["OneWayFreqs".to_string()]);
    execute(&x_ast(), &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(listing.is_empty(), "listing should be empty: {listing}");
}

/// Interaction ODS OUTPUT : `ods select Quantiles;` n'empêche pas la capture
/// de la table Moments (exclue de l'affichage) — la capture n'est pas un
/// affichage (doc SAS).
#[test]
fn capture_still_works_for_sections_hidden_by_the_selection() {
    let mut session = x_session();
    session.set_ods_selection(false, &["Quantiles".to_string()]);
    session.set_ods_output(&[(
        "Moments".to_string(),
        DatasetRef {
            libref: None,
            name: "m".to_string(),
        },
    )]);
    execute(&x_ast(), &mut session).unwrap();
    session.flush_ods_output().unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("M").unwrap();
    assert_eq!(out.n_obs(), 6, "6 lignes de la table Moments");
    let listing = session.listing.take_string();
    assert!(!listing.contains("Std Error Mean"), "listing: {listing}");
    assert!(
        listing.contains("Quantiles (Definition 5)"),
        "listing: {listing}"
    );
}
