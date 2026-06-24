//! ODS routing layer (M22.1) — `OutputDestination` trait + destinations.
//!
//! M22 introduit la couche de routage ODS (Output Delivery System). Toute
//! sortie de résultats (titres de page, tables de procs, lignes de texte) passe
//! désormais par le trait [`OutputDestination`], ce qui permet de router la même
//! sortie vers plusieurs destinations (listing texte, HTML, RTF, PDF, Excel).
//!
//! ## Périmètre M22.1
//! - [`OutputDestination`] : trait de destination (page_header / write_table /
//!   write_line / blank + accès au titre et à la LINESIZE, drain final).
//! - [`TextListing`] : destination texte par défaut. C'est un mince *adaptateur*
//!   au-dessus de [`crate::listing::ListingWriter`] (le rendu texte prouvé
//!   octet-identique de M1–M21) — AUCUNE logique de mise en forme n'est
//!   réécrite ici, on délègue verbatim. Invariant CRITIQUE : le listing texte
//!   par défaut reste **octet-identique** aux snapshots m1–m21.
//! - [`HtmlDestination`], [`RtfDestination`], [`PdfDestination`],
//!   [`ExcelDestination`] : stubs no-op qui implémentent le trait, remplis en
//!   M22.4 / M23.
//!
//! ## Choix de signature de `write_table`
//! Le plan M22 esquisse `write_table(df: &DataFrame, vars: &[VarMeta])`. En
//! pratique les ~31 sites d'appel existants (PROC PRINT/MEANS/CORR/REPORT/…)
//! fournissent déjà des cellules **pré-formatées** (en-têtes, alignements,
//! lignes de chaînes) — c'est ce contrat, prouvé octet-identique, que le trait
//! expose ici. La variante DataFrame pourra être ajoutée plus tard pour les
//! destinations riches (HTML/Excel) sans casser ce chemin. Réexporté pour les
//! destinations : [`Align`].

pub use crate::listing::Align;
use crate::listing::ListingWriter;

/// Une destination de sortie ODS. Reçoit les résultats déjà mis en forme
/// (cellules de table, lignes de texte) et les matérialise selon son format.
///
/// Le listing texte ([`TextListing`]) est la destination par défaut ; les
/// destinations HTML/RTF/PDF/Excel partagent le même trait et seront branchées
/// par le statement `ODS` (M22.2+).
pub trait OutputDestination {
    /// En-tête de page au début de la sortie d'un proc (titre centré + ligne
    /// blanche). Insère une ligne blanche de séparation si du contenu a déjà
    /// été écrit.
    fn page_header(&mut self);

    /// Rend une table de résultats : en-têtes, alignement par colonne, lignes
    /// de cellules (toutes pré-formatées en chaînes par l'appelant).
    fn write_table(&mut self, headers: &[String], aligns: &[Align], rows: &[Vec<String>]);

    /// Variante PROC PRINT (M33.6) : double-interligne optionnel et ligne de
    /// totaux optionnelle. L'implémentation par défaut (destinations ODS et
    /// stubs) ignore ces extensions et rend la table normalement, suivie de la
    /// ligne de totaux si présente (sans alignement colonne). Seule la
    /// destination texte ([`TextListing`]) surcharge cette méthode pour aligner
    /// les totaux sous leurs colonnes — l'invariant byte-identique du listing
    /// par défaut est ainsi préservé.
    fn write_table_ext(
        &mut self,
        headers: &[String],
        aligns: &[Align],
        rows: &[Vec<String>],
        _double: bool,
        totals: Option<&Vec<String>>,
    ) {
        self.write_table(headers, aligns, rows);
        if let Some(t) = totals {
            self.write_line(&t.join("  "));
        }
    }

    /// Écrit une ligne de texte libre (justifiée à gauche, colonne 0).
    fn write_line(&mut self, line: &str);

    /// Émet une ligne vide.
    fn blank(&mut self);

    /// Pose les titres actifs (TITLE1..TITLE9), dans l'ordre des niveaux, gaps
    /// retirés. Vide = défaut « The SAS System ».
    fn set_titles(&mut self, titles: &[String]);

    /// Pose les footnotes actives (FOOTNOTE1..FOOTNOTE9), dans l'ordre des
    /// niveaux, gaps retirés. Vide = aucune footnote. Implémentation par défaut
    /// no-op (destinations qui ne rendent pas encore les footnotes).
    fn set_footnotes(&mut self, _footnotes: &[String]) {}

    /// Pose le titre courant (TITLE1). `None` = défaut « The SAS System ».
    /// Compatibilité : délègue à [`set_titles`](Self::set_titles).
    fn set_title(&mut self, title: Option<String>) {
        match title {
            None => self.set_titles(&[]),
            Some(t) => self.set_titles(std::slice::from_ref(&t)),
        }
    }

    /// Pose la LINESIZE (LS=) servant à centrer la sortie.
    fn set_ls(&mut self, ls: usize);

    /// Lit la LINESIZE courante (certains procs en ont besoin pour leur propre
    /// mise en page).
    fn ls(&self) -> usize;

    /// Draine la sortie accumulée sous forme de chaîne, laissant la destination
    /// vide. Pour le listing texte c'est le contenu rendu ; pour les
    /// destinations fichier (à venir) ce sera typiquement vide (déjà écrit sur
    /// disque). Remplace l'ancien `ListingWriter::into_string` (qui consommait
    /// `self`, impossible derrière un `Box<dyn …>`).
    fn into_string(&mut self) -> String;

    /// Finalise la destination : si elle cible un fichier, renvoie
    /// `Some((path, contenu))` pour que l'appelant écrive le fichier sur disque.
    /// La valeur par défaut (listing texte et stubs) renvoie `None`.
    fn finalize(&mut self) -> Option<(std::path::PathBuf, String)> {
        None
    }

    /// Finalise pour les formats binaires (Excel, PDF) : retourne les octets
    /// du fichier à écrire sur disque. Défaut : None (format texte ou pas de
    /// fichier). Les destinations binaires DOIVENT implémenter cette méthode
    /// plutôt que `finalize()`.
    fn finalize_to_bytes(&mut self) -> Option<(std::path::PathBuf, Vec<u8>)> {
        None
    }

    /// Étiquette du type de destination pour les messages de log (NOTE "Writing
    /// <label> file: …"). Chaque destination surcharge cette méthode.
    fn dest_type_label(&self) -> &'static str {
        "HTML Body"
    }
}

/// Destination texte par défaut : adaptateur au-dessus de [`ListingWriter`].
///
/// Délègue verbatim au rendu texte historique de M1–M21 ⇒ sortie
/// octet-identique. Aucune mise en forme n'est dupliquée ici.
pub struct TextListing {
    inner: ListingWriter,
}

impl TextListing {
    /// Crée une destination texte avec la LINESIZE donnée.
    pub fn new(ls: usize) -> Self {
        TextListing {
            inner: ListingWriter::new(ls),
        }
    }
}

impl OutputDestination for TextListing {
    fn page_header(&mut self) {
        self.inner.page_header();
    }

    fn write_table(&mut self, headers: &[String], aligns: &[Align], rows: &[Vec<String>]) {
        self.inner.write_table(headers, aligns, rows);
    }

    fn write_table_ext(
        &mut self,
        headers: &[String],
        aligns: &[Align],
        rows: &[Vec<String>],
        double: bool,
        totals: Option<&Vec<String>>,
    ) {
        self.inner.write_table_ext(headers, aligns, rows, double, totals);
    }

    fn write_line(&mut self, line: &str) {
        self.inner.write_line(line);
    }

    fn blank(&mut self) {
        self.inner.blank();
    }

    fn set_titles(&mut self, titles: &[String]) {
        self.inner.titles = titles.to_vec();
    }

    fn set_footnotes(&mut self, footnotes: &[String]) {
        self.inner.footnotes = footnotes.to_vec();
    }

    fn set_ls(&mut self, ls: usize) {
        self.inner.ls = ls;
    }

    fn ls(&self) -> usize {
        self.inner.ls
    }

    fn into_string(&mut self) -> String {
        // Remplace le writer interne par un writer vide de même LINESIZE et
        // rend la chaîne accumulée. Équivalent à l'ancien `into_string` qui
        // consommait `self`, mais utilisable derrière un trait object. Les
        // titres/footnotes actifs survivent au drain (un proc qui suit les
        // réutilise tant qu'aucun nouveau statement TITLE/FOOTNOTE ne les change).
        let ls = self.inner.ls;
        let titles = self.inner.titles.clone();
        let footnotes = self.inner.footnotes.clone();
        let mut fresh = ListingWriter::new(ls);
        fresh.titles = titles;
        fresh.footnotes = footnotes;
        let old = std::mem::replace(&mut self.inner, fresh);
        old.into_string()
    }
}

// ---------------------------------------------------------------------------
// HtmlDestination — M22.4 : destination HTML réelle (tables CSS + fichier)
// ---------------------------------------------------------------------------

/// Destination HTML (tables CSS, fichier `.html`).
///
/// Génère du HTML valide avec une feuille de style CSS embarquée. La sortie
/// est accumulée en mémoire dans `buf` puis drainée par [`into_string`] soit
/// explicitement (via le trait [`OutputDestination`]), soit implicitement lors
/// d'un [`close_destination`] via [`finalize`].
///
/// Cycle de vie :
/// - `new(ls)` : pas de fichier cible → `finalize()` renvoie `None`.
/// - `with_file(ls, path)` : fichier cible → `finalize()` renvoie
///   `Some((path, html_complet))`.
pub struct HtmlDestination {
    buf: String,
    titles: Vec<String>,
    footnotes: Vec<String>,
    ls: usize,
    file: Option<std::path::PathBuf>,
    wrote_anything: bool,
}

impl HtmlDestination {
    /// Crée la destination HTML sans fichier cible (sortie en mémoire seulement).
    pub fn new(ls: usize) -> Self {
        HtmlDestination {
            buf: String::new(),
            titles: Vec::new(),
            footnotes: Vec::new(),
            ls,
            file: None,
            wrote_anything: false,
        }
    }

    /// Crée la destination HTML avec un fichier cible.
    pub fn with_file(ls: usize, file: std::path::PathBuf) -> Self {
        HtmlDestination {
            buf: String::new(),
            titles: Vec::new(),
            footnotes: Vec::new(),
            ls,
            file: Some(file),
            wrote_anything: false,
        }
    }

    /// Échappe les caractères HTML spéciaux.
    ///
    /// L'ordre est critique : `&` doit être traité EN PREMIER pour éviter de
    /// ré-échapper les séquences `&amp;` produites ensuite.
    fn html_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }
}

impl OutputDestination for HtmlDestination {
    fn page_header(&mut self) {
        if self.titles.is_empty() {
            self.buf.push_str(&format!(
                "<h1 class=\"systitle\">{}</h1>\n",
                Self::html_escape("The SAS System")
            ));
        } else {
            for t in &self.titles {
                self.buf.push_str(&format!(
                    "<h1 class=\"systitle\">{}</h1>\n",
                    Self::html_escape(t)
                ));
            }
        }
        self.wrote_anything = true;
    }

    fn write_table(&mut self, headers: &[String], aligns: &[Align], rows: &[Vec<String>]) {
        self.buf.push_str("<table class=\"sas\">\n<thead>\n<tr>");
        for (i, h) in headers.iter().enumerate() {
            let align_attr = match aligns.get(i).copied().unwrap_or(Align::Left) {
                Align::Right => " style=\"text-align:right\"",
                Align::Left => "",
            };
            self.buf.push_str(&format!(
                "<th{attr}>{text}</th>",
                attr = align_attr,
                text = Self::html_escape(h)
            ));
        }
        self.buf.push_str("</tr>\n</thead>\n<tbody>\n");
        for row in rows {
            self.buf.push_str("<tr>");
            for (i, cell) in row.iter().enumerate() {
                let align_attr = match aligns.get(i).copied().unwrap_or(Align::Left) {
                    Align::Right => " style=\"text-align:right\"",
                    Align::Left => "",
                };
                self.buf.push_str(&format!(
                    "<td{attr}>{text}</td>",
                    attr = align_attr,
                    text = Self::html_escape(cell)
                ));
            }
            self.buf.push_str("</tr>\n");
        }
        self.buf.push_str("</tbody>\n</table>\n");
        self.wrote_anything = true;
    }

    fn write_line(&mut self, line: &str) {
        self.buf
            .push_str(&format!("<p>{}</p>\n", Self::html_escape(line)));
        self.wrote_anything = true;
    }

    fn blank(&mut self) {
        // no-op : les paragraphes HTML séparent naturellement le contenu.
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

    /// Draine la sortie accumulée sous forme de document HTML complet.
    ///
    /// Si `buf` est vide (rien n'a été écrit), renvoie une chaîne vide
    /// (comportement idempotent identique à `TextListing::into_string`).
    /// Après cet appel `buf` est vide : un second appel renvoie `""`.
    fn into_string(&mut self) -> String {
        if self.buf.is_empty() {
            return String::new();
        }
        // Footnotes actives rendues en bas du document.
        for f in &self.footnotes {
            self.buf.push_str(&format!(
                "<p class=\"sysfootnote\">{}</p>\n",
                Self::html_escape(f)
            ));
        }
        let body = std::mem::take(&mut self.buf);
        self.wrote_anything = false;
        format!(
            "<!DOCTYPE html>\n\
             <html>\n\
             <head>\n\
             <meta charset=\"utf-8\">\n\
             <style>\
table.sas{{border-collapse:collapse;}} \
table.sas th,table.sas td{{border:1px solid #888;padding:4px;}}\
</style>\n\
             </head>\n\
             <body>\n\
             {body}\
             </body>\n\
             </html>\n"
        )
    }

    /// Finalise la destination : si un fichier cible a été configuré, renvoie
    /// `Some((path, html_complet))` pour que l'appelant l'écrive sur disque.
    /// Sinon renvoie `None`.
    fn finalize(&mut self) -> Option<(std::path::PathBuf, String)> {
        let path = self.file.clone()?;
        let html = self.into_string();
        if html.is_empty() {
            // Rien à écrire (destination ouverte mais inutilisée).
            None
        } else {
            Some((path, html))
        }
    }
}

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
        RtfDestination { buf: String::new(), titles: Vec::new(), footnotes: Vec::new(), ls, file: None }
    }

    /// Crée la destination RTF avec un fichier cible.
    pub fn with_file(ls: usize, file: std::path::PathBuf) -> Self {
        RtfDestination { buf: String::new(), titles: Vec::new(), footnotes: Vec::new(), ls, file: Some(file) }
    }

    /// Échappe les caractères spéciaux RTF.
    fn rtf_escape(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 8);
        for c in s.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '{' => out.push_str("\\{"),
                '}' => out.push_str("\\}"),
                c if c.is_ascii() => out.push(c),
                c if (c as u32) <= 0xFF => {
                    out.push_str(&format!("\\'{}",
                        format!("{:02x}", c as u32)));
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
        let col_widths: Vec<usize> = (0..headers.len()).map(|i| {
            let header_len = headers.get(i).map(|s| s.len()).unwrap_or(0);
            let max_data_len = rows.iter()
                .map(|r| r.get(i).map(|s| s.len()).unwrap_or(0))
                .max()
                .unwrap_or(0);
            (header_len.max(max_data_len) * 120).max(720)
        }).collect();

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
                let align_ctrl = match align { Align::Right => "\\qr", Align::Left => "\\ql" };
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
        self.buf.push_str(&format!("\\pard {}\\par\n", Self::rtf_escape(line)));
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

    fn into_string(&mut self) -> String {
        if self.buf.is_empty() {
            return String::new();
        }
        // Footnotes actives rendues (centrées) en fin de document.
        for f in &self.footnotes {
            self.buf.push_str(&format!(
                "\\pard\\qc {}\\par\n",
                Self::rtf_escape(f)
            ));
        }
        let body = std::mem::take(&mut self.buf);
        format!(
            "{{\\rtf1\\ansi\\ansicpg1252\\deff0\n{{\\fonttbl{{\\f0\\froman\\fcharset0 Times New Roman;}}}}\n\\f0\\fs24\n{body}}}"
        )
    }

    fn finalize(&mut self) -> Option<(std::path::PathBuf, String)> {
        let path = self.file.clone()?;
        let content = self.into_string();
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

// ---------------------------------------------------------------------------
// ExcelDestination — M23.3 : destination Excel réelle (rust_xlsxwriter)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ExcelTable {
    sheet_name: String,
    pre_lines: Vec<String>,
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
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
            titles: Vec::new(), footnotes: Vec::new(), ls, file: None,
            tables: Vec::new(), pending_lines: Vec::new(),
        }
    }

    /// Crée la destination Excel avec un fichier cible.
    pub fn with_file(ls: usize, file: std::path::PathBuf) -> Self {
        ExcelDestination {
            titles: Vec::new(), footnotes: Vec::new(), ls, file: Some(file),
            tables: Vec::new(), pending_lines: Vec::new(),
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

    fn into_string(&mut self) -> String {
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
fn xlsx_col_ref(mut n: usize) -> String {
    let mut s = String::new();
    loop {
        s.push(char::from(b'A' + (n % 26) as u8));
        if n < 26 { break; }
        n = n / 26 - 1;
    }
    s.chars().rev().collect()
}

/// Échappe un contenu pour l'insérer dans un attribut ou texte XML.
fn xlsx_xml_escape(v: &str) -> String {
    v.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
}

/// Génère le XML d'une feuille (`xl/worksheets/sheetN.xml`).
fn xlsx_sheet_xml(pre_lines: &[String], headers: &[String], rows: &[Vec<String>]) -> Vec<u8> {
    let mut x = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
        <sheetData>"
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
                xlsx_col_ref(c), xlsx_xml_escape(h)
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
                xlsx_col_ref(c), xlsx_xml_escape(v)
            ));
        }
        x.push_str("</row>");
        r += 1;
    }
    x.push_str("</sheetData></worksheet>");
    x.into_bytes()
}

/// CRC-32 variante ZIP (polynôme 0xEDB88320).
fn crc32_zip(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        let idx = ((crc ^ b as u32) & 0xFF) as usize;
        // Calcule le coefficient à la volée pour éviter une table statique globale.
        let mut coeff = idx as u32;
        for _ in 0..8 {
            coeff = if coeff & 1 != 0 { 0xEDB88320 ^ (coeff >> 1) } else { coeff >> 1 };
        }
        crc = coeff ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

fn zip_u16(buf: &mut Vec<u8>, v: u16) { buf.extend_from_slice(&v.to_le_bytes()); }
fn zip_u32(buf: &mut Vec<u8>, v: u32) { buf.extend_from_slice(&v.to_le_bytes()); }

/// Construit un ZIP sans compression (store) à partir de paires (nom, octets).
fn build_zip_stored(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
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
        zip_u16(&mut out, 20);         // version needed
        zip_u16(&mut out, 0);          // flags
        zip_u16(&mut out, 0);          // compression = store
        zip_u16(&mut out, 0);          // mod time
        zip_u16(&mut out, 0);          // mod date
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
        zip_u16(&mut out, 20);         // version made by
        zip_u16(&mut out, 20);         // version needed
        zip_u16(&mut out, 0);
        zip_u16(&mut out, 0);          // compression
        zip_u16(&mut out, 0);
        zip_u16(&mut out, 0);
        zip_u32(&mut out, crcs[i]);
        zip_u32(&mut out, data.len() as u32);
        zip_u32(&mut out, data.len() as u32);
        zip_u16(&mut out, nb.len() as u16);
        zip_u16(&mut out, 0);  // extra length
        zip_u16(&mut out, 0);  // comment length
        zip_u16(&mut out, 0);  // disk start
        zip_u16(&mut out, 0);  // internal attrs
        zip_u32(&mut out, 0);  // external attrs
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
fn xlsx_build(tables: &[ExcelTable], pending_lines: &[String]) -> Vec<u8> {
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
        <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>"
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
        xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets>"
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
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">"
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
    let sheet_names: Vec<String> = (1..=n).map(|i| format!("xl/worksheets/sheet{i}.xml")).collect();
    for (i, (_, xml_bytes)) in sheets.into_iter().enumerate() {
        zip_entries.push((sheet_names[i].as_str(), xml_bytes));
    }

    build_zip_stored(&zip_entries)
}

// ---------------------------------------------------------------------------
// PdfDestination — M23.2 : destination PDF pure Rust (PDF 1.4 minimal)
// ---------------------------------------------------------------------------

enum PdfSection {
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

    fn pdf_escape(s: &str) -> String {
        s.chars().map(|c| match c {
            '(' => "\\(".to_string(),
            ')' => "\\)".to_string(),
            '\\' => "\\\\".to_string(),
            c if c.is_ascii() && c >= ' ' => c.to_string(),
            _ => "?".to_string(),
        }).collect()
    }

    fn build_pdf_content(&self) -> String {
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

    fn build_pdf_document(content: String) -> Vec<u8> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_listing_is_text_listing() {
        // Le listing par défaut d'une session est une `TextListing` derrière le
        // trait object. On vérifie qu'on peut écrire au travers du trait.
        let tmp = std::env::temp_dir();
        let session = crate::session::Session::new(None, tmp, true).expect("session");
        let dest: &dyn OutputDestination = session.listing.as_ref();
        // LINESIZE par défaut = 96 (SasOptions::default()).
        assert_eq!(dest.ls(), 96);
    }

    #[test]
    fn write_line_renders_text() {
        let mut d = TextListing::new(96);
        d.write_line("test");
        let out = d.into_string();
        assert_eq!(out, "test\n");
    }

    #[test]
    fn page_header_default_title_centered() {
        let mut d = TextListing::new(40);
        d.page_header();
        let out = d.into_string();
        assert!(out.contains("The SAS System"), "out: {out:?}");
        // Centré dans LS=40 : padding gauche de (40-14)/2 = 13 espaces.
        assert!(out.starts_with("             The SAS System"), "out: {out:?}");
    }

    #[test]
    fn set_title_overrides_default() {
        let mut d = TextListing::new(40);
        d.set_title(Some("Mon Titre".to_string()));
        d.page_header();
        let out = d.into_string();
        assert!(out.contains("Mon Titre"), "out: {out:?}");
        assert!(!out.contains("The SAS System"), "out: {out:?}");
    }

    #[test]
    fn write_table_matches_listing_writer() {
        // Le rendu de table passe verbatim par ListingWriter ⇒ octet-identique.
        let mut d = TextListing::new(40);
        d.page_header();
        d.write_table(
            &["Obs".into(), "x".into()],
            &[Align::Right, Align::Right],
            &[
                vec!["1".into(), "10".into()],
                vec!["2".into(), "200".into()],
            ],
        );
        let via_trait = d.into_string();

        let mut l = ListingWriter::new(40);
        l.page_header();
        l.write_table(
            &["Obs".into(), "x".into()],
            &[Align::Right, Align::Right],
            &[
                vec!["1".into(), "10".into()],
                vec!["2".into(), "200".into()],
            ],
        );
        let direct = l.into_string();

        assert_eq!(via_trait, direct);
    }

    #[test]
    fn blank_emits_empty_line() {
        let mut d = TextListing::new(40);
        d.write_line("a");
        d.blank();
        d.write_line("b");
        assert_eq!(d.into_string(), "a\n\nb\n");
    }

    #[test]
    fn into_string_leaves_destination_empty() {
        // Deux drains successifs ne dupliquent pas le contenu.
        let mut d = TextListing::new(40);
        d.write_line("once");
        assert_eq!(d.into_string(), "once\n");
        assert_eq!(d.into_string(), "");
    }

    // html_stub_is_noop retiré : HtmlDestination est désormais une vraie
    // destination (M22.4), remplacé par les tests html_* ci-dessous.

    #[test]
    fn stub_destinations_implement_trait() {
        // Les destinations stub RTF/PDF/Excel sont utilisables comme trait objects.
        let dests: Vec<Box<dyn OutputDestination>> = vec![
            Box::new(RtfDestination::new(80)),
            Box::new(PdfDestination::new(80)),
            Box::new(ExcelDestination::new(80)),
        ];
        for d in dests {
            assert_eq!(d.ls(), 80);
        }
    }

    // --- Tests M22.4 : HtmlDestination réelle ---

    #[test]
    fn html_table_renders_escaped_cells() {
        let mut h = HtmlDestination::new(96);
        h.write_table(
            &["Name".into(), "Value <x>".into()],
            &[Align::Left, Align::Right],
            &[
                vec!["a & b".into(), "42".into()],
                vec!["<tag>".into(), "99".into()],
            ],
        );
        let out = h.into_string();
        // Présence de la structure de table.
        assert!(out.contains("<table"), "pas de <table : {out}");
        assert!(out.contains("</table>"), "pas de </table> : {out}");
        // Échappement dans en-tête.
        assert!(out.contains("Value &lt;x&gt;"), "échappement header raté : {out}");
        // Échappement dans cellule.
        assert!(out.contains("a &amp; b"), "échappement & raté : {out}");
        assert!(out.contains("&lt;tag&gt;"), "échappement < raté : {out}");
        // Alignement droite sur la 2ᵉ colonne.
        assert!(out.contains("text-align:right"), "alignement right manquant : {out}");
    }

    #[test]
    fn html_into_string_wraps_document() {
        let mut h = HtmlDestination::new(96);
        h.write_line("hello");
        let out = h.into_string();
        // Structure HTML obligatoire.
        assert!(out.contains("<!DOCTYPE html>"), "DOCTYPE manquant : {out}");
        assert!(out.contains("<style"), "style manquant : {out}");
        assert!(out.contains("<body>"), "<body> manquant : {out}");
        assert!(out.contains("</body>"), "</body> manquant : {out}");
        assert!(out.contains("<p>hello</p>"), "<p> manquant : {out}");
        // Second drain → chaîne vide (idempotent).
        assert_eq!(h.into_string(), "", "second drain non vide");
    }

    #[test]
    fn html_without_file_finalize_none() {
        let mut h = HtmlDestination::new(96);
        h.write_line("test");
        // Sans fichier cible, finalize() renvoie None.
        assert!(h.finalize().is_none());
    }

    #[test]
    fn html_empty_into_string_is_empty() {
        // Rien d'écrit → into_string() renvoie "" (pas de document vide).
        let mut h = HtmlDestination::new(96);
        assert_eq!(h.into_string(), "");
    }

    #[test]
    fn html_page_header_uses_title() {
        let mut h = HtmlDestination::new(96);
        h.set_title(Some("Mon Rapport".to_string()));
        h.page_header();
        let out = h.into_string();
        assert!(out.contains("Mon Rapport"), "titre absent : {out}");
        assert!(out.contains("class=\"systitle\""), "classe systitle absente : {out}");
    }

    #[test]
    fn html_page_header_default_title() {
        let mut h = HtmlDestination::new(96);
        h.page_header();
        let out = h.into_string();
        assert!(out.contains("The SAS System"), "titre par défaut absent : {out}");
    }

    #[test]
    fn html_ls_accessor() {
        let h = HtmlDestination::new(80);
        assert_eq!(h.ls(), 80);
    }

    #[test]
    fn html_with_file_finalize_some() {
        let tmp = std::env::temp_dir().join("test_html_finalize.html");
        let mut h = HtmlDestination::with_file(96, tmp.clone());
        h.write_line("content");
        let result = h.finalize();
        assert!(result.is_some(), "finalize devrait renvoyer Some");
        let (path, html) = result.unwrap();
        assert_eq!(path, tmp);
        assert!(html.contains("<!DOCTYPE html>"), "HTML complet attendu");
        assert!(html.contains("<p>content</p>"), "contenu attendu");
        // Après finalize, buf est vide.
        assert_eq!(h.into_string(), "", "buf doit être vide après finalize");
    }

    // --- Tests M23.1 : RtfDestination réelle ---

    #[test]
    fn rtf_table_renders_structure() {
        let mut r = RtfDestination::new(96);
        r.write_table(
            &["Name".into(), "Age".into()],
            &[Align::Left, Align::Right],
            &[vec!["Alfred".into(), "14".into()]],
        );
        let out = r.into_string();
        assert!(out.starts_with("{\\rtf1"), "RTF header manquant: {out}");
        assert!(out.contains("\\trowd"), "table RTF manquante: {out}");
        assert!(out.contains("Alfred"), "valeur manquante: {out}");
        assert!(out.contains("\\qr"), "alignement right manquant: {out}");
        assert!(out.contains("14"), "age manquant: {out}");
    }

    #[test]
    fn rtf_escape_special_chars() {
        let mut r = RtfDestination::new(96);
        r.write_line("a\\b{c}d");
        let out = r.into_string();
        assert!(out.contains("a\\\\b\\{c\\}d"), "RTF escape rate: {out}");
    }

    #[test]
    fn rtf_without_file_finalize_none() {
        let mut r = RtfDestination::new(96);
        r.write_line("test");
        assert!(r.finalize().is_none());
    }

    #[test]
    fn rtf_with_file_finalize_some() {
        let tmp = std::env::temp_dir().join("test_ods.rtf");
        let mut r = RtfDestination::with_file(96, tmp.clone());
        r.write_line("hello");
        let result = r.finalize();
        assert!(result.is_some());
        let (path, content) = result.unwrap();
        assert_eq!(path, tmp);
        assert!(content.starts_with("{\\rtf1"), "RTF content: {content}");
    }

    // --- Tests M23.3 : ExcelDestination réelle ---

    #[test]
    fn excel_without_file_finalize_to_bytes_none() {
        let mut e = ExcelDestination::new(96);
        e.write_table(
            &["x".into()],
            &[Align::Right],
            &[vec!["1".into()]],
        );
        assert!(e.finalize_to_bytes().is_none());
    }

    #[test]
    fn excel_with_file_finalize_to_bytes_some() {
        let tmp = std::env::temp_dir().join("test_ods.xlsx");
        let mut e = ExcelDestination::with_file(96, tmp.clone());
        e.write_table(
            &["Name".into(), "Age".into()],
            &[Align::Left, Align::Right],
            &[vec!["Alfred".into(), "14".into()]],
        );
        let result = e.finalize_to_bytes();
        assert!(result.is_some(), "finalize_to_bytes devrait retourner Some");
        let (path, bytes) = result.unwrap();
        assert_eq!(path, tmp);
        // Les fichiers XLSX commencent par PK (ZIP magic bytes)
        assert!(bytes.starts_with(b"PK"), "XLSX doit commencer par PK: {:?}", &bytes[..4]);
    }

    // --- Tests M23.2 : PdfDestination réelle ---

    #[test]
    fn pdf_without_file_finalize_to_bytes_none() {
        let mut p = PdfDestination::new(96);
        p.write_line("test");
        assert!(p.finalize_to_bytes().is_none());
    }

    #[test]
    fn pdf_with_file_finalize_to_bytes_some() {
        let tmp = std::env::temp_dir().join("test_ods.pdf");
        let mut p = PdfDestination::with_file(96, tmp.clone());
        p.write_line("The SAS System");
        p.write_table(
            &["Name".into(), "Age".into()],
            &[Align::Left, Align::Right],
            &[vec!["Alfred".into(), "14".into()]],
        );
        let result = p.finalize_to_bytes();
        assert!(result.is_some(), "finalize_to_bytes devrait retourner Some");
        let (path, bytes) = result.unwrap();
        assert_eq!(path, tmp);
        assert!(bytes.starts_with(b"%PDF-"), "PDF magic bytes: {:?}", &bytes[..5]);
        let _ = std::fs::remove_file(&tmp);
    }

    // ── M38.1 : titres/footnotes multi-niveaux par destination ────────────────

    #[test]
    fn html_renders_multiple_titles_and_footnotes() {
        let mut h = HtmlDestination::new(96);
        h.set_titles(&["T1".to_string(), "T2".to_string()]);
        h.set_footnotes(&["F1".to_string()]);
        h.page_header();
        h.write_line("body");
        let out = h.into_string();
        let p1 = out.find("T1").unwrap();
        let p2 = out.find("T2").unwrap();
        assert!(p1 < p2, "titres dans l'ordre des niveaux");
        assert!(out.contains("sysfootnote"), "classe footnote absente : {out}");
        assert!(out.find("F1").unwrap() > out.find("body").unwrap(), "footnote après le corps");
    }

    #[test]
    fn rtf_renders_multiple_titles_and_footnotes() {
        let mut r = RtfDestination::new(96);
        r.set_titles(&["T1".to_string(), "T2".to_string()]);
        r.set_footnotes(&["F1".to_string()]);
        r.page_header();
        r.write_line("body");
        let out = r.into_string();
        assert!(out.find("T1").unwrap() < out.find("T2").unwrap());
        assert!(out.find("F1").unwrap() > out.find("body").unwrap());
    }

    #[test]
    fn pdf_renders_titles_and_footnotes_idempotent() {
        let tmp = std::env::temp_dir().join("test_ods_tf.pdf");
        let mut p = PdfDestination::with_file(96, tmp.clone());
        p.set_titles(&["T1".to_string(), "T2".to_string()]);
        p.set_footnotes(&["F1".to_string()]);
        p.page_header();
        p.write_line("body");
        let (_, bytes1) = p.finalize_to_bytes().unwrap();
        let s1 = String::from_utf8_lossy(&bytes1);
        assert!(s1.contains("(T1)") && s1.contains("(T2)") && s1.contains("(F1)"));
        // Idempotence : un second finalize produit le même contenu (pas de
        // duplication de footnotes dans self.sections).
        let (_, bytes2) = p.finalize_to_bytes().unwrap();
        assert_eq!(bytes1.len(), bytes2.len(), "finalize doit être idempotent");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn excel_renders_titles_and_footnotes_as_rows() {
        let tmp = std::env::temp_dir().join("test_ods_tf.xlsx");
        let mut e = ExcelDestination::with_file(96, tmp.clone());
        e.set_titles(&["Top Title".to_string()]);
        e.set_footnotes(&["Bottom Note".to_string()]);
        e.write_table(
            &["Name".into()],
            &[Align::Left],
            &[vec!["Alice".into()]],
        );
        let (_, bytes) = e.finalize_to_bytes().unwrap();
        // XLSX = ZIP : la chaîne partagée contient titres et footnotes.
        let blob = String::from_utf8_lossy(&bytes);
        assert!(blob.contains("Top Title"), "titre absent du XLSX");
        assert!(blob.contains("Bottom Note"), "footnote absente du XLSX");
        let _ = std::fs::remove_file(&tmp);
    }
}
