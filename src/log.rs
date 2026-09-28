//! Écriture du log SAS : écho du source, NOTE/WARNING/ERROR, temps d'étape.
//!
//! [`LogWriter`] numérote les lignes du source comme SAS, compte les warnings
//! et les erreurs (ils déterminent le code retour) et met en forme les NOTEs
//! de fin d'étape. `StepTimer` fournit les temps réel/CPU, figés sous
//! `--deterministic` pour que les snapshots soient stables.

use crate::api::{Diagnostic, Severity};
use std::io::Write;
use std::time::Instant;

/// SAS-style log writer: numbered source echo, NOTE/WARNING/ERROR lines
/// with the standard continuation indent, and per-step timing blocks.
///
/// J06-P2 : en plus du texte (inchangé octet pour octet), l'écrivain
/// accumule une liste de [`Diagnostic`] structurés — un par message
/// NOTE/WARNING/ERROR émis via [`LogWriter::note`] /
/// [`LogWriter::warning`] / [`LogWriter::error`] / [`LogWriter::forward`].
pub struct LogWriter {
    buf: String,
    src_line: usize,
    pub errors: u32,
    pub warnings: u32,
    deterministic: bool,
    /// J06-P2 — diagnostics structurés, dans l'ordre d'émission.
    pub diagnostics: Vec<Diagnostic>,
    /// J06-P2 — étape courante ("DATA", "PROC PRINT", …), maintenue en
    /// observant l'écho du source (statements `data`/`proc`).
    current_step: String,
    /// J07-P5 — PROC PRINTTO LOG= : route ouverte vers un fichier externe.
    /// Tant qu'elle est active, tout ce qui est ajouté à `buf` à partir de
    /// `start` appartient au fichier routé (vide vers le fichier à la fermeture
    /// de la route, puis retiré du log par défaut). `None` = pas de route
    /// (comportement byte-identique d'avant J07-P5).
    route: Option<LogRoute>,
}

/// J07-P5 — destination LOG= ouverte par PROC PRINTTO.
struct LogRoute {
    /// Handle ouvert en mode append (défaut) ou truncaté (NEW).
    file: std::fs::File,
    /// Indice de `buf` où commence le segment routé (non encore écrit).
    start: usize,
}

impl LogWriter {
    pub fn new(deterministic: bool) -> Self {
        LogWriter {
            buf: String::new(),
            src_line: 0,
            errors: 0,
            warnings: 0,
            deterministic,
            diagnostics: Vec::new(),
            current_step: String::new(),
            route: None,
        }
    }

    pub fn into_string(self) -> String {
        self.into_parts().0
    }

    /// J07-P5 — copie du texte accumulé à ce jour (segment routé compris),
    /// sans consommer l'écrivain — pour l'observation et les tests.
    pub fn current_text(&self) -> String {
        self.buf.clone()
    }

    /// J06-P2 — consomme l'écrivain et rend le texte du log AVEC la liste
    /// des diagnostics structurés accumulés.
    ///
    /// J07-P5 — une route PRINTTO encore ouverte à la consommation est vidée
    /// vers son fichier : le segment routé ne rejoint JAMAIS le log par
    /// défaut. Les erreurs d'ouverture, elles, ont été comptées au moment du
    /// `PROC PRINTTO` (voir [`LogWriter::begin_route`]).
    pub fn into_parts(mut self) -> (String, Vec<Diagnostic>) {
        if let Err(e) = self.flush_route_segment() {
            // Compteurs déjà lus par l'appelant : l'échec d'écriture tardif
            // reste au moins visible dans le texte rendu.
            let msg = format!("ERROR: Could not write routed log file: {e}");
            self.buf.push_str(&msg);
            self.buf.push('\n');
        }
        self.route = None;
        (self.buf, self.diagnostics)
    }

    /// J07-P5 — ouvre (ou remplace) la route LOG= de PROC PRINTTO. Le fichier
    /// est ouvert immédiatement (l'échec d'ouverture remonte à l'appelant pour
    /// une ERROR comptée) : `truncate=true` correspond à l'option `NEW`,
    /// `truncate=false` au mode ajout par défaut. Une route déjà active est
    /// d'abord vidée vers son fichier.
    pub fn begin_route(&mut self, path: &std::path::Path, truncate: bool) -> std::io::Result<()> {
        self.end_route();
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).write(true);
        if truncate {
            opts.truncate(true);
        } else {
            opts.append(true);
        }
        let file = opts.open(path)?;
        self.route = Some(LogRoute {
            file,
            start: self.buf.len(),
        });
        Ok(())
    }

    /// J07-P5 — referme la route LOG= active (PROC PRINTTO nu, remplacement
    /// ou fin de soumission) : le segment accumulé est écrit dans le fichier
    /// routé puis retiré du log par défaut. Sans route active : no-op.
    /// Un échec d'écriture devient une ERROR comptée dans le log par défaut.
    pub fn end_route(&mut self) {
        if let Err(e) = self.flush_route_segment() {
            self.error(&format!("Could not write routed log file: {e}"));
        }
        self.route = None;
    }

    /// J07-P5 — une route LOG= est-elle actuellement ouverte ?
    pub fn route_active(&self) -> bool {
        self.route.is_some()
    }

    /// J07-P5 — écrit le segment routé (`buf[start..]`) dans le fichier de la
    /// route et le retire du buffer par défaut. En cas d'échec le segment est
    /// restauré dans `buf` (aucune perte de contenu) et l'erreur remonte.
    fn flush_route_segment(&mut self) -> std::io::Result<()> {
        let start = match self.route.as_ref() {
            Some(r) => r.start,
            None => return Ok(()),
        };
        let segment = self.buf.split_off(start);
        let written = self.route.as_mut().map(|r| {
            r.file
                .write_all(segment.as_bytes())
                .and_then(|_| r.file.flush())
        });
        match written {
            Some(Ok(())) => {
                if let Some(r) = self.route.as_mut() {
                    r.start = self.buf.len();
                }
                Ok(())
            }
            Some(Err(e)) => {
                self.buf.push_str(&segment);
                Err(e)
            }
            None => Ok(()),
        }
    }

    fn raw(&mut self, line: &str) {
        self.buf.push_str(line);
        self.buf.push('\n');
    }

    /// J06-P2 — étape courante dérivée d'une ligne de source échoée :
    /// `proc xxx` → "PROC XXX", `data` → "DATA" ; inchangée sinon.
    fn observe_step(&mut self, line: &str) {
        let lower = line.trim_start().to_ascii_lowercase();
        if let Some(rest) = lower
            .strip_prefix("proc")
            .filter(|r| r.starts_with(char::is_whitespace))
        {
            let name = rest
                .trim_start()
                .split(|c: char| !c.is_alphanumeric())
                .next()
                .unwrap_or_default();
            if !name.is_empty() {
                self.current_step = format!("PROC {}", name.to_ascii_uppercase());
            }
        } else if let Some(_after) = lower
            .strip_prefix("data")
            .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace) || r.starts_with(';'))
        {
            self.current_step = "DATA".to_string();
        }
    }

    /// Echo submitted source lines with running statement numbers,
    /// preceded by a blank separator line like the SAS log.
    pub fn echo_source(&mut self, lines: &[&str]) {
        self.raw("");
        for line in lines {
            self.src_line += 1;
            self.observe_step(line);
            self.raw(&format!("{:<5} {}", self.src_line, line));
        }
    }

    /// A message with `PREFIX: ` on the first line and matching indent on
    /// continuation lines, as SAS does. Also records one structured
    /// diagnostic (J06-P2): severity, source line of the step being
    /// executed, current step name and the message.
    fn message(&mut self, prefix: &str, msg: &str) {
        let severity = match prefix {
            "ERROR" => Severity::Error,
            "WARNING" => Severity::Warning,
            _ => Severity::Note,
        };
        self.diagnostics.push(Diagnostic {
            severity,
            line: (self.src_line > 0).then_some(self.src_line as u32),
            step: self.current_step.clone(),
            message: msg.to_string(),
        });
        let indent = " ".repeat(prefix.len() + 2);
        for (i, line) in msg.lines().enumerate() {
            if i == 0 {
                self.raw(&format!("{prefix}: {line}"));
            } else {
                self.raw(&format!("{indent}{line}"));
            }
        }
    }

    pub fn note(&mut self, msg: &str) {
        self.message("NOTE", msg);
    }

    pub fn warning(&mut self, msg: &str) {
        self.warnings += 1;
        self.message("WARNING", msg);
    }

    pub fn error(&mut self, msg: &str) {
        self.errors += 1;
        self.message("ERROR", msg);
    }

    /// A verbatim line written by a DATA step PUT to `file log;` (M14.2):
    /// no "NOTE:" prefix, no source numbering — just the rendered text,
    /// through the same buffer as every other log line.
    pub fn put_line(&mut self, line: &str) {
        self.raw(line);
    }

    /// Forward a pre-prefixed line ("NOTE: ..." / "WARNING: ...") coming
    /// from lower layers (e.g. parquet type coercion).
    pub fn forward(&mut self, line: &str) {
        if let Some(msg) = line.strip_prefix("WARNING: ") {
            self.warning(msg);
        } else if let Some(msg) = line.strip_prefix("ERROR: ") {
            self.error(msg);
        } else if let Some(msg) = line.strip_prefix("NOTE: ") {
            self.note(msg);
        } else {
            self.note(line);
        }
    }

    /// The end-of-step timing NOTE. `what` is e.g. "DATA statement" or
    /// "PROCEDURE PRINT". Times are frozen under --deterministic so test
    /// snapshots stay byte-stable.
    pub fn step_used(&mut self, what: &str, timer: &StepTimer) {
        let (real, cpu) = if self.deterministic {
            ("0.00".to_string(), "0.00".to_string())
        } else {
            (
                format!("{:.2}", timer.start.elapsed().as_secs_f64()),
                format!("{:.2}", timer.cpu_elapsed()),
            )
        };
        self.note(&format!(
            "{what} used (Total process time):\n      real time           {real} seconds\n      cpu time            {cpu} seconds"
        ));
    }
}

pub struct StepTimer {
    start: Instant,
    cpu_start: f64,
}

impl StepTimer {
    pub fn start() -> Self {
        StepTimer {
            start: Instant::now(),
            cpu_start: process_cpu_seconds().unwrap_or(0.0),
        }
    }

    fn cpu_elapsed(&self) -> f64 {
        (process_cpu_seconds().unwrap_or(self.cpu_start) - self.cpu_start).max(0.0)
    }
}

/// utime+stime of this process in seconds, from /proc/self/stat (Linux).
/// Returns None on other platforms; cpu time then reads 0.00.
fn process_cpu_seconds() -> Option<f64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // The comm field (2nd) is parenthesized and may contain spaces; fields
    // utime and stime are the 12th and 13th after the closing paren.
    let (_, rest) = stat.rsplit_once(')')?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let utime: f64 = fields.get(11)?.parse().ok()?;
    let stime: f64 = fields.get(12)?.parse().ok()?;
    // Clock ticks; _SC_CLK_TCK is 100 on every mainstream Linux.
    Some((utime + stime) / 100.0)
}

#[cfg(test)]
mod tests;
