use super::*;

// ---------------------------------------------------------------------------
// RtfDestination — M23.1 : destination RTF réelle
// ---------------------------------------------------------------------------

/// Destination RTF (Rich Text Format). Génère un fichier RTF valide avec
/// tables et mise en forme de base.
pub struct RtfDestination {
    buf: String,
    titles: Vec<String>,
    footnotes: Vec<String>,
    ls: usize,
    file: Option<std::path::PathBuf>,
}

impl RtfDestination {
    /// Crée la destination RTF sans fichier cible.
    pub fn new(ls: usize) -> Self {
        RtfDestination {
            buf: String::new(),
            titles: Vec::new(),
            footnotes: Vec::new(),
            ls,
            file: None,
        }
    }

    /// Crée la destination RTF avec un fichier cible.
    pub fn with_file(ls: usize, file: std::path::PathBuf) -> Self {
        RtfDestination {
            buf: String::new(),
            titles: Vec::new(),
            footnotes: Vec::new(),
            ls,
            file: Some(file),
        }
    }

    /// Échappe les caractères spéciaux RTF.
    pub(super) fn rtf_escape(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 8);
        for c in s.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '{' => out.push_str("\\{"),
                '}' => out.push_str("\\}"),
                c if c.is_ascii() => out.push(c),
                c if (c as u32) <= 0xFF => {
                    out.push_str(&format!("\\'{:02x}", c as u32));
                }
                c => {
                    out.push_str(&format!("\\u{}?", c as u32));
                }
            }
        }
        out
    }
}

impl OutputDestination for RtfDestination {
    fn page_header(&mut self) {
        if self.titles.is_empty() {
            self.buf.push_str(&format!(
                "\\pard\\sb200\\sa100\\b {}\\b0\\par\n",
                Self::rtf_escape("The SAS System")
            ));
        } else {
            for t in &self.titles {
                self.buf.push_str(&format!(
                    "\\pard\\sb200\\sa100\\b {}\\b0\\par\n",
                    Self::rtf_escape(t)
                ));
            }
        }
    }

    fn write_table(&mut self, headers: &[String], aligns: &[Align], rows: &[Vec<String>]) {
        // Compute column widths in twips
        let col_widths: Vec<usize> = (0..headers.len())
            .map(|i| {
                let header_len = headers.get(i).map(|s| s.len()).unwrap_or(0);
                let max_data_len = rows
                    .iter()
                    .map(|r| r.get(i).map(|s| s.len()).unwrap_or(0))
                    .max()
                    .unwrap_or(0);
                (header_len.max(max_data_len) * 120).max(720)
            })
            .collect();

        // Build header row
        let mut cum_widths: Vec<usize> = Vec::with_capacity(col_widths.len());
        let mut cum = 0usize;
        for w in &col_widths {
            cum += w;
            cum_widths.push(cum);
        }

        // Helper closure to emit a row
        let emit_row = |buf: &mut String, cells: &[String], is_header: bool, aligns: &[Align]| {
            buf.push_str("\\trowd\\trgaph100");
            for cw in &cum_widths {
                buf.push_str(&format!("\\cellx{}", cw));
            }
            buf.push('\n');
            for (i, cell) in cells.iter().enumerate() {
                let align = aligns.get(i).copied().unwrap_or(Align::Left);
                let align_ctrl = match align {
                    Align::Right => "\\qr",
                    Align::Left => "\\ql",
                };
                if is_header {
                    buf.push_str(&format!(
                        "\\pard\\intbl{} \\b {}\\b0\\cell ",
                        align_ctrl,
                        RtfDestination::rtf_escape(cell)
                    ));
                } else {
                    buf.push_str(&format!(
                        "\\pard\\intbl{} {}\\cell ",
                        align_ctrl,
                        RtfDestination::rtf_escape(cell)
                    ));
                }
            }
            buf.push_str("\\row\n");
        };

        emit_row(&mut self.buf, headers, true, aligns);
        for row in rows {
            emit_row(&mut self.buf, row, false, aligns);
        }
        self.buf.push_str("\\pard\\par\n");
    }

    fn write_line(&mut self, line: &str) {
        self.buf
            .push_str(&format!("\\pard {}\\par\n", Self::rtf_escape(line)));
    }

    fn blank(&mut self) {
        self.buf.push_str("\\pard\\par\n");
    }

    fn set_titles(&mut self, titles: &[String]) {
        self.titles = titles.to_vec();
    }

    fn set_footnotes(&mut self, footnotes: &[String]) {
        self.footnotes = footnotes.to_vec();
    }

    fn set_ls(&mut self, ls: usize) {
        self.ls = ls;
    }

    fn ls(&self) -> usize {
        self.ls
    }

    fn take_string(&mut self) -> String {
        if self.buf.is_empty() {
            return String::new();
        }
        // Footnotes actives rendues (centrées) en fin de document.
        for f in &self.footnotes {
            self.buf
                .push_str(&format!("\\pard\\qc {}\\par\n", Self::rtf_escape(f)));
        }
        let body = std::mem::take(&mut self.buf);
        format!(
            "{{\\rtf1\\ansi\\ansicpg1252\\deff0\n{{\\fonttbl{{\\f0\\froman\\fcharset0 Times New Roman;}}}}\n\\f0\\fs24\n{body}}}"
        )
    }

    fn finalize(&mut self) -> Option<(std::path::PathBuf, String)> {
        let path = self.file.clone()?;
        let content = self.take_string();
        if content.is_empty() {
            None
        } else {
            Some((path, content))
        }
    }

    fn dest_type_label(&self) -> &'static str {
        "RTF Body"
    }
}
