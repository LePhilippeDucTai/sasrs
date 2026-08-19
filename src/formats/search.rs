//! M39.3 — `FMTSEARCH=` : ordre de résolution multi-catalogue explicite.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! Deux fonctions, volontairement séparées :
//! - [`resolve_search_order`] — pure, testable sans `Session` : traduit
//!   `session.options.fmtsearch` (une `Vec<String>` déjà upcase, posée par
//!   `OPTIONS FMTSEARCH=(...)`, voir `executor/global/options.rs`) en l'ordre
//!   EFFECTIF de recherche, en insérant `WORK`/`LIBRARY` en tête s'ils sont
//!   absents de la liste.
//! - [`rebuild_format_catalog`] — reconstruit `session.format_catalog` EN
//!   ENTIER à partir de `session.format_catalog_own_work` (le "slot" WORK) et
//!   `session.libref_format_catalogs` (un slot par libref), fusionnés dans
//!   l'ordre de [`resolve_search_order`] via
//!   [`crate::formats::FormatCatalog::merge_missing_from`] (premier arrivé,
//!   premier servi — cohérent avec la sémantique déjà utilisée par le chemin
//!   legacy `LIBNAME`, voir le doc de module de `formats::mod`).
//!
//! Appelée UNIQUEMENT quand `session.options.fmtsearch` n'est pas vide — voir
//! les points d'appel dans `executor/global/options.rs` (au `OPTIONS
//! FMTSEARCH=`), `executor/global/libname.rs` (au `LIBNAME`, pour qu'un
//! nouveau libref rejoigne immédiatement un ordre déjà posé) et
//! `procs/format/mod.rs` (en fin de `PROC FORMAT`, pour qu'une nouvelle
//! définition — WORK ou libref — soit reprise par un ordre déjà posé). Tant
//! que `FMTSEARCH=` n'a jamais été posée, aucun de ces points d'appel
//! n'invoque cette fonction : `session.format_catalog` reste peuplé
//! EXCLUSIVEMENT par le chemin legacy (fusion immédiate au `LIBNAME` +
//! écriture directe par `PROC FORMAT`), garantissant l'octet-identité des
//! jalons précédents (M39 DoD).

use crate::session::Session;

/// Traduit `fmtsearch` (déjà upcase) en ordre de recherche effectif.
///
/// Règle SAS : WORK et LIBRARY sont toujours cherchés, et par défaut EN TÊTE
/// (dans cet ordre), SAUF s'ils figurent explicitement dans `fmtsearch` — ils
/// gardent alors la position écrite par l'utilisateur. Concrètement :
/// - `WORK` absent → inséré en position 0.
/// - `LIBRARY` absent → inséré juste après la position de `WORK` (qui, à ce
///   stade, est garantie présente).
///
/// `"LIBRARY"` n'a pas de statut spécial ici au-delà de ce positionnement par
/// défaut : ce n'est un libref consultable que si l'utilisateur l'a
/// lui-même assigné via `LIBNAME LIBRARY ...` (ce build n'a pas de catalogue
/// LIBRARY préassigné comme le fait SAS) ; sinon [`rebuild_format_catalog`]
/// ne trouve simplement rien à ce nom et passe au suivant.
pub fn resolve_search_order(fmtsearch: &[String]) -> Vec<String> {
    let mut order: Vec<String> = fmtsearch.to_vec();
    if !order.iter().any(|s| s == "WORK") {
        order.insert(0, "WORK".to_string());
    }
    if !order.iter().any(|s| s == "LIBRARY") {
        let work_pos = order
            .iter()
            .position(|s| s == "WORK")
            .expect("WORK was just ensured present above");
        order.insert(work_pos + 1, "LIBRARY".to_string());
    }
    order
}

/// Reconstruit `session.format_catalog` EN ENTIER selon l'ordre effectif de
/// `session.options.fmtsearch` (voir [`resolve_search_order`]). Ne rien
/// appeler ici quand `fmtsearch` est vide — le chemin legacy reste seul
/// responsable de `format_catalog` dans ce cas (voir le doc de module).
pub fn rebuild_format_catalog(session: &mut Session) {
    let order = resolve_search_order(&session.options.fmtsearch);
    let mut merged = crate::formats::FormatCatalog::default();
    for name in &order {
        if name == "WORK" {
            merged.merge_missing_from(&session.format_catalog_own_work);
        } else if let Some(cat) = session.libref_format_catalogs.get(name.as_str()) {
            merged.merge_missing_from(cat);
        }
    }
    session.format_catalog = std::rc::Rc::new(merged);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_search_order_empty_defaults_work_then_library() {
        assert_eq!(resolve_search_order(&[]), vec!["WORK", "LIBRARY"]);
    }

    #[test]
    fn resolve_search_order_prepends_work_and_library_when_absent() {
        let fs = vec!["MYLIB".to_string()];
        assert_eq!(resolve_search_order(&fs), vec!["WORK", "LIBRARY", "MYLIB"]);
    }

    #[test]
    fn resolve_search_order_keeps_explicit_work_position() {
        let fs = vec!["MYLIB".to_string(), "WORK".to_string()];
        assert_eq!(resolve_search_order(&fs), vec!["MYLIB", "WORK", "LIBRARY"]);
    }

    #[test]
    fn resolve_search_order_keeps_explicit_library_position() {
        let fs = vec!["MYLIB".to_string(), "LIBRARY".to_string()];
        assert_eq!(resolve_search_order(&fs), vec!["WORK", "MYLIB", "LIBRARY"]);
    }

    #[test]
    fn resolve_search_order_both_explicit_unchanged() {
        let fs = vec![
            "MYLIB".to_string(),
            "LIBRARY".to_string(),
            "WORK".to_string(),
            "OTHERLIB".to_string(),
        ];
        assert_eq!(
            resolve_search_order(&fs),
            vec!["MYLIB", "LIBRARY", "WORK", "OTHERLIB"]
        );
    }

    #[test]
    fn rebuild_format_catalog_orders_by_fmtsearch() {
        use crate::formats::FormatSpec;
        use crate::formats::userdef::{Bound, Range, UserFormat};
        use crate::value::Value;

        let mk = |label: &str| UserFormat {
            is_char: false,
            ranges: vec![Range {
                from: Bound::Num(1.0),
                to: Bound::Num(1.0),
                from_exclusive: false,
                to_exclusive: false,
                label: label.to_string(),
            }],
            other: None,
            ..Default::default()
        };

        let mut session = Session::new(None, std::env::temp_dir(), true).unwrap();
        session
            .format_catalog_own_work
            .define("COLOR", mk("Work-Red"));
        let mut lib_cat = crate::formats::FormatCatalog::default();
        lib_cat.define("COLOR", mk("Lib-Red"));
        session
            .libref_format_catalogs
            .insert("MYLIB".to_string(), lib_cat);

        let spec = FormatSpec::parse("COLOR.").unwrap();

        // MYLIB before WORK: MYLIB wins.
        session.options.fmtsearch = vec!["MYLIB".to_string(), "WORK".to_string()];
        rebuild_format_catalog(&mut session);
        assert_eq!(
            session.format_catalog.format(&Value::Num(1.0), &spec),
            "Lib-Red"
        );

        // WORK before MYLIB: WORK wins.
        session.options.fmtsearch = vec!["WORK".to_string(), "MYLIB".to_string()];
        rebuild_format_catalog(&mut session);
        assert_eq!(
            session.format_catalog.format(&Value::Num(1.0), &spec),
            "Work-Red"
        );
    }
}
