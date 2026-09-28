//! Écrivain XLSX pur Rust partagé (J08-P1).
//!
//! Historiquement le writer XLSX vivait dans `excel.rs` (destination ODS
//! EXCEL, M23.3) et ne produisait que des cellules `inlineStr`. PROC EXPORT
//! `DBMS=XLSX` (J08-P1) réutilise ce writer mais a besoin de cellules TYPÉES
//! (numériques, dates avec style) pour garantir un aller-retour EXPORT →
//! IMPORT sans perte : le module est donc extrait ici et expose deux points
//! d'entrée :
//!
//! - [`xlsx_build`] : cellules chaînes pures (destination ODS EXCEL —
//!   comportement inchangé, octet-identique aux snapshots existants) ;
//! - [`xlsx_build_typed`] : cellules typées ([`XlsxCell`]) pour PROC EXPORT,
//!   avec `styles.xml` portant un format de date pour les cellules
//!   [`XlsxCell::Date`] (relues comme dates par calamine à l'import).
//!
//! ## Époques Excel ↔ SAS
//! Excel compte les jours depuis le 1899-12-30 (sérial 25569 = 1970-01-01),
//! SAS depuis le 1960-01-01 (3653 = 1970-01-01). D'où l'offset
//! [`EXCEL_SAS_EPOCH_OFFSET_DAYS`] = 25569 − 3653 = 21916 jours.

/// Offset en jours entre l'époque Excel (1899-12-30) et l'époque SAS
/// (1960-01-01) : `sas_days = excel_serial − 21916`.
pub(crate) const EXCEL_SAS_EPOCH_OFFSET_DAYS: f64 = 21916.0;

/// Cellule typée pour [`xlsx_build_typed`] (PROC EXPORT DBMS=XLSX).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum XlsxCell {
    /// Cellule chaîne (`inlineStr`).
    Str(String),
    /// Cellule numérique brute.
    Num(f64),
    /// Cellule date/heure : sérial Excel (jours depuis 1899-12-30), portant
    /// le style de date `s="1"`.
    Date(f64),
}

/// Cellule manquante (SAS missing) → `None` dans les lignes typées.
pub(crate) type XlsxRow = Vec<Option<XlsxCell>>;

/// Feuille typée pour [`xlsx_build_typed`].
pub(crate) struct XlsxSheet {
    /// Nom de la feuille (≤ 31 chars, sans `[]:*?/\`).
    pub(crate) name: String,
    /// En-têtes (noms de variables SAS).
    pub(crate) headers: Vec<String>,
    /// Lignes de données ; `None` = cellule vide (missing SAS).
    pub(crate) rows: Vec<XlsxRow>,
}

/// Table Excel de la destination ODS (`ExcelDestination`, M23.3) : contenu
/// accumulé en chaînes pré-formatées, matérialisé par `finalize_to_bytes`.
#[derive(Clone)]
pub(super) struct ExcelTable {
    pub(super) sheet_name: String,
    pub(super) pre_lines: Vec<String>,
    pub(super) headers: Vec<String>,
    pub(super) rows: Vec<Vec<String>>,
}

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

/// Génère le XML d'une feuille (`xl/worksheets/sheetN.xml`) — cellules
/// chaînes uniquement (destination ODS EXCEL).
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

/// Génère le XML d'une feuille typée : en-têtes `inlineStr`, puis cellules
/// numériques (`<v>`), dates (`<v>` + style `s="1"`) ou chaînes. Les cellules
/// `None` (missings SAS) sont omises — cellules vides à la relecture.
fn xlsx_sheet_xml_typed(sheet: &XlsxSheet) -> Vec<u8> {
    let mut x = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
        <sheetData>",
    );
    let mut r = 1usize;
    if !sheet.headers.is_empty() {
        x.push_str(&format!("<row r=\"{r}\">"));
        for (c, h) in sheet.headers.iter().enumerate() {
            x.push_str(&format!(
                "<c r=\"{}{r}\" t=\"inlineStr\"><is><t>{}</t></is></c>",
                xlsx_col_ref(c),
                xlsx_xml_escape(h)
            ));
        }
        x.push_str("</row>");
        r += 1;
    }
    for row in &sheet.rows {
        x.push_str(&format!("<row r=\"{r}\">"));
        for (c, cell) in row.iter().enumerate() {
            let ref_attr = format!("{}{r}", xlsx_col_ref(c));
            match cell {
                None => {}
                Some(XlsxCell::Str(s)) => x.push_str(&format!(
                    "<c r=\"{ref_attr}\" t=\"inlineStr\"><is><t>{}</t></is></c>",
                    xlsx_xml_escape(s)
                )),
                Some(XlsxCell::Num(v)) => {
                    x.push_str(&format!("<c r=\"{ref_attr}\"><v>{v}</v></c>"));
                }
                Some(XlsxCell::Date(serial)) => {
                    x.push_str(&format!("<c r=\"{ref_attr}\" s=\"1\"><v>{serial}</v></c>"));
                }
            }
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

/// Assemble le ZIP XLSX complet (feuilles + classeur + relations). Les
/// feuilles sont données sous forme de XML déjà généré + nom.
fn assemble_workbook(sheets: Vec<(String, Vec<u8>)>, styles_xml: Option<Vec<u8>>) -> Vec<u8> {
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
    if styles_xml.is_some() {
        ct.push_str(
            "<Override PartName=\"/xl/styles.xml\" \
             ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>",
        );
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

    // xl/_rels/workbook.xml.rels — les feuilles puis (option) les styles.
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
    if styles_xml.is_some() {
        wb_rels.push_str(&format!(
            "<Relationship Id=\"rId{}\" \
             Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" \
             Target=\"styles.xml\"/>",
            n + 1
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
    if let Some(styles) = styles_xml {
        zip_entries.push(("xl/styles.xml", styles));
    }
    // Les noms des feuilles doivent vivre assez longtemps pour la construction.
    let sheet_names: Vec<String> = (1..=n)
        .map(|i| format!("xl/worksheets/sheet{i}.xml"))
        .collect();
    for (i, (_, xml_bytes)) in sheets.into_iter().enumerate() {
        zip_entries.push((sheet_names[i].as_str(), xml_bytes));
    }

    build_zip_stored(&zip_entries)
}

/// Construit un fichier XLSX complet pour les tables et lignes libres données
/// (destination ODS EXCEL — cellules chaînes, sans styles).
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
    assemble_workbook(sheets, None)
}

/// `xl/styles.xml` minimal : le style `1` (`cellXfs`) applique le format de
/// nombre date `yyyy-mm-dd` (numFmt custom 164). C'est ce style qui fait
/// qu'un sérial Excel écrit par PROC EXPORT est relu comme une DATE par
/// calamine à l'import (aller-retour sans perte).
fn xlsx_styles_xml() -> Vec<u8> {
    const XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
        <numFmts count=\"1\"><numFmt numFmtId=\"164\" formatCode=\"yyyy-mm-dd\"/></numFmts>\
        <fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>\
        <fills count=\"1\"><fill><patternFill patternType=\"none\"/></fill></fills>\
        <borders count=\"1\"><border/></borders>\
        <cellStyleXfs count=\"1\"><xf numFmtId=\"0\"/></cellStyleXfs>\
        <cellXfs count=\"2\">\
        <xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/>\
        <xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"0\" applyNumberFormat=\"1\"/>\
        </cellXfs>\
        </styleSheet>";
    XML.as_bytes().to_vec()
}

/// Construit un fichier XLSX complet à partir de feuilles typées (PROC EXPORT
/// `DBMS=XLSX`) : cellules numériques / dates stylées / chaînes, plus le
/// `styles.xml` portant le format de date.
pub(crate) fn xlsx_build_typed(sheets: &[XlsxSheet]) -> Vec<u8> {
    let rendered: Vec<(String, Vec<u8>)> = sheets
        .iter()
        .map(|s| (s.name.clone(), xlsx_sheet_xml_typed(s)))
        .collect();
    assemble_workbook(rendered, Some(xlsx_styles_xml()))
}

/// Convertit des jours SAS (depuis 1960-01-01) en sérial Excel.
pub(crate) fn sas_days_to_excel_serial(sas_days: f64) -> f64 {
    sas_days + EXCEL_SAS_EPOCH_OFFSET_DAYS
}

/// Convertit des secondes SAS (datetime, depuis 1960-01-01) en sérial Excel
/// (jours fractionnaires depuis 1899-12-30).
pub(crate) fn sas_seconds_to_excel_serial(sas_seconds: f64) -> f64 {
    sas_seconds / 86400.0 + EXCEL_SAS_EPOCH_OFFSET_DAYS
}

/// Convertit un sérial Excel en jours SAS (depuis 1960-01-01).
pub(crate) fn excel_serial_to_sas_days(excel_serial: f64) -> f64 {
    excel_serial - EXCEL_SAS_EPOCH_OFFSET_DAYS
}

/// Convertit un sérial Excel en secondes SAS (datetime, depuis 1960-01-01).
pub(crate) fn excel_serial_to_sas_seconds(excel_serial: f64) -> f64 {
    (excel_serial - EXCEL_SAS_EPOCH_OFFSET_DAYS) * 86400.0
}

// ---------------------------------------------------------------------------
// Tests — conversions d'époque et writer typé (J08-P1)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_conversions_roundtrip() {
        // 1970-01-01 : sérial Excel 25569, jours SAS 3653.
        assert_eq!(sas_days_to_excel_serial(3653.0), 25569.0);
        assert_eq!(excel_serial_to_sas_days(25569.0), 3653.0);
        // 1960-01-01 : sérial Excel 21916, jours SAS 0.
        assert_eq!(sas_days_to_excel_serial(0.0), 21916.0);
        assert_eq!(excel_serial_to_sas_days(21916.0), 0.0);
        // Datetime : midi le 1970-01-01 → 3653 jours + 43200 s.
        assert!((sas_seconds_to_excel_serial(3653.0 * 86400.0 + 43200.0) - 25569.5).abs() < 1e-9);
        assert!((excel_serial_to_sas_seconds(25569.5) - (3653.0 * 86400.0 + 43200.0)).abs() < 1e-6);
    }

    #[test]
    fn typed_build_contains_numeric_and_date_cells() {
        let sheet = XlsxSheet {
            name: "T".into(),
            headers: vec!["x".into(), "d".into()],
            rows: vec![vec![
                Some(XlsxCell::Num(42.0)),
                Some(XlsxCell::Date(25569.0)),
            ]],
        };
        let bytes = xlsx_build_typed(&[sheet]);
        assert!(bytes.starts_with(b"PK"), "XLSX doit commencer par PK");
        let blob = String::from_utf8_lossy(&bytes);
        assert!(
            blob.contains("<v>42</v>"),
            "cellule numérique absente : {blob}"
        );
        assert!(
            blob.contains("s=\"1\"><v>25569</v>"),
            "cellule date stylée absente : {blob}"
        );
        assert!(
            blob.contains("numFmtId=\"164\""),
            "styles.xml absent : {blob}"
        );
        assert!(blob.contains("yyyy-mm-dd"), "format date absent : {blob}");
    }

    #[test]
    fn typed_build_missing_cell_omitted() {
        let sheet = XlsxSheet {
            name: "T".into(),
            headers: vec!["x".into()],
            rows: vec![vec![None], vec![Some(XlsxCell::Num(1.0))]],
        };
        let bytes = xlsx_build_typed(&[sheet]);
        let blob = String::from_utf8_lossy(&bytes);
        // Ligne 2 (première de données) : aucune cellule, juste <row r="2"/>.
        assert!(
            blob.contains("<row r=\"2\"></row>"),
            "row vide absent : {blob}"
        );
    }

    #[test]
    fn col_ref_beyond_z() {
        assert_eq!(xlsx_col_ref(0), "A");
        assert_eq!(xlsx_col_ref(25), "Z");
        assert_eq!(xlsx_col_ref(26), "AA");
        assert_eq!(xlsx_col_ref(701), "ZZ");
        assert_eq!(xlsx_col_ref(702), "AAA");
    }
}
