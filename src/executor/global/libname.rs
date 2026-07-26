//! LIBNAME : NOTE d'assignation de libref (MQ9.6).

use super::*;

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
