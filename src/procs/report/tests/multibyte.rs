//! J03-P4 — tests `multibyte*` : PROC REPORT en caractères (contrat D-001,
//! docs/encoding.md). `pad_cell` et les largeurs de colonnes comptaient des
//! octets : une valeur accentuée/CJK décalait l'alignement, et une WIDTH=
//! tombant au milieu d'un caractère multioctet paniquait (`s.truncate(w)`).

use super::*;
use crate::dataset::SasDataset;
use crate::value::VarType;
use polars::df;

#[test]
fn multibyte_width_truncates_on_char_boundary() {
    // WIDTH=4 sur « Café » : garde les 4 caractères entiers (5 octets —
    // l'ancien truncate(4) paniquait sur la frontière du « é »).
    let mut session = make_session();
    let df = df!["name" => ["Café"]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("name", 8)],
    };
    write_dataset(&mut session, "T", ds);
    let ast = ReportAst {
        data: Some(work_ref("T")),
        columns: Some(vec!["name".into()]),
        defines: vec![def("name", Usage::Display, None, None, Some(4), None)],
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(listing.contains("Café"), "whole word kept: {listing}");
}

#[test]
fn multibyte_width_cjk_cut_keeps_whole_characters() {
    // WIDTH=2 sur « 日本語 » → « 日本 », le « 語 » tronqué disparaît.
    let mut session = make_session();
    let df = df!["kanji" => ["日本語"]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("kanji", 8)],
    };
    write_dataset(&mut session, "T", ds);
    let ast = ReportAst {
        data: Some(work_ref("T")),
        columns: Some(vec!["kanji".into()]),
        defines: vec![def("kanji", Usage::Display, None, None, Some(2), None)],
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    assert!(listing.contains("日本"), "two first chars kept: {listing}");
    assert!(
        !listing.contains('語'),
        "third char truncated away: {listing}"
    );
}

#[test]
fn multibyte_auto_width_pads_in_chars() {
    // Colonne sans WIDTH= : la largeur auto est le max en CARACTÈRES.
    // « Thé » (3 cars) et « Café » (4 cars) → les deux lignes s'alignent
    // sur 4 : « Thé » reçoit un espace de queue, pas deux.
    let mut session = make_session();
    let df = df!["w" => ["Café", "日本語"], "n" => [1.0_f64, 2.0]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("w", 8), num_meta("n")],
    };
    write_dataset(&mut session, "T", ds);
    let ast = ReportAst {
        data: Some(work_ref("T")),
        columns: Some(vec!["w".into(), "n".into()]),
        // WIDTH=8 sur la colonne numérique force le chemin write_table_layout
        // (le seul où pad_cell / largeur auto s'appliquent).
        defines: vec![def("n", Usage::Display, None, None, Some(8), None)],
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    // Largeur auto de la colonne w = max(4, 3) = 4 CARACTÈRES. « Café »
    // (4 cars, 5 octets) n'est pas paddé ; « 日本語 » (3 cars) l'est de 1.
    // Ensuite spacing 2 + champ numérique largeur 8 justifié droite (« 1 »
    // → 7 espaces de tête) : « Café » + 9 espaces + « 1 » ; « 日本語 » +
    // 10 espaces + « 2 ». En comptant les octets, la largeur serait
    // max(5, 9) = 9 et le padding différerait.
    assert!(
        listing.contains(&format!("Café{}1", " ".repeat(9))),
        "Cafe at char width 4: {listing}"
    );
    assert!(
        listing.contains(&format!("日本語{}2", " ".repeat(10))),
        "kanji padded by one char: {listing}"
    );
}

#[test]
fn multibyte_out_char_length_in_chars() {
    // OUT= : la longueur inférée de la colonne caractère compte les
    // CARACTÈRES — « Café » donne 4, pas 5 (octets).
    let mut session = make_session();
    let df = df!["w" => ["Café"], "n" => [1.0_f64]].unwrap();
    let ds = SasDataset {
        df,
        vars: vec![char_meta("w", 8), num_meta("n")],
    };
    write_dataset(&mut session, "T", ds);
    let ast = ReportAst {
        data: Some(work_ref("T")),
        columns: Some(vec!["w".into(), "n".into()]),
        out: Some(work_ref("R")),
        ..report_defaults()
    };
    execute(&ast, &mut session).unwrap();
    let (out, _) = session.libs.get("WORK").unwrap().read("R").unwrap();
    assert_eq!(out.vars[0].ty, VarType::Char);
    assert_eq!(out.vars[0].length, 4, "length in chars, not bytes");
}
