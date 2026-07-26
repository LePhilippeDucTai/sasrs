use super::*;

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
    pub(super) fn html_escape(s: &str) -> String {
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
    fn take_string(&mut self) -> String {
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
        let html = self.take_string();
        if html.is_empty() {
            // Rien à écrire (destination ouverte mais inutilisée).
            None
        } else {
            Some((path, html))
        }
    }
}
