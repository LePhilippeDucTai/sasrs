use crate::token::Span;

/// A submitted SAS source file, with byte-offset → line mapping used for
/// log echo and error reporting.
pub struct SourceFile {
    pub text: String,
    /// Byte offset of the start of each line.
    line_starts: Vec<usize>,
}

impl SourceFile {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let mut line_starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        SourceFile { text, line_starts }
    }

    /// 0-based line index containing the byte offset.
    pub fn line_of(&self, offset: usize) -> usize {
        match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        }
    }

    /// Full text of the 0-based line index, without trailing newline.
    pub fn line_text(&self, line: usize) -> &str {
        let start = self.line_starts[line];
        let end = self
            .line_starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches(['\n', '\r'])
    }

    /// All full lines covered by a span, as (1-based line number, text) pairs.
    pub fn lines_of_span(&self, span: Span) -> Vec<(usize, &str)> {
        if self.text.is_empty() {
            return Vec::new();
        }
        let first = self.line_of(span.start.min(self.text.len().saturating_sub(1)));
        let last = self.line_of(span.end.saturating_sub(1).min(self.text.len().saturating_sub(1)));
        (first..=last)
            .map(|l| (l + 1, self.line_text(l)))
            .collect()
    }
}
