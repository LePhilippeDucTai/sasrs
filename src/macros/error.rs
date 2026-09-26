//! Type d'erreur d'évaluation macro (`%eval`, conditions `%if`) et émission.

use super::*;

/// Erreur d'évaluation d'une expression macro (`%eval`, conditions `%if`,
/// bornes `%to`/`%by`). Le diagnostic rejoint le log de la session, jamais
/// le texte SAS généré.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MacroError {
    /// Message lisible (proche du libellé SAS quand pertinent).
    pub message: String,
}

impl MacroError {
    pub(super) fn new(msg: impl Into<String>) -> Self {
        MacroError {
            message: msg.into(),
        }
    }
}

impl std::fmt::Display for MacroError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl MacroEngine {
    pub(super) fn emit_error(&mut self, err: &MacroError) {
        self.error(err.message.strip_prefix("ERROR: ").unwrap_or(&err.message));
    }

    pub(super) fn error(&mut self, message: impl Into<String>) {
        self.pending
            .log_lines
            .push(MacroLogEntry::Error(message.into()));
    }

    pub(super) fn warning(&mut self, message: impl Into<String>) {
        self.pending
            .log_lines
            .push(MacroLogEntry::Warning(message.into()));
    }

    pub(super) fn note(&mut self, message: impl Into<String>) {
        self.pending
            .log_lines
            .push(MacroLogEntry::Note(message.into()));
    }
}

/// Ordered output channel: diagnostics have a severity, while %PUT and trace
/// output remain literal. The executor drains this once per expanded segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MacroLogEntry {
    Line(String),
    Error(String),
    Warning(String),
    Note(String),
}

impl MacroLogEntry {
    pub fn into_line(self) -> String {
        match self {
            Self::Line(s) => s,
            Self::Error(s) => format!("ERROR: {s}"),
            Self::Warning(s) => format!("WARNING: {s}"),
            Self::Note(s) => format!("NOTE: {s}"),
        }
    }
}
