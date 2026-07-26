//! Échafaudage partagé des tests unitaires (MQ8.10).
//!
//! Compilé UNIQUEMENT sous `cfg(test)`. Avant ce module, chaque module de
//! tests réécrivait les mêmes quatre helpers : `make_session` (41 copies),
//! `num_meta` (23), `char_meta` (17), `write_dataset` (16).
//!
//! `num_meta`/`char_meta` délèguent aux constructeurs de PRODUCTION
//! (`procs::common::{num_var_meta, char_var_meta}`) : une divergence entre ce
//! que les tests fabriquent et ce que les procs écrivent devient impossible.
//!
//! `char_meta` prend sa longueur en PARAMÈTRE, à dessein : les copies locales
//! la fixaient tacitement à 1, 4 ou 8 selon le fichier, et cette longueur
//! porte la sémantique de troncature SAS — elle doit être lisible dans le test.

use crate::dataset::{SasDataset, VarMeta};
use crate::session::Session;
use std::path::PathBuf;

/// Session déterministe dont les LIBNAME relatifs se résolvent depuis le
/// répertoire courant. `WORK` est un tempdir détruit avec la session.
pub(crate) fn make_session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

/// Variante dont la base de résolution est le répertoire temporaire du
/// système — pour les tests qui écrivent des fichiers de sortie (graphiques).
pub(crate) fn make_session_in_temp() -> Session {
    Session::new(None, std::env::temp_dir(), true).unwrap()
}

/// Métadonnée d'une variable numérique (longueur SAS 8, ni format ni label).
pub(crate) fn num_meta(name: &str) -> VarMeta {
    crate::procs::common::num_var_meta(name)
}

/// Métadonnée d'une variable caractère de longueur `length`.
pub(crate) fn char_meta(name: &str, length: usize) -> VarMeta {
    crate::procs::common::char_var_meta(name, length)
}

/// Écrit `ds` dans `WORK.<table>` et le pose comme `_LAST_`.
pub(crate) fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
    session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
    session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
}
