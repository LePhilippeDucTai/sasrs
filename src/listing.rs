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
    /// TITLE1 text; None = default "The SAS System".
    pub title: Option<String>,
    wrote_anything: bool,
}

impl ListingWriter {
    pub fn new(ls: usize) -> Self {
        ListingWriter {
            buf: String::new(),
            ls,
            title: None,
            wrote_anything: false,
        }
    }

    pub fn into_string(self) -> String {
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

    /// Page header at the start of each proc's output.
    pub fn page_header(&mut self) {
        if self.wrote_anything {
            self.raw("");
        }
        self.wrote_anything = true;
        let title = self.title.clone().unwrap_or_else(|| "The SAS System".to_string());
        self.centered(&title);
        self.raw("");
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
}
