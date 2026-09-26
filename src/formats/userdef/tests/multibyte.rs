//! J03-P4 — tests `multibyte*` : largeur par défaut des formats utilisateur
//! comptée en CARACTÈRES (contrat D-001, docs/encoding.md). `max_label_len`
//! comptait des octets : un label « Très bien » (9 cars, 10 octets)
//! sur-dimensionnait la colonne d'une unité.

use super::*;
use crate::formats::{FormatCatalog, FormatSpec};

#[test]
fn multibyte_default_width_counts_chars_not_bytes() {
    // MIN=1 force le chemin de calcul (hors fast-path) ; sans DEFAULT=, la
    // largeur est la longueur du plus long label — 9 caractères ici.
    let uf = UserFormat {
        is_char: false,
        ranges: vec![num_single(1.0, "Très bien")],
        other: Some("Néant".to_string()),
        min: Some(1),
        ..Default::default()
    };
    assert_eq!(uf.effective_width(None), Some(9));

    let mut cat = FormatCatalog::default();
    cat.define("ACCENTF", uf);
    let spec = FormatSpec::parse("ACCENTF.").unwrap();
    let s = cat.format(&Value::Num(1.0), &spec);
    // 9 caractères exactement : « Très bien » tient sans padding ni coupe
    // (l'ancien comptage octet, 10, aurait ajouté un espace parasite).
    assert_eq!(s, "Très bien");
    assert_eq!(s.chars().count(), 9);
}

#[test]
fn multibyte_explicit_width_right_justifies_in_chars() {
    // Largeur explicite 10 sur « Très bien » (9 cars) : un espace de tête.
    let uf = UserFormat {
        is_char: false,
        ranges: vec![num_single(1.0, "Très bien")],
        other: None,
        ..Default::default()
    };
    let mut cat = FormatCatalog::default();
    cat.define("ACCENTF", uf);
    let spec = FormatSpec::parse("ACCENTF10.").unwrap();
    let s = cat.format(&Value::Num(1.0), &spec);
    assert_eq!(s, " Très bien");
}

#[test]
fn multibyte_cjk_label_width() {
    // « 東京 » : 2 caractères, 6 octets → largeur par défaut 2, pas 6.
    let uf = UserFormat {
        is_char: false,
        ranges: vec![num_single(1.0, "東京")],
        other: None,
        min: Some(1),
        ..Default::default()
    };
    assert_eq!(uf.effective_width(None), Some(2));
}
