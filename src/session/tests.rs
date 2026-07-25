use super::*;
use crate::output::{HtmlDestination, TextListing};

fn new_session() -> Session {
    let tmp = std::env::temp_dir();
    Session::new(None, tmp, true).expect("session")
}

#[test]
fn default_ods_options_are_sas_defaults() {
    let s = new_session();
    assert!(!s.ods_options.nocenter);
    assert!(s.ods_options.date);
    assert!(s.ods_options.number);
}

#[test]
fn set_ods_option_flags() {
    let mut s = new_session();
    assert!(s.set_ods_option("nocenter"));
    assert!(s.ods_options.nocenter);
    assert!(s.set_ods_option("center"));
    assert!(!s.ods_options.nocenter);

    assert!(s.set_ods_option("nodate"));
    assert!(!s.ods_options.date);
    assert!(s.set_ods_option("date"));
    assert!(s.ods_options.date);

    assert!(s.set_ods_option("nonumber"));
    assert!(!s.ods_options.number);
    assert!(s.set_ods_option("NUMBER"));
    assert!(s.ods_options.number);

    // Option inconnue → false, pas de modification.
    assert!(!s.set_ods_option("frobnicate"));
}

#[test]
fn open_listing_keeps_current() {
    let mut s = new_session();
    // La destination courante par défaut est un listing texte avec LS=96.
    assert_eq!(s.listing.ls(), 96);
    s.open_destination("listing", Box::new(TextListing::new(96)));
    assert_eq!(s.listing.ls(), 96);
    // Aucune destination additionnelle enregistrée.
    assert!(s.output_destinations.is_empty());
}

#[test]
fn open_html_becomes_current() {
    let mut s = new_session();
    s.open_destination("html", Box::new(HtmlDestination::new(96)));
    // HTML est la destination courante : son rendu mémoire est vide (stub),
    // et son nom est mémorisé.
    assert_eq!(s.current_destination, "HTML");
    assert_eq!(s.listing.into_string(), "");
}

#[test]
fn close_html_restores_text_listing() {
    let mut s = new_session();
    s.open_destination("html", Box::new(HtmlDestination::new(96)));
    s.close_destination("html");
    assert_eq!(s.current_destination, "LISTING");
    // Le listing texte par défaut redevient la destination courante :
    // un write_line est rendu verbatim.
    s.listing.write_line("hello");
    assert_eq!(s.listing.into_string(), "hello\n");
}

/// M22.4 — close_destination avec un fichier cible écrit le HTML sur disque.
#[test]
fn close_html_with_file_writes_document() {
    use crate::output::HtmlDestination;

    let tmp_file = std::env::temp_dir().join("sas_test_close_html.html");
    // Nettoyer une éventuelle exécution précédente.
    let _ = std::fs::remove_file(&tmp_file);

    let mut s = new_session();
    s.open_destination(
        "html",
        Box::new(HtmlDestination::with_file(96, tmp_file.clone())),
    );
    // Écrire une table connue via la destination courante.
    s.listing.write_table(
        &["Name".into(), "Score".into()],
        &[crate::output::Align::Left, crate::output::Align::Right],
        &[vec!["Alice".into(), "42".into()]],
    );
    // Fermer → doit écrire le fichier + NOTE + rétablir TextListing.
    s.close_destination("html");

    // La destination est redevenue un TextListing.
    assert_eq!(s.current_destination, "LISTING");
    s.listing.write_line("after");
    assert_eq!(s.listing.into_string(), "after\n");

    // Le fichier HTML existe et contient la table + une valeur connue.
    assert!(tmp_file.exists(), "fichier HTML non créé");
    let content = std::fs::read_to_string(&tmp_file).expect("lecture fichier");
    assert!(content.contains("<table"), "balise <table absente");
    assert!(content.contains("Alice"), "valeur Alice absente");
    assert!(content.contains("42"), "valeur 42 absente");
    assert!(content.contains("<!DOCTYPE html>"), "DOCTYPE manquant");

    // Nettoyer.
    let _ = std::fs::remove_file(&tmp_file);
}

// ── M38.1 : sémantique d'effacement TITLE/FOOTNOTE ─────────────────────────

fn active(levels: &[Option<String>; 9]) -> Vec<String> {
    Session::compact_levels(levels)
}

#[test]
fn title1_clears_levels_above() {
    let mut s = new_session();
    s.set_title_level(1, Some("A".into()));
    s.set_title_level(2, Some("B".into()));
    s.set_title_level(3, Some("C".into()));
    assert_eq!(active(&s.titles), vec!["A", "B", "C"]);
    // TITLE 'X' ≡ TITLE1 'X' : pose niveau 1, efface 2..9.
    s.set_title_level(1, Some("X".into()));
    assert_eq!(active(&s.titles), vec!["X"]);
}

#[test]
fn title3_keeps_below_clears_above() {
    let mut s = new_session();
    s.set_title_level(1, Some("A".into()));
    s.set_title_level(2, Some("B".into()));
    s.set_title_level(3, Some("C".into()));
    s.set_title_level(4, Some("D".into()));
    // TITLE3 'Y' : pose 3, conserve 1-2, efface 4..9.
    s.set_title_level(3, Some("Y".into()));
    assert_eq!(active(&s.titles), vec!["A", "B", "Y"]);
}

#[test]
fn title_empty_clears_everything() {
    let mut s = new_session();
    s.set_title_level(1, Some("A".into()));
    s.set_title_level(2, Some("B".into()));
    // TITLE; ≡ TITLE1; : efface tout.
    s.set_title_level(1, None);
    assert!(active(&s.titles).is_empty());
}

#[test]
fn title_n_empty_clears_n_and_above() {
    let mut s = new_session();
    for (i, t) in ["A", "B", "C", "D"].iter().enumerate() {
        s.set_title_level((i + 1) as u8, Some((*t).into()));
    }
    // TITLE3; efface 3 et 4 (et au-dessus), conserve 1-2.
    s.set_title_level(3, None);
    assert_eq!(active(&s.titles), vec!["A", "B"]);
}

#[test]
fn footnote_multilevel_same_semantics() {
    let mut s = new_session();
    s.set_footnote_level(1, Some("F1".into()));
    s.set_footnote_level(2, Some("F2".into()));
    s.set_footnote_level(3, Some("F3".into()));
    assert_eq!(active(&s.footnotes), vec!["F1", "F2", "F3"]);
    // FOOTNOTE2; efface 2 et 3.
    s.set_footnote_level(2, None);
    assert_eq!(active(&s.footnotes), vec!["F1"]);
    // FOOTNOTE 'X' efface tout au-dessus.
    s.set_footnote_level(1, Some("only".into()));
    assert_eq!(active(&s.footnotes), vec!["only"]);
}

#[test]
fn titles_pushed_to_current_destination() {
    let mut s = new_session();
    s.set_title_level(1, Some("Hello".into()));
    s.set_title_level(2, Some("World".into()));
    s.listing.page_header();
    let out = s.listing.into_string();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0].trim(), "Hello");
    assert_eq!(lines[1].trim(), "World");
}
