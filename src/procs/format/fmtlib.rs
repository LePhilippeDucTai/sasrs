//! M39.3 — `PROC FORMAT ... FMTLIB;` : liste le contenu d'un catalogue.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `FMTLIB` est une option d'en-tête sans valeur (voir `mod.rs::parse`) :
//! après RUN, elle imprime dans le listing un bloc par entrée VALUE/PICTURE/
//! INVALUE du catalogue CIBLÉ par `LIBRARY=`/`LIB=` (WORK par défaut) —
//! `render_fmtlib` ci-dessous. Oracle M39.3 : la liste des formats affichés
//! doit être EXACTEMENT ceux définis dans ce catalogue — ni plus (pas de
//! formats "vus" via un autre libref ou via `FMTSEARCH=`, qui n'ont aucune
//! influence ici : `render_fmtlib` lit `session.format_catalog_own_work` /
//! `session.libref_format_catalogs`, jamais `session.format_catalog`), ni
//! moins.
//!
//! Chaque bloc a un en-tête sobre à la SAS réelle (`FORMAT NAME:`/
//! `INFORMAT NAME:`, `LENGTH:`, `NUMBER OF VALUES:`) suivi d'une table
//! START/END/LABEL (une ligne par plage, plus une ligne `OTHER` si le format
//! en a un). Les bornes `LOW`/`HIGH` sont affichées en toutes lettres ; une
//! borne exclusive porte le marqueur `<` du côté concerné (`21<` = start
//! exclusif, `<30` = end exclusif) ; une plage à valeur unique (`from == to`,
//! aucune borne exclusive) n'affiche l'END qu'une fois, comme SAS.
//!
//! Déviation assumée : SAS calcule `LENGTH`/`MIN LENGTH`/`MAX LENGTH`/`FUZZ`
//! à partir de la largeur de stockage réelle du format compilé ; ce build n'a
//! pas cette notion (les formats utilisateur restent des structures en
//! mémoire, jamais "compilées"), donc `LENGTH` ici est simplement la longueur
//! du plus long LABEL/gabarit affiché — une approximation sobre et stable,
//! pas une reproduction byte-exacte de la sortie SAS réelle.

use super::*;
use crate::formats::FormatCatalog;
use crate::listing::Align;
use crate::session::Session;

struct FmtlibBlock {
    header: String,
    rows: Vec<Vec<String>>,
}

/// Numeric/char bound → display text. `LOW`/`HIGH` spelled out; numbers via
/// `Display` (shortest round-trip form, same convention as `cntl::format_num_bound`
/// but kept local — `cntl`'s helper is private to that module).
fn bound_text(b: &Bound) -> String {
    match b {
        Bound::Low => "LOW".to_string(),
        Bound::High => "HIGH".to_string(),
        Bound::Num(n) => format!("{n}"),
        Bound::Char(s) => format!("'{s}'"),
    }
}

fn bounds_equal(a: &Bound, b: &Bound) -> bool {
    match (a, b) {
        (Bound::Low, Bound::Low) | (Bound::High, Bound::High) => true,
        (Bound::Num(x), Bound::Num(y)) => x == y,
        (Bound::Char(x), Bound::Char(y)) => x == y,
        _ => false,
    }
}

/// START/END cell pair for one range. A single-value range (equal bounds,
/// both inclusive) shows the value once (START only, blank END) — mirrors
/// SAS's own FMTLIB rendering of `1='Male'`-style ranges.
fn start_end_cells(from: &Bound, to: &Bound, from_excl: bool, to_excl: bool) -> (String, String) {
    if bounds_equal(from, to) && !from_excl && !to_excl {
        (bound_text(from), String::new())
    } else {
        let start = if from_excl {
            format!("{}<", bound_text(from))
        } else {
            bound_text(from)
        };
        let end = if to_excl {
            format!("<{}", bound_text(to))
        } else {
            bound_text(to)
        };
        (start, end)
    }
}

fn max_len(rows: &[Vec<String>], col: usize) -> usize {
    rows.iter()
        .map(|r| r.get(col).map(String::len).unwrap_or(0))
        .max()
        .unwrap_or(0)
        .max(1)
}

fn build_value_block(name: &str, uf: &UserFormat) -> FmtlibBlock {
    let mut rows: Vec<Vec<String>> = uf
        .ranges
        .iter()
        .map(|r| {
            let (start, end) = start_end_cells(&r.from, &r.to, r.from_exclusive, r.to_exclusive);
            vec![start, end, r.label.clone()]
        })
        .collect();
    let n_values = uf.ranges.len() + usize::from(uf.other.is_some());
    if let Some(other) = &uf.other {
        rows.push(vec!["OTHER".to_string(), String::new(), other.clone()]);
    }
    let ty = if uf.is_char { "CHARACTER" } else { "NUMERIC" };
    let header = format!(
        "FORMAT NAME: {name}    TYPE: {ty}    LENGTH: {}    NUMBER OF VALUES: {n_values}",
        max_len(&rows, 2),
    );
    FmtlibBlock { header, rows }
}

fn picture_label(template: &str, dirs: &PictureDirectives) -> String {
    let mut s = format!("'{template}'");
    let mut extras: Vec<String> = Vec::new();
    if let Some(p) = &dirs.prefix {
        extras.push(format!("PREFIX='{p}'"));
    }
    if let Some(m) = dirs.mult {
        extras.push(format!("MULT={m}"));
    }
    if let Some(f) = dirs.fill {
        extras.push(format!("FILL='{f}'"));
    }
    if !extras.is_empty() {
        s.push_str(&format!(" ({})", extras.join(" ")));
    }
    s
}

fn build_picture_block(name: &str, up: &UserPicture) -> FmtlibBlock {
    let mut rows: Vec<Vec<String>> = up
        .ranges
        .iter()
        .map(|r| {
            let (start, end) = start_end_cells(&r.from, &r.to, r.from_exclusive, r.to_exclusive);
            (start, end, picture_label(&r.template, &r.directives))
        })
        .map(|(s, e, l)| vec![s, e, l])
        .collect();
    let n_values = up.ranges.len() + usize::from(up.other.is_some());
    if let Some((tpl, dirs)) = &up.other {
        rows.push(vec![
            "OTHER".to_string(),
            String::new(),
            picture_label(tpl, dirs),
        ]);
    }
    let header = format!(
        "FORMAT NAME: {name}    TYPE: PICTURE (NUMERIC)    LENGTH: {}    NUMBER OF VALUES: {n_values}",
        max_len(&rows, 2),
    );
    FmtlibBlock { header, rows }
}

fn informat_result_text(iv: &InformatValue) -> String {
    match iv {
        InformatValue::Num(n) => format!("{n}"),
        InformatValue::Char(s) => s.clone(),
        InformatValue::Same => "_SAME_".to_string(),
        InformatValue::Missing(kind) => match kind.as_str() {
            "." | "" => ".".to_string(),
            "_" => "._".to_string(),
            s => format!(".{}", s.to_uppercase()),
        },
    }
}

fn build_informat_block(name: &str, ui: &UserInformat) -> FmtlibBlock {
    let mut rows: Vec<Vec<String>> = ui
        .ranges
        .iter()
        .map(|r| {
            let (start, end) = start_end_cells(&r.from, &r.to, r.from_exclusive, r.to_exclusive);
            vec![start, end, informat_result_text(&r.result)]
        })
        .collect();
    let n_values = ui.ranges.len() + usize::from(ui.other.is_some());
    if let Some(other) = &ui.other {
        rows.push(vec![
            "OTHER".to_string(),
            String::new(),
            informat_result_text(other),
        ]);
    }
    let ty = if ui.is_char_result {
        "CHARACTER"
    } else {
        "NUMERIC"
    };
    let header = format!(
        "INFORMAT NAME: {name}    RESULT: {ty}    LENGTH: {}    NUMBER OF VALUES: {n_values}",
        max_len(&rows, 2),
    );
    FmtlibBlock { header, rows }
}

/// Render `FMTLIB` for the catalog targeted by `lib` (already uppercase,
/// `"WORK"` for the default). See module doc for the exact source consulted
/// per target and the oracle it must satisfy.
pub(super) fn render_fmtlib(session: &mut Session, lib: &str) {
    let empty = FormatCatalog::default();
    let blocks: Vec<FmtlibBlock> = {
        let cat: &FormatCatalog = if lib == "WORK" {
            &session.format_catalog_own_work
        } else {
            session.libref_format_catalogs.get(lib).unwrap_or(&empty)
        };

        let mut entries: Vec<(String, FmtlibBlock)> = Vec::new();
        for (name, uf) in cat.user_formats() {
            entries.push((name.to_string(), build_value_block(name, uf)));
        }
        for (name, up) in cat.user_pictures() {
            entries.push((name.to_string(), build_picture_block(name, up)));
        }
        for (name, ui) in cat.user_informats() {
            entries.push((name.to_string(), build_informat_block(name, ui)));
        }
        // Sorted by name (upcase, `$` sorts before letters — ASCII order),
        // giving a stable, deterministic listing order across VALUE/PICTURE/
        // INVALUE alike.
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries.into_iter().map(|(_, b)| b).collect()
    };

    session.listing.page_header();
    if blocks.is_empty() {
        session.listing.write_line(&format!(
            "NOTE: No formats or informats are defined in the {lib} catalog."
        ));
        return;
    }
    let headers = vec!["START".to_string(), "END".to_string(), "LABEL".to_string()];
    let aligns = vec![Align::Left, Align::Left, Align::Left];
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 {
            session.listing.blank();
        }
        session.listing.write_line(&block.header);
        session.listing.blank();
        session.listing.write_table(&headers, &aligns, &block.rows);
    }
}
