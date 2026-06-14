use std::time::Instant;

/// SAS-style log writer: numbered source echo, NOTE/WARNING/ERROR lines
/// with the standard continuation indent, and per-step timing blocks.
pub struct LogWriter {
    buf: String,
    src_line: usize,
    pub errors: u32,
    pub warnings: u32,
    deterministic: bool,
}

impl LogWriter {
    pub fn new(deterministic: bool) -> Self {
        LogWriter {
            buf: String::new(),
            src_line: 0,
            errors: 0,
            warnings: 0,
            deterministic,
        }
    }

    pub fn into_string(self) -> String {
        self.buf
    }

    fn raw(&mut self, line: &str) {
        self.buf.push_str(line);
        self.buf.push('\n');
    }

    /// Echo submitted source lines with running statement numbers,
    /// preceded by a blank separator line like the SAS log.
    pub fn echo_source(&mut self, lines: &[&str]) {
        self.raw("");
        for line in lines {
            self.src_line += 1;
            self.raw(&format!("{:<5} {}", self.src_line, line));
        }
    }

    /// A message with `PREFIX: ` on the first line and matching indent on
    /// continuation lines, as SAS does.
    fn message(&mut self, prefix: &str, msg: &str) {
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
mod tests {
    use super::*;

    #[test]
    fn echo_and_messages() {
        let mut log = LogWriter::new(true);
        log.echo_source(&["data a;", "run;"]);
        log.note("The data set WORK.A has 1 observations and 1 variables.");
        log.error("Syntax error.");
        let s = log.into_string();
        assert!(s.contains("1     data a;"));
        assert!(s.contains("2     run;"));
        assert!(s.contains("NOTE: The data set WORK.A"));
        assert!(s.contains("ERROR: Syntax error."));
    }

    #[test]
    fn deterministic_timing() {
        let mut log = LogWriter::new(true);
        log.step_used("DATA statement", &StepTimer::start());
        let s = log.into_string();
        assert!(s.contains("real time           0.00 seconds"));
        assert!(s.contains("cpu time            0.00 seconds"));
    }
}
