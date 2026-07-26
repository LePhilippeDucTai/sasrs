use super::*;

// ---------------------------------------------------------------------------
// ExcelDestination — M23.3 : destination Excel réelle (rust_xlsxwriter)
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(super) struct ExcelTable {
    pub(super) sheet_name: String,
    pub(super) pre_lines: Vec<String>,
    pub(super) headers: Vec<String>,
    pub(super) rows: Vec<Vec<String>>,
}

/// Destination Excel (`ODS EXCEL`). Utilise `rust_xlsxwriter` pour générer
/// un fichier `.xlsx` valide. Le contenu est accumulé en mémoire et matérialisé
/// lors de `finalize_to_bytes()`.
pub struct ExcelDestination {
    titles: Vec<String>,
    footnotes: Vec<String>,
    ls: usize,
    file: Option<std::path::PathBuf>,
    tables: Vec<ExcelTable>,
    pending_lines: Vec<String>,
}

impl ExcelDestination {
    /// Crée la destination Excel sans fichier cible.
    pub fn new(ls: usize) -> Self {
        ExcelDestination {
            titles: Vec::new(),
            footnotes: Vec::new(),
            ls,
            file: None,
            tables: Vec::new(),
            pending_lines: Vec::new(),
        }
    }

    /// Crée la destination Excel avec un fichier cible.
    pub fn with_file(ls: usize, file: std::path::PathBuf) -> Self {
        ExcelDestination {
            titles: Vec::new(),
            footnotes: Vec::new(),
            ls,
            file: Some(file),
            tables: Vec::new(),
            pending_lines: Vec::new(),
        }
    }
}

impl OutputDestination for ExcelDestination {
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
            if !self.titles.is_empty() {
                let mut pre = self.titles.clone();
                pre.extend(std::mem::take(&mut first.pre_lines));
                first.pre_lines = pre;
            }
            if let Some(last) = tables.last_mut() {
                for f in &self.footnotes {
                    last.rows.push(vec![f.clone()]);
                }
            }
        } else {
            // Pas de table : titres puis lignes libres puis footnotes.
            let mut lines = self.titles.clone();
            lines.append(&mut trailing);
            lines.extend(self.footnotes.iter().cloned());
            trailing = lines;
        }
        let bytes = xlsx_build(&tables, &trailing);
        Some((path, bytes))
    }

    fn dest_type_label(&self) -> &'static str {
        "Excel"
    }
}

// ---------------------------------------------------------------------------
// XLSX writer pur Rust — utilisé par ExcelDestination (M23.3)
// Produit un fichier XLSX (ZIP de fichiers XML) sans dépendance externe.
// ---------------------------------------------------------------------------

/// Référence de colonne Excel (0→"A", 25→"Z", 26→"AA", …).
pub(super) fn xlsx_col_ref(mut n: usize) -> String {
    let mut s = String::new();
    loop {
        s.push(char::from(b'A' + (n % 26) as u8));
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    s.chars().rev().collect()
}

/// Échappe un contenu pour l'insérer dans un attribut ou texte XML.
pub(super) fn xlsx_xml_escape(v: &str) -> String {
    v.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Génère le XML d'une feuille (`xl/worksheets/sheetN.xml`).
pub(super) fn xlsx_sheet_xml(
    pre_lines: &[String],
    headers: &[String],
    rows: &[Vec<String>],
) -> Vec<u8> {
    let mut x = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
        <sheetData>",
    );
    let mut r = 1usize;
    for line in pre_lines {
        x.push_str(&format!(
            "<row r=\"{r}\"><c r=\"A{r}\" t=\"inlineStr\"><is><t>{}</t></is></c></row>",
            xlsx_xml_escape(line)
        ));
        r += 1;
    }
    if !headers.is_empty() {
        x.push_str(&format!("<row r=\"{r}\">"));
        for (c, h) in headers.iter().enumerate() {
            x.push_str(&format!(
                "<c r=\"{}{r}\" t=\"inlineStr\"><is><t>{}</t></is></c>",
                xlsx_col_ref(c),
                xlsx_xml_escape(h)
            ));
        }
        x.push_str("</row>");
        r += 1;
    }
    for row in rows {
        x.push_str(&format!("<row r=\"{r}\">"));
        for (c, v) in row.iter().enumerate() {
            x.push_str(&format!(
                "<c r=\"{}{r}\" t=\"inlineStr\"><is><t>{}</t></is></c>",
                xlsx_col_ref(c),
                xlsx_xml_escape(v)
            ));
        }
        x.push_str("</row>");
        r += 1;
    }
    x.push_str("</sheetData></worksheet>");
    x.into_bytes()
}

/// CRC-32 variante ZIP (polynôme 0xEDB88320).
pub(super) fn crc32_zip(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        let idx = ((crc ^ b as u32) & 0xFF) as usize;
        // Calcule le coefficient à la volée pour éviter une table statique globale.
        let mut coeff = idx as u32;
        for _ in 0..8 {
            coeff = if coeff & 1 != 0 {
                0xEDB88320 ^ (coeff >> 1)
            } else {
                coeff >> 1
            };
        }
        crc = coeff ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

pub(super) fn zip_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}
pub(super) fn zip_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

/// Construit un ZIP sans compression (store) à partir de paires (nom, octets).
pub(super) fn build_zip_stored(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut offsets: Vec<u32> = Vec::new();
    let mut crcs: Vec<u32> = Vec::new();

    // Enregistrements locaux
    for (name, data) in entries.iter() {
        let crc = crc32_zip(data);
        crcs.push(crc);
        offsets.push(out.len() as u32);
        let nb = name.as_bytes();
        zip_u32(&mut out, 0x04034B50); // local file header signature
        zip_u16(&mut out, 20); // version needed
        zip_u16(&mut out, 0); // flags
        zip_u16(&mut out, 0); // compression = store
        zip_u16(&mut out, 0); // mod time
        zip_u16(&mut out, 0); // mod date
        zip_u32(&mut out, crc);
        zip_u32(&mut out, data.len() as u32); // compressed size
        zip_u32(&mut out, data.len() as u32); // uncompressed size
        zip_u16(&mut out, nb.len() as u16);
        zip_u16(&mut out, 0); // extra field length
        out.extend_from_slice(nb);
        out.extend_from_slice(data);
    }

    // Répertoire central
    let cd_start = out.len() as u32;
    for (i, (name, data)) in entries.iter().enumerate() {
        let nb = name.as_bytes();
        zip_u32(&mut out, 0x02014B50); // central dir signature
        zip_u16(&mut out, 20); // version made by
        zip_u16(&mut out, 20); // version needed
        zip_u16(&mut out, 0);
        zip_u16(&mut out, 0); // compression
        zip_u16(&mut out, 0);
        zip_u16(&mut out, 0);
        zip_u32(&mut out, crcs[i]);
        zip_u32(&mut out, data.len() as u32);
        zip_u32(&mut out, data.len() as u32);
        zip_u16(&mut out, nb.len() as u16);
        zip_u16(&mut out, 0); // extra length
        zip_u16(&mut out, 0); // comment length
        zip_u16(&mut out, 0); // disk start
        zip_u16(&mut out, 0); // internal attrs
        zip_u32(&mut out, 0); // external attrs
        zip_u32(&mut out, offsets[i]);
        out.extend_from_slice(nb);
    }
    let cd_end = out.len() as u32;

    // End of central directory
    zip_u32(&mut out, 0x06054B50);
    zip_u16(&mut out, 0);
    zip_u16(&mut out, 0);
    zip_u16(&mut out, entries.len() as u16);
    zip_u16(&mut out, entries.len() as u16);
    zip_u32(&mut out, cd_end - cd_start);
    zip_u32(&mut out, cd_start);
    zip_u16(&mut out, 0); // comment length

    out
}

/// Construit un fichier XLSX complet pour les tables et lignes libres données.
pub(super) fn xlsx_build(tables: &[ExcelTable], pending_lines: &[String]) -> Vec<u8> {
    // Feuilles : une par table, ou une feuille vide/texte si pas de tables.
    let mut sheets: Vec<(String, Vec<u8>)> = Vec::new();
    if tables.is_empty() {
        sheets.push(("Sheet1".into(), xlsx_sheet_xml(pending_lines, &[], &[])));
    } else {
        for t in tables {
            sheets.push((
                t.sheet_name.clone(),
                xlsx_sheet_xml(&t.pre_lines, &t.headers, &t.rows),
            ));
        }
    }
    let n = sheets.len();

    // [Content_Types].xml
    let mut ct = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
        <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
        <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
        <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>",
    );
    for i in 1..=n {
        ct.push_str(&format!(
            "<Override PartName=\"/xl/worksheets/sheet{i}.xml\" \
             ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"
        ));
    }
    ct.push_str("</Types>");

    // _rels/.rels
    let rels = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" \
        Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" \
        Target=\"xl/workbook.xml\"/></Relationships>";

    // xl/workbook.xml
    let mut wb = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
        xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets>",
    );
    for (i, (name, _)) in sheets.iter().enumerate() {
        let id = i + 1;
        wb.push_str(&format!(
            "<sheet name=\"{}\" sheetId=\"{id}\" r:id=\"rId{id}\"/>",
            xlsx_xml_escape(name)
        ));
    }
    wb.push_str("</sheets></workbook>");

    // xl/_rels/workbook.xml.rels
    let mut wb_rels = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for i in 1..=n {
        wb_rels.push_str(&format!(
            "<Relationship Id=\"rId{i}\" \
             Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" \
             Target=\"worksheets/sheet{i}.xml\"/>"
        ));
    }
    wb_rels.push_str("</Relationships>");

    // Assemblage ZIP
    let mut zip_entries: Vec<(&str, Vec<u8>)> = vec![
        ("[Content_Types].xml", ct.into_bytes()),
        ("_rels/.rels", rels.as_bytes().to_vec()),
        ("xl/workbook.xml", wb.into_bytes()),
        ("xl/_rels/workbook.xml.rels", wb_rels.into_bytes()),
    ];
    // Les noms des feuilles doivent vivre assez longtemps pour la construction.
    let sheet_names: Vec<String> = (1..=n)
        .map(|i| format!("xl/worksheets/sheet{i}.xml"))
        .collect();
    for (i, (_, xml_bytes)) in sheets.into_iter().enumerate() {
        zip_entries.push((sheet_names[i].as_str(), xml_bytes));
    }

    build_zip_stored(&zip_entries)
}
