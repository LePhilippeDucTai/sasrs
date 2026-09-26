use super::*;

#[test]
fn table_layout() {
    let mut l = ListingWriter::new(40);
    l.page_header();
    l.write_table(
        &["Obs".into(), "x".into()],
        &[Align::Right, Align::Right],
        &[
            vec!["1".into(), "10".into()],
            vec!["2".into(), "200".into()],
        ],
    );
    let s = l.into_string();
    assert!(s.contains("The SAS System"));
    assert!(s.contains("Obs      x"));
    assert!(s.contains("  1     10"));
    assert!(s.contains("  2    200"));
}

/// Byte-identity guard: a single title plus default rendering is unchanged.
#[test]
fn single_title_byte_identical() {
    let mut l = ListingWriter::new(40);
    l.page.titles = vec!["My Report".into()];
    l.page_header();
    // pad = (40 - 9) / 2 = 15 spaces, then text, then blank line.
    assert_eq!(l.into_string(), format!("{}My Report\n\n", " ".repeat(15)));
}

/// Three titles render centered, in level order, with one trailing blank.
#[test]
fn three_titles_centered_in_order() {
    let mut l = ListingWriter::new(20);
    l.page.titles = vec!["A".into(), "BB".into(), "CCC".into()];
    l.page_header();
    let s = l.into_string();
    let lines: Vec<&str> = s.lines().collect();
    // Title order preserved.
    assert_eq!(lines[0].trim(), "A");
    assert_eq!(lines[1].trim(), "BB");
    assert_eq!(lines[2].trim(), "CCC");
    // Centering: pad = (20 - len) / 2.
    assert_eq!(lines[0], format!("{}A", " ".repeat((20 - 1) / 2)));
    assert_eq!(lines[2], format!("{}CCC", " ".repeat((20 - 3) / 2)));
    // Exactly one trailing blank line after all titles (line index 3).
    assert_eq!(lines[3], "");
    assert_eq!(lines.len(), 4);
}

/// Footnotes render centered at the bottom on drain.
#[test]
fn footnotes_centered_on_drain() {
    let mut l = ListingWriter::new(20);
    l.page.footnotes = vec!["Note1".into(), "Note2".into()];
    l.page_header();
    l.write_line("body");
    let s = l.into_string();
    let lines: Vec<&str> = s.lines().collect();
    // Footnotes appear after the body, centered, preceded by a blank.
    let f1 = lines.iter().position(|x| x.trim() == "Note1").unwrap();
    assert_eq!(lines[f1 - 1], "", "footnotes preceded by a blank separator");
    assert_eq!(lines[f1], format!("{}Note1", " ".repeat((20 - 5) / 2)));
    assert_eq!(lines[f1 + 1].trim(), "Note2");
}

/// No active footnote → no footnote output (byte-identity preserved).
#[test]
fn no_footnote_no_extra_output() {
    let mut l = ListingWriter::new(40);
    l.page_header();
    l.write_line("body");
    let s = l.into_string();
    assert_eq!(
        s,
        format!("{}The SAS System\n\nbody\n", " ".repeat((40 - 14) / 2))
    );
}

// ── char_width : mise en page comptée en caractères (contrat D-001) ────────

/// Un titre accentué est centré sur sa largeur en CARACTÈRES : « Café » fait
/// 4 caractères (5 octets UTF-8), pad = (20 − 4) / 2 = 8 — l'ancien calcul en
/// octets aurait produit 7 et décalé le titre d'une colonne.
#[test]
fn char_width_centered_title_accented() {
    let mut l = ListingWriter::new(20);
    l.page.titles = vec!["Café".into()];
    l.page_header();
    let out = l.into_string();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], format!("{}Café", " ".repeat((20 - 4) / 2)));
}

/// Une table à valeurs accentuées s'aligne en caractères : la colonne fait
/// 9 caractères (« Thé glacé »), pas 10 octets — l'ancien calcul en octets
/// paddait à 10 et décalait « Thé glacé » d'une colonne. En Align::Right les
/// fins de cellule tombent toutes sur la même colonne (largeur en caractères).
#[test]
fn char_width_table_accented_values_aligned() {
    let mut l = ListingWriter::new(60);
    l.write_table(
        &["Nom".into()],
        &[Align::Right],
        &[
            vec!["Café".into()],
            vec!["Dé".into()],
            vec!["Thé glacé".into()],
        ],
    );
    let s = l.into_string();
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines.len(), 5); // header, blank, 3 data rows
    // Colonne « Nom » : 9 caractères, left_pad = (60 − 9) / 2 = 25.
    // Chaque cellule alignée à droite se termine en colonne 25 + 9 = 34.
    for (cell, line) in [("Café", 2), ("Dé", 3), ("Thé glacé", 4)] {
        let start = 25 + (9 - char_width(cell));
        let rendered = lines[line];
        assert_eq!(
            &rendered[start..],
            cell,
            "cellule {cell:?} mal alignée dans {rendered:?}"
        );
    }
}

/// `char_width` compte les caractères, pas les octets (contrat D-001).
#[test]
fn char_width_counts_chars_not_bytes() {
    assert_eq!(char_width("Café"), 4);
    assert_eq!("Café".len(), 5, "garde : l'UTF-8 encode « é » sur 2 octets");
    assert_eq!(char_width(""), 0);
}
