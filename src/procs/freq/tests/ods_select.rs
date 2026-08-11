//! M38.4 — ODS SELECT/EXCLUDE consulté au point d'émission de la table
//! « OneWayFreqs » : la table exclue disparaît du listing (en-tête de proc
//! compris quand plus rien ne s'affiche), tandis que la capture ODS OUTPUT et
//! le OUT= continuent d'être produits (l'exclusion ne gouverne que
//! l'affichage — doc SAS).

use super::*;

fn class_sex_session() -> Session {
    let mut session = make_session();
    let df = df!["sex" => ["M", "F", "F", "M", "M"]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("sex", 8)],
    };
    write_dataset(&mut session, "T", ds);
    session
}

fn t_ref() -> DatasetRef {
    DatasetRef {
        libref: Some("WORK".into()),
        name: "T".into(),
    }
}

/// `ods exclude OneWayFreqs;` + FREQ une voie : plus AUCUNE sortie listing —
/// pas même l'en-tête « The FREQ Procedure » (SAS ne produit pas de page
/// vide). Le log du proc n'est pas concerné (vérifié via l'absence d'erreur).
#[test]
fn exclude_one_way_freqs_suppresses_table_and_header() {
    let mut session = class_sex_session();
    session.set_ods_selection(true, &["OneWayFreqs".to_string()]);

    execute(
        &fast(t_ref(), vec![tr(&["sex"], false, None)]),
        &mut session,
    )
    .unwrap();

    let listing = session.listing.take_string();
    assert!(listing.is_empty(), "listing should be empty: {listing}");
}

/// `ods select OneWayFreqs;` : la table sélectionnée s'affiche normalement
/// (sortie identique au chemin sans liste de sélection).
#[test]
fn select_one_way_freqs_keeps_the_table() {
    let mut base = class_sex_session();
    execute(&fast(t_ref(), vec![tr(&["sex"], false, None)]), &mut base).unwrap();
    let expected = base.listing.take_string();

    let mut session = class_sex_session();
    session.set_ods_selection(false, &["onewayfreqs".to_string()]);
    execute(
        &fast(t_ref(), vec![tr(&["sex"], false, None)]),
        &mut session,
    )
    .unwrap();

    assert_eq!(session.listing.take_string(), expected);
}

/// Interaction ODS OUTPUT (doc SAS) : une table EXCLUE du listing est quand
/// même capturée par `ods output OneWayFreqs=f;` — la capture n'est pas un
/// affichage.
#[test]
fn excluded_table_is_still_captured_by_ods_output() {
    let mut session = class_sex_session();
    session.set_ods_selection(true, &["OneWayFreqs".to_string()]);
    session.set_ods_output(&[(
        "OneWayFreqs".to_string(),
        DatasetRef {
            libref: None,
            name: "f".to_string(),
        },
    )]);

    execute(
        &fast(t_ref(), vec![tr(&["sex"], false, None)]),
        &mut session,
    )
    .unwrap();
    session.flush_ods_output().unwrap();

    let (out, _) = session.libs.get("WORK").unwrap().read("F").unwrap();
    assert_eq!(out.n_obs(), 2, "une observation par catégorie");
    assert!(
        session.listing.take_string().is_empty(),
        "le listing reste vide malgré la capture"
    );
}

/// Le bloc CHISQ une voie porte son propre nom d'objet (« OneWayChiSq ») :
/// `ods select OneWayChiSq;` garde le test mais supprime la table de
/// fréquences (l'en-tête du proc reste, car un objet s'affiche).
#[test]
fn select_one_way_chisq_only_keeps_the_chisq_block() {
    let mut session = class_sex_session();
    session.set_ods_selection(false, &["OneWayChiSq".to_string()]);

    let mut req = tr(&["sex"], false, None);
    req.chisq = true;
    execute(&fast(t_ref(), vec![req]), &mut session).unwrap();

    let listing = session.listing.take_string();
    assert!(listing.contains("The FREQ Procedure"), "listing: {listing}");
    assert!(
        listing.contains("Chi-Square Test for Equal Proportions"),
        "listing: {listing}"
    );
    assert!(
        !listing.contains("Cumulative"),
        "la table de fréquences doit être supprimée: {listing}"
    );
}
