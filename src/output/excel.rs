use super::xlsx::{ExcelTable, xlsx_build};
use super::*;

// ---------------------------------------------------------------------------
// ExcelDestination — M23.3 : destination Excel réelle (writer XLSX pur Rust)
// ---------------------------------------------------------------------------

/// Destination Excel (`ODS EXCEL`). Le writer XLSX (ZIP de fichiers XML,
/// sans dépendance externe) vit dans le module partagé `output::xlsx`
/// (J08-P1 — également utilisé par PROC EXPORT `DBMS=XLSX`). Le contenu est
/// accumulé en mémoire et matérialisé lors de `finalize_to_bytes()`.
pub struct ExcelDestination {
    page: PageState,
    file: Option<std::path::PathBuf>,
    tables: Vec<ExcelTable>,
    pending_lines: Vec<String>,
}

impl ExcelDestination {
    /// Crée la destination Excel sans fichier cible.
    pub fn new(ls: usize) -> Self {
        ExcelDestination {
            page: PageState {
                ls,
                ..Default::default()
            },
            file: None,
            tables: Vec::new(),
            pending_lines: Vec::new(),
        }
    }

    /// Crée la destination Excel avec un fichier cible.
    pub fn with_file(ls: usize, file: std::path::PathBuf) -> Self {
        ExcelDestination {
            page: PageState {
                ls,
                ..Default::default()
            },
            file: Some(file),
            tables: Vec::new(),
            pending_lines: Vec::new(),
        }
    }
}

impl OutputDestination for ExcelDestination {
    fn page_state(&self) -> &PageState {
        &self.page
    }

    fn page_state_mut(&mut self) -> &mut PageState {
        &mut self.page
    }

    fn page_header(&mut self) {
        // no-op : le titre/en-tête est géré par table
    }

    fn write_table(&mut self, headers: &[String], _aligns: &[Align], rows: &[Vec<String>]) {
        let sheet_name = format!("Table {}", self.tables.len() + 1);
        let pre_lines = std::mem::take(&mut self.pending_lines);
        self.tables.push(ExcelTable {
            sheet_name,
            pre_lines,
            headers: headers.to_vec(),
            rows: rows.to_vec(),
        });
    }

    fn write_line(&mut self, line: &str) {
        self.pending_lines.push(line.to_string());
    }

    fn blank(&mut self) {
        // no-op
    }

    fn take_string(&mut self) -> String {
        String::new()
    }

    fn finalize_to_bytes(&mut self) -> Option<(std::path::PathBuf, Vec<u8>)> {
        let path = self.file.clone()?;
        if self.tables.is_empty() && self.pending_lines.is_empty() {
            return None;
        }
        // M38.1 : rend les titres actifs en tête (avant la 1ʳᵉ table, ou comme
        // lignes libres s'il n'y a pas de table) et les footnotes en fin
        // (après la dernière table, ou comme lignes libres sinon). `xlsx_build`
        // n'affiche `pending_lines` que s'il n'y a aucune table, d'où l'ajout en
        // ligne (cellule unique) à la dernière table quand une table existe.
        let mut tables = self.tables.clone();
        let mut trailing = self.pending_lines.clone();
        if let Some(first) = tables.first_mut() {
            if !self.page.titles.is_empty() {
                let mut pre = self.page.titles.clone();
                pre.extend(std::mem::take(&mut first.pre_lines));
                first.pre_lines = pre;
            }
            if let Some(last) = tables.last_mut() {
                for f in &self.page.footnotes {
                    last.rows.push(vec![f.clone()]);
                }
            }
        } else {
            // Pas de table : titres puis lignes libres puis footnotes.
            let mut lines = self.page.titles.clone();
            lines.append(&mut trailing);
            lines.extend(self.page.footnotes.iter().cloned());
            trailing = lines;
        }
        let bytes = xlsx_build(&tables, &trailing);
        Some((path, bytes))
    }

    fn dest_type_label(&self) -> &'static str {
        "Excel"
    }
}
