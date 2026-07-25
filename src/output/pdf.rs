use super::*;

// ---------------------------------------------------------------------------
// PdfDestination — M23.2 : destination PDF pure Rust (PDF 1.4 minimal)
// ---------------------------------------------------------------------------

pub(super) enum PdfSection {
    PageHeader(String),
    Table { headers: Vec<String>, rows: Vec<Vec<String>> },
    Line(String),
    Blank,
}

/// Destination PDF (PDF 1.4 minimal, sans dépendance externe). Génère un
/// fichier PDF valide avec texte et tables simples.
pub struct PdfDestination {
    titles: Vec<String>,
    footnotes: Vec<String>,
    ls: usize,
    file: Option<std::path::PathBuf>,
    sections: Vec<PdfSection>,
}

impl PdfDestination {
    /// Crée la destination PDF sans fichier cible.
    pub fn new(ls: usize) -> Self {
        PdfDestination { titles: Vec::new(), footnotes: Vec::new(), ls, file: None, sections: Vec::new() }
    }

    /// Crée la destination PDF avec un fichier cible.
    pub fn with_file(ls: usize, file: std::path::PathBuf) -> Self {
        PdfDestination { titles: Vec::new(), footnotes: Vec::new(), ls, file: Some(file), sections: Vec::new() }
    }

    pub(super) fn pdf_escape(s: &str) -> String {
        s.chars().map(|c| match c {
            '(' => "\\(".to_string(),
            ')' => "\\)".to_string(),
            '\\' => "\\\\".to_string(),
            c if c.is_ascii() && c >= ' ' => c.to_string(),
            _ => "?".to_string(),
        }).collect()
    }

    pub(super) fn build_pdf_content(&self) -> String {
        let mut out = String::new();
        out.push_str("BT\n");

        let margin_x: f32 = 50.0;
        let mut y: f32 = 742.0;
        let line_h: f32 = 14.0;
        let col_gap: f32 = 6.0;

        // M38.1 : footnotes actives rendues (lignes simples) après le contenu.
        // Construites localement pour ne pas muter `self.sections` (finalize
        // idempotent : pas de duplication si appelé plusieurs fois).
        let footnote_sections: Vec<PdfSection> =
            self.footnotes.iter().cloned().map(PdfSection::Line).collect();
        for section in self.sections.iter().chain(footnote_sections.iter()) {
            match section {
                PdfSection::PageHeader(title) => {
                    out.push_str("/F1 14 Tf\n");
                    out.push_str(&format!("{:.1} {:.1} Tm\n", margin_x, y));
                    out.push_str(&format!("({}) Tj\n", Self::pdf_escape(title)));
                    y -= 20.0;
                }
                PdfSection::Line(text) => {
                    out.push_str("/F1 10 Tf\n");
                    out.push_str(&format!("{:.1} {:.1} Tm\n", margin_x, y));
                    out.push_str(&format!("({}) Tj\n", Self::pdf_escape(text)));
                    y -= line_h;
                }
                PdfSection::Blank => {
                    y -= line_h;
                }
                PdfSection::Table { headers, rows } => {
                    out.push_str("/F1 10 Tf\n");
                    let col_widths: Vec<f32> = (0..headers.len()).map(|i| {
                        let max_len = std::iter::once(headers.get(i).map(|s| s.len()).unwrap_or(0))
                            .chain(rows.iter().map(|r| r.get(i).map(|s| s.len()).unwrap_or(0)))
                            .max().unwrap_or(6);
                        (max_len as f32 * col_gap).max(50.0)
                    }).collect();

                    // Header row
                    let mut cx = margin_x;
                    for (i, header) in headers.iter().enumerate() {
                        out.push_str(&format!("{:.1} {:.1} Tm\n", cx, y));
                        out.push_str(&format!("({}) Tj\n", Self::pdf_escape(header)));
                        cx += col_widths.get(i).copied().unwrap_or(50.0);
                    }
                    y -= line_h;

                    // Data rows
                    for row in rows {
                        let mut cx = margin_x;
                        for (i, cell) in row.iter().enumerate() {
                            out.push_str(&format!("{:.1} {:.1} Tm\n", cx, y));
                            out.push_str(&format!("({}) Tj\n", Self::pdf_escape(cell)));
                            cx += col_widths.get(i).copied().unwrap_or(50.0);
                        }
                        y -= line_h;
                        if y < 50.0 { y = 742.0; }
                    }
                    y -= line_h;
                }
            }
        }

        out.push_str("ET\n");
        out
    }

    pub(super) fn build_pdf_document(content: String) -> Vec<u8> {
        let content_bytes = content.as_bytes().len();

        let obj1 = "<<\n/Type /Catalog\n/Pages 2 0 R\n>>".to_string();
        let obj2 = "<<\n/Type /Pages\n/Kids [3 0 R]\n/Count 1\n>>".to_string();
        let obj3 = "<<\n/Type /Page\n/Parent 2 0 R\n/MediaBox [0 0 612 792]\n/Contents 4 0 R\n/Resources <<\n/Font <<\n/F1 5 0 R\n>>\n>>\n>>".to_string();
        let obj4 = format!("<<\n/Length {}\n>>\nstream\n{}\nendstream", content_bytes, content);
        let obj5 = "<<\n/Type /Font\n/Subtype /Type1\n/BaseFont /Helvetica\n>>".to_string();

        let objects: Vec<(usize, String)> = vec![
            (1, obj1), (2, obj2), (3, obj3), (4, obj4), (5, obj5),
        ];

        let mut pdf: Vec<u8> = Vec::new();
        let header = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n";
        pdf.extend_from_slice(header);

        let mut offsets: Vec<usize> = Vec::new();
        for (obj_num, body) in &objects {
            offsets.push(pdf.len());
            let obj_str = format!("{} 0 obj\n{}\nendobj\n", obj_num, body);
            pdf.extend_from_slice(obj_str.as_bytes());
        }

        let xref_offset = pdf.len();
        let xref_header = format!("xref\n0 {}\n", objects.len() + 1);
        pdf.extend_from_slice(xref_header.as_bytes());
        // free entry
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in &offsets {
            pdf.extend_from_slice(format!("{:010} 00000 n \n", offset).as_bytes());
        }

        let trailer = format!(
            "trailer\n<<\n/Size {}\n/Root 1 0 R\n>>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            xref_offset
        );
        pdf.extend_from_slice(trailer.as_bytes());

        pdf
    }
}

impl OutputDestination for PdfDestination {
    fn page_header(&mut self) {
        if self.titles.is_empty() {
            self.sections.push(PdfSection::PageHeader("The SAS System".to_string()));
        } else {
            for t in self.titles.clone() {
                self.sections.push(PdfSection::PageHeader(t));
            }
        }
    }

    fn write_table(&mut self, headers: &[String], _aligns: &[Align], rows: &[Vec<String>]) {
        self.sections.push(PdfSection::Table {
            headers: headers.to_vec(),
            rows: rows.to_vec(),
        });
    }

    fn write_line(&mut self, line: &str) {
        self.sections.push(PdfSection::Line(line.to_string()));
    }

    fn blank(&mut self) {
        self.sections.push(PdfSection::Blank);
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

    fn into_string(&mut self) -> String {
        String::new()
    }

    fn finalize_to_bytes(&mut self) -> Option<(std::path::PathBuf, Vec<u8>)> {
        let path = self.file.clone()?;
        if self.sections.is_empty() {
            return None;
        }
        let content = self.build_pdf_content();
        let bytes = Self::build_pdf_document(content);
        Some((path, bytes))
    }

    fn dest_type_label(&self) -> &'static str {
        "PDF"
    }
}
