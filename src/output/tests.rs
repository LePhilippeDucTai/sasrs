use super::*;

#[test]
fn new_session_listing_is_text_listing() {
    // Le listing par défaut d'une session est une `TextListing` derrière le
    // trait object. On vérifie qu'on peut écrire au travers du trait.
    let tmp = std::env::temp_dir();
    let session = crate::session::Session::new(None, tmp, true).expect("session");
    let dest: &dyn OutputDestination = session.listing.as_ref();
    // LINESIZE par défaut = 96 (SasOptions::default()).
    assert_eq!(dest.ls(), 96);
}

#[test]
fn write_line_renders_text() {
    let mut d = TextListing::new(96);
    d.write_line("test");
    let out = d.into_string();
    assert_eq!(out, "test\n");
}

#[test]
fn page_header_default_title_centered() {
    let mut d = TextListing::new(40);
    d.page_header();
    let out = d.into_string();
    assert!(out.contains("The SAS System"), "out: {out:?}");
    // Centré dans LS=40 : padding gauche de (40-14)/2 = 13 espaces.
    assert!(out.starts_with("             The SAS System"), "out: {out:?}");
}

#[test]
fn set_title_overrides_default() {
    let mut d = TextListing::new(40);
    d.set_title(Some("Mon Titre".to_string()));
    d.page_header();
    let out = d.into_string();
    assert!(out.contains("Mon Titre"), "out: {out:?}");
    assert!(!out.contains("The SAS System"), "out: {out:?}");
}

#[test]
fn write_table_matches_listing_writer() {
    // Le rendu de table passe verbatim par ListingWriter ⇒ octet-identique.
    let mut d = TextListing::new(40);
    d.page_header();
    d.write_table(
        &["Obs".into(), "x".into()],
        &[Align::Right, Align::Right],
        &[
            vec!["1".into(), "10".into()],
            vec!["2".into(), "200".into()],
        ],
    );
    let via_trait = d.into_string();

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
    let direct = l.into_string();

    assert_eq!(via_trait, direct);
}

#[test]
fn blank_emits_empty_line() {
    let mut d = TextListing::new(40);
    d.write_line("a");
    d.blank();
    d.write_line("b");
    assert_eq!(d.into_string(), "a\n\nb\n");
}

#[test]
fn into_string_leaves_destination_empty() {
    // Deux drains successifs ne dupliquent pas le contenu.
    let mut d = TextListing::new(40);
    d.write_line("once");
    assert_eq!(d.into_string(), "once\n");
    assert_eq!(d.into_string(), "");
}

// html_stub_is_noop retiré : HtmlDestination est désormais une vraie
// destination (M22.4), remplacé par les tests html_* ci-dessous.

#[test]
fn stub_destinations_implement_trait() {
    // Les destinations stub RTF/PDF/Excel sont utilisables comme trait objects.
    let dests: Vec<Box<dyn OutputDestination>> = vec![
        Box::new(RtfDestination::new(80)),
        Box::new(PdfDestination::new(80)),
        Box::new(ExcelDestination::new(80)),
    ];
    for d in dests {
        assert_eq!(d.ls(), 80);
    }
}

// --- Tests M22.4 : HtmlDestination réelle ---

#[test]
fn html_table_renders_escaped_cells() {
    let mut h = HtmlDestination::new(96);
    h.write_table(
        &["Name".into(), "Value <x>".into()],
        &[Align::Left, Align::Right],
        &[
            vec!["a & b".into(), "42".into()],
            vec!["<tag>".into(), "99".into()],
        ],
    );
    let out = h.into_string();
    // Présence de la structure de table.
    assert!(out.contains("<table"), "pas de <table : {out}");
    assert!(out.contains("</table>"), "pas de </table> : {out}");
    // Échappement dans en-tête.
    assert!(out.contains("Value &lt;x&gt;"), "échappement header raté : {out}");
    // Échappement dans cellule.
    assert!(out.contains("a &amp; b"), "échappement & raté : {out}");
    assert!(out.contains("&lt;tag&gt;"), "échappement < raté : {out}");
    // Alignement droite sur la 2ᵉ colonne.
    assert!(out.contains("text-align:right"), "alignement right manquant : {out}");
}

#[test]
fn html_into_string_wraps_document() {
    let mut h = HtmlDestination::new(96);
    h.write_line("hello");
    let out = h.into_string();
    // Structure HTML obligatoire.
    assert!(out.contains("<!DOCTYPE html>"), "DOCTYPE manquant : {out}");
    assert!(out.contains("<style"), "style manquant : {out}");
    assert!(out.contains("<body>"), "<body> manquant : {out}");
    assert!(out.contains("</body>"), "</body> manquant : {out}");
    assert!(out.contains("<p>hello</p>"), "<p> manquant : {out}");
    // Second drain → chaîne vide (idempotent).
    assert_eq!(h.into_string(), "", "second drain non vide");
}

#[test]
fn html_without_file_finalize_none() {
    let mut h = HtmlDestination::new(96);
    h.write_line("test");
    // Sans fichier cible, finalize() renvoie None.
    assert!(h.finalize().is_none());
}

#[test]
fn html_empty_into_string_is_empty() {
    // Rien d'écrit → into_string() renvoie "" (pas de document vide).
    let mut h = HtmlDestination::new(96);
    assert_eq!(h.into_string(), "");
}

#[test]
fn html_page_header_uses_title() {
    let mut h = HtmlDestination::new(96);
    h.set_title(Some("Mon Rapport".to_string()));
    h.page_header();
    let out = h.into_string();
    assert!(out.contains("Mon Rapport"), "titre absent : {out}");
    assert!(out.contains("class=\"systitle\""), "classe systitle absente : {out}");
}

#[test]
fn html_page_header_default_title() {
    let mut h = HtmlDestination::new(96);
    h.page_header();
    let out = h.into_string();
    assert!(out.contains("The SAS System"), "titre par défaut absent : {out}");
}

#[test]
fn html_ls_accessor() {
    let h = HtmlDestination::new(80);
    assert_eq!(h.ls(), 80);
}

#[test]
fn html_with_file_finalize_some() {
    let tmp = std::env::temp_dir().join("test_html_finalize.html");
    let mut h = HtmlDestination::with_file(96, tmp.clone());
    h.write_line("content");
    let result = h.finalize();
    assert!(result.is_some(), "finalize devrait renvoyer Some");
    let (path, html) = result.unwrap();
    assert_eq!(path, tmp);
    assert!(html.contains("<!DOCTYPE html>"), "HTML complet attendu");
    assert!(html.contains("<p>content</p>"), "contenu attendu");
    // Après finalize, buf est vide.
    assert_eq!(h.into_string(), "", "buf doit être vide après finalize");
}

// --- Tests M23.1 : RtfDestination réelle ---

#[test]
fn rtf_table_renders_structure() {
    let mut r = RtfDestination::new(96);
    r.write_table(
        &["Name".into(), "Age".into()],
        &[Align::Left, Align::Right],
        &[vec!["Alfred".into(), "14".into()]],
    );
    let out = r.into_string();
    assert!(out.starts_with("{\\rtf1"), "RTF header manquant: {out}");
    assert!(out.contains("\\trowd"), "table RTF manquante: {out}");
    assert!(out.contains("Alfred"), "valeur manquante: {out}");
    assert!(out.contains("\\qr"), "alignement right manquant: {out}");
    assert!(out.contains("14"), "age manquant: {out}");
}

#[test]
fn rtf_escape_special_chars() {
    let mut r = RtfDestination::new(96);
    r.write_line("a\\b{c}d");
    let out = r.into_string();
    assert!(out.contains("a\\\\b\\{c\\}d"), "RTF escape rate: {out}");
}

#[test]
fn rtf_without_file_finalize_none() {
    let mut r = RtfDestination::new(96);
    r.write_line("test");
    assert!(r.finalize().is_none());
}

#[test]
fn rtf_with_file_finalize_some() {
    let tmp = std::env::temp_dir().join("test_ods.rtf");
    let mut r = RtfDestination::with_file(96, tmp.clone());
    r.write_line("hello");
    let result = r.finalize();
    assert!(result.is_some());
    let (path, content) = result.unwrap();
    assert_eq!(path, tmp);
    assert!(content.starts_with("{\\rtf1"), "RTF content: {content}");
}

// --- Tests M23.3 : ExcelDestination réelle ---

#[test]
fn excel_without_file_finalize_to_bytes_none() {
    let mut e = ExcelDestination::new(96);
    e.write_table(
        &["x".into()],
        &[Align::Right],
        &[vec!["1".into()]],
    );
    assert!(e.finalize_to_bytes().is_none());
}

#[test]
fn excel_with_file_finalize_to_bytes_some() {
    let tmp = std::env::temp_dir().join("test_ods.xlsx");
    let mut e = ExcelDestination::with_file(96, tmp.clone());
    e.write_table(
        &["Name".into(), "Age".into()],
        &[Align::Left, Align::Right],
        &[vec!["Alfred".into(), "14".into()]],
    );
    let result = e.finalize_to_bytes();
    assert!(result.is_some(), "finalize_to_bytes devrait retourner Some");
    let (path, bytes) = result.unwrap();
    assert_eq!(path, tmp);
    // Les fichiers XLSX commencent par PK (ZIP magic bytes)
    assert!(bytes.starts_with(b"PK"), "XLSX doit commencer par PK: {:?}", &bytes[..4]);
}

// --- Tests M23.2 : PdfDestination réelle ---

#[test]
fn pdf_without_file_finalize_to_bytes_none() {
    let mut p = PdfDestination::new(96);
    p.write_line("test");
    assert!(p.finalize_to_bytes().is_none());
}

#[test]
fn pdf_with_file_finalize_to_bytes_some() {
    let tmp = std::env::temp_dir().join("test_ods.pdf");
    let mut p = PdfDestination::with_file(96, tmp.clone());
    p.write_line("The SAS System");
    p.write_table(
        &["Name".into(), "Age".into()],
        &[Align::Left, Align::Right],
        &[vec!["Alfred".into(), "14".into()]],
    );
    let result = p.finalize_to_bytes();
    assert!(result.is_some(), "finalize_to_bytes devrait retourner Some");
    let (path, bytes) = result.unwrap();
    assert_eq!(path, tmp);
    assert!(bytes.starts_with(b"%PDF-"), "PDF magic bytes: {:?}", &bytes[..5]);
    let _ = std::fs::remove_file(&tmp);
}

// ── M38.1 : titres/footnotes multi-niveaux par destination ────────────────

#[test]
fn html_renders_multiple_titles_and_footnotes() {
    let mut h = HtmlDestination::new(96);
    h.set_titles(&["T1".to_string(), "T2".to_string()]);
    h.set_footnotes(&["F1".to_string()]);
    h.page_header();
    h.write_line("body");
    let out = h.into_string();
    let p1 = out.find("T1").unwrap();
    let p2 = out.find("T2").unwrap();
    assert!(p1 < p2, "titres dans l'ordre des niveaux");
    assert!(out.contains("sysfootnote"), "classe footnote absente : {out}");
    assert!(out.find("F1").unwrap() > out.find("body").unwrap(), "footnote après le corps");
}

#[test]
fn rtf_renders_multiple_titles_and_footnotes() {
    let mut r = RtfDestination::new(96);
    r.set_titles(&["T1".to_string(), "T2".to_string()]);
    r.set_footnotes(&["F1".to_string()]);
    r.page_header();
    r.write_line("body");
    let out = r.into_string();
    assert!(out.find("T1").unwrap() < out.find("T2").unwrap());
    assert!(out.find("F1").unwrap() > out.find("body").unwrap());
}

#[test]
fn pdf_renders_titles_and_footnotes_idempotent() {
    let tmp = std::env::temp_dir().join("test_ods_tf.pdf");
    let mut p = PdfDestination::with_file(96, tmp.clone());
    p.set_titles(&["T1".to_string(), "T2".to_string()]);
    p.set_footnotes(&["F1".to_string()]);
    p.page_header();
    p.write_line("body");
    let (_, bytes1) = p.finalize_to_bytes().unwrap();
    let s1 = String::from_utf8_lossy(&bytes1);
    assert!(s1.contains("(T1)") && s1.contains("(T2)") && s1.contains("(F1)"));
    // Idempotence : un second finalize produit le même contenu (pas de
    // duplication de footnotes dans self.sections).
    let (_, bytes2) = p.finalize_to_bytes().unwrap();
    assert_eq!(bytes1.len(), bytes2.len(), "finalize doit être idempotent");
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn excel_renders_titles_and_footnotes_as_rows() {
    let tmp = std::env::temp_dir().join("test_ods_tf.xlsx");
    let mut e = ExcelDestination::with_file(96, tmp.clone());
    e.set_titles(&["Top Title".to_string()]);
    e.set_footnotes(&["Bottom Note".to_string()]);
    e.write_table(
        &["Name".into()],
        &[Align::Left],
        &[vec!["Alice".into()]],
    );
    let (_, bytes) = e.finalize_to_bytes().unwrap();
    // XLSX = ZIP : la chaîne partagée contient titres et footnotes.
    let blob = String::from_utf8_lossy(&bytes);
    assert!(blob.contains("Top Title"), "titre absent du XLSX");
    assert!(blob.contains("Bottom Note"), "footnote absente du XLSX");
    let _ = std::fs::remove_file(&tmp);
}
