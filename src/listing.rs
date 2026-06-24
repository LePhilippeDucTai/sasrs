//! SAS-style listing output: centered titles and monospace tables.
//!
//! M1 scope: centered page title ("The SAS System" or TITLE statement),
//! PROC PRINT-style tables (blank line between header and data, columns
//! centered as a block within the line size). Divergence from SAS 9.4,
//! documented in README: no date/page-number header line (equivalent to
//! OPTIONS NODATE NONUMBER) and no form feeds / PS= pagination yet.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

pub struct ListingWriter {
    buf: String,
    /// LINESIZE: width used to center output.
    pub ls: usize,
    /// Active title levels (TITLE1..TITLE9), in level order, gaps removed.
    /// Empty = default "The SAS System".
    pub titles: Vec<String>,
    /// Active footnote levels (FOOTNOTE1..FOOTNOTE9), in level order, gaps
    /// removed. Empty = no footnotes.
    pub footnotes: Vec<String>,
    wrote_anything: bool,
}

impl ListingWriter {
    pub fn new(ls: usize) -> Self {
        ListingWriter {
            buf: String::new(),
            ls,
            titles: Vec::new(),
            footnotes: Vec::new(),
            wrote_anything: false,
        }
    }

    pub fn into_string(mut self) -> String {
        // Flush the last proc's footnotes (if any) before draining, so they
        // follow the final block of content. No-op when no content was written
        // or no footnotes are active.
        if self.wrote_anything {
            self.flush_footnotes();
        }
        self.buf
    }

    fn raw(&mut self, line: &str) {
        self.buf.push_str(line.trim_end());
        self.buf.push('\n');
    }

    fn centered(&mut self, text: &str) {
        let pad = self.ls.saturating_sub(text.len()) / 2;
        self.raw(&format!("{}{}", " ".repeat(pad), text));
    }

    /// Left-justified raw line: output `text` as-is at column 0.
    pub fn write_line(&mut self, text: &str) {
        self.raw(text);
    }

    /// Emit a blank line.
    pub fn blank(&mut self) {
        self.raw("");
    }

    /// Page header at the start of each proc's output.
    ///
    /// Renders the active titles centered, in level order, followed by a single
    /// blank line. With no active title, the default "The SAS System" is
    /// centered (byte-identical to the historical single-title behavior). If a
    /// previous proc's content is already present, its footnotes are flushed and
    /// an inter-proc separator blank line is inserted first.
    pub fn page_header(&mut self) {
        if self.wrote_anything {
            self.flush_footnotes();
            self.raw("");
        }
        self.wrote_anything = true;
        if self.titles.is_empty() {
            self.centered("The SAS System");
        } else {
            for t in self.titles.clone() {
                self.centered(&t);
            }
        }
        self.raw("");
    }

    /// Emit the active footnotes (centered, in level order) preceded by a blank
    /// separator line. No-op when no footnotes are active. Called before each
    /// inter-proc separator (so footnotes follow their proc's content) and at
    /// drain time so the last proc's footnotes are emitted.
    pub fn flush_footnotes(&mut self) {
        if self.footnotes.is_empty() {
            return;
        }
        self.raw("");
        for f in self.footnotes.clone() {
            self.centered(&f);
        }
    }

    /// Render a table: column widths fit header and cells, two-space gaps,
    /// the whole block centered in LS, blank line between header and rows.
    pub fn write_table(&mut self, headers: &[String], aligns: &[Align], rows: &[Vec<String>]) {
        let ncol = headers.len();
        let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
        for row in rows {
            for (i, cell) in row.iter().enumerate().take(ncol) {
                widths[i] = widths[i].max(cell.len());
            }
        }
        let gap = "    ";
        let total: usize = widths.iter().sum::<usize>() + gap.len() * ncol.saturating_sub(1);
        let left_pad = " ".repeat(self.ls.saturating_sub(total) / 2);

        let fmt_row = |cells: &[String], widths: &[usize]| -> String {
            let mut parts = Vec::with_capacity(ncol);
            for i in 0..ncol {
                let cell = cells.get(i).map(String::as_str).unwrap_or("");
                let w = widths[i];
                match aligns.get(i).copied().unwrap_or(Align::Left) {
                    Align::Left => parts.push(format!("{cell:<w$}")),
                    Align::Right => parts.push(format!("{cell:>w$}")),
                }
            }
            parts.join(gap)
        };

        // Headers are centered over their column in SAS PRINT; keep the
        // simpler convention of header following the column alignment.
        self.raw(&format!("{left_pad}{}", fmt_row(headers, &widths)));
        self.raw("");
        for row in rows {
            self.raw(&format!("{left_pad}{}", fmt_row(row, &widths)));
        }
    }

    /// Render a table with PROC PRINT extensions (M33.6): optional double
    /// spacing between body rows and an optional trailing totals row.
    ///
    /// Geometry (column widths, two-space-block gap, centering) is computed
    /// identically to [`write_table`], but the totals row is included in the
    /// width fit so a wide sum does not overflow its column. The totals row is
    /// preceded by a blank line and rendered with the same alignment as the
    /// body. When `double` is true, body rows are separated by a blank line
    /// (SAS DOUBLE option).
    #[allow(clippy::too_many_arguments)]
    pub fn write_table_ext(
        &mut self,
        headers: &[String],
        aligns: &[Align],
        rows: &[Vec<String>],
        double: bool,
        totals: Option<&Vec<String>>,
    ) {
        let ncol = headers.len();
        let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
        let extra_rows = totals.into_iter();
        for row in rows.iter().chain(extra_rows) {
            for (i, cell) in row.iter().enumerate().take(ncol) {
                widths[i] = widths[i].max(cell.len());
            }
        }
        let gap = "    ";
        let total: usize = widths.iter().sum::<usize>() + gap.len() * ncol.saturating_sub(1);
        let left_pad = " ".repeat(self.ls.saturating_sub(total) / 2);

        let fmt_row = |cells: &[String], widths: &[usize]| -> String {
            let mut parts = Vec::with_capacity(ncol);
            for i in 0..ncol {
                let cell = cells.get(i).map(String::as_str).unwrap_or("");
                let w = widths[i];
                match aligns.get(i).copied().unwrap_or(Align::Left) {
                    Align::Left => parts.push(format!("{cell:<w$}")),
                    Align::Right => parts.push(format!("{cell:>w$}")),
                }
            }
            parts.join(gap)
        };

        self.raw(&format!("{left_pad}{}", fmt_row(headers, &widths)));
        self.raw("");
        for (i, row) in rows.iter().enumerate() {
            if double && i > 0 {
                self.raw("");
            }
            self.raw(&format!("{left_pad}{}", fmt_row(row, &widths)));
        }
        if let Some(t) = totals {
            self.raw("");
            self.raw(&format!("{left_pad}{}", fmt_row(t, &widths)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_layout() {
        let mut l = ListingWriter::new(40);
        l.page_header();
        l.write_table(
            &["Obs".into(), "x".into()],
            &[Align::Right, Align::Right],
            &[vec!["1".into(), "10".into()], vec!["2".into(), "200".into()]],
        );
        let s = l.into_string();
        assert!(s.contains("The SAS System"));
        assert!(s.contains("Obs      x"));
        assert!(s.contains("  1     10"));
        assert!(s.contains("  2    200"));
    }

    /// Byte-identity guard: a single title plus default rendering is unchanged.
    #[test]
    fn single_title_byte_identical() {
        let mut l = ListingWriter::new(40);
        l.titles = vec!["My Report".into()];
        l.page_header();
        // pad = (40 - 9) / 2 = 15 spaces, then text, then blank line.
        assert_eq!(l.into_string(), format!("{}My Report\n\n", " ".repeat(15)));
    }

    /// Three titles render centered, in level order, with one trailing blank.
    #[test]
    fn three_titles_centered_in_order() {
        let mut l = ListingWriter::new(20);
        l.titles = vec!["A".into(), "BB".into(), "CCC".into()];
        l.page_header();
        let s = l.into_string();
        let lines: Vec<&str> = s.lines().collect();
        // Title order preserved.
        assert_eq!(lines[0].trim(), "A");
        assert_eq!(lines[1].trim(), "BB");
        assert_eq!(lines[2].trim(), "CCC");
        // Centering: pad = (20 - len) / 2.
        assert_eq!(lines[0], format!("{}A", " ".repeat((20 - 1) / 2)));
        assert_eq!(lines[2], format!("{}CCC", " ".repeat((20 - 3) / 2)));
        // Exactly one trailing blank line after all titles (line index 3).
        assert_eq!(lines[3], "");
        assert_eq!(lines.len(), 4);
    }

    /// Footnotes render centered at the bottom on drain.
    #[test]
    fn footnotes_centered_on_drain() {
        let mut l = ListingWriter::new(20);
        l.footnotes = vec!["Note1".into(), "Note2".into()];
        l.page_header();
        l.write_line("body");
        let s = l.into_string();
        let lines: Vec<&str> = s.lines().collect();
        // Footnotes appear after the body, centered, preceded by a blank.
        let f1 = lines.iter().position(|x| x.trim() == "Note1").unwrap();
        assert_eq!(lines[f1 - 1], "", "footnotes preceded by a blank separator");
        assert_eq!(lines[f1], format!("{}Note1", " ".repeat((20 - 5) / 2)));
        assert_eq!(lines[f1 + 1].trim(), "Note2");
    }

    /// No active footnote → no footnote output (byte-identity preserved).
    #[test]
    fn no_footnote_no_extra_output() {
        let mut l = ListingWriter::new(40);
        l.page_header();
        l.write_line("body");
        let s = l.into_string();
        assert_eq!(s, format!("{}The SAS System\n\nbody\n", " ".repeat((40 - 14) / 2)));
    }
}
