//! Type d'erreur d'évaluation macro (`%eval`, conditions `%if`) et émission.

use super::*;

/// Erreur d'évaluation d'une expression macro (`%eval`, conditions `%if`,
/// bornes `%to`/`%by`). Portée par la feature `macros`. On ne `panic` jamais
/// sur une entrée macro invalide : l'expanseur transforme cette erreur en une
/// note SAS-like émise dans le flux de sortie et poursuit le scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MacroError {
    /// Message lisible (proche du libellé SAS quand pertinent).
    pub message: String,
}

impl MacroError {
    pub(super) fn new(msg: impl Into<String>) -> Self {
        MacroError { message: msg.into() }
    }
}

impl std::fmt::Display for MacroError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl MacroEngine {
    /// Émet une note d'erreur macro SAS-like dans le flux et poursuit. Jamais
    /// de `panic` sur entrée invalide.
    pub(super) fn emit_error(out: &mut String, err: &MacroError) {
        out.push_str(&format!("/* {} */", err.message));
    }
}
