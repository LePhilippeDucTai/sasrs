//! LIBNAME : NOTE d'assignation de libref (MQ9.6).

use super::*;

/// M39.1 — charge le sidecar de catalogue de formats (`formats.sascat.json`)
/// du libref tout juste assigné, s'il existe, et le fusionne dans le
/// catalogue de session (`session.format_catalog`) SANS écraser une clé déjà
/// présente (voir [`crate::formats::FormatCatalog::merge_missing_from`] pour
/// le raisonnement d'ordre WORK-d'abord). Ne fait RIEN pour WORK : le
/// catalogue WORK reste purement en mémoire, jamais touché par le disque
/// (garantit l'octet-identité du chemin par défaut, M39 DoD).
///
/// Rafraîchit toujours `session.libref_format_catalogs[libref]` (le supprime
/// puis le repose si un sidecar existe) : un `LIBNAME` réassigne le libref à
/// un (possiblement nouveau) répertoire, l'ancien catalogue en mémoire pour
/// ce nom de libref n'est plus pertinent.
pub(crate) fn load_format_catalog_sidecar(session: &mut Session, libref: &str) {
    let key = libref.to_uppercase();
    if key == "WORK" {
        return;
    }
    session.libref_format_catalogs.remove(&key);

    let Ok(provider) = session.libs.get(libref) else {
        return;
    };
    let Some(dir) = provider.catalog_dir() else {
        return;
    };
    let dir = dir.to_path_buf();
    drop(provider);

    let Some(catalog) = crate::formats::FormatCatalog::load_sidecar(&dir) else {
        return;
    };
    if !catalog.is_empty() {
        let merged = std::rc::Rc::make_mut(&mut session.format_catalog);
        merged.merge_missing_from(&catalog);
    }
    session.libref_format_catalogs.insert(key, catalog);

    // M39.3 — when `OPTIONS FMTSEARCH=` has been posed, this LIBNAME's newly
    // (re)loaded sidecar must be picked up according to the explicit search
    // order, not the implicit merge just above (which stays for the
    // FMTSEARCH=-never-set default path only — see `formats::mod` doc).
    if !session.options.fmtsearch.is_empty() {
        crate::formats::search::rebuild_format_catalog(session);
    }
}

/// NOTE de succès (ou ERROR) commune aux trois moteurs de LIBNAME.
pub(crate) fn log_libref_assignment(
    session: &mut Session,
    libref: &str,
    engine: &str,
    physical: &str,
    result: crate::error::Result<()>,
) {
    match result {
        Ok(()) => session.log.note(&format!(
            "Libref {} was successfully assigned as follows:\n      Engine:        {engine}\n      Physical Name: {physical}",
            libref.to_uppercase()
        )),
        Err(e) => session.log.error(&e.to_string()),
    }
}
